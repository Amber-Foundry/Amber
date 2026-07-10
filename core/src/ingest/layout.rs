use crate::ocr::engine::Rect;
use serde::{Deserialize, Serialize};

/// Type classification for a document layout text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockType {
    /// Heading with hierarchy level (1 for H1, 2 for H2, 3 for H3, 4+ for H4+).
    Heading(u8),
    /// Standard body paragraph.
    Body,
    /// List item block (bulleted, numbered, or lettered).
    ListItem,
    /// Table or tabular data block.
    Table,
}

/// Raw input text block with spatial bounding box and optional typography metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawLayoutBlock {
    pub text: String,
    pub bbox: Rect,
    pub font_size: Option<f32>,
    pub confidence: Option<f32>,
}

impl RawLayoutBlock {
    pub fn new(text: impl Into<String>, bbox: Rect) -> Self {
        Self {
            text: text.into(),
            bbox,
            font_size: None,
            confidence: None,
        }
    }

    pub fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = Some(font_size);
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

/// Fully processed layout block classified into structural document components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub text: String,
    pub block_type: BlockType,
    pub bbox: Option<Rect>,
    pub confidence: Option<f32>,
}

impl TextBlock {
    pub fn new(text: impl Into<String>, block_type: BlockType) -> Self {
        Self {
            text: text.into(),
            block_type,
            bbox: None,
            confidence: None,
        }
    }
}

/// Analyzes a page of raw layout blocks, performing band-based multi-column layout clustering
/// and structural classification (Headings, ListItems, Tables, Body).
pub fn analyze_layout(
    raw_blocks: Vec<RawLayoutBlock>,
    page_width: f32,
    _page_height: f32,
) -> Vec<TextBlock> {
    if raw_blocks.is_empty() {
        return Vec::new();
    }

    // Filter out purely whitespace blocks
    let valid_blocks: Vec<RawLayoutBlock> = raw_blocks
        .into_iter()
        .filter(|b| !b.text.trim().is_empty())
        .collect();

    if valid_blocks.is_empty() {
        return Vec::new();
    }

    // Compute median metric (font size if available, otherwise bounding box height)
    let median_metric = compute_median_metric(&valid_blocks);

    // Step 1: Perform Band-Based Multi-Column Layout Clustering
    let clustered_raw_blocks = cluster_multi_column_layout(valid_blocks, page_width);

    // Step 2: Classify block types
    clustered_raw_blocks
        .into_iter()
        .map(|raw| {
            let block_type = classify_block_type(&raw, median_metric);
            TextBlock {
                text: raw.text.trim().to_string(),
                block_type,
                bbox: Some(raw.bbox),
                confidence: raw.confidence,
            }
        })
        .collect()
}

/// Group blocks into vertical bands split by full-width elements (>60% page width),
/// then detect column gutters within each band to order text left-to-right by column,
/// top-to-bottom within each column.
fn cluster_multi_column_layout(
    blocks: Vec<RawLayoutBlock>,
    page_width: f32,
) -> Vec<RawLayoutBlock> {
    let full_width_threshold = 0.60 * page_width;

    // Separate blocks into full-width band dividers vs normal blocks
    // We sort all blocks by top y-coordinate first to establish vertical band bounds
    let mut sorted_blocks = blocks;
    sorted_blocks.sort_by(|a, b| {
        a.bbox
            .y
            .partial_cmp(&b.bbox.y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut result = Vec::with_capacity(sorted_blocks.len());
    let mut current_band: Vec<RawLayoutBlock> = Vec::new();

    for block in sorted_blocks {
        let is_full_width = block.bbox.width >= full_width_threshold
            || block_crosses_page_center(&block.bbox, page_width);

        if is_full_width {
            // Flush current band column clusters before pushing full-width block
            if !current_band.is_empty() {
                result.extend(cluster_single_band(
                    std::mem::take(&mut current_band),
                    page_width,
                ));
            }
            result.push(block);
        } else {
            current_band.push(block);
        }
    }

    if !current_band.is_empty() {
        result.extend(cluster_single_band(current_band, page_width));
    }

    result
}

/// Clusters blocks within a single vertical band into 1 or 2 (or N) columns by detecting X gutters.
fn cluster_single_band(band_blocks: Vec<RawLayoutBlock>, page_width: f32) -> Vec<RawLayoutBlock> {
    if band_blocks.len() <= 1 {
        return band_blocks;
    }

    // Determine if there is a multi-column layout split in this band
    // Collect x-centers
    let mid_page = page_width / 2.0;

    let mut left_column = Vec::new();
    let mut right_column = Vec::new();

    let mut has_left = false;
    let mut has_right = false;

    for block in band_blocks {
        let x_center = block.bbox.x + (block.bbox.width / 2.0);
        // If block is clearly in the left or right half
        if x_center < mid_page {
            has_left = true;
            left_column.push(block);
        } else {
            has_right = true;
            right_column.push(block);
        }
    }

    // If we have both left and right blocks, only treat as multi-column when a clear
    // vertical gutter exists (no significant horizontal overlap between column envelopes).
    if has_left && has_right && columns_have_clear_gutter(&left_column, &right_column, page_width) {
        // Sort top-to-bottom within each column
        left_column.sort_by(|a, b| {
            a.bbox
                .y
                .partial_cmp(&b.bbox.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        right_column.sort_by(|a, b| {
            a.bbox
                .y
                .partial_cmp(&b.bbox.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut ordered = Vec::with_capacity(left_column.len() + right_column.len());
        ordered.extend(left_column);
        ordered.extend(right_column);
        ordered
    } else {
        // Single column band: sort top-to-bottom
        let mut single_col = left_column;
        single_col.extend(right_column);
        single_col.sort_by(|a, b| {
            a.bbox
                .y
                .partial_cmp(&b.bbox.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        single_col
    }
}

/// Centered headings often span the page midline but stay narrower than the full-width threshold.
/// Treat them as band dividers so two-column content below is clustered separately.
fn block_crosses_page_center(bbox: &Rect, page_width: f32) -> bool {
    let mid_page = page_width / 2.0;
    let left = bbox.x;
    let right = bbox.x + bbox.width;
    let center = left + (bbox.width / 2.0);
    let center_margin = page_width * 0.08;
    left < mid_page && right > mid_page && (center - mid_page).abs() <= center_margin
}

/// Returns true when left and right column block envelopes are separated by a horizontal gutter.
fn columns_have_clear_gutter(
    left: &[RawLayoutBlock],
    right: &[RawLayoutBlock],
    page_width: f32,
) -> bool {
    let left_max_x = left
        .iter()
        .map(|b| b.bbox.x + b.bbox.width)
        .fold(0.0f32, f32::max);
    let right_min_x = right.iter().map(|b| b.bbox.x).fold(f32::MAX, f32::min);
    let min_gutter = (page_width * 0.05).max(12.0);
    left_max_x + min_gutter <= right_min_x
}

/// Computes the median font size or median bounding box height.
fn compute_median_metric(blocks: &[RawLayoutBlock]) -> f32 {
    let mut metrics: Vec<f32> = blocks
        .iter()
        .map(|b| b.font_size.unwrap_or(b.bbox.height))
        .filter(|&m| m > 0.0)
        .collect();

    if metrics.is_empty() {
        return 12.0;
    }

    metrics.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Use (len - 1) / 2 to select the lower-middle element as baseline median
    metrics[(metrics.len() - 1) / 2]
}

/// Classifies a raw block into a `BlockType` using metrics and pattern matching.
fn classify_block_type(block: &RawLayoutBlock, median_metric: f32) -> BlockType {
    let text = block.text.trim();
    let metric = block.font_size.unwrap_or(block.bbox.height);
    let ratio = if median_metric > 0.0 {
        metric / median_metric
    } else {
        1.0
    };

    let char_count = text.chars().count();
    let is_short = char_count <= 60;
    let is_all_caps =
        is_short && char_count >= 3 && text.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
    let matches_heading_keyword = matches_heading_pattern(text);

    // Heading Classification Rules:
    // H1: ratio >= 1.55 or (is_short && ratio >= 1.4 && (is_all_caps || matches_heading_keyword))
    if ratio >= 1.55 || (is_short && ratio >= 1.4 && (is_all_caps || matches_heading_keyword)) {
        return BlockType::Heading(1);
    }
    // H2: ratio >= 1.30 or (is_short && ratio >= 1.20 && (is_all_caps || matches_heading_keyword))
    if ratio >= 1.30 || (is_short && ratio >= 1.20 && (is_all_caps || matches_heading_keyword)) {
        return BlockType::Heading(2);
    }
    // H3: ratio >= 1.15 or (is_short && matches_heading_keyword)
    if ratio >= 1.15 || (is_short && matches_heading_keyword) {
        return BlockType::Heading(3);
    }

    // List Item Detection
    if is_list_item(text) {
        return BlockType::ListItem;
    }

    // Table Detection (grid text containing multiple pipes or multiple tab/space separated numeric columns)
    if is_table_block(text) {
        return BlockType::Table;
    }

    BlockType::Body
}

/// Checks if string starts with structural heading prefixes like "ARTICLE", "SECTION", "CHAPTER", etc.
fn matches_heading_pattern(text: &str) -> bool {
    let upper = text.to_uppercase();
    upper.starts_with("ARTICLE")
        || upper.starts_with("SECTION")
        || upper.starts_with("CHAPTER")
        || upper.starts_with("TITLE ")
        || upper.starts_with("PART ")
        || (text.chars().next().is_some_and(|c| c.is_ascii_digit())
            && (text.contains(" Introduction")
                || text.contains(" Summary")
                || text.contains(" Overview")
                || upper.contains("AMENDMENT")))
}

/// Checks if string matches list item bullet/numeric/letter prefix patterns.
fn is_list_item(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.starts_with('•')
        || trimmed.starts_with('-')
        || trimmed.starts_with('*')
        || trimmed.starts_with('◦')
        || trimmed.starts_with('▪')
    {
        return true;
    }

    // Numbered pattern: e.g. "1.", "2)", "(1)"
    let bytes = trimmed.as_bytes();
    if !bytes.is_empty() && (bytes[0].is_ascii_digit() || bytes[0] == b'(') {
        if let Some(pos) = trimmed.find(&['.', ')'][..]) {
            if pos <= 4 {
                return true;
            }
        }
    }

    false
}

/// Checks if text looks like formatted table rows.
fn is_table_block(text: &str) -> bool {
    text.contains('|') || (text.lines().count() > 1 && text.matches('\t').count() > 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_item_detection() {
        assert!(is_list_item("• First bullet point"));
        assert!(is_list_item("- Second bullet point"));
        assert!(is_list_item("* Third bullet point"));
        assert!(is_list_item("1. First numbered item"));
        assert!(is_list_item("2) Second numbered item"));
        assert!(!is_list_item("This is regular paragraph text."));
    }

    #[test]
    fn test_heading_classification() {
        let h1_block = RawLayoutBlock::new(
            "CONSTITUTION OF THE UNITED STATES",
            Rect::new(50.0, 50.0, 500.0, 30.0),
        )
        .with_font_size(24.0);
        let body_block = RawLayoutBlock::new(
            "We the People of the United States...",
            Rect::new(50.0, 100.0, 500.0, 15.0),
        )
        .with_font_size(12.0);

        let result = analyze_layout(vec![h1_block, body_block], 600.0, 800.0);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].block_type, BlockType::Heading(1));
        assert_eq!(result[1].block_type, BlockType::Body);
    }

    #[test]
    fn test_two_column_layout_clustering() {
        // Full-width title at top
        let title = RawLayoutBlock::new(
            "CONSTITUTION OF THE UNITED STATES",
            Rect::new(50.0, 20.0, 500.0, 30.0),
        );

        // Column 1 (Left): Section 7 lines (x = 50..250)
        let col1_line1 = RawLayoutBlock::new(
            "Section 7. All Bills for raising Revenue",
            Rect::new(50.0, 100.0, 200.0, 15.0),
        );
        let col1_line2 = RawLayoutBlock::new(
            "shall originate in the House of Representatives.",
            Rect::new(50.0, 120.0, 200.0, 15.0),
        );

        // Column 2 (Right): Section 8 lines (x = 350..550) - sharing same Y coordinates!
        let col2_line1 = RawLayoutBlock::new(
            "Section 8. The Congress shall have Power",
            Rect::new(350.0, 100.0, 200.0, 15.0),
        );
        let col2_line2 = RawLayoutBlock::new(
            "To lay and collect Taxes, Duties, Imposts.",
            Rect::new(350.0, 120.0, 200.0, 15.0),
        );

        // Pass blocks in interleaved order (as raw PDF or OCR line detection might yield)
        let raw_blocks = vec![title, col1_line1, col2_line1, col1_line2, col2_line2];

        let result = analyze_layout(raw_blocks, 600.0, 800.0);

        assert_eq!(result.len(), 5);
        assert_eq!(result[0].text, "CONSTITUTION OF THE UNITED STATES");
        // Column 1 must precede Column 2 completely without interleaving Section 7 and Section 8!
        assert_eq!(result[1].text, "Section 7. All Bills for raising Revenue");
        assert_eq!(
            result[2].text,
            "shall originate in the House of Representatives."
        );
        assert_eq!(result[3].text, "Section 8. The Congress shall have Power");
        assert_eq!(result[4].text, "To lay and collect Taxes, Duties, Imposts.");
    }

    #[test]
    fn test_centered_heading_divides_two_column_band() {
        let heading = RawLayoutBlock::new("Chapter I", Rect::new(220.0, 50.0, 160.0, 20.0));
        let col1_line1 =
            RawLayoutBlock::new("Left column line one", Rect::new(50.0, 100.0, 200.0, 15.0));
        let col2_line1 = RawLayoutBlock::new(
            "Right column line one",
            Rect::new(350.0, 100.0, 200.0, 15.0),
        );
        let col1_line2 =
            RawLayoutBlock::new("Left column line two", Rect::new(50.0, 120.0, 200.0, 15.0));
        let col2_line2 = RawLayoutBlock::new(
            "Right column line two",
            Rect::new(350.0, 120.0, 200.0, 15.0),
        );

        let raw_blocks = vec![heading, col1_line1, col2_line1, col1_line2, col2_line2];

        let result = analyze_layout(raw_blocks, 600.0, 800.0);

        assert_eq!(result.len(), 5);
        assert_eq!(result[0].text, "Chapter I");
        assert_eq!(result[1].text, "Left column line one");
        assert_eq!(result[2].text, "Left column line two");
        assert_eq!(result[3].text, "Right column line one");
        assert_eq!(result[4].text, "Right column line two");
    }

    #[test]
    fn test_centered_heading_stays_single_column() {
        let heading = RawLayoutBlock::new("Introduction", Rect::new(220.0, 50.0, 160.0, 20.0));
        let para1 = RawLayoutBlock::new(
            "This paragraph starts below the heading.",
            Rect::new(80.0, 90.0, 440.0, 15.0),
        );
        let para2 = RawLayoutBlock::new(
            "Indented continuation on the next line.",
            Rect::new(100.0, 110.0, 420.0, 15.0),
        );

        let result = analyze_layout(vec![heading, para1, para2], 600.0, 800.0);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].text, "Introduction");
        assert_eq!(result[1].text, "This paragraph starts below the heading.");
        assert_eq!(result[2].text, "Indented continuation on the next line.");
    }
}
