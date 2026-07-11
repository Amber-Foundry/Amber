use crate::ocr::engine::OcrError;
use crate::ocr::ocr_models_dir;
use crate::{ingest::layout::RawLayoutBlock, ocr::engine::Rect};
use image::DynamicImage;
use pdfium_render::prelude::*;
use std::path::Path;

/// Classification of a PDF page's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PdfPageType {
    /// Pure digital text layer (contains text objects, no scanned image objects).
    Digital,
    /// Scanned image / raster (contains image objects, no selectable text layer).
    Ocr,
    /// Hybrid page containing both digital text and embedded scanned image objects.
    Hybrid,
}

impl std::fmt::Display for PdfPageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfPageType::Digital => write!(f, "digital"),
            PdfPageType::Ocr => write!(f, "ocr-bundled"),
            PdfPageType::Hybrid => write!(f, "hybrid"),
        }
    }
}

/// Metadata summary for a scanned PDF page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PdfPageInfo {
    pub page_index: usize,
    pub page_type: PdfPageType,
    pub width_pts: f32,
    pub height_pts: f32,
}

/// Configuration settings for PDF page rendering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PdfRasterizerConfig {
    pub dpi: u16,
}

impl Default for PdfRasterizerConfig {
    fn default() -> Self {
        Self { dpi: 300 }
    }
}

/// A PDF document loaded once for multi-page extraction.
pub type LoadedPdf<'a> = PdfDocument<'a>;

/// Helper for PDF object scanning, page classification, and image rendering.
pub struct PdfRasterizer {
    pdfium: Pdfium,
}

impl PdfRasterizer {
    /// Initializes `PdfRasterizer` by attempting to bind to system `pdfium` library
    /// or loading from default directory `~/.amber/resources/pdfium/`.
    pub fn new() -> Result<Self, OcrError> {
        let pdfium = match Pdfium::bind_to_system_library() {
            Ok(bindings) => Pdfium::new(bindings),
            Err(_) => {
                let ocr_dir = ocr_models_dir()?;
                let amber_dir = ocr_dir
                    .parent()
                    .and_then(|p| p.parent())
                    .unwrap_or_else(|| Path::new("."));
                let resources_dir = amber_dir.join("resources").join("pdfium");
                let pdfium_file = resources_dir.join(if cfg!(target_os = "windows") {
                    "pdfium.dll"
                } else if cfg!(target_os = "macos") {
                    "libpdfium.dylib"
                } else {
                    "libpdfium.so"
                });

                let bindings = Pdfium::bind_to_library(&pdfium_file)
                    .or_else(|_| {
                        let lib_name = Pdfium::pdfium_platform_library_name_at_path(&resources_dir);
                        Pdfium::bind_to_library(lib_name)
                    })
                    .or_else(|_| Pdfium::bind_to_system_library())
                    .map_err(|e| {
                        OcrError::ModelNotFound(format!(
                            "Failed binding to native pdfium library at {}: {e}. Place pdfium.dll / libpdfium in ~/.amber/resources/pdfium/",
                            pdfium_file.display()
                        ))
                    })?;
                Pdfium::new(bindings)
            }
        };

        Ok(Self { pdfium })
    }

    /// Helper constructor taking a custom native library path.
    pub fn from_library_path(library_dir: &Path) -> Result<Self, OcrError> {
        let lib_name = Pdfium::pdfium_platform_library_name_at_path(library_dir);
        let bindings = Pdfium::bind_to_library(lib_name).map_err(|e| {
            OcrError::ModelNotFound(format!(
                "Failed binding to native pdfium library at {}: {e}",
                library_dir.display()
            ))
        })?;
        Ok(Self {
            pdfium: Pdfium::new(bindings),
        })
    }

    /// Loads a PDF document from a file path for reuse across page operations.
    pub fn load_document_from_file(&self, file_path: &Path) -> Result<PdfDocument<'_>, OcrError> {
        self.pdfium
            .load_pdf_from_file(file_path, None)
            .map_err(|e| {
                OcrError::IoError(format!(
                    "Failed to load PDF file at {}: {e}",
                    file_path.display()
                ))
            })
    }

    /// Loads a PDF document from bytes for reuse across page operations.
    pub fn load_document(&self, pdf_bytes: Vec<u8>) -> Result<PdfDocument<'_>, OcrError> {
        self.pdfium
            .load_pdf_from_byte_vec(pdf_bytes, None)
            .map_err(|e| OcrError::InferenceFailed(format!("Failed to load PDF document: {e}")))
    }

    /// Scans a PDF file on disk and returns metadata for all pages.
    pub fn scan_file(&self, file_path: &Path) -> Result<Vec<PdfPageInfo>, OcrError> {
        let document = self.load_document_from_file(file_path)?;
        Self::scan_loaded_document(&document)
    }

    /// Scans a PDF document from bytes and returns metadata and page classification for all pages.
    pub fn scan_document(&self, pdf_bytes: &[u8]) -> Result<Vec<PdfPageInfo>, OcrError> {
        let document = self.load_document(pdf_bytes.to_vec())?;
        Self::scan_loaded_document(&document)
    }

    /// Scans an already-loaded PDF document and returns metadata for all pages.
    pub fn scan_loaded_document(document: &PdfDocument<'_>) -> Result<Vec<PdfPageInfo>, OcrError> {
        let mut page_infos = Vec::new();

        for (idx, page) in document.pages().iter().enumerate() {
            let width_pts = page.width().value;
            let height_pts = page.height().value;

            let mut has_text = false;
            let mut has_images = false;

            for object in page.objects().iter() {
                match object.object_type() {
                    PdfPageObjectType::Text => {
                        has_text = true;
                    }
                    PdfPageObjectType::Image => {
                        has_images = true;
                    }
                    _ => {}
                }
            }

            let page_type = match (has_text, has_images) {
                (true, true) => PdfPageType::Hybrid,
                (true, false) => PdfPageType::Digital,
                (false, true) => PdfPageType::Ocr,
                (false, false) => PdfPageType::Digital,
            };

            page_infos.push(PdfPageInfo {
                page_index: idx,
                page_type,
                width_pts,
                height_pts,
            });
        }

        Ok(page_infos)
    }

    /// Renders a specific 0-indexed PDF page into a `DynamicImage` at configured DPI.
    pub fn render_page(
        &self,
        pdf_bytes: &[u8],
        page_index: usize,
        config: &PdfRasterizerConfig,
    ) -> Result<DynamicImage, OcrError> {
        let document = self.load_document(pdf_bytes.to_vec())?;
        self.render_loaded_page(&document, page_index, config)
    }

    /// Renders a page from an already-loaded PDF document.
    pub fn render_loaded_page(
        &self,
        document: &PdfDocument<'_>,
        page_index: usize,
        config: &PdfRasterizerConfig,
    ) -> Result<DynamicImage, OcrError> {
        let pages = document.pages();
        let page_idx = u16::try_from(page_index).map_err(|_| {
            OcrError::InferenceFailed(format!(
                "Page index {page_index} exceeds maximum supported value ({})",
                u16::MAX
            ))
        })?;
        let page = pages.get(page_idx).map_err(|_| {
            OcrError::InferenceFailed(format!(
                "Page index {page_index} out of bounds (total pages: {})",
                pages.len()
            ))
        })?;

        let target_width = (page.width().value * config.dpi as f32 / 72.0) as i32;
        let render_config = PdfRenderConfig::new().set_target_width(target_width);

        let bitmap = page.render_with_config(&render_config).map_err(|e| {
            OcrError::InferenceFailed(format!("Failed rendering PDF page {page_index}: {e}"))
        })?;

        Ok(bitmap.as_image())
    }

    /// Extracts digital text layer from a PDF page if available.
    pub fn extract_digital_text(
        &self,
        pdf_bytes: &[u8],
        page_index: usize,
    ) -> Result<String, OcrError> {
        let document = self.load_document(pdf_bytes.to_vec())?;
        Self::extract_digital_text_from_document(&document, page_index)
    }

    /// Extracts digital text from a page in an already-loaded PDF document.
    pub fn extract_digital_text_from_document(
        document: &PdfDocument<'_>,
        page_index: usize,
    ) -> Result<String, OcrError> {
        let pages = document.pages();
        let page_idx = u16::try_from(page_index).map_err(|_| {
            OcrError::InferenceFailed(format!(
                "Page index {page_index} exceeds maximum supported value ({})",
                u16::MAX
            ))
        })?;
        let page = pages.get(page_idx).map_err(|_| {
            OcrError::InferenceFailed(format!(
                "Page index {page_index} out of bounds (total pages: {})",
                pages.len()
            ))
        })?;

        let text = page.text().map_err(|e| {
            OcrError::InferenceFailed(format!(
                "Failed extracting text from page {page_index}: {e}"
            ))
        })?;
        Ok(text.all())
    }

    /// Extracts structured `RawLayoutBlock` objects containing spatial coordinates and font sizes for digital PDF pages.
    pub fn extract_digital_blocks(
        &self,
        pdf_bytes: &[u8],
        page_index: usize,
    ) -> Result<Vec<RawLayoutBlock>, OcrError> {
        let document = self.load_document(pdf_bytes.to_vec())?;
        Self::extract_digital_blocks_from_document(&document, page_index)
    }

    /// Extracts structured layout blocks from a page in an already-loaded PDF document.
    pub fn extract_digital_blocks_from_document(
        document: &PdfDocument<'_>,
        page_index: usize,
    ) -> Result<Vec<RawLayoutBlock>, OcrError> {
        let pages = document.pages();
        let page_idx = u16::try_from(page_index).map_err(|_| {
            OcrError::InferenceFailed(format!(
                "Page index {page_index} exceeds maximum supported value ({})",
                u16::MAX
            ))
        })?;
        let page = pages.get(page_idx).map_err(|_| {
            OcrError::InferenceFailed(format!(
                "Page index {page_index} out of bounds (total pages: {})",
                pages.len()
            ))
        })?;

        let height_pts = page.height().value;
        let mut raw_blocks = Vec::new();

        // Collect objects upfront to avoid the known pdfium-render bug where
        // chars_for_object() resets the page's object iterator mid-loop.
        let objects: Vec<_> = page.objects().iter().collect();
        let text_page = page.text().ok();

        for object in &objects {
            if let Some(text_object) = object.as_text_object() {
                let object_text = text_object.text();
                let font_size = text_object.unscaled_font_size().value;
                let bounds = match text_object.bounds() {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                // Gap-reconstruct from per-character boxes when available. pdfium's object_text
                // often already contains phantom spaces around narrow glyphs in Word/Docs PDFs,
                // so we must not prefer it just because it contains whitespace.
                let reconstructed = if let Some(ref tp) = text_page {
                    match tp.chars_for_object(text_object) {
                        Ok(chars) => {
                            let char_boxes: Vec<PdfCharBox> = chars
                                .iter()
                                .filter_map(|c| {
                                    let ch = printable_pdf_char(c.unicode_char())?;
                                    let bounds = c.loose_bounds().ok()?;
                                    Some(PdfCharBox {
                                        left: bounds.left().value,
                                        right: bounds.right().value,
                                        ch,
                                    })
                                })
                                .collect();
                            choose_text_object_reconstruction(&object_text, &char_boxes, font_size)
                        }
                        Err(_) => object_text,
                    }
                } else {
                    object_text
                };

                let reconstructed = normalize_short_text_object(&reconstructed);
                let reconstructed = sanitize_pdf_text(&reconstructed);
                if reconstructed.trim().is_empty() {
                    continue;
                }

                let pdf_left = bounds.left().value;
                let pdf_bottom = bounds.bottom().value;
                let width = bounds.width().value;
                let height = bounds.height().value;
                let screen_y = (height_pts - (pdf_bottom + height)).max(0.0);

                let bbox = Rect::new(pdf_left, screen_y, width, height);
                raw_blocks.push(RawLayoutBlock::new(reconstructed, bbox).with_font_size(font_size));
            }
        }

        // Fallback: if text object iteration returned no blocks but a digital
        // text layer exists, use page.text().all() with synthetic coordinates.
        if raw_blocks.is_empty() {
            if let Some(ref tp) = text_page {
                let full_text = tp.all();
                let page_width = page.width().value;
                let mut y_offset = 20.0;
                for line in full_text.lines() {
                    if line.trim().is_empty() {
                        y_offset += 15.0;
                        continue;
                    }
                    let bbox = Rect::new(20.0, y_offset, page_width - 40.0, 15.0);
                    raw_blocks.push(RawLayoutBlock::new(line, bbox).with_font_size(12.0));
                    y_offset += 20.0;
                }
            }
        }

        Ok(merge_text_objects_on_visual_lines(raw_blocks))
    }
}

fn printable_pdf_char(ch: Option<char>) -> Option<char> {
    let ch = ch?;
    if ch == '\t' || ch == '\n' || ch == '\r' || !ch.is_control() {
        Some(ch)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
struct PdfCharBox {
    left: f32,
    right: f32,
    ch: char,
}

/// Reconstruct spaced text from per-character boxes using object-level metrics.
///
/// Thresholding against each glyph's own width treats narrow letters (`i`, `l`) as if they
/// define the expected gap, inserting phantom spaces around them in proportional fonts.
fn reconstruct_text_from_char_boxes(char_boxes: &[PdfCharBox], font_size: f32) -> String {
    if char_boxes.is_empty() {
        return String::new();
    }

    let avg_char_width = char_boxes
        .iter()
        .map(|b| (b.right - b.left).max(0.1))
        .sum::<f32>()
        / char_boxes.len() as f32;
    // Inter-word gaps are ~¼ em; intra-letter kerning is much tighter than either heuristic.
    let space_threshold = font_size
        .mul_add(0.25, 0.0)
        .max(avg_char_width.mul_add(0.5, 0.0))
        .max(1.0);

    let mut result = String::with_capacity(char_boxes.len());
    let mut prev_right: Option<f32> = None;
    for b in char_boxes {
        if let Some(prev) = prev_right {
            let gap = b.left - prev;
            if gap > space_threshold {
                result.push(' ');
            }
        }
        result.push(b.ch);
        prev_right = Some(b.right);
    }
    result
}

/// Pick gap-reconstructed text, falling back to pdfium object text only when reconstruction
/// cannot recover word boundaries (very tight character boxes).
fn choose_text_object_reconstruction(
    object_text: &str,
    char_boxes: &[PdfCharBox],
    font_size: f32,
) -> String {
    if char_boxes.is_empty() {
        return object_text.to_string();
    }

    let from_chars = reconstruct_text_from_char_boxes(char_boxes, font_size);
    if from_chars.is_empty() {
        return object_text.to_string();
    }

    let object_has_spaces = object_text.chars().any(char::is_whitespace);
    let chars_have_spaces = from_chars.chars().any(char::is_whitespace);
    if object_has_spaces
        && !chars_have_spaces
        && from_chars.len() > 10
        && object_text.len().saturating_sub(from_chars.len()) <= object_text.len() / 4
    {
        // Tight character boxes with no measurable inter-word gaps — trust pdfium's object text.
        return object_text.to_string();
    }

    from_chars
}

/// Word/Docs PDFs often emit one glyph per text object. Interior glyphs are bare letters;
/// word-final glyphs carry trailing whitespace in `object_text` that marks the boundary.
fn normalize_short_text_object(text: &str) -> String {
    if text.chars().all(char::is_whitespace) {
        return " ".to_string();
    }
    let letters: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if letters.chars().count() <= 3 {
        if text.chars().last().is_some_and(char::is_whitespace) {
            let narrow_word_final = letters.chars().count() == 1
                && letters
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c, 'i' | 'l' | 'I' | '1'));
            if narrow_word_final {
                letters
            } else {
                format!("{letters} ")
            }
        } else {
            letters
        }
    } else {
        text.to_string()
    }
}

fn inter_block_space_threshold(prev: &RawLayoutBlock, next: &RawLayoutBlock) -> f32 {
    let prev_letters = prev.text.chars().filter(|c| !c.is_whitespace()).count();
    let next_letters = next.text.chars().filter(|c| !c.is_whitespace()).count();
    // Per-glyph Word exports: kerning gaps ~1pt, word gaps ~3–5pt.
    if prev_letters <= 2 && next_letters <= 2 {
        return 2.0;
    }

    let font_size = next.font_size.or(prev.font_size).unwrap_or_else(|| {
        average_char_width(&next.text, next.bbox.width)
            .max(average_char_width(&prev.text, prev.bbox.width))
            * 2.0
    });
    font_size.mul_add(0.25, 0.0).max(1.0)
}

fn sanitize_pdf_text(text: &str) -> String {
    text.chars()
        .filter(|&ch| ch == '\t' || ch == '\n' || ch == '\r' || !ch.is_control())
        .collect()
}

fn merge_text_objects_on_visual_lines(mut blocks: Vec<RawLayoutBlock>) -> Vec<RawLayoutBlock> {
    if blocks.len() <= 1 {
        return blocks;
    }

    blocks.sort_by(|a, b| {
        a.bbox
            .y
            .partial_cmp(&b.bbox.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.bbox
                    .x
                    .partial_cmp(&b.bbox.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    #[derive(Debug)]
    struct Line {
        center_y: f32,
        max_height: f32,
        blocks: Vec<RawLayoutBlock>,
    }

    let mut lines: Vec<Line> = Vec::new();
    for block in blocks {
        let center_y = block.bbox.y + (block.bbox.height / 2.0);
        if let Some(line) = lines.iter_mut().find(|line| {
            let tolerance = (line.max_height.min(block.bbox.height) * 0.6).max(2.0);
            (line.center_y - center_y).abs() <= tolerance
        }) {
            let line_len = line.blocks.len() as f32;
            line.center_y = ((line.center_y * line_len) + center_y) / (line_len + 1.0);
            line.max_height = line.max_height.max(block.bbox.height);
            line.blocks.push(block);
        } else {
            lines.push(Line {
                center_y,
                max_height: block.bbox.height,
                blocks: vec![block],
            });
        }
    }

    lines.sort_by(|a, b| {
        a.center_y
            .partial_cmp(&b.center_y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    lines
        .into_iter()
        .flat_map(|mut line| {
            line.blocks.sort_by(|a, b| {
                a.bbox
                    .x
                    .partial_cmp(&b.bbox.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            merge_line_blocks(line.blocks)
        })
        .collect()
}

fn merge_object_text_for_line(text: &str) -> Option<&str> {
    if text.is_empty() {
        None
    } else if text.chars().all(char::is_whitespace) {
        Some(" ")
    } else {
        Some(text.trim())
    }
}

fn merge_line_blocks(blocks: Vec<RawLayoutBlock>) -> Vec<RawLayoutBlock> {
    let mut merged_blocks = Vec::new();
    let mut current_run = Vec::new();
    let mut prev_right: Option<f32> = None;
    let mut prev_block: Option<RawLayoutBlock> = None;

    for block in blocks {
        let Some(_text) = merge_object_text_for_line(&block.text) else {
            continue;
        };

        if let (Some(prev), Some(prev_block)) = (prev_right, prev_block.as_ref()) {
            let gap = block.bbox.x - prev;
            let column_split_threshold = inter_block_space_threshold(prev_block, &block)
                .mul_add(8.0, 0.0)
                .max(24.0);
            if gap > column_split_threshold && !current_run.is_empty() {
                if let Some(merged) = merge_nearby_line_run(std::mem::take(&mut current_run)) {
                    merged_blocks.push(merged);
                }
            }
        }

        prev_right = Some(block.bbox.x + block.bbox.width);
        prev_block = Some(block.clone());
        current_run.push(block);
    }

    if let Some(merged) = merge_nearby_line_run(current_run) {
        merged_blocks.push(merged);
    }

    merged_blocks
}

fn merge_nearby_line_run(blocks: Vec<RawLayoutBlock>) -> Option<RawLayoutBlock> {
    let mut merged_text = String::new();
    let mut prev_right: Option<f32> = None;
    let mut prev_block: Option<RawLayoutBlock> = None;
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_right = f32::MIN;
    let mut max_bottom = f32::MIN;
    let mut font_sum = 0.0f32;
    let mut font_count = 0usize;

    for block in blocks {
        let Some(text) = merge_object_text_for_line(&block.text) else {
            continue;
        };

        if let Some(prev_block) = prev_block.as_ref() {
            if let Some(prev) = prev_right {
                let gap = block.bbox.x - prev;
                let threshold = inter_block_space_threshold(prev_block, &block);
                if gap > threshold
                    && !merged_text.ends_with(char::is_whitespace)
                    && !text.starts_with(char::is_whitespace)
                {
                    merged_text.push(' ');
                }
            }
        }
        merged_text.push_str(text);

        min_x = min_x.min(block.bbox.x);
        min_y = min_y.min(block.bbox.y);
        max_right = max_right.max(block.bbox.x + block.bbox.width);
        max_bottom = max_bottom.max(block.bbox.y + block.bbox.height);
        prev_right = Some(prev_right.map_or(block.bbox.x + block.bbox.width, |right| {
            right.max(block.bbox.x + block.bbox.width)
        }));
        prev_block = Some(block.clone());

        if let Some(font_size) = block.font_size {
            font_sum += font_size;
            font_count += 1;
        }
    }

    if merged_text.trim().is_empty() {
        return None;
    }

    let bbox = Rect::new(min_x, min_y, max_right - min_x, max_bottom - min_y);
    let mut merged = RawLayoutBlock::new(merged_text, bbox);
    if font_count > 0 {
        merged.font_size = Some(font_sum / font_count as f32);
    }
    Some(merged)
}

fn average_char_width(text: &str, width: f32) -> f32 {
    let char_count = text.chars().filter(|ch| !ch.is_whitespace()).count().max(1) as f32;
    (width / char_count).max(0.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_short_text_object_strips_per_glyph_spaces() {
        assert_eq!(normalize_short_text_object("i "), "i");
        assert_eq!(normalize_short_text_object("s "), "s ");
        assert_eq!(normalize_short_text_object("d "), "d ");
        assert_eq!(normalize_short_text_object(" "), " ");
        assert_eq!(normalize_short_text_object("Th"), "Th");
        assert_eq!(normalize_short_text_object("This report"), "This report");
    }

    #[test]
    fn test_pdf_page_type_display() {
        assert_eq!(PdfPageType::Digital.to_string(), "digital");
        assert_eq!(PdfPageType::Ocr.to_string(), "ocr-bundled");
        assert_eq!(PdfPageType::Hybrid.to_string(), "hybrid");
    }

    #[test]
    fn test_pdf_rasterizer_config_default() {
        let config = PdfRasterizerConfig::default();
        assert_eq!(config.dpi, 300);
    }

    #[test]
    fn test_sanitize_pdf_text_filters_control_chars() {
        assert_eq!(sanitize_pdf_text("Sen\u{0002}ate"), "Senate");
    }

    #[test]
    fn test_reconstruct_text_keeps_narrow_letters_unspaced() {
        // Proportional sans-serif at 12pt: `i`/`l` boxes are much narrower than average.
        let font_size = 12.0;
        let boxes = vec![
            PdfCharBox {
                left: 0.0,
                right: 7.0,
                ch: 'C',
            },
            PdfCharBox {
                left: 7.0,
                right: 13.0,
                ch: 'o',
            },
            PdfCharBox {
                left: 13.0,
                right: 19.0,
                ch: 'n',
            },
            PdfCharBox {
                left: 19.0,
                right: 24.0,
                ch: 't',
            },
            PdfCharBox {
                left: 24.0,
                right: 29.0,
                ch: 'r',
            },
            PdfCharBox {
                left: 29.0,
                right: 31.0,
                ch: 'i',
            },
            PdfCharBox {
                left: 31.0,
                right: 37.0,
                ch: 'b',
            },
            PdfCharBox {
                left: 37.0,
                right: 43.0,
                ch: 'u',
            },
            PdfCharBox {
                left: 43.0,
                right: 49.0,
                ch: 't',
            },
            PdfCharBox {
                left: 49.0,
                right: 53.0,
                ch: 'i',
            },
            PdfCharBox {
                left: 53.0,
                right: 59.0,
                ch: 'o',
            },
            PdfCharBox {
                left: 59.0,
                right: 65.0,
                ch: 'n',
            },
            PdfCharBox {
                left: 65.0,
                right: 71.0,
                ch: 's',
            },
        ];

        assert_eq!(
            reconstruct_text_from_char_boxes(&boxes, font_size),
            "Contributions"
        );
    }

    #[test]
    fn test_reconstruct_text_inserts_word_spaces() {
        let font_size = 12.0;
        let mut boxes = vec![
            PdfCharBox {
                left: 0.0,
                right: 6.0,
                ch: 'T',
            },
            PdfCharBox {
                left: 6.0,
                right: 12.0,
                ch: 'h',
            },
            PdfCharBox {
                left: 12.0,
                right: 14.0,
                ch: 'i',
            },
            PdfCharBox {
                left: 14.0,
                right: 19.0,
                ch: 's',
            },
        ];
        // ~¼ em word gap before "report".
        let word_gap = font_size * 0.3;
        let next_left = 19.0 + word_gap;
        boxes.extend([
            PdfCharBox {
                left: next_left,
                right: next_left + 6.0,
                ch: 'r',
            },
            PdfCharBox {
                left: next_left + 6.0,
                right: next_left + 12.0,
                ch: 'e',
            },
            PdfCharBox {
                left: next_left + 12.0,
                right: next_left + 18.0,
                ch: 'p',
            },
            PdfCharBox {
                left: next_left + 18.0,
                right: next_left + 24.0,
                ch: 'o',
            },
            PdfCharBox {
                left: next_left + 24.0,
                right: next_left + 30.0,
                ch: 'r',
            },
            PdfCharBox {
                left: next_left + 30.0,
                right: next_left + 35.0,
                ch: 't',
            },
        ]);

        assert_eq!(
            reconstruct_text_from_char_boxes(&boxes, font_size),
            "This report"
        );
    }

    #[test]
    fn test_reconstruct_text_old_threshold_would_break_narrow_letters() {
        // Typical kerning gaps (~0.5–1pt) exceed 0.18× a narrow glyph's width.
        let font_size = 12.0;
        let boxes = vec![
            PdfCharBox {
                left: 0.0,
                right: 6.0,
                ch: 'T',
            },
            PdfCharBox {
                left: 6.5,
                right: 12.0,
                ch: 'h',
            },
            PdfCharBox {
                left: 12.5,
                right: 14.5,
                ch: 'i',
            },
            PdfCharBox {
                left: 15.5,
                right: 20.0,
                ch: 's',
            },
        ];

        let mut broken = String::new();
        let mut prev_right: Option<f32> = None;
        for b in &boxes {
            if let Some(prev) = prev_right {
                let gap = b.left - prev;
                let char_width = (b.right - b.left).max(0.1);
                if gap > 0.18 * char_width {
                    broken.push(' ');
                }
            }
            broken.push(b.ch);
            prev_right = Some(b.right);
        }
        assert_eq!(broken, "Th i s");

        assert_eq!(reconstruct_text_from_char_boxes(&boxes, font_size), "This");
    }

    #[test]
    fn test_merge_text_objects_on_visual_lines_joins_word_fragments() {
        let blocks = vec![
            RawLayoutBlock::new("Mem", Rect::new(10.0, 20.0, 18.0, 8.0)).with_font_size(8.0),
            RawLayoutBlock::new("bers", Rect::new(28.2, 20.1, 24.0, 8.0)).with_font_size(8.0),
            RawLayoutBlock::new("chosen", Rect::new(70.0, 20.0, 36.0, 8.0)).with_font_size(8.0),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Members chosen");
    }

    #[test]
    fn test_merge_text_objects_on_visual_lines_keeps_columns_separate() {
        let blocks = vec![
            RawLayoutBlock::new("Left column", Rect::new(10.0, 20.0, 60.0, 8.0))
                .with_font_size(8.0),
            RawLayoutBlock::new("Right column", Rect::new(220.0, 20.0, 70.0, 8.0))
                .with_font_size(8.0),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "Left column");
        assert_eq!(merged[1].text, "Right column");
    }
}
