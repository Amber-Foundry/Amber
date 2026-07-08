use crate::ocr::engine::OcrError;
use crate::ocr::ocr_models_dir;
use crate::{ingest::layout::RawLayoutBlock, ocr::engine::Rect};
use image::DynamicImage;
use pdfium_render::prelude::*;
use std::fs::File;
use std::io::Read;
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
                let ocr_dir = ocr_models_dir().map_err(|e| OcrError::IoError(e.to_string()))?;
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

    /// Scans a PDF file on disk and returns metadata for all pages.
    pub fn scan_file(&self, file_path: &Path) -> Result<Vec<PdfPageInfo>, OcrError> {
        let mut file = File::open(file_path).map_err(|e| {
            OcrError::IoError(format!(
                "Failed to open PDF file at {}: {e}",
                file_path.display()
            ))
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| {
            OcrError::IoError(format!(
                "Failed to read PDF file at {}: {e}",
                file_path.display()
            ))
        })?;
        self.scan_document(&bytes)
    }

    /// Scans a PDF document from bytes and returns metadata and page classification for all pages.
    pub fn scan_document(&self, pdf_bytes: &[u8]) -> Result<Vec<PdfPageInfo>, OcrError> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| OcrError::InferenceFailed(format!("Failed to load PDF document: {e}")))?;

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
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| OcrError::InferenceFailed(format!("Failed loading PDF document: {e}")))?;

        let pages = document.pages();
        let page = pages.get(page_index as u16).map_err(|_| {
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

        let image = bitmap.as_image();

        Ok(image)
    }

    /// Extracts digital text layer from a PDF page if available.
    pub fn extract_digital_text(
        &self,
        pdf_bytes: &[u8],
        page_index: usize,
    ) -> Result<String, OcrError> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| OcrError::InferenceFailed(format!("Failed loading PDF document: {e}")))?;

        let pages = document.pages();
        let page = pages.get(page_index as u16).map_err(|_| {
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
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| OcrError::InferenceFailed(format!("Failed loading PDF document: {e}")))?;

        let pages = document.pages();
        let page = pages.get(page_index as u16).map_err(|_| {
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

                // Prefer pdfium's object text when it already preserves spaces.
                // Some PDFs expose character boxes that are too tight for reliable
                // gap-based word reconstruction even though object text is usable.
                let reconstructed = if object_text.chars().any(char::is_whitespace) {
                    object_text
                } else if let Some(ref tp) = text_page {
                    match tp.chars_for_object(text_object) {
                        Ok(chars) => {
                            let mut result = String::new();
                            let mut prev_right: Option<f32> = None;
                            for c in chars.iter() {
                                if let Ok(char_bounds) = c.loose_bounds() {
                                    let char_left = char_bounds.left().value;
                                    let char_right = char_bounds.right().value;
                                    let char_width = (char_right - char_left).max(0.1);
                                    if let Some(prev) = prev_right {
                                        let gap = char_left - prev;
                                        if gap > 0.18 * char_width {
                                            result.push(' ');
                                        }
                                    }
                                    if let Some(ch) = printable_pdf_char(c.unicode_char()) {
                                        result.push(ch);
                                    }
                                    prev_right = Some(char_right);
                                } else if let Some(ch) = printable_pdf_char(c.unicode_char()) {
                                    result.push(ch);
                                }
                            }
                            result
                        }
                        Err(_) => object_text,
                    }
                } else {
                    object_text
                };

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

        Ok(merge_text_objects_on_visual_lines(
            raw_blocks,
            page.width().value,
        ))
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

fn sanitize_pdf_text(text: &str) -> String {
    text.chars()
        .filter(|&ch| ch == '\t' || ch == '\n' || ch == '\r' || !ch.is_control())
        .collect()
}

fn merge_text_objects_on_visual_lines(
    mut blocks: Vec<RawLayoutBlock>,
    page_width: f32,
) -> Vec<RawLayoutBlock> {
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
            merge_line_blocks(line.blocks, page_width)
        })
        .collect()
}

fn merge_line_blocks(blocks: Vec<RawLayoutBlock>, page_width: f32) -> Vec<RawLayoutBlock> {
    let mut merged_blocks = Vec::new();
    let mut current_run = Vec::new();
    let mut prev_right: Option<f32> = None;
    let mut prev_char_width: Option<f32> = None;

    for block in blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

        let char_width = average_char_width(text, block.bbox.width);
        if let Some(prev) = prev_right {
            let gap = block.bbox.x - prev;
            let crosses_midpoint = prev < page_width / 2.0
                && block.bbox.x + (block.bbox.width / 2.0) > page_width / 2.0;
            let column_split_threshold = prev_char_width
                .map(|prev_width| prev_width.min(char_width))
                .unwrap_or(char_width)
                .mul_add(8.0, 0.0)
                .max(24.0);
            if (gap > column_split_threshold || (crosses_midpoint && gap > 4.0))
                && !current_run.is_empty()
            {
                if let Some(merged) = merge_nearby_line_run(std::mem::take(&mut current_run)) {
                    merged_blocks.push(merged);
                }
            }
        }

        prev_right = Some(block.bbox.x + block.bbox.width);
        prev_char_width = Some(char_width);
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
    let mut prev_char_width: Option<f32> = None;
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_right = f32::MIN;
    let mut max_bottom = f32::MIN;
    let mut font_sum = 0.0f32;
    let mut font_count = 0usize;

    for block in blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

        let char_width = average_char_width(text, block.bbox.width);
        if let Some(prev) = prev_right {
            let gap = block.bbox.x - prev;
            let threshold = prev_char_width
                .map(|prev_width| prev_width.min(char_width))
                .unwrap_or(char_width)
                .mul_add(0.5, 0.0)
                .max(1.0);
            if gap > threshold
                && !merged_text.ends_with(char::is_whitespace)
                && !text.starts_with(char::is_whitespace)
            {
                merged_text.push(' ');
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
        prev_char_width = Some(char_width);

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
    fn test_merge_text_objects_on_visual_lines_joins_word_fragments() {
        let blocks = vec![
            RawLayoutBlock::new("Mem", Rect::new(10.0, 20.0, 18.0, 8.0)).with_font_size(8.0),
            RawLayoutBlock::new("bers", Rect::new(28.2, 20.1, 24.0, 8.0)).with_font_size(8.0),
            RawLayoutBlock::new("chosen", Rect::new(70.0, 20.0, 36.0, 8.0)).with_font_size(8.0),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks, 300.0);

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

        let merged = merge_text_objects_on_visual_lines(blocks, 300.0);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "Left column");
        assert_eq!(merged[1].text, "Right column");
    }
}
