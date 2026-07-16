use crate::ingest::text::{
    attaches_to_previous_char, average_char_width, has_spurious_punctuation_spacing,
    is_decimal_continuation, is_decimal_continuation_char, is_punctuation_only,
    should_insert_inter_object_space, word_gap_threshold, InterObjectJoinContext,
};
use crate::ocr::engine::OcrError;
use crate::ocr::ocr_models_dir;
use crate::{ingest::layout::RawLayoutBlock, ocr::engine::Rect};
use image::DynamicImage;
use pdfium_render::prelude::*;
use std::collections::HashSet;
use std::path::Path;

/// Maximum nesting depth when walking Form XObject trees during page classification.
const FORM_SCAN_MAX_DEPTH: usize = 8;

/// Ignore nested images smaller than this fraction of page area (logos / tracking pixels).
/// No raster figure found in US Traffic to calibrate against; using documented floor
/// pending a real calibration source.
const MIN_SIGNIFICANT_IMAGE_AREA_RATIO: f32 = 0.0005;

/// Best-effort identity for cycle detection on the current DFS ancestor path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FormSig {
    len: u16,
    matrix_a: u32,
    matrix_b: u32,
    matrix_c: u32,
    matrix_d: u32,
    matrix_e: u32,
    matrix_f: u32,
}

fn form_signature(form: &PdfPageXObjectFormObject<'_>) -> FormSig {
    let matrix = form.matrix().unwrap_or(PdfMatrix::IDENTITY);
    FormSig {
        len: form.len().min(u16::MAX as usize) as u16,
        matrix_a: matrix.a().to_bits(),
        matrix_b: matrix.b().to_bits(),
        matrix_c: matrix.c().to_bits(),
        matrix_d: matrix.d().to_bits(),
        matrix_e: matrix.e().to_bits(),
        matrix_f: matrix.f().to_bits(),
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct PageContentScan {
    has_text: bool,
    has_significant_images: bool,
    qualifying_image_count: usize,
    ignored_image_count: usize,
    smallest_ignored_image_area_ratio: f32,
    largest_ignored_image_area_ratio: f32,
    largest_image_area_pts2: f32,
    largest_image_area_ratio: f32,
    max_depth_reached: bool,
    forms_skipped_cycle: usize,
}

enum ScanStackItem<'a> {
    Object {
        object: PdfPageObject<'a>,
        depth: usize,
    },
    ExitForm(FormSig),
}

fn image_bounds_area_pts2(object: &PdfPageObject<'_>) -> f32 {
    object
        .bounds()
        .ok()
        .map(|bounds| bounds.width().value * bounds.height().value)
        .unwrap_or(0.0)
        .max(0.0)
}

fn record_image_area(
    scan: &mut PageContentScan,
    area_pts2: f32,
    page_area_pts2: f32,
    min_area_pts2: f32,
) {
    if area_pts2 <= 0.0 || page_area_pts2 <= 0.0 {
        return;
    }
    let ratio = area_pts2 / page_area_pts2;
    if area_pts2 >= min_area_pts2 {
        scan.has_significant_images = true;
        scan.qualifying_image_count += 1;
        if area_pts2 > scan.largest_image_area_pts2 {
            scan.largest_image_area_pts2 = area_pts2;
            scan.largest_image_area_ratio = ratio;
        }
    } else {
        scan.ignored_image_count += 1;
        if scan.ignored_image_count == 1 {
            scan.smallest_ignored_image_area_ratio = ratio;
            scan.largest_ignored_image_area_ratio = ratio;
        } else {
            scan.smallest_ignored_image_area_ratio =
                scan.smallest_ignored_image_area_ratio.min(ratio);
            scan.largest_ignored_image_area_ratio =
                scan.largest_ignored_image_area_ratio.max(ratio);
        }
    }
}

fn enter_xobject_form<'a>(
    stack: &mut Vec<ScanStackItem<'a>>,
    path_forms: &mut HashSet<FormSig>,
    scan: &mut PageContentScan,
    object: PdfPageObject<'a>,
    depth: usize,
) {
    if depth >= FORM_SCAN_MAX_DEPTH {
        scan.max_depth_reached = true;
        return;
    }

    let PdfPageObject::XObjectForm(ref form) = object else {
        return;
    };

    let sig = form_signature(form);
    if path_forms.contains(&sig) {
        scan.forms_skipped_cycle += 1;
        return;
    }
    path_forms.insert(sig);

    let len = form.len();
    // INVARIANT: ExitForm(sig) before children; each child at depth + 1.
    stack.push(ScanStackItem::ExitForm(sig));
    for index in (0..len).rev() {
        if let Ok(child) = PdfPageObjectsCommon::get(form, index) {
            stack.push(ScanStackItem::Object {
                object: child,
                depth: depth + 1,
            });
        }
    }
}

fn scan_page_content(page: &PdfPage<'_>, page_area_pts2: f32) -> PageContentScan {
    let min_area_pts2 = page_area_pts2 * MIN_SIGNIFICANT_IMAGE_AREA_RATIO;
    let mut scan = PageContentScan::default();
    let mut stack: Vec<ScanStackItem<'_>> = Vec::new();
    let mut path_forms: HashSet<FormSig> = HashSet::new();

    let top_level: Vec<PdfPageObject<'_>> = page.objects().iter().collect();
    for object in top_level.into_iter().rev() {
        stack.push(ScanStackItem::Object { object, depth: 0 });
    }

    while let Some(item) = stack.pop() {
        match item {
            ScanStackItem::ExitForm(sig) => {
                path_forms.remove(&sig);
            }
            ScanStackItem::Object { object, depth } => match object.object_type() {
                PdfPageObjectType::Text => {
                    scan.has_text = true;
                }
                PdfPageObjectType::Image => {
                    record_image_area(
                        &mut scan,
                        image_bounds_area_pts2(&object),
                        page_area_pts2,
                        min_area_pts2,
                    );
                }
                PdfPageObjectType::XObjectForm => {
                    enter_xobject_form(&mut stack, &mut path_forms, &mut scan, object, depth);
                }
                PdfPageObjectType::Path
                | PdfPageObjectType::Shading
                | PdfPageObjectType::Unsupported => {}
            },
        }
    }

    scan
}

fn classify_page_type(has_text: bool, has_images: bool) -> PdfPageType {
    match (has_text, has_images) {
        (true, true) => PdfPageType::Hybrid,
        (true, false) => PdfPageType::Digital,
        (false, true) => PdfPageType::Ocr,
        (false, false) => PdfPageType::Digital,
    }
}

fn page_scan_debug_enabled() -> bool {
    std::env::var("AMBER_PAGE_SCAN_DEBUG")
        .ok()
        .is_some_and(|v| v == "1")
}

fn log_page_scan_debug(
    page_index: usize,
    page_type: PdfPageType,
    scan: &PageContentScan,
    page_area_pts2: f32,
) {
    if !page_scan_debug_enabled() {
        return;
    }

    eprintln!(
        "[page-scan] page={page_index} type={page_type} text={} images={} largest_area={:.2}% depth_cap={} cycles_skipped={}",
        scan.has_text,
        scan.qualifying_image_count,
        scan.largest_image_area_ratio * 100.0,
        scan.max_depth_reached,
        scan.forms_skipped_cycle,
    );

    if scan.ignored_image_count > 0 && page_area_pts2 > 0.0 {
        eprintln!(
            "[page-scan] page={page_index} ignored_images={} threshold={:.4}% largest_ignored={:.4}% smallest_ignored={:.4}%",
            scan.ignored_image_count,
            MIN_SIGNIFICANT_IMAGE_AREA_RATIO * 100.0,
            scan.largest_ignored_image_area_ratio * 100.0,
            scan.smallest_ignored_image_area_ratio * 100.0,
        );
    }
}

#[cfg(test)]
mod form_scan_logic {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct TestSig(u8);

    #[derive(Debug, Clone, Copy)]
    enum TestItem {
        Enter(TestSig),
        Child,
        Exit(TestSig),
    }

    fn simulate_path_guard(items: &[TestItem]) -> (usize, usize, bool) {
        let mut path: HashSet<TestSig> = HashSet::new();
        let mut stack: Vec<TestItem> = items.iter().copied().rev().collect();
        let mut visited_children = 0usize;
        let mut skipped_cycles = 0usize;

        while let Some(item) = stack.pop() {
            match item {
                TestItem::Exit(sig) => {
                    path.remove(&sig);
                }
                TestItem::Enter(sig) => {
                    if path.contains(&sig) {
                        skipped_cycles += 1;
                        continue;
                    }
                    path.insert(sig);
                    stack.push(TestItem::Exit(sig));
                }
                TestItem::Child => {
                    visited_children += 1;
                }
            }
        }

        (visited_children, skipped_cycles, path.is_empty())
    }

    #[test]
    fn exit_marker_pops_path_only_after_subtree() {
        let sig = TestSig(1);
        let (visited, skipped, empty) = simulate_path_guard(&[
            TestItem::Enter(sig),
            TestItem::Child,
            TestItem::Child,
            TestItem::Exit(sig),
        ]);
        assert_eq!(visited, 2);
        assert_eq!(skipped, 0);
        assert!(empty);
    }

    #[test]
    fn identical_sibling_forms_both_visited() {
        let sig = TestSig(7);
        let (visited, skipped, empty) = simulate_path_guard(&[
            TestItem::Enter(sig),
            TestItem::Child,
            TestItem::Exit(sig),
            TestItem::Enter(sig),
            TestItem::Child,
            TestItem::Exit(sig),
        ]);
        assert_eq!(visited, 2);
        assert_eq!(skipped, 0);
        assert!(empty);
    }

    #[test]
    fn cycle_on_active_path_is_skipped() {
        let sig = FormSig {
            len: 1,
            matrix_a: 0,
            matrix_b: 0,
            matrix_c: 0,
            matrix_d: 0,
            matrix_e: 0,
            matrix_f: 0,
        };
        let mut path = HashSet::new();
        path.insert(sig);
        assert!(path.contains(&sig));
        let skipped = if path.contains(&sig) { 1 } else { 0 };
        path.remove(&sig);
        assert_eq!(skipped, 1);
        assert!(path.is_empty());
    }

    #[test]
    fn image_area_threshold_counts_qualifying_only() {
        let mut scan = PageContentScan::default();
        record_image_area(&mut scan, 50.0, 100_000.0, 100.0);
        record_image_area(&mut scan, 500.0, 100_000.0, 100.0);
        assert!(scan.has_significant_images);
        assert_eq!(scan.qualifying_image_count, 1);
        assert_eq!(scan.ignored_image_count, 1);
        assert_eq!(scan.largest_image_area_pts2, 500.0);
        assert!((scan.smallest_ignored_image_area_ratio - 0.0005).abs() < f32::EPSILON);
        assert!((scan.largest_ignored_image_area_ratio - 0.0005).abs() < f32::EPSILON);
    }

    #[test]
    fn ignored_image_area_ratio_tracks_min_and_max() {
        let mut scan = PageContentScan::default();
        let page_area = 100_000.0;
        let min_area = 100.0;
        record_image_area(&mut scan, 10.0, page_area, min_area);
        record_image_area(&mut scan, 80.0, page_area, min_area);
        record_image_area(&mut scan, 40.0, page_area, min_area);
        assert_eq!(scan.ignored_image_count, 3);
        assert!((scan.smallest_ignored_image_area_ratio - 0.0001).abs() < f32::EPSILON);
        assert!((scan.largest_ignored_image_area_ratio - 0.0008).abs() < f32::EPSILON);
    }

    #[test]
    fn classify_routing_table() {
        assert_eq!(classify_page_type(true, true), PdfPageType::Hybrid);
        assert_eq!(classify_page_type(true, false), PdfPageType::Digital);
        assert_eq!(classify_page_type(false, true), PdfPageType::Ocr);
        assert_eq!(classify_page_type(false, false), PdfPageType::Digital);
    }
}

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
            let page_area_pts2 = width_pts * height_pts;

            let scan = scan_page_content(&page, page_area_pts2);
            let page_type = classify_page_type(scan.has_text, scan.has_significant_images);
            log_page_scan_debug(idx, page_type, &scan, page_area_pts2);

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
        let extract_debug = std::env::var("AMBER_EXTRACT_DEBUG")
            .ok()
            .is_some_and(|v| v == "1");

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
                            let text = choose_text_object_reconstruction(
                                &object_text,
                                &char_boxes,
                                font_size,
                            );
                            let pdf_left = bounds.left().value;
                            let pdf_bottom = bounds.bottom().value;
                            let width = bounds.width().value;
                            let height = bounds.height().value;
                            let screen_y = (height_pts - (pdf_bottom + height)).max(0.0);
                            let split_blocks = split_text_object_at_column_gutter(
                                &char_boxes,
                                font_size,
                                page.width().value,
                                Rect::new(pdf_left, screen_y, width, height),
                            );
                            if !split_blocks.is_empty() {
                                raw_blocks.extend(split_blocks);
                                continue;
                            }
                            text
                        }
                        Err(_) => object_text.clone(),
                    }
                } else {
                    object_text.clone()
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

                if extract_debug {
                    eprintln!(
                        "[extract] object_text={object_text:?} reconstructed={reconstructed:?} \
                         bbox=({pdf_left:.1},{screen_y:.1},{width:.1},{height:.1})"
                    );
                }

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

        if extract_debug {
            eprintln!("[extract] pre-merge blocks={}", raw_blocks.len());
            for (idx, block) in raw_blocks.iter().enumerate() {
                eprintln!("  [{idx}] {:?}", block.text);
            }
        }

        let merged = merge_text_objects_on_visual_lines(raw_blocks, page.width().value);

        if extract_debug {
            eprintln!("[extract] post-merge blocks={}", merged.len());
            for (idx, block) in merged.iter().enumerate() {
                eprintln!("  [{idx}] {:?}", block.text);
            }
        }

        Ok(merged)
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
    let space_threshold = word_gap_threshold(font_size).max(avg_char_width.mul_add(0.5, 0.0));

    let mut result = String::with_capacity(char_boxes.len());
    let mut prev_right: Option<f32> = None;
    for b in char_boxes {
        if let Some(prev) = prev_right {
            let gap = b.left - prev;
            if gap > space_threshold
                && !attaches_to_previous_char(b.ch)
                && !is_decimal_continuation_char(&result, b.ch)
            {
                result.push(' ');
            }
        }
        result.push(b.ch);
        prev_right = Some(b.right);
    }
    result
}

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
        && has_spurious_punctuation_spacing(object_text)
        && !has_spurious_punctuation_spacing(&from_chars)
    {
        return from_chars;
    }
    if object_has_spaces
        && !chars_have_spaces
        && from_chars.len() > 10
        && object_text.len().saturating_sub(from_chars.len()) <= object_text.len() / 4
    {
        // Tight character boxes with no measurable inter-word gaps — trust pdfium's object text.
        return object_text.to_string();
    }

    let letters: String = object_text.chars().filter(|c| !c.is_whitespace()).collect();
    if letters.chars().count() <= 3
        && object_text.chars().last().is_some_and(char::is_whitespace)
        && from_chars.trim() == letters
    {
        return format!("{letters} ");
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
        if is_punctuation_only(&letters) {
            return letters;
        }
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

/// Splits one pdfium text object into separate column blocks when glyph boxes bridge a gutter.
fn split_text_object_at_column_gutter(
    char_boxes: &[PdfCharBox],
    font_size: f32,
    page_width: f32,
    bbox: Rect,
) -> Vec<RawLayoutBlock> {
    if char_boxes.len() < 8 || bbox.width < page_width * 0.45 {
        return Vec::new();
    }

    let mut sorted_boxes = char_boxes.to_vec();
    sorted_boxes.sort_by(|a, b| {
        a.left
            .partial_cmp(&b.left)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let char_boxes = &sorted_boxes;

    let avg_char_width = char_boxes
        .iter()
        .map(|b| (b.right - b.left).max(0.1))
        .sum::<f32>()
        / char_boxes.len() as f32;
    let min_gutter = (font_size * 2.0)
        .max(page_width * 0.03)
        .max(avg_char_width * 4.0);

    let mut best_gap: Option<(usize, f32)> = None;
    for idx in 0..char_boxes.len().saturating_sub(1) {
        let gap = char_boxes[idx + 1].left - char_boxes[idx].right;
        if gap >= min_gutter {
            let gap_width = gap;
            match best_gap {
                Some((_, best_width)) if gap_width <= best_width => {}
                _ => best_gap = Some((idx, gap_width)),
            }
        }
    }
    let Some((split_idx, _)) = best_gap else {
        return Vec::new();
    };

    let left_boxes = &char_boxes[..=split_idx];
    let right_boxes = &char_boxes[split_idx + 1..];
    if left_boxes.len() < 3 || right_boxes.len() < 3 {
        return Vec::new();
    }

    let left_text = sanitize_pdf_text(&reconstruct_text_from_char_boxes(left_boxes, font_size));
    let right_text = sanitize_pdf_text(&reconstruct_text_from_char_boxes(right_boxes, font_size));
    if left_text.trim().is_empty() || right_text.trim().is_empty() {
        return Vec::new();
    }

    let left_bbox = bbox_from_char_boxes(left_boxes, bbox.y, bbox.height);
    let right_bbox = bbox_from_char_boxes(right_boxes, bbox.y, bbox.height);
    vec![
        RawLayoutBlock::new(left_text, left_bbox).with_font_size(font_size),
        RawLayoutBlock::new(right_text, right_bbox).with_font_size(font_size),
    ]
}

fn bbox_from_char_boxes(char_boxes: &[PdfCharBox], top: f32, height: f32) -> Rect {
    let min_x = char_boxes
        .iter()
        .map(|b| b.left)
        .fold(f32::INFINITY, f32::min);
    let max_x = char_boxes
        .iter()
        .map(|b| b.right)
        .fold(f32::NEG_INFINITY, f32::max);
    Rect::new(min_x, top, (max_x - min_x).max(0.1), height.max(0.1))
}

fn inter_block_space_threshold(prev: &RawLayoutBlock, next: &RawLayoutBlock) -> f32 {
    let font_size = next.font_size.or(prev.font_size).unwrap_or_else(|| {
        average_char_width(&next.text, next.bbox.width)
            .max(average_char_width(&prev.text, prev.bbox.width))
            * 2.0
    });
    word_gap_threshold(font_size)
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
        let punct_fragment = is_punctuation_only(block.text.trim());
        if let Some(line) = lines.iter_mut().rev().find(|line| {
            let mut tolerance = (line.max_height.min(block.bbox.height) * 0.6).max(2.0);
            if punct_fragment {
                tolerance = tolerance.max(8.0);
            }
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

fn merge_object_text_for_line(text: &str) -> Option<&str> {
    if text.is_empty() {
        None
    } else if text.chars().all(char::is_whitespace) {
        Some(" ")
    } else {
        let trimmed_start = text.trim_start();
        if trimmed_start.is_empty() {
            Some(" ")
        } else {
            // Keep trailing whitespace — Word/Docs per-glyph exports mark word ends with it.
            Some(trimmed_start)
        }
    }
}

fn merge_line_blocks(blocks: Vec<RawLayoutBlock>, page_width: f32) -> Vec<RawLayoutBlock> {
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
            let column_split_threshold = line_run_split_threshold(prev_block, &block, page_width);
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

/// Preserves normal word-fragment runs while splitting adjacent line-length objects into
/// columns. PDF producers commonly use one object per visual column line, where a gutter
/// can be only one to two font-heights wide.
fn line_run_split_threshold(prev: &RawLayoutBlock, next: &RawLayoutBlock, page_width: f32) -> f32 {
    let gap = next.bbox.x - (prev.bbox.x + prev.bbox.width);
    if page_width > 0.0 && gap >= page_width * 0.06 {
        return 0.0;
    }
    let font_size = next.font_size.or(prev.font_size).unwrap_or_else(|| {
        average_char_width(&next.text, next.bbox.width)
            .max(average_char_width(&prev.text, prev.bbox.width))
            * 2.0
    });
    if is_line_length_object(prev, font_size) || is_line_length_object(next, font_size) {
        return font_size.max(1.0);
    }
    inter_block_space_threshold(prev, next)
        .mul_add(8.0, 0.0)
        .max(24.0)
}

fn is_line_length_object(block: &RawLayoutBlock, font_size: f32) -> bool {
    block.text.split_whitespace().count() >= 3 && block.bbox.width >= font_size.max(1.0) * 12.0
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
                let font_size = block.font_size.or(prev_block.font_size).unwrap_or_else(|| {
                    average_char_width(&block.text, block.bbox.width)
                        .max(average_char_width(&prev_block.text, prev_block.bbox.width))
                        * 2.0
                });
                let immediate_prev = merge_object_text_for_line(&prev_block.text).unwrap_or("");
                let accumulated_prev = merged_text.trim_end();
                let next_trim = text.trim_start();
                let prev_join_text = if is_decimal_continuation(accumulated_prev, next_trim) {
                    accumulated_prev
                } else {
                    immediate_prev
                };
                let join_ctx = InterObjectJoinContext {
                    prev_text: prev_join_text,
                    next_text: text,
                    gap,
                    font_size,
                    prev_bbox: Some(&prev_block.bbox),
                    next_bbox: Some(&block.bbox),
                };
                let insert_space = should_insert_inter_object_space(&join_ctx);
                if std::env::var("AMBER_EXTRACT_DEBUG")
                    .ok()
                    .is_some_and(|v| v == "1")
                {
                    eprintln!(
                        "join {:?}+{:?} gap={gap:.2} font={font_size:.1} insert_space={insert_space}",
                        prev_block.text.trim(),
                        text.trim()
                    );
                }
                if insert_space
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

    let merged_text = merged_text.trim_end().to_string();
    let bbox = Rect::new(min_x, min_y, max_right - min_x, max_bottom - min_y);
    let mut merged = RawLayoutBlock::new(merged_text, bbox);
    if font_count > 0 {
        merged.font_size = Some(font_sum / font_count as f32);
    }
    Some(merged)
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
        assert_eq!(normalize_short_text_object(", "), ",");
        assert_eq!(normalize_short_text_object(". "), ".");
        assert_eq!(normalize_short_text_object("; "), ";");
    }

    #[test]
    fn test_normalize_does_not_add_space_after_comma_object() {
        assert_eq!(normalize_short_text_object(","), ",");
        assert_eq!(normalize_short_text_object("."), ".");
    }

    #[test]
    fn test_merge_joins_word_comma_without_space() {
        let blocks = vec![
            RawLayoutBlock::new("Hello", Rect::new(10.0, 20.0, 30.0, 8.0)).with_font_size(12.0),
            RawLayoutBlock::new(",", Rect::new(40.5, 20.0, 4.0, 8.0)).with_font_size(12.0),
            RawLayoutBlock::new("world", Rect::new(46.0, 20.0, 36.0, 8.0)).with_font_size(12.0),
        ];
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Hello, world");
    }

    #[test]
    fn test_merge_joins_sentence_period_without_space() {
        let blocks = vec![
            RawLayoutBlock::new("end", Rect::new(10.0, 20.0, 18.0, 8.0)).with_font_size(12.0),
            RawLayoutBlock::new(".", Rect::new(28.5, 20.0, 4.0, 8.0)).with_font_size(12.0),
        ];
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "end.");
    }

    #[test]
    fn test_merge_preserves_word_space() {
        let blocks = vec![
            RawLayoutBlock::new("Hello", Rect::new(10.0, 20.0, 30.0, 8.0)).with_font_size(12.0),
            RawLayoutBlock::new("world", Rect::new(50.0, 20.0, 36.0, 8.0)).with_font_size(12.0),
        ];
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Hello world");
    }

    #[test]
    fn test_reconstruct_text_joins_punctuation_without_space() {
        let font_size = 12.0;
        let word_gap = font_size * 0.3;
        let boxes = vec![
            PdfCharBox {
                left: 0.0,
                right: 6.0,
                ch: 'H',
            },
            PdfCharBox {
                left: 6.0,
                right: 12.0,
                ch: 'i',
            },
            PdfCharBox {
                left: 12.5,
                right: 14.5,
                ch: ',',
            },
            PdfCharBox {
                left: 14.5 + word_gap,
                right: 20.5 + word_gap,
                ch: 'w',
            },
        ];
        assert_eq!(reconstruct_text_from_char_boxes(&boxes, font_size), "Hi, w");
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
    fn test_choose_text_preserves_word_final_trailing_space() {
        let font_size = 12.0;
        let boxes = vec![PdfCharBox {
            left: 0.0,
            right: 6.0,
            ch: 'd',
        }];
        assert_eq!(
            choose_text_object_reconstruction("d ", &boxes, font_size),
            "d "
        );
        assert_eq!(
            normalize_short_text_object(&choose_text_object_reconstruction(
                "d ", &boxes, font_size
            )),
            "d "
        );
    }

    #[test]
    fn test_merge_joins_per_glyph_letters_without_spaces() {
        let font_size = 12.0;
        let char_w = 8.0;
        let gap = 3.0;
        let mut blocks = Vec::new();
        let word = "Maximum";
        let mut x = 10.0;
        for ch in word.chars() {
            blocks.push(
                RawLayoutBlock::new(ch.to_string(), Rect::new(x, 20.0, char_w, font_size))
                    .with_font_size(font_size),
            );
            x += char_w + gap;
        }
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Maximum");
    }

    #[test]
    fn test_merge_per_glyph_decimal_without_space() {
        let font_size = 12.0;
        let char_w = 8.0;
        let gap = font_size * 0.3;
        let blocks = vec![
            RawLayoutBlock::new("2", Rect::new(10.0, 20.0, char_w, font_size))
                .with_font_size(font_size),
            RawLayoutBlock::new(
                ".",
                Rect::new(10.0 + char_w + gap, 20.0, char_w * 0.4, font_size),
            )
            .with_font_size(font_size),
            RawLayoutBlock::new(
                "5",
                Rect::new(
                    10.0 + char_w + gap + char_w * 0.4 + gap,
                    20.0,
                    char_w,
                    font_size,
                ),
            )
            .with_font_size(font_size),
        ];
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "2.5");
    }

    #[test]
    fn test_merge_separates_tight_word_fragments() {
        let font_size = 12.0;
        let gap = font_size * 0.24;
        let blocks = vec![
            RawLayoutBlock::new("of", Rect::new(10.0, 20.0, 14.0, font_size))
                .with_font_size(font_size),
            RawLayoutBlock::new("this", Rect::new(10.0 + 14.0 + gap, 20.0, 22.0, font_size))
                .with_font_size(font_size),
        ];
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "of this");
    }

    #[test]
    fn test_merge_text_objects_on_visual_lines_joins_word_fragments() {
        let blocks = vec![
            RawLayoutBlock::new("Mem", Rect::new(10.0, 20.0, 18.0, 8.0)).with_font_size(8.0),
            RawLayoutBlock::new("bers", Rect::new(28.2, 20.1, 24.0, 8.0)).with_font_size(8.0),
            RawLayoutBlock::new("chosen", Rect::new(70.0, 20.0, 36.0, 8.0)).with_font_size(8.0),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);

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

        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "Left column");
        assert_eq!(merged[1].text, "Right column");
    }

    #[test]
    fn test_merge_per_glyph_word_boundaries_via_trailing_space() {
        let font_size = 12.0;
        let char_w = 6.0;
        let kerning = 2.0;
        let word_gap = 5.0;
        let mut blocks = Vec::new();
        let mut x = 10.0;
        for word in ["We", "have"] {
            for (i, ch) in word.chars().enumerate() {
                let is_final = i == word.len() - 1;
                let text = if is_final {
                    format!("{ch} ")
                } else {
                    ch.to_string()
                };
                blocks.push(
                    RawLayoutBlock::new(text, Rect::new(x, 20.0, char_w, font_size))
                        .with_font_size(font_size),
                );
                x += char_w + if is_final { word_gap } else { kerning };
            }
        }
        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "We have");
    }

    #[test]
    fn test_merge_text_objects_on_visual_lines_splits_narrow_line_object_gutter() {
        let blocks = vec![
            RawLayoutBlock::new(
                "A complete left-column visual line with several words",
                Rect::new(50.0, 20.0, 250.0, 9.0),
            )
            .with_font_size(9.0),
            RawLayoutBlock::new(
                "A complete right-column visual line with several words",
                Rect::new(312.0, 20.5, 250.0, 9.0),
            )
            .with_font_size(9.0),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged[0].text,
            "A complete left-column visual line with several words"
        );
        assert_eq!(
            merged[1].text,
            "A complete right-column visual line with several words"
        );
    }

    #[test]
    fn test_merge_text_objects_on_visual_lines_keeps_short_column_tail_separate() {
        let blocks = vec![
            RawLayoutBlock::new(
                "A complete left-column visual line with several words",
                Rect::new(50.0, 20.0, 250.0, 9.0),
            )
            .with_font_size(9.0),
            RawLayoutBlock::new("tail", Rect::new(312.0, 20.5, 24.0, 9.0)).with_font_size(9.0),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[1].text, "tail");
    }

    #[test]
    fn test_merge_short_word_does_not_widen_line_tolerance() {
        let font_size = 12.0;
        let blocks = vec![
            RawLayoutBlock::new("Dr", Rect::new(10.0, 20.0, 14.0, font_size))
                .with_font_size(font_size),
            RawLayoutBlock::new("Smith", Rect::new(10.0, 30.0, 30.0, font_size))
                .with_font_size(font_size),
        ];

        let merged = merge_text_objects_on_visual_lines(blocks, 612.0);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "Dr");
        assert_eq!(merged[1].text, "Smith");
    }

    /// Phase 0 calibration probe — run with `cargo test us_traffic_form_spike -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual US Traffic calibration spike"]
    fn us_traffic_form_spike() -> Result<(), OcrError> {
        let path = std::path::Path::new(
            r"C:\Users\aashi\Desktop\MSU\Masters\CSE802\US_Traffic_Sign_Classification_Report.pdf",
        );
        if !path.is_file() {
            eprintln!("skip: US Traffic PDF not found at {}", path.display());
            return Ok(());
        }
        let rasterizer = PdfRasterizer::new()?;
        let document = rasterizer.load_document_from_file(path)?;
        for (idx, page) in document.pages().iter().enumerate() {
            let page_area = page.width().value * page.height().value;
            let scan = scan_page_content(&page, page_area);
            eprintln!(
                "page={idx} type={} text={} images={} largest={:.3}%",
                classify_page_type(scan.has_text, scan.has_significant_images),
                scan.has_text,
                scan.qualifying_image_count,
                scan.largest_image_area_ratio * 100.0,
            );
        }
        Ok(())
    }
}
