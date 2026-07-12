use crate::embed::chunking::count_tokens;
use crate::ingest::coords::normalize_blocks_to_pdf_points;
use crate::ingest::layout::{
    analyze_layout, analyze_layout_with_snapshot, LayoutDebugSnapshot, RawLayoutBlock,
};
use crate::ingest::markdown::{assemble_markdown_blocks, join_ingest_blocks, IngestBlock};
use crate::memory_agent::parser::{CandidateAction, CandidateNode};
use crate::ocr::bundled::BundledOcrEngine;
use crate::ocr::engine::{OcrEngine, OcrError};
use crate::ocr::pdf::{LoadedPdf, PdfPageType, PdfRasterizer, PdfRasterizerConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

/// Chooses which extraction source wins where digital text and OCR overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HybridMergeStrategy {
    /// Preserve selectable PDF text and add OCR-only regions (the legacy behavior).
    #[default]
    DigitalPreferred,
    /// Prefer OCR text in overlapping regions, while preserving digital-only regions.
    OcrPreferred,
    /// Explicit name for the legacy digital base plus OCR supplement behavior.
    OcrNonOverlappingOnly,
}

impl HybridMergeStrategy {
    pub fn from_env_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "digital_preferred" => Some(Self::DigitalPreferred),
            "ocr_preferred" => Some(Self::OcrPreferred),
            "ocr_non_overlapping_only" => Some(Self::OcrNonOverlappingOnly),
            _ => None,
        }
    }
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
    #[serde(default)]
    pub hybrid_merge_strategy: HybridMergeStrategy,
    /// Include Header/Footer blocks in generated import chunks (default: false).
    #[serde(default)]
    pub include_margin_blocks_in_chunks: bool,
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
            hybrid_merge_strategy: HybridMergeStrategy::DigitalPreferred,
            include_margin_blocks_in_chunks: false,
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
    #[serde(default)]
    pub source_page_indices: Vec<usize>,
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
    #[serde(default)]
    pub layout_debug: Vec<LayoutDebugSnapshot>,
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

        let rasterizer = PdfRasterizer::new()?;
        let document = rasterizer.load_document_from_file(file_path)?;
        let pages_info = PdfRasterizer::scan_loaded_document(&document)?;
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
        let debug_layout = std::env::var_os("AMBER_INGEST_DEBUG").is_some();
        let mut layout_debug = Vec::new();

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

            let (raw_blocks, page_width, page_height) = match p.page_type {
                PdfPageType::Digital => {
                    let raw_blocks =
                        PdfRasterizer::extract_digital_blocks_from_document(&document, i)?;
                    (raw_blocks, p.width_pts, p.height_pts)
                }
                PdfPageType::Ocr => {
                    let (ocr_blocks, image_width, image_height, confidence) =
                        Self::recognize_ocr_page(
                            &rasterizer,
                            &document,
                            i,
                            &rasterizer_config,
                            &mut cached_ocr_engine,
                        )?;
                    total_ocr_confidence_sum += confidence;
                    ocr_pass_count += 1;

                    let raw_blocks = normalize_blocks_to_pdf_points(
                        ocr_blocks,
                        image_width,
                        image_height,
                        p.width_pts,
                        p.height_pts,
                    );
                    (raw_blocks, p.width_pts, p.height_pts)
                }
                PdfPageType::Hybrid => {
                    let (ocr_blocks, image_width, image_height, confidence) =
                        Self::recognize_ocr_page(
                            &rasterizer,
                            &document,
                            i,
                            &rasterizer_config,
                            &mut cached_ocr_engine,
                        )?;
                    total_ocr_confidence_sum += confidence;
                    ocr_pass_count += 1;

                    let ocr_blocks = normalize_blocks_to_pdf_points(
                        ocr_blocks,
                        image_width,
                        image_height,
                        p.width_pts,
                        p.height_pts,
                    );
                    let digital_blocks =
                        PdfRasterizer::extract_digital_blocks_from_document(&document, i)?;
                    let page_area = p.width_pts * p.height_pts;
                    let strategy = if digital_extraction_sparse(&digital_blocks, page_area) {
                        eprintln!(
                            "[ingest] Sparse digital text on page {}; preferring OCR.",
                            i + 1
                        );
                        HybridMergeStrategy::OcrPreferred
                    } else {
                        config.hybrid_merge_strategy
                    };
                    let raw_blocks = merge_hybrid_raw_blocks(digital_blocks, ocr_blocks, strategy);

                    (raw_blocks, p.width_pts, p.height_pts)
                }
            };

            let layout_blocks = if debug_layout {
                let (blocks, mut snapshot) =
                    analyze_layout_with_snapshot(raw_blocks, page_width, page_height);
                snapshot.page_index = i;
                layout_debug.push(snapshot);
                blocks
            } else {
                analyze_layout(raw_blocks, page_width, page_height)
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
            config.include_margin_blocks_in_chunks,
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
            let mut chunk_candidates = IngestJobEngine::extract_chunk_candidates(
                chunk,
                &source_name,
                &config,
                &runtime_handle,
            );
            for candidate in &mut chunk_candidates {
                attach_integrity_trace(chunk, candidate, &chunks);
            }
            candidates.extend(chunk_candidates);
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
            layout_debug,
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
            config.include_margin_blocks_in_chunks,
        );

        let (runtime_handle, _owned_runtime) = prepare_job_runtime()?;

        let mut candidates = Vec::new();
        for chunk in &chunks {
            let mut chunk_candidates = IngestJobEngine::extract_chunk_candidates(
                chunk,
                &source_name,
                &config,
                &runtime_handle,
            );
            for candidate in &mut chunk_candidates {
                attach_integrity_trace(chunk, candidate, &chunks);
            }
            candidates.extend(chunk_candidates);
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
            layout_debug: Vec::new(),
        })
    }

    fn recognize_ocr_page(
        rasterizer: &PdfRasterizer,
        document: &LoadedPdf<'_>,
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

        let page_img = rasterizer.render_loaded_page(document, page_index, rasterizer_config)?;
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
        const MIN_SUMMARY_LEN: usize = 20;
        const MAX_SUMMARY_LEN: usize = 120;
        let mut summary = if let Some(end) = first_summary_boundary(&chunk.text, MIN_SUMMARY_LEN) {
            chunk.text[..end].to_string()
        } else if chunk.text.chars().count() >= MIN_SUMMARY_LEN {
            chunk.text.clone()
        } else {
            chunk.text.chars().take(MAX_SUMMARY_LEN).collect()
        };
        if summary.ends_with('.') {
            summary.pop();
        }
        if summary.chars().count() > MAX_SUMMARY_LEN {
            summary = summary
                .chars()
                .take(MAX_SUMMARY_LEN.saturating_sub(3))
                .collect::<String>()
                + "...";
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
        let resolved_vault = match &config.allowed_vault_keys {
            None => Some("learning".to_string()),
            Some(keys) if keys.is_empty() => None,
            Some(keys) => keys.first().cloned(),
        };

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

/// Verifies that fallback output remains traceable to the exact chunk it represents.
fn validate_fallback_candidate(chunk: &ImportChunkSpec, candidate: &CandidateNode) -> Vec<String> {
    validate_candidate_integrity(chunk, candidate, &[])
}

fn validate_candidate_integrity(
    chunk: &ImportChunkSpec,
    candidate: &CandidateNode,
    all_chunks: &[ImportChunkSpec],
) -> Vec<String> {
    let mut warnings = Vec::new();
    let normalized_chunk = normalize_line_endings(&chunk.text);
    if let Some(detail) = candidate.detail.as_deref() {
        if !normalized_chunk.contains(&normalize_line_endings(detail)) {
            warnings.push("candidate detail is not contained in its source chunk".to_string());
        }
    }
    if candidate.summary.chars().count() > 120 {
        warnings.push("candidate summary exceeds the 120-character ingestion limit".to_string());
    }

    let title_matches_current_heading = chunk
        .heading_context
        .as_deref()
        .is_some_and(|heading| heading == candidate.title);
    if !title_matches_current_heading {
        for other in all_chunks {
            let title_matches_other_heading = other
                .heading_context
                .as_deref()
                .is_some_and(|heading| heading == candidate.title);
            if other.chunk_index != chunk.chunk_index
                && title_matches_other_heading
                && text_overlap_ratio(&chunk.text, &other.text) <= 0.5
            {
                warnings.push(
                    "candidate title matches a different chunk heading without sufficient text overlap"
                        .to_string(),
                );
                break;
            }
        }
    }
    warnings
}

fn attach_integrity_trace(
    chunk: &ImportChunkSpec,
    candidate: &mut CandidateNode,
    all_chunks: &[ImportChunkSpec],
) {
    let mut warnings = validate_fallback_candidate(chunk, candidate);
    warnings.extend(validate_candidate_integrity(chunk, candidate, all_chunks));
    warnings.sort();
    warnings.dedup();
    if !warnings.is_empty() {
        eprintln!(
            "[ingest] Candidate integrity warnings for chunk {}: {}",
            chunk.chunk_index,
            warnings.join("; ")
        );
    }

    let meta = candidate.meta.get_or_insert_with(|| serde_json::json!({}));
    if let Some(map) = meta.as_object_mut() {
        map.insert(
            "source_page_indices".to_string(),
            serde_json::json!(chunk.source_page_indices),
        );
        if !warnings.is_empty() {
            map.insert("warnings".to_string(), serde_json::json!(warnings));
        }
    }
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn text_overlap_ratio(a: &str, b: &str) -> f32 {
    let words = |text: &str| {
        text.split_whitespace()
            .map(|word| {
                word.trim_matches(|ch: char| !ch.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|word| !word.is_empty())
            .collect::<HashSet<_>>()
    };
    let a_words = words(a);
    let b_words = words(b);
    let union = a_words.union(&b_words).count();
    if union == 0 {
        return 1.0;
    }
    a_words.intersection(&b_words).count() as f32 / union as f32
}

fn first_summary_boundary(text: &str, min_chars: usize) -> Option<usize> {
    let mut seen = 0usize;
    for (idx, ch) in text.char_indices() {
        seen += 1;
        if matches!(ch, '.' | '?' | '!') {
            if ch == '.' && is_likely_abbreviation_period(text, idx) {
                continue;
            }
            if seen >= min_chars {
                return Some(idx + ch.len_utf8());
            }
        }
    }
    None
}

fn is_likely_abbreviation_period(text: &str, period_idx: usize) -> bool {
    let before = text[..period_idx].trim_end();
    let Some(last_token) = before.split_whitespace().next_back() else {
        return false;
    };
    let token = last_token.trim_end_matches('.');
    matches!(
        token,
        "Mr" | "Mrs"
            | "Ms"
            | "Dr"
            | "Prof"
            | "Sr"
            | "Jr"
            | "St"
            | "vs"
            | "etc"
            | "Inc"
            | "Ltd"
            | "No"
    ) || (!token.is_empty() && token.len() <= 2 && token.chars().all(|c| c.is_ascii_uppercase()))
}

fn prepare_job_runtime() -> Result<(tokio::runtime::Handle, tokio::runtime::Runtime), OcrError> {
    // Ingest jobs run on blocking threads; always use a job-owned runtime so LLM
    // block_on never borrows the Tauri async runtime handle.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            OcrError::InferenceFailed(format!("Failed to build Tokio runtime: {err}"))
        })?;
    let handle = runtime.handle().clone();
    Ok((handle, runtime))
}

fn merge_hybrid_raw_blocks(
    digital_blocks: Vec<RawLayoutBlock>,
    ocr_blocks: Vec<RawLayoutBlock>,
    strategy: HybridMergeStrategy,
) -> Vec<RawLayoutBlock> {
    let (mut merged, supplement) = match strategy {
        HybridMergeStrategy::DigitalPreferred | HybridMergeStrategy::OcrNonOverlappingOnly => {
            (digital_blocks, ocr_blocks)
        }
        HybridMergeStrategy::OcrPreferred => (ocr_blocks, digital_blocks),
    };
    let non_overlapping_supplement = supplement
        .into_iter()
        .filter(|block| !merged.iter().any(|base| blocks_overlap(base, block)))
        .collect::<Vec<_>>();
    merged.extend(non_overlapping_supplement);
    merged
}

/// Treat digital text as sparse when it cannot represent even a modest fraction of the page.
/// The geometry ratio remains resolution-independent; the block-count guard handles pages with
/// a single selectable label over otherwise scanned content.
fn digital_extraction_sparse(digital_blocks: &[RawLayoutBlock], page_area: f32) -> bool {
    if digital_blocks.len() < 2 {
        return true;
    }
    let text_area: f32 = digital_blocks
        .iter()
        .map(|block| block.bbox.width.max(0.0) * block.bbox.height.max(0.0))
        .sum();
    text_area / page_area.max(1.0) < 0.01
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
    include_margin_blocks: bool,
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
        if !include_margin_blocks
            && matches!(
                block.block_type,
                crate::ingest::layout::BlockType::Header | crate::ingest::layout::BlockType::Footer
            )
        {
            continue;
        }

        let block_tokens = count_tokens(&block.formatted_text);

        let next_heading_context =
            if let crate::ingest::layout::BlockType::Heading(_) = block.block_type {
                if !block.fragment {
                    let heading = block
                        .formatted_text
                        .lines()
                        .map(|line| line.trim().trim_start_matches('#').trim())
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    (!heading.is_empty()).then_some(heading)
                } else {
                    None
                }
            } else {
                None
            };

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
                source_page_indices: source_page_indices(&current_blocks),
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

        if let Some(heading) = next_heading_context {
            current_heading_context = Some(heading);
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
            source_page_indices: source_page_indices(&current_blocks),
        });
    }

    chunks
}

fn calculate_chunk_metrics(blocks: &[&IngestBlock]) -> (Option<f32>, bool) {
    let mut has_tables = false;

    for block in blocks {
        if block.block_type == crate::ingest::layout::BlockType::Table {
            has_tables = true;
        }
    }

    let entries = blocks.iter().filter_map(|block| {
        block
            .confidence
            .map(|confidence| (block.recognized_text.as_str(), confidence))
    });
    let avg_ocr = crate::ocr::engine::char_weighted_confidence(entries);

    (avg_ocr, has_tables)
}

fn source_page_indices(blocks: &[&IngestBlock]) -> Vec<usize> {
    let mut pages = blocks
        .iter()
        .map(|block| block.page_index)
        .collect::<Vec<_>>();
    pages.sort_unstable();
    pages.dedup();
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::engine::Rect;

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
                    recognized_text: s.to_string(),
                    formatted_text: s.to_string(),
                    block_type,
                    confidence: None,
                    page_index: 0,
                    fragment: false,
                }
            })
            .collect();
        chunk_ingest_blocks(&blocks, target_tokens, overlap_tokens, false)
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
            source_page_indices: vec![0],
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
            source_page_indices: vec![0],
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(node.target_vault_key, None);
    }

    #[test]
    fn test_fallback_summary_preserves_question_and_exclamation_marks() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "Is this a question? Yes it is! And here is more trailing text.".to_string(),
            token_count: 12,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(node.summary, "Is this a question? Yes it is!");
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
            source_page_indices: vec![0],
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(node.summary, "Mr. Smith met Dr. Jones at the clinic");
    }

    #[test]
    fn test_fallback_summary_handles_abbreviation_after_tab() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "Meet\tMr. Smith went to the store and bought many things for the party."
                .to_string(),
            token_count: 12,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(
            node.summary,
            "Meet\tMr. Smith went to the store and bought many things for the party"
        );
    }

    #[test]
    fn test_fallback_summary_does_not_treat_leading_period_as_abbreviation() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: ". This sentence starts with a period and is long enough to summarize."
                .to_string(),
            token_count: 12,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        };
        let node = IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        assert_eq!(
            node.summary,
            ". This sentence starts with a period and is long enough to summarize"
        );
    }

    #[test]
    fn test_extract_chunk_candidates_custom_vault_leaves_target_unset() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "Custom vault target should not default to learning.".to_string(),
            token_count: 9,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        };
        let config = IngestJobConfig {
            provider: None,
            allowed_vault_keys: Some(vec![]),
            ..Default::default()
        };

        let (runtime_handle, _owned_runtime) = prepare_job_runtime()
            .unwrap_or_else(|err| panic!("expected job runtime in test: {err}"));
        let nodes =
            IngestJobEngine::extract_chunk_candidates(&chunk, "test.pdf", &config, &runtime_handle);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].target_vault_key, None);
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
            source_page_indices: vec![0],
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

        let merged = merge_hybrid_raw_blocks(digital, ocr, HybridMergeStrategy::DigitalPreferred);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "Congress of the United States");
        assert_eq!(merged[1].text, "Image-only caption");
    }

    #[test]
    fn ocr_preferred_hybrid_merge_keeps_ocr_text_on_overlap() {
        let digital = vec![
            RawLayoutBlock::new("Digital overlap", Rect::new(10.0, 10.0, 120.0, 12.0)),
            RawLayoutBlock::new("Digital only", Rect::new(10.0, 50.0, 120.0, 12.0)),
        ];
        let ocr = vec![
            RawLayoutBlock::new("OCR overlap", Rect::new(12.0, 10.0, 118.0, 12.0)),
            RawLayoutBlock::new("OCR only", Rect::new(10.0, 90.0, 120.0, 12.0)),
        ];
        let merged = merge_hybrid_raw_blocks(digital, ocr, HybridMergeStrategy::OcrPreferred);
        let text = merged
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(text, vec!["OCR overlap", "OCR only", "Digital only"]);
    }

    #[test]
    fn sparse_digital_text_prefers_ocr() {
        let sparse = vec![RawLayoutBlock::new(
            "Selectable label",
            Rect::new(20.0, 20.0, 100.0, 12.0),
        )];
        assert!(digital_extraction_sparse(&sparse, 600.0 * 800.0));
    }

    #[test]
    fn calculate_chunk_metrics_char_weighted_prefers_large_block() {
        let paragraph = IngestBlock {
            recognized_text: "A".repeat(200),
            formatted_text: "A".repeat(200),
            block_type: crate::ingest::layout::BlockType::Body,
            confidence: Some(0.95),
            page_index: 0,
            fragment: false,
        };
        let speck = IngestBlock {
            recognized_text: "?".to_string(),
            formatted_text: "?".to_string(),
            block_type: crate::ingest::layout::BlockType::Footer,
            confidence: Some(0.01),
            page_index: 0,
            fragment: false,
        };
        let blocks = vec![&paragraph, &speck];
        let (ocr_conf, _) = calculate_chunk_metrics(&blocks);
        let ocr_conf = match ocr_conf {
            Some(value) => value,
            None => panic!("expected chunk ocr confidence"),
        };
        assert!(
            ocr_conf > 0.90,
            "large paragraph should dominate speck: {ocr_conf}"
        );
    }

    #[test]
    fn calculate_chunk_metrics_hybrid_excludes_digital_blocks() {
        let digital = IngestBlock {
            recognized_text: "A".repeat(500),
            formatted_text: "A".repeat(500),
            block_type: crate::ingest::layout::BlockType::Body,
            confidence: None,
            page_index: 0,
            fragment: false,
        };
        let ocr_high = IngestBlock {
            recognized_text: "High confidence OCR body.".to_string(),
            formatted_text: "High confidence OCR body.".to_string(),
            block_type: crate::ingest::layout::BlockType::Body,
            confidence: Some(0.9),
            page_index: 0,
            fragment: false,
        };
        let ocr_low = IngestBlock {
            recognized_text: "Lo".to_string(),
            formatted_text: "Lo".to_string(),
            block_type: crate::ingest::layout::BlockType::Body,
            confidence: Some(0.1),
            page_index: 0,
            fragment: false,
        };
        let blocks = vec![&digital, &ocr_high, &ocr_low];
        let (ocr_conf, _) = calculate_chunk_metrics(&blocks);
        let expected = match crate::ocr::engine::char_weighted_confidence([
            (ocr_high.recognized_text.as_str(), 0.9),
            (ocr_low.recognized_text.as_str(), 0.1),
        ]) {
            Some(value) => value,
            None => panic!("expected char-weighted confidence for ocr blocks"),
        };
        let ocr_conf = match ocr_conf {
            Some(value) => value,
            None => panic!("expected hybrid chunk confidence"),
        };
        assert!((ocr_conf - expected).abs() < 1e-4);
    }

    #[test]
    fn chunk_tracks_full_heading_context_fragment_and_source_pages() {
        let blocks = vec![
            IngestBlock {
                recognized_text: "Full Document Title".to_string(),
                formatted_text: "# Full Document Title".to_string(),
                block_type: crate::ingest::layout::BlockType::Heading(1),
                confidence: None,
                page_index: 0,
                fragment: false,
            },
            IngestBlock {
                recognized_text: "x = a + b / c".to_string(),
                formatted_text: "x = a + b / c".to_string(),
                block_type: crate::ingest::layout::BlockType::Body,
                confidence: None,
                page_index: 1,
                fragment: true,
            },
        ];
        let chunks = chunk_ingest_blocks(&blocks, 500, 0, false);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].heading_context.as_deref(),
            Some("Full Document Title")
        );
        assert_eq!(chunks[0].source_page_indices, vec![0, 1]);
    }

    #[test]
    fn integrity_trace_warns_for_untraceable_candidate_detail() {
        let chunk = ImportChunkSpec {
            chunk_index: 0,
            text: "This source chunk is intentionally short.".to_string(),
            token_count: 7,
            heading_context: Some("Source heading".to_string()),
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        };
        let mut candidate =
            IngestJobEngine::run_fallback_extraction(&chunk, "test.pdf", None, None);
        candidate.detail = Some("invented detail".to_string());
        let warnings = validate_fallback_candidate(&chunk, &candidate);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("not contained")));
    }
}
