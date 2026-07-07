use crate::embed::chunking::count_tokens;
use crate::ingest::layout::{analyze_layout, RawLayoutBlock};
use crate::ingest::markdown::{assemble_markdown_blocks, join_ingest_blocks, IngestBlock};
use crate::ocr::bundled::BundledOcrEngine;
use crate::ocr::engine::{OcrEngine, OcrError};
use crate::ocr::pdf::{PdfPageType, PdfRasterizer, PdfRasterizerConfig};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::mpsc;

/// Configuration options for an import job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestJobConfig {
    /// Rasterization resolution for scanned PDF pages (default: 300).
    pub rasterization_dpi: u16,
    /// Target maximum token length for each generated import chunk (default: 350).
    pub target_chunk_tokens: usize,
    /// Token overlap between consecutive import chunks (default: 60).
    pub overlap_chunk_tokens: usize,
}

impl Default for IngestJobConfig {
    fn default() -> Self {
        Self {
            rasterization_dpi: 300,
            target_chunk_tokens: 350,
            overlap_chunk_tokens: 60,
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
                PdfPageType::Ocr | PdfPageType::Hybrid => {
                    // Lazily initialize BundledOcrEngine on first OCR/Hybrid page
                    if cached_ocr_engine.is_none() {
                        cached_ocr_engine = Some(BundledOcrEngine::new()?);
                    }
                    let ocr_engine = match cached_ocr_engine.as_mut() {
                        Some(e) => e,
                        None => {
                            return Err(OcrError::InferenceFailed(
                                "Failed initializing OCR engine session".to_string(),
                            ));
                        }
                    };

                    let page_img = rasterizer.render_page(&pdf_bytes, i, &rasterizer_config)?;
                    let ocr_output = ocr_engine.recognize(&page_img)?;

                    total_ocr_confidence_sum += ocr_output.avg_confidence;
                    ocr_pass_count += 1;

                    let raw_blocks: Vec<RawLayoutBlock> = ocr_output
                        .blocks
                        .into_iter()
                        .map(|b| RawLayoutBlock::new(b.text, b.bbox).with_confidence(b.confidence))
                        .collect();

                    analyze_layout(raw_blocks, p.width_pts, p.height_pts)
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
        })
    }
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
    fn test_process_pdf_job_digital() -> Result<(), Box<dyn std::error::Error>> {
        let pdf_path = Path::new(r"C:\Users\aashi\Downloads\constitution.pdf");
        if !pdf_path.is_file() {
            eprintln!("Skipping test_process_pdf_job_digital: constitution.pdf not found");
            return Ok(());
        }

        let config = IngestJobConfig {
            rasterization_dpi: 150,
            target_chunk_tokens: 300,
            overlap_chunk_tokens: 50,
        };

        let (tx, rx) = mpsc::channel();
        let result = IngestJobEngine::process_pdf_job("job-test-1", pdf_path, config, Some(tx))?;

        let mut progress_count = 0;
        while let Ok(prog) = rx.try_recv() {
            progress_count += 1;
            assert!(prog.total_pages > 0);
        }

        assert!(progress_count > 0);
        assert_eq!(result.job_id, "job-test-1");
        assert!(result.total_pages > 0);
        assert!(result.digital_pages > 0);
        assert!(!result.assembled_markdown.is_empty());
        assert!(!result.chunks.is_empty());
        Ok(())
    }
}
