use crate::embed::chunking::count_tokens;
use crate::ingest::layout::{analyze_layout, RawLayoutBlock};
use crate::ingest::markdown::{assemble_markdown_blocks, join_ingest_blocks, IngestBlock};
use crate::memory_agent::parser::{CandidateAction, CandidateNode};
use crate::ocr::bundled::BundledOcrEngine;
use crate::ocr::engine::{OcrEngine, OcrError, Rect};
use crate::ocr::pdf::{PdfPageType, PdfRasterizer, PdfRasterizerConfig};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

/// Which extraction pipeline a page should be routed through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionPath {
    /// Digital text extraction via pdfium character-level API.
    Digital,
    /// OCR via BundledOcrEngine (rasterize → detect → recognize).
    Ocr,
}

/// Determines the extraction path for a given page classification.
pub fn extraction_path_for_page(page_type: PdfPageType) -> ExtractionPath {
    match page_type {
        PdfPageType::Digital => ExtractionPath::Digital,
        PdfPageType::Ocr | PdfPageType::Hybrid => ExtractionPath::Ocr,
    }
}

/// Derives the document-level extraction path label stored on `import_jobs.extraction_path`.
pub fn derive_document_extraction_path(digital: usize, ocr: usize, hybrid: usize) -> &'static str {
    if hybrid > 0 || (digital > 0 && ocr > 0) {
        "hybrid"
    } else if ocr > 0 {
        "ocr-bundled"
    } else {
        "digital"
    }
}

pub struct ImportJobHandle {
    pub job_id: String,
    pub cancel: Arc<AtomicBool>,
}

/// Configuration options for an import job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestJobConfig {
    /// Rasterization resolution for scanned PDF pages (default: 300).
    pub rasterization_dpi: u16,
    /// Target maximum token length for each generated import chunk (default: 350).
    pub target_chunk_tokens: usize,
    /// Token overlap between consecutive import chunks (default: 60).
    pub overlap_chunk_tokens: usize,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub allowed_vault_keys: Option<Vec<String>>,
}

impl Default for IngestJobConfig {
    fn default() -> Self {
        Self {
            rasterization_dpi: 300,
            target_chunk_tokens: 350,
            overlap_chunk_tokens: 60,
            provider: None,
            endpoint: None,
            model: None,
            allowed_vault_keys: None,
        }
    }
}

/// Granular status metrics reported during job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportJobProgress {
    pub job_id: String,
    pub current_page: usize,
    pub total_pages: usize,
    pub digital_pages: usize,
    pub ocr_pages: usize,
    pub hybrid_pages: usize,
    pub avg_ocr_confidence: f32,
    pub status: String,
}

/// A chunk of extracted import text prepared for LLM processing / embedding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportChunkSpec {
    pub chunk_index: usize,
    pub text: String,
    pub token_count: usize,
    pub heading_context: Option<String>,
    pub chunk_type: String,
    pub ocr_confidence: Option<f32>,
    pub tables_unstructured: bool,
}

/// Final result payload from a completed document ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestJobResult {
    pub job_id: String,
    pub source_name: String,
    pub total_pages: usize,
    pub digital_pages: usize,
    pub ocr_pages: usize,
    pub hybrid_pages: usize,
    pub assembled_markdown: String,
    pub chunks: Vec<ImportChunkSpec>,
    pub avg_ocr_confidence: f32,
    pub tables_detected_unpreserved: i32,
    pub candidates: Vec<CandidateNode>,
}

/// Core background engine managing job execution, lazy ONNX initialization, and chunking.
pub struct IngestJobEngine;

impl IngestJobEngine {
    /// Executes an ingestion job on a PDF document page-by-page.
    /// Lazily instantiates `BundledOcrEngine` if an `Ocr` or `Hybrid` page is encountered,
    /// caches it for the job duration, and drops it when finished.
    pub fn process_pdf_job(
        job_id: impl Into<String>,
        file_path: &Path,
        config: IngestJobConfig,
        progress_tx: Option<mpsc::Sender<ImportJobProgress>>,
        cancel: Option<&AtomicBool>,
    ) -> Result<IngestJobResult, OcrError> {
        let job_id = job_id.into();
        let source_name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("document.pdf")
            .to_string();

        let mut file = File::open(file_path)
            .map_err(|e| OcrError::IoError(format!("Failed opening PDF file: {e}")))?;
        let mut pdf_bytes = Vec::new();
        file.read_to_end(&mut pdf_bytes)
            .map_err(|e| OcrError::IoError(format!("Failed reading PDF file: {e}")))?;

        let rasterizer = PdfRasterizer::new()?;
        let pages_info = rasterizer.scan_document(&pdf_bytes)?;
        let total_pages = pages_info.len();

        let mut digital_pages = 0;
        let mut ocr_pages = 0;
        let mut hybrid_pages = 0;

        for p in &pages_info {
            match p.page_type {
                PdfPageType::Digital => digital_pages += 1,
                PdfPageType::Ocr => ocr_pages += 1,
                PdfPageType::Hybrid => hybrid_pages += 1,
            }
        }

        let rasterizer_config = PdfRasterizerConfig {
            dpi: config.rasterization_dpi,
        };

        let mut cached_ocr_engine: Option<BundledOcrEngine> = None;
        let mut all_ingest_blocks = Vec::new();
        let mut total_ocr_confidence_sum = 0.0f32;
        let mut ocr_pass_count = 0usize;
        let mut tables_detected_unpreserved = 0;

        for (i, p) in pages_info.iter().enumerate() {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(OcrError::Cancelled);
            }

            let current_page = i + 1;

            if let Some(ref tx) = progress_tx {
                let current_avg = if ocr_pass_count > 0 {
                    total_ocr_confidence_sum / (ocr_pass_count as f32)
                } else {
                    1.0
                };
                let _ = tx.send(ImportJobProgress {
                    job_id: job_id.clone(),
                    current_page,
                    total_pages,
                    digital_pages,
                    ocr_pages,
                    hybrid_pages,
                    avg_ocr_confidence: current_avg,
                    status: "extracting".to_string(),
                });
            }

            let layout_blocks = match p.page_type {
                PdfPageType::Digital => {
                    let raw_blocks = rasterizer.extract_digital_blocks(&pdf_bytes, i)?;
                    analyze_layout(raw_blocks, p.width_pts, p.height_pts)
                }
                PdfPageType::Ocr => {
                    let (raw_blocks, image_width, image_height, confidence) =
                        Self::recognize_ocr_page(
                            &rasterizer,
                            &pdf_bytes,
                            i,
                            &rasterizer_config,
                            &mut cached_ocr_engine,
                        )?;
                    total_ocr_confidence_sum += confidence;
                    ocr_pass_count += 1;

                    analyze_layout(raw_blocks, image_width, image_height)
                }
                PdfPageType::Hybrid => {
                    let (ocr_blocks, image_width, image_height, confidence) =
                        Self::recognize_ocr_page(
                            &rasterizer,
                            &pdf_bytes,
                            i,
                            &rasterizer_config,
                            &mut cached_ocr_engine,
                        )?;
                    total_ocr_confidence_sum += confidence;
                    ocr_pass_count += 1;

                    let digital_blocks = rasterizer.extract_digital_blocks(&pdf_bytes, i)?;
                    let scaled_digital_blocks = scale_blocks_to_page(
                        digital_blocks,
                        image_width / p.width_pts.max(1.0),
                        image_height / p.height_pts.max(1.0),
                    );
                    let raw_blocks = merge_hybrid_raw_blocks(scaled_digital_blocks, ocr_blocks);

                    analyze_layout(raw_blocks, image_width, image_height)
                }
            };

            // Count table blocks across all page types
            for block in &layout_blocks {
                if block.block_type == crate::ingest::layout::BlockType::Table {
                    tables_detected_unpreserved += 1;
                }
            }

            let page_ingest_blocks = assemble_markdown_blocks(&layout_blocks, i);
            all_ingest_blocks.extend(page_ingest_blocks);
        }

        let assembled_markdown = join_ingest_blocks(&all_ingest_blocks);
        let avg_ocr_confidence = if ocr_pass_count > 0 {
            (total_ocr_confidence_sum / (ocr_pass_count as f32)).clamp(0.0, 1.0)
        } else {
            1.0
        };

        let chunks = chunk_ingest_blocks(
            &all_ingest_blocks,
            config.target_chunk_tokens,
            config.overlap_chunk_tokens,
        );

        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ImportJobProgress {
                job_id: job_id.clone(),
                current_page: total_pages,
                total_pages,
                digital_pages,
                ocr_pages,
                hybrid_pages,
                avg_ocr_confidence,
                status: "staged".to_string(),
            });
        }

        let (runtime_handle, _owned_runtime) = prepare_job_runtime()?;

        let mut candidates = Vec::new();
        for chunk in &chunks {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(OcrError::Cancelled);
            }
            candidates.extend(IngestJobEngine::extract_chunk_candidates(
                chunk,
                &source_name,
                &config,
                &runtime_handle,
            ));
        }

        Ok(IngestJobResult {
            job_id,
            source_name,
            total_pages,
            digital_pages,
            ocr_pages,
            hybrid_pages,
            assembled_markdown,
            chunks,
            avg_ocr_confidence,
            tables_detected_unpreserved,
            candidates,
        })
    }

    /// Executes an ingestion job on a single image file (PNG / JPG / WEBP).
    pub fn process_image_job(
        job_id: impl Into<String>,
        file_path: &Path,
        config: IngestJobConfig,
    ) -> Result<IngestJobResult, OcrError> {
        let job_id = job_id.into();
        let source_name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("image.png")
            .to_string();

        let img = image::open(file_path)
            .map_err(|e| OcrError::IoError(format!("Failed opening image file: {e}")))?;

        let ocr_engine = BundledOcrEngine::new()?;
        let ocr_output = ocr_engine.recognize(&img)?;

        let (width_pts, height_pts) = (img.width() as f32, img.height() as f32);
        let raw_blocks: Vec<RawLayoutBlock> = ocr_output
            .blocks
            .into_iter()
            .map(|b| RawLayoutBlock::new(b.text, b.bbox).with_confidence(b.confidence))
            .collect();

        let layout_blocks = analyze_layout(raw_blocks, width_pts, height_pts);

        let mut tables_detected_unpreserved = 0;
        for block in &layout_blocks {
            if block.block_type == crate::ingest::layout::BlockType::Table {
                tables_detected_unpreserved += 1;
            }
        }

        let ingest_blocks = assemble_markdown_blocks(&layout_blocks, 0);
        let assembled_markdown = join_ingest_blocks(&ingest_blocks);

        let chunks = chunk_ingest_blocks(
            &ingest_blocks,
            config.target_chunk_tokens,
            config.overlap_chunk_tokens,
        );

        let (runtime_handle, _owned_runtime) = prepare_job_runtime()?;

        let mut candidates = Vec::new();
        for chunk in &chunks {
            candidates.extend(IngestJobEngine::extract_chunk_candidates(
                chunk,
                &source_name,
                &config,
                &runtime_handle,
            ));
        }

        Ok(IngestJobResult {
            job_id,
            source_name,
            total_pages: 1,
            digital_pages: 0,
            ocr_pages: 1,
            hybrid_pages: 0,
            assembled_markdown,
            chunks,
            avg_ocr_confidence: ocr_output.avg_confidence,
            tables_detected_unpreserved,
            candidates,
        })
    }

    fn recognize_ocr_page(
        rasterizer: &PdfRasterizer,
        pdf_bytes: &[u8],
        page_index: usize,
        rasterizer_config: &PdfRasterizerConfig,
        cached_ocr_engine: &mut Option<BundledOcrEngine>,
    ) -> Result<(Vec<RawLayoutBlock>, f32, f32, f32), OcrError> {
        if cached_ocr_engine.is_none() {
            *cached_ocr_engine = Some(BundledOcrEngine::new()?);
        }
        let ocr_engine = cached_ocr_engine.as_mut().ok_or_else(|| {
            OcrError::InferenceFailed("Failed initializing OCR engine session".to_string())
        })?;

        let page_img = rasterizer.render_page(pdf_bytes, page_index, rasterizer_config)?;
        let (image_width, image_height) = (page_img.width() as f32, page_img.height() as f32);
        let ocr_output = ocr_engine.recognize(&page_img)?;
        let confidence = ocr_output.avg_confidence;
        let raw_blocks = ocr_output
            .blocks
            .into_iter()
            .map(|b| RawLayoutBlock::new(b.text, b.bbox).with_confidence(b.confidence))
            .collect();

        Ok((raw_blocks, image_width, image_height, confidence))
    }

    fn run_fallback_extraction(
        chunk: &ImportChunkSpec,
        source_name: &str,
        reason: Option<&str>,
        target_vault_key: Option<String>,
    ) -> CandidateNode {
        let title = chunk
            .heading_context
            .clone()
            .unwrap_or_else(|| format!("Imported Chunk {}", chunk.chunk_index));
        let detail = Some(chunk.text.clone());
        let mut summary = String::new();
        const MIN_SUMMARY_LEN: usize = 20;
        let segments: Vec<&str> = chunk
            .text
            .split(&['.', '?', '!'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                summary.push_str(". ");
            }
            summary.push_str(segment);
            if summary.chars().count() >= MIN_SUMMARY_LEN {
                break;
            }
        }
        if summary.is_empty() {
            summary = chunk.text.chars().take(120).collect();
        } else if summary.chars().count() > 120 {
            summary = summary.chars().take(120).collect::<String>() + "...";
        }

        let mut meta_obj = serde_json::json!({
            "ocr_confidence": chunk.ocr_confidence,
            "tables_unstructured": chunk.tables_unstructured,
        });
        if let Some(r) = reason {
            if let Some(map) = meta_obj.as_object_mut() {
                map.insert("fallback_reason".to_string(), serde_json::json!(r));
            }
        }

        CandidateNode {
            title,
            summary,
            detail,
            node_type: Some("fact".to_string()),
            target_vault_key,
            tags: Some(vec!["pdf_import".to_string()]),
            confidence: 0.5,
            action: CandidateAction::Add,
            source: Some(source_name.to_string()),
            source_type: Some("pdf_import".to_string()),
            meta: Some(meta_obj),
        }
    }

    fn extract_chunk_candidates(
        chunk: &ImportChunkSpec,
        source_name: &str,
        config: &IngestJobConfig,
        runtime: &tokio::runtime::Handle,
    ) -> Vec<CandidateNode> {
        let resolved_vault = config
            .allowed_vault_keys
            .as_ref()
            .and_then(|keys| keys.first())
            .cloned()
            .or_else(|| Some("learning".to_string()));

        if crate::ingest::security::scan_prompt_injection(&chunk.text) {
            return vec![Self::run_fallback_extraction(
                chunk,
                source_name,
                Some("injection_flagged"),
                resolved_vault,
            )];
        }

        let (provider, endpoint, model) = match (&config.provider, &config.model) {
            (Some(p), Some(m)) => (p, config.endpoint.as_deref().unwrap_or(""), m),
            _ => {
                return vec![Self::run_fallback_extraction(
                    chunk,
                    source_name,
                    Some("no_llm_configured"),
                    resolved_vault,
                )]
            }
        };

        let parsed_provider = match provider.trim().to_lowercase().as_str() {
            "ollama" => crate::llm::client::LlmProvider::Ollama,
            "lmstudio" => crate::llm::client::LlmProvider::LmStudio,
            "anthropic" => crate::llm::client::LlmProvider::Anthropic,
            "openai" => crate::llm::client::LlmProvider::OpenAi,
            "google" => crate::llm::client::LlmProvider::Google,
            "xai" => crate::llm::client::LlmProvider::XAi,
            _ => {
                eprintln!(
                    "[ingest] Unsupported LLM provider '{}', falling back to layout extraction.",
                    provider
                );
                return vec![Self::run_fallback_extraction(
                    chunk,
                    source_name,
                    Some("unsupported_provider"),
                    resolved_vault.clone(),
                )];
            }
        };

        let user_content = crate::ingest::prompt::wrap_ingestion_payload(&chunk.text);
        let messages = [crate::llm::client::LlmMessage {
            role: "user".to_string(),
            content: user_content,
        }];

        let client = crate::llm::client::UniversalClient::new(
            parsed_provider,
            endpoint.trim().to_string(),
            model.trim().to_string(),
        );

        let allowed_keys = config.allowed_vault_keys.clone().unwrap_or_default();
        let sys_prompt = crate::ingest::prompt::get_system_prompt(&allowed_keys);

        let complete_fut = crate::llm::client::LlmClient::complete(&client, &sys_prompt, &messages);
        let raw_res = runtime.block_on(complete_fut);

        let raw = match raw_res {
            Ok(r) => r,
            Err(err) => {
                eprintln!(
                    "[ingest] LLM extraction failed: {err:?}. Falling back to layout extraction."
                );
                return vec![Self::run_fallback_extraction(
                    chunk,
                    source_name,
                    Some("llm_call_failed"),
                    resolved_vault,
                )];
            }
        };

        match crate::memory_agent::parser::parse_candidates_from_llm_output(
            &raw,
            config.allowed_vault_keys.as_deref(),
        ) {
            Ok(mut candidates) => {
                for node in &mut candidates {
                    node.source = Some(source_name.to_string());
                    node.source_type = Some("pdf_import".to_string());
                    node.meta = Some(serde_json::json!({
                        "ocr_confidence": chunk.ocr_confidence,
                        "tables_unstructured": chunk.tables_unstructured,
                    }));
                }
                candidates
            }
            Err(err) => {
                eprintln!(
                    "[ingest] Failed to parse candidate nodes from LLM JSON: {err}. Falling back to layout extraction."
                );
                vec![Self::run_fallback_extraction(
                    chunk,
                    source_name,
                    Some("llm_parse_failed"),
                    resolved_vault,
                )]
            }
        }
    }
}

fn prepare_job_runtime(
) -> Result<(tokio::runtime::Handle, Option<tokio::runtime::Runtime>), OcrError> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => Ok((handle, None)),
        Err(_) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    OcrError::InferenceFailed(format!("Failed to build Tokio runtime: {err}"))
                })?;
            let handle = runtime.handle().clone();
            Ok((handle, Some(runtime)))
        }
    }
}

fn scale_blocks_to_page(
    blocks: Vec<RawLayoutBlock>,
    scale_x: f32,
    scale_y: f32,
) -> Vec<RawLayoutBlock> {
    blocks
        .into_iter()
        .map(|mut block| {
            block.bbox = Rect::new(
                block.bbox.x * scale_x,
                block.bbox.y * scale_y,
                block.bbox.width * scale_x,
                block.bbox.height * scale_y,
            );
            if let Some(font_size) = block.font_size {
                block.font_size = Some(font_size * scale_y);
            }
            block
        })
        .collect()
}

fn merge_hybrid_raw_blocks(
    digital_blocks: Vec<RawLayoutBlock>,
    ocr_blocks: Vec<RawLayoutBlock>,
) -> Vec<RawLayoutBlock> {
    if digital_blocks.is_empty() {
        return ocr_blocks;
    }

    let mut merged = digital_blocks;
    let non_overlapping_ocr: Vec<RawLayoutBlock> = ocr_blocks
        .into_iter()
        .filter(|ocr| !merged.iter().any(|digital| blocks_overlap(digital, ocr)))
        .collect();
    merged.extend(non_overlapping_ocr);
    merged
}

fn blocks_overlap(a: &RawLayoutBlock, b: &RawLayoutBlock) -> bool {
    let left = a.bbox.x.max(b.bbox.x);
    let top = a.bbox.y.max(b.bbox.y);
    let right = (a.bbox.x + a.bbox.width).min(b.bbox.x + b.bbox.width);
    let bottom = (a.bbox.y + a.bbox.height).min(b.bbox.y + b.bbox.height);
    let intersection = (right - left).max(0.0) * (bottom - top).max(0.0);
    if intersection <= 0.0 {
        return false;
    }

    let a_area = (a.bbox.width * a.bbox.height).max(1.0);
    let b_area = (b.bbox.width * b.bbox.height).max(1.0);
    intersection / a_area.min(b_area) > 0.35
}

/// Chunks structured `IngestBlock`s into logical import sections, calculating chunk-level
/// OCR confidence and checking for unstructured tables.
pub fn chunk_ingest_blocks(
    blocks: &[IngestBlock],
    target_tokens: usize,
    overlap_tokens: usize,
) -> Vec<ImportChunkSpec> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let target_tokens = target_tokens.max(50);
    let overlap_tokens = overlap_tokens.min(target_tokens / 2);

    let mut chunks = Vec::new();
    let mut current_blocks: Vec<&IngestBlock> = Vec::new();
    let mut current_token_count = 0usize;
    let mut current_heading_context: Option<String> = None;

    for block in blocks {
        let block_tokens = count_tokens(&block.formatted_text);

        // Track current section heading if block is of type Heading
        if let crate::ingest::layout::BlockType::Heading(_) = block.block_type {
            current_heading_context = Some(
                block
                    .formatted_text
                    .lines()
                    .next()
                    .unwrap_or(&block.formatted_text)
                    .trim_start_matches('#')
                    .trim()
                    .to_string(),
            );
        }

        if current_token_count + block_tokens > target_tokens && !current_blocks.is_empty() {
            let chunk_text = join_ingest_blocks(
                &current_blocks
                    .iter()
                    .map(|&b| b.clone())
                    .collect::<Vec<_>>(),
            );
            let token_count = count_tokens(&chunk_text);

            let (ocr_conf, has_tables) = calculate_chunk_metrics(&current_blocks);

            chunks.push(ImportChunkSpec {
                chunk_index: chunks.len(),
                text: chunk_text,
                token_count,
                heading_context: current_heading_context.clone(),
                chunk_type: "import".to_string(),
                ocr_confidence: ocr_conf,
                tables_unstructured: has_tables,
            });

            // Handle overlap by keeping trailing blocks up to overlap_tokens
            let mut overlap_acc = Vec::new();
            let mut overlap_count = 0usize;
            for &prev_b in current_blocks.iter().rev() {
                let b_tokens = count_tokens(&prev_b.formatted_text);
                if overlap_count + b_tokens <= overlap_tokens {
                    overlap_acc.push(prev_b);
                    overlap_count += b_tokens;
                } else {
                    break;
                }
            }
            overlap_acc.reverse();
            current_blocks = overlap_acc;
            current_token_count = overlap_count;
        }

        current_blocks.push(block);
        current_token_count += block_tokens;
    }

    if !current_blocks.is_empty() {
        let chunk_text = join_ingest_blocks(
            &current_blocks
                .iter()
                .map(|&b| b.clone())
                .collect::<Vec<_>>(),
        );
        let token_count = count_tokens(&chunk_text);

        let (ocr_conf, has_tables) = calculate_chunk_metrics(&current_blocks);

        chunks.push(ImportChunkSpec {
            chunk_index: chunks.len(),
            text: chunk_text,
            token_count,
            heading_context: current_heading_context,
            chunk_type: "import".to_string(),
            ocr_confidence: ocr_conf,
            tables_unstructured: has_tables,
        });
    }

    chunks
}

fn calculate_chunk_metrics(blocks: &[&IngestBlock]) -> (Option<f32>, bool) {
    let mut ocr_sum = 0.0f32;
    let mut ocr_count = 0usize;
    let mut has_tables = false;

    for block in blocks {
        if let Some(conf) = block.confidence {
            ocr_sum += conf;
            ocr_count += 1;
        }
        if block.block_type == crate::ingest::layout::BlockType::Table {
            has_tables = true;
        }
    }

    let avg_ocr = if ocr_count > 0 {
        Some(ocr_sum / (ocr_count as f32))
    } else {
        None
    };

    (avg_ocr, has_tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk_markdown_string(
        md: &str,
        target_tokens: usize,
        overlap_tokens: usize,
    ) -> Vec<ImportChunkSpec> {
        let blocks: Vec<IngestBlock> = md
            .split("\n\n")
            .map(|s| {
                let block_type = if s.starts_with('#') {
                    crate::ingest::layout::BlockType::Heading(1)
                } else {
                    crate::ingest::layout::BlockType::Body
                };
                IngestBlock {
                    formatted_text: s.to_string(),
                    block_type,
                    confidence: None,
                    page_index: 0,
                }
            })
            .collect();
        chunk_ingest_blocks(&blocks, target_tokens, overlap_tokens)
    }

    #[test]
    fn test_chunk_markdown_import_basic() {
        let md =
            "# Heading 1\n\nFirst paragraph content.\n\n## Heading 2\n\nSecond paragraph content.";
        let chunks = test_chunk_markdown_string(md, 500, 50);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].chunk_type, "import");
        assert!(chunks[0].token_count > 0);
    }

    #[test]
    fn test_chunk_markdown_import_split_and_overlap() {
        let mut md = String::new();
        md.push_str("# Section Title\n\n");
        for i in 1..=30 {
            md.push_str(&format!(
                "Paragraph number {} contains meaningful sentences for token split testing.\n\n",
                i
            ));
        }

        let chunks = test_chunk_markdown_string(&md, 100, 20);
        assert!(chunks.len() > 1);

        for (idx, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, idx);
            assert_eq!(chunk.chunk_type, "import");
            assert!(chunk.token_count > 0);
        }
    }

    #[test]
    fn test_derive_document_extraction_path_digital_only() {
        assert_eq!(derive_document_extraction_path(10, 0, 0), "digital");
    }

    #[test]
    fn test_derive_document_extraction_path_ocr_only() {
        assert_eq!(derive_document_extraction_path(0, 5, 0), "ocr-bundled");
    }

    #[test]
    fn test_derive_document_extraction_path_hybrid_only() {
        assert_eq!(derive_document_extraction_path(0, 0, 3), "hybrid");
    }

    #[test]
    fn test_derive_document_extraction_path_mixed_digital_and_ocr() {
        assert_eq!(derive_document_extraction_path(8, 2, 0), "hybrid");
    }

    #[test]
    fn test_derive_document_extraction_path_all_zero_defaults_digital() {
        assert_eq!(derive_document_extraction_path(0, 0, 0), "digital");
    }

    #[test]
    fn test_extraction_path_digital() {
        assert_eq!(
            extraction_path_for_page(PdfPageType::Digital),
            ExtractionPath::Digital
        );
    }

    #[test]
    fn test_extraction_path_ocr() {
        assert_eq!(
            extraction_path_for_page(PdfPageType::Ocr),
            ExtractionPath::Ocr
        );
    }

    #[test]
    fn test_extraction_path_hybrid_routes_to_ocr() {
        assert_eq!(
            extraction_path_for_page(PdfPageType::Hybrid),
            ExtractionPath::Ocr
        );
    }

    #[test]
    fn test_fallback_extraction_uses_configured_vault_key() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "Some document text for testing.".to_string(),
            token_count: 6,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
        };
        let node = IngestJobEngine::run_fallback_extraction(
            &chunk,
            "test.pdf",
            Some("no_llm_configured"),
            Some("finance".to_string()),
        );
        assert_eq!(node.target_vault_key, Some("finance".to_string()));
    }

    #[test]
    fn test_fallback_extraction_defaults_to_none_when_no_vault() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "Some document text for testing.".to_string(),
            token_count: 6,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(node.target_vault_key, None);
    }

    #[test]
    fn test_fallback_summary_joins_past_abbreviations() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "Mr. Smith met Dr. Jones at the clinic.".to_string(),
            token_count: 10,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(node.summary, "Mr. Smith met Dr. Jones at the clinic");
    }

    #[test]
    fn test_extract_chunk_candidates_defaults_to_learning_vault() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "This chunk goes to fallback because provider is None.".to_string(),
            token_count: 9,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
        };
        let config = IngestJobConfig {
            provider: None, // Forces the 'no_llm_configured' fallback path
            allowed_vault_keys: None,
            ..Default::default()
        };

        let (runtime_handle, _owned_runtime) = prepare_job_runtime()
            .unwrap_or_else(|err| panic!("expected job runtime in test: {err}"));
        let nodes =
            IngestJobEngine::extract_chunk_candidates(&chunk, "test.pdf", &config, &runtime_handle);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].target_vault_key, Some("learning".to_string()));
    }

    #[test]
    fn test_merge_hybrid_raw_blocks_keeps_digital_text_and_non_overlapping_ocr() {
        let digital = vec![RawLayoutBlock::new(
            "Congress of the United States",
            Rect::new(10.0, 10.0, 120.0, 12.0),
        )];
        let ocr = vec![
            RawLayoutBlock::new(
                "CongressoftheUnitedStates",
                Rect::new(12.0, 10.0, 118.0, 12.0),
            ),
            RawLayoutBlock::new("Image-only caption", Rect::new(10.0, 50.0, 90.0, 12.0)),
        ];

        let merged = merge_hybrid_raw_blocks(digital, ocr);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "Congress of the United States");
        assert_eq!(merged[1].text, "Image-only caption");
    }
}
