use crate::ocr::engine::OcrError;
use crate::ocr::ocr_models_dir;
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
    ) -> Result<Vec<crate::ingest::layout::RawLayoutBlock>, OcrError> {
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
                                    if let Some(ch) = c.unicode_char() {
                                        result.push(ch);
                                    }
                                    prev_right = Some(char_right);
                                } else if let Some(ch) = c.unicode_char() {
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

                if reconstructed.trim().is_empty() {
                    continue;
                }

                let pdf_left = bounds.left().value;
                let pdf_bottom = bounds.bottom().value;
                let width = bounds.width().value;
                let height = bounds.height().value;
                let screen_y = (height_pts - (pdf_bottom + height)).max(0.0);

                let bbox = crate::ocr::engine::Rect::new(pdf_left, screen_y, width, height);
                raw_blocks.push(
                    crate::ingest::layout::RawLayoutBlock::new(reconstructed, bbox)
                        .with_font_size(font_size),
                );
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
                    let bbox =
                        crate::ocr::engine::Rect::new(20.0, y_offset, page_width - 40.0, 15.0);
                    raw_blocks.push(
                        crate::ingest::layout::RawLayoutBlock::new(line, bbox).with_font_size(12.0),
                    );
                    y_offset += 20.0;
                }
            }
        }

        Ok(raw_blocks)
    }
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
}
