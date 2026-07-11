use crate::ingest::templates::{detect_non_flow_layout, evaluate_template_match};
use crate::ocr::engine::Rect;
use serde::{Deserialize, Serialize};

/// Top fraction of page height treated as header margin band.
const HEADER_BAND_RATIO: f32 = 0.04;
const HEADER_BAND_MAX_PT: f32 = 36.0;
/// Bottom fraction of page height treated as footer margin band.
const FOOTER_BAND_RATIO: f32 = 0.08;

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
    /// Running header or page-number band at the top of the page.
    Header,
    /// Running footer, page stamp, or bottom margin band.
    Footer,
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
    /// Formula and algorithm fragments remain body text and never become section anchors.
    #[serde(default)]
    pub fragment: bool,
}

impl TextBlock {
    pub fn new(text: impl Into<String>, block_type: BlockType) -> Self {
        Self {
            text: text.into(),
            block_type,
            bbox: None,
            confidence: None,
            fragment: false,
        }
    }
}

/// Layout-only counters used by the CLI to diagnose a page without exposing its text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutDebugSnapshot {
    pub page_index: usize,
    pub band_count: usize,
    pub column_splits: usize,
    pub fragment_count: usize,
    #[serde(default)]
    pub layout_hint: Option<String>,
    #[serde(default)]
    pub template_match_confidence: Option<f32>,
    #[serde(default)]
    pub layout_warnings: Vec<String>,
}

/// Analyzes a page of raw layout blocks, performing band-based multi-column layout clustering
/// and structural classification (Headings, ListItems, Tables, Body).
pub fn analyze_layout(
    raw_blocks: Vec<RawLayoutBlock>,
    page_width: f32,
    page_height: f32,
) -> Vec<TextBlock> {
    analyze_layout_with_snapshot(raw_blocks, page_width, page_height).0
}

/// Produces the normal layout output plus structural counters for diagnostics.
pub(crate) fn analyze_layout_with_snapshot(
    raw_blocks: Vec<RawLayoutBlock>,
    page_width: f32,
    page_height: f32,
) -> (Vec<TextBlock>, LayoutDebugSnapshot) {
    if raw_blocks.is_empty() {
        return (Vec::new(), LayoutDebugSnapshot::default());
    }

    // Filter out purely whitespace blocks
    let valid_blocks: Vec<RawLayoutBlock> = raw_blocks
        .into_iter()
        .filter(|b| !b.text.trim().is_empty())
        .collect();

    if valid_blocks.is_empty() {
        return (Vec::new(), LayoutDebugSnapshot::default());
    }

    let block_count = valid_blocks.len();

    // Compute median metric (font size if available, otherwise bounding box height)
    let median_metric = compute_median_metric(&valid_blocks);
    let median_line_height = compute_median_line_height(&valid_blocks);

    // Build vertical bands before looking for gutters. This prevents a short line at the
    // bottom of a full-width region from being spliced into the two-column region below.
    let bands =
        group_into_vertical_bands(valid_blocks, median_line_height, page_width, page_height);
    let mut snapshot = LayoutDebugSnapshot {
        band_count: bands.len(),
        column_splits: 0,
        fragment_count: 0,
        ..LayoutDebugSnapshot::default()
    };
    let mut detected_gutter_x: Option<f32> = None;
    let clustered_raw_blocks = bands
        .into_iter()
        .flat_map(|band| {
            let band_line_height = compute_median_line_height(&band);
            if let Some(split) = band_column_split(&band, page_width, page_height, band_line_height)
            {
                snapshot.column_splits += 1;
                detected_gutter_x = Some(split);
            }
            order_band_blocks(band, page_width, page_height, band_line_height)
        })
        .collect::<Vec<_>>();

    // Step 2: Classify block types
    let blocks = clustered_raw_blocks
        .into_iter()
        .map(|raw| {
            let fragment = is_structured_fragment(&raw.text);
            let block_type = if fragment {
                snapshot.fragment_count += 1;
                BlockType::Body
            } else if let Some(margin_type) = classify_margin_block(&raw, page_width, page_height) {
                margin_type
            } else if is_centered_footer_stamp(&raw, page_width, median_line_height) {
                BlockType::Footer
            } else {
                classify_block_type(&raw, median_metric)
            };
            TextBlock {
                text: raw.text.trim().to_string(),
                block_type,
                bbox: Some(raw.bbox),
                confidence: raw.confidence,
                fragment,
            }
        })
        .collect();

    let template_match = evaluate_template_match(
        page_width,
        snapshot.band_count,
        snapshot.column_splits,
        detected_gutter_x,
    );
    snapshot.layout_hint = template_match.layout_hint;
    snapshot.template_match_confidence = template_match.confidence;
    snapshot.layout_warnings = template_match.warnings;

    if let Some(warning) = detect_non_flow_layout(&snapshot, block_count, page_width, page_height) {
        snapshot.layout_warnings.push(warning);
    }

    (blocks, snapshot)
}

fn compute_median_line_height(blocks: &[RawLayoutBlock]) -> f32 {
    let mut heights: Vec<f32> = blocks
        .iter()
        .map(|block| block.bbox.height)
        .filter(|height| *height > 0.0)
        .collect();
    if heights.is_empty() {
        return compute_median_metric(blocks);
    }
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    heights[(heights.len() - 1) / 2]
}

/// Groups nearby visual lines into vertical regions using per-band line height.
fn group_into_vertical_bands(
    mut blocks: Vec<RawLayoutBlock>,
    initial_line_height: f32,
    page_width: f32,
    page_height: f32,
) -> Vec<Vec<RawLayoutBlock>> {
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

    let mut band_line_height = initial_line_height;
    let mut bands = Vec::new();
    let mut current_band = Vec::new();
    let mut current_bottom = f32::NEG_INFINITY;

    for block in blocks {
        let gap_threshold = band_line_height.max(1.0) * 1.5;
        if !current_band.is_empty() && block.bbox.y > current_bottom + gap_threshold {
            band_line_height = compute_median_line_height(&current_band).max(initial_line_height);
            bands.push(std::mem::take(&mut current_band));
            current_bottom = f32::NEG_INFINITY;
        }
        current_bottom = current_bottom.max(block.bbox.y + block.bbox.height);
        current_band.push(block);
    }
    if !current_band.is_empty() {
        bands.push(current_band);
    }
    bands
        .into_iter()
        .flat_map(|band| {
            let band_height = compute_median_line_height(&band);
            subdivide_band_at_two_column_onset(band, band_height, page_width, page_height)
        })
        .collect()
}

/// Keeps title/author headers out of the first two-column body band on IEEE-style pages.
fn subdivide_band_at_two_column_onset(
    band: Vec<RawLayoutBlock>,
    line_height: f32,
    page_width: f32,
    page_height: f32,
) -> Vec<Vec<RawLayoutBlock>> {
    let column_blocks = column_line_blocks(&band, page_width, page_height, line_height);
    let Some(split) = find_column_split(&column_blocks, page_width, line_height) else {
        return vec![band];
    };

    let rows = group_visual_rows(&band, line_height);
    let first_two_col_idx = rows.iter().position(|row| {
        let narrow: Vec<_> = row
            .iter()
            .filter(|block| !is_full_width_block(block, page_width))
            .cloned()
            .collect();
        narrow.len() >= 2 && row_has_blocks_on_both_sides(&narrow, split)
    });
    let Some(first_two_col_idx) = first_two_col_idx else {
        return vec![band];
    };
    if first_two_col_idx == 0 {
        return vec![band];
    }

    let mut prefix = Vec::new();
    let mut body = Vec::new();
    for (idx, row) in rows.into_iter().enumerate() {
        if idx < first_two_col_idx {
            prefix.extend(row);
        } else {
            body.extend(row);
        }
    }
    if prefix.is_empty() || body.is_empty() {
        vec![band]
    } else {
        vec![prefix, body]
    }
}

fn project_x_intervals(blocks: &[RawLayoutBlock]) -> Vec<(f32, f32)> {
    blocks
        .iter()
        .map(|block| (block.bbox.x, block.bbox.x + block.bbox.width))
        .collect()
}

/// Blocks spanning most of the page width pollute x-projection gutter detection when
/// mixed with two-column lines in the same vertical band (e.g. IEEE abstract + body).
fn is_full_width_block(block: &RawLayoutBlock, page_width: f32) -> bool {
    block.bbox.width > page_width * 0.65
}

fn classify_margin_block(
    block: &RawLayoutBlock,
    page_width: f32,
    page_height: f32,
) -> Option<BlockType> {
    let text = block.text.trim();
    if is_structured_fragment(text) || matches_heading_pattern(text) {
        return None;
    }
    let bbox = &block.bbox;
    if page_height <= 0.0 || page_width <= 0.0 {
        return None;
    }
    let header_limit = (page_height * HEADER_BAND_RATIO).min(HEADER_BAND_MAX_PT);
    let footer_start = page_height * (1.0 - FOOTER_BAND_RATIO);
    let running_head = bbox.width <= page_width * 0.35;
    let compact = bbox.height <= page_height * 0.025;
    if bbox.y + bbox.height <= header_limit && compact && running_head {
        return Some(BlockType::Header);
    }
    if bbox.y >= footer_start && compact && running_head {
        return Some(BlockType::Footer);
    }
    None
}

fn is_margin_gutter_excluded_block(
    block: &RawLayoutBlock,
    page_width: f32,
    page_height: f32,
    line_height: f32,
) -> bool {
    if is_structured_fragment(&block.text) || matches_heading_pattern(block.text.trim()) {
        return false;
    }
    classify_margin_block(block, page_width, page_height).is_some()
        || is_centered_footer_stamp(block, page_width, line_height)
}

/// Centered footer/metadata lines (e.g. arXiv stamps) that should not participate in gutters.
fn is_centered_footer_stamp(block: &RawLayoutBlock, page_width: f32, line_height: f32) -> bool {
    if page_width <= 0.0 {
        return false;
    }
    let center = block.bbox.x + (block.bbox.width / 2.0);
    let page_center = page_width / 2.0;
    let centered = (center - page_center).abs() <= page_width * 0.08;
    let span = block.bbox.width >= page_width * 0.22 && block.bbox.width <= page_width * 0.52;
    centered && span && block.bbox.height <= line_height.max(1.0) * 1.1
}

fn is_margin_gutter_excluded(
    block: &RawLayoutBlock,
    page_width: f32,
    page_height: f32,
    line_height: f32,
) -> bool {
    is_margin_gutter_excluded_block(block, page_width, page_height, line_height)
}

fn column_line_blocks(
    band: &[RawLayoutBlock],
    page_width: f32,
    page_height: f32,
    line_height: f32,
) -> Vec<RawLayoutBlock> {
    band.iter()
        .filter(|block| {
            !is_full_width_block(block, page_width)
                && !is_margin_gutter_excluded(block, page_width, page_height, line_height)
        })
        .cloned()
        .collect()
}

fn band_column_split(
    band: &[RawLayoutBlock],
    page_width: f32,
    page_height: f32,
    line_height: f32,
) -> Option<f32> {
    let column_blocks = column_line_blocks(band, page_width, page_height, line_height);
    find_column_split(&column_blocks, page_width, line_height)
}

/// Visual rows within a band: blocks whose baselines fall within one line height.
const ROW_Y_TOLERANCE_RATIO: f32 = 0.6;

fn group_visual_rows(blocks: &[RawLayoutBlock], line_height: f32) -> Vec<Vec<RawLayoutBlock>> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut sorted = blocks.to_vec();
    sort_reading_order(&mut sorted);
    let tolerance = line_height.max(1.0) * ROW_Y_TOLERANCE_RATIO;
    let mut rows = Vec::new();
    let mut current_row = vec![sorted[0].clone()];
    let mut row_y = sorted[0].bbox.y;

    for block in sorted.into_iter().skip(1) {
        if (block.bbox.y - row_y).abs() <= tolerance {
            current_row.push(block);
        } else {
            rows.push(current_row);
            current_row = vec![block.clone()];
            row_y = block.bbox.y;
        }
    }
    rows.push(current_row);
    rows
}

/// Samples the leftmost x on each visual row so hanging-indent continuations do not
/// bridge the column gutter in merged x-projections.
fn project_row_left_edge_intervals(blocks: &[RawLayoutBlock], line_height: f32) -> Vec<(f32, f32)> {
    let sample_width = median_character_width(blocks).max(1.0);
    group_visual_rows(blocks, line_height)
        .into_iter()
        .filter_map(|row| {
            row.iter()
                .min_by(|a, b| {
                    a.bbox
                        .x
                        .partial_cmp(&b.bbox.x)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|block| (block.bbox.x, block.bbox.x + sample_width))
        })
        .collect()
}

fn find_largest_gap_in_left_edges(blocks: &[RawLayoutBlock], page_width: f32) -> Option<f32> {
    if blocks.len() < 4 {
        return None;
    }
    let mut edges: Vec<f32> = blocks.iter().map(|block| block.bbox.x).collect();
    edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_char_width = median_character_width(blocks);
    let min_gutter = (median_char_width * 2.0)
        .max(page_width * 0.03)
        .min(median_block_width(blocks) * 0.5);
    edges
        .windows(2)
        .filter_map(|pair| {
            let gap_width = pair[1] - pair[0];
            (gap_width >= min_gutter).then_some((gap_width, (pair[0] + pair[1]) / 2.0))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, split)| split)
}

fn find_column_split(
    column_blocks: &[RawLayoutBlock],
    page_width: f32,
    line_height: f32,
) -> Option<f32> {
    if column_blocks.len() < 2 {
        return None;
    }
    let row_intervals = project_row_left_edge_intervals(column_blocks, line_height);
    let candidate = find_largest_horizontal_gap(&row_intervals, column_blocks, page_width)
        .or_else(|| find_largest_gap_in_left_edges(column_blocks, page_width))
        .or_else(|| {
            find_largest_horizontal_gap(
                &project_x_intervals(column_blocks),
                column_blocks,
                page_width,
            )
        });
    candidate.filter(|split| column_split_has_row_evidence(column_blocks, line_height, *split))
}

/// Require multiple visual rows with blocks on both sides of the gutter so hanging indents
/// and single-column paragraphs do not false-trigger column clustering.
fn column_split_has_row_evidence(blocks: &[RawLayoutBlock], line_height: f32, split: f32) -> bool {
    let rows = group_visual_rows(blocks, line_height);
    if rows.len() < 2 {
        return false;
    }
    let two_column_rows = rows
        .iter()
        .filter(|row| row_has_blocks_on_both_sides(row, split))
        .count();
    if two_column_rows >= 2 && (two_column_rows as f32 / rows.len() as f32) >= 0.25 {
        return true;
    }
    let left_blocks = blocks
        .iter()
        .filter(|block| block.bbox.x + (block.bbox.width / 2.0) < split)
        .count();
    let right_blocks = blocks.iter().filter(|block| block.bbox.x >= split).count();
    left_blocks >= 2 && right_blocks >= 2 && two_column_rows >= 1
}

fn row_has_blocks_on_both_sides(row: &[RawLayoutBlock], split: f32) -> bool {
    let has_left = row
        .iter()
        .any(|block| block.bbox.x + (block.bbox.width / 2.0) < split);
    let has_right = row.iter().any(|block| block.bbox.x >= split);
    if has_left && has_right {
        return true;
    }
    row.iter()
        .any(|block| block.bbox.x < split && block.bbox.x + block.bbox.width > split)
}

/// Returns the midpoint of the largest significant gap between merged x-projections.
fn find_largest_horizontal_gap(
    intervals: &[(f32, f32)],
    blocks: &[RawLayoutBlock],
    page_width: f32,
) -> Option<f32> {
    if intervals.len() < 2 {
        return None;
    }
    let mut intervals = intervals.to_vec();
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut merged = Vec::new();
    for (start, end) in intervals {
        if let Some((_, merged_end)) = merged.last_mut() {
            if start <= *merged_end {
                *merged_end = (*merged_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    let median_width = median_block_width(blocks);
    let median_char_width = median_character_width(blocks);
    // A two-column PDF can have line objects hundreds of points wide while the gutter is
    // only a few character widths. Cap the width-derived floor with text metrics so those
    // legitimate narrow gutters are retained without mistaking normal word gaps for columns.
    let min_gutter = (median_char_width * 2.0)
        .max(page_width * 0.03)
        .min(median_width * 0.5);
    merged
        .windows(2)
        .filter_map(|pair| {
            let gap_start = pair[0].1;
            let gap_end = pair[1].0;
            let gap_width = gap_end - gap_start;
            (gap_width >= min_gutter).then_some((gap_width, (gap_start + gap_end) / 2.0))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, split)| split)
}

fn median_block_width(blocks: &[RawLayoutBlock]) -> f32 {
    let mut widths: Vec<f32> = blocks
        .iter()
        .map(|block| block.bbox.width)
        .filter(|width| *width > 0.0)
        .collect();
    if widths.is_empty() {
        return 0.0;
    }
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    widths[(widths.len() - 1) / 2]
}

fn median_character_width(blocks: &[RawLayoutBlock]) -> f32 {
    let mut widths: Vec<f32> = blocks
        .iter()
        .map(|block| {
            let chars = block
                .text
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .count()
                .max(1) as f32;
            block.bbox.width / chars
        })
        .filter(|width| *width > 0.0)
        .collect();
    if widths.is_empty() {
        return 1.0;
    }
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    widths[(widths.len() - 1) / 2]
}

/// Orders one vertical region as a single column or two empirical x-clusters.
fn order_band_blocks(
    band_blocks: Vec<RawLayoutBlock>,
    page_width: f32,
    page_height: f32,
    line_height: f32,
) -> Vec<RawLayoutBlock> {
    if band_blocks.len() <= 1 {
        return band_blocks;
    }

    let column_blocks = column_line_blocks(&band_blocks, page_width, page_height, line_height);
    let split = find_column_split(&column_blocks, page_width, line_height);

    if let Some(split) = split {
        let (mut full_width, narrow): (Vec<_>, Vec<_>) =
            band_blocks.into_iter().partition(|block| {
                is_full_width_block(block, page_width)
                    && !is_margin_gutter_excluded(block, page_width, page_height, line_height)
            });
        let (mut margin, narrow): (Vec<_>, Vec<_>) = narrow.into_iter().partition(|block| {
            is_margin_gutter_excluded(block, page_width, page_height, line_height)
        });
        sort_reading_order(&mut full_width);
        sort_reading_order(&mut margin);

        let mut left_column = Vec::new();
        let mut right_column = Vec::new();
        for block in narrow {
            // Left-edge assignment keeps hanging-indent continuations in the left column
            // even when a long line's centroid would cross the gutter split.
            if block.bbox.x < split {
                left_column.push(block);
            } else {
                right_column.push(block);
            }
        }
        sort_reading_order(&mut left_column);
        sort_reading_order(&mut right_column);
        let mut ordered = full_width;
        ordered.extend(left_column);
        ordered.extend(right_column);
        merge_margin_inline(ordered, margin)
    } else {
        let mut single_column = band_blocks;
        sort_reading_order(&mut single_column);
        single_column
    }
}

/// Reinsert margin blocks at their y-order position within a column-ordered band.
fn merge_margin_inline(
    mut ordered: Vec<RawLayoutBlock>,
    margin: Vec<RawLayoutBlock>,
) -> Vec<RawLayoutBlock> {
    for m in margin {
        let y = m.bbox.y;
        let pos = ordered
            .iter()
            .position(|b| b.bbox.y > y)
            .unwrap_or(ordered.len());
        ordered.insert(pos, m);
    }
    ordered
}

fn sort_reading_order(blocks: &mut [RawLayoutBlock]) {
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

/// Detects equation-like or algorithmic fragments that should flow as body text instead
/// of acting as noisy structural headings. This deliberately degrades safely rather than
/// attempting to parse mathematical notation.
pub(crate) fn is_structured_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    let char_count = trimmed.chars().count();
    if char_count == 0 {
        return false;
    }

    let mut words = trimmed.split_whitespace();
    let algorithm_label = words
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("algorithm"))
        && words.next().is_some_and(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_digit())
                .chars()
                .any(|ch| ch.is_ascii_digit())
        });
    if algorithm_label || is_equation_number(trimmed) {
        return true;
    }

    let symbol_count = trimmed
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    if char_count > 10 && (symbol_count as f32 / char_count as f32) > 0.25 {
        return true;
    }

    char_count < 80
        && trimmed
            .chars()
            .filter(|ch| {
                matches!(
                    ch,
                    '=' | '+' | '-' | '−' | '*' | '/' | '^' | '_' | '∑' | '∫'
                )
            })
            .count()
            > 3
}

fn is_equation_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    matches!(
        (bytes[0], *bytes.last().unwrap_or(&0)),
        (b'(', b')') | (b'[', b']')
    ) && text[1..text.len() - 1]
        .chars()
        .all(|ch| ch.is_ascii_digit())
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
                let after_delim = trimmed.get(pos + 1..);
                if after_delim.is_none_or(|s| s.is_empty() || s.starts_with(' ')) {
                    return true;
                }
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
        assert!(!is_list_item("The value is 1.5 meters wide."));
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

    #[test]
    fn full_width_lines_do_not_mask_column_gutter_in_same_band() {
        let blocks = vec![
            RawLayoutBlock::new(
                "Abstract-Full-width opener spanning the page",
                Rect::new(50.0, 50.0, 500.0, 15.0),
            ),
            RawLayoutBlock::new("LEFT_ABSTRACT_ONE", Rect::new(50.0, 80.0, 200.0, 15.0)),
            RawLayoutBlock::new("RIGHT_MOTIVATION_ONE", Rect::new(350.0, 80.0, 200.0, 15.0)),
            RawLayoutBlock::new("LEFT_ABSTRACT_TWO", Rect::new(50.0, 100.0, 200.0, 15.0)),
            RawLayoutBlock::new("RIGHT_MOTIVATION_TWO", Rect::new(350.0, 100.0, 200.0, 15.0)),
        ];

        let (result, snapshot) = analyze_layout_with_snapshot(blocks, 600.0, 800.0);
        let text = result
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();

        assert!(snapshot.column_splits >= 1);
        assert_eq!(text[0], "Abstract-Full-width opener spanning the page");
        assert!(
            text.iter().position(|value| *value == "LEFT_ABSTRACT_TWO")
                < text
                    .iter()
                    .position(|value| *value == "RIGHT_MOTIVATION_ONE")
        );
    }

    #[test]
    fn band_splits_trailing_short_line_from_two_column_body() {
        let blocks = vec![
            RawLayoutBlock::new(
                "Full-width introduction",
                Rect::new(50.0, 50.0, 500.0, 15.0),
            ),
            RawLayoutBlock::new("Short tail", Rect::new(50.0, 90.0, 220.0, 15.0)),
            RawLayoutBlock::new("LEFT BODY ONE", Rect::new(50.0, 110.0, 200.0, 15.0)),
            RawLayoutBlock::new("RIGHT BODY ONE", Rect::new(350.0, 110.0, 200.0, 15.0)),
            RawLayoutBlock::new("LEFT BODY TWO", Rect::new(50.0, 130.0, 200.0, 15.0)),
            RawLayoutBlock::new("RIGHT BODY TWO", Rect::new(350.0, 130.0, 200.0, 15.0)),
        ];

        let result = analyze_layout(blocks, 600.0, 800.0);
        let text = result
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(text[0], "Full-width introduction");
        assert_eq!(text[1], "Short tail");
        assert!(
            text.iter().position(|value| *value == "LEFT BODY TWO")
                < text.iter().position(|value| *value == "RIGHT BODY ONE")
        );
    }

    #[test]
    fn hanging_indent_two_column_left_before_right() {
        let indent = 25.0;
        let blocks = vec![
            RawLayoutBlock::new("LEFT_ROW_A first line", Rect::new(50.0, 100.0, 180.0, 12.0)),
            RawLayoutBlock::new(
                "RIGHT_ROW_A first line",
                Rect::new(350.0, 100.0, 180.0, 12.0),
            ),
            RawLayoutBlock::new(
                "LEFT_ROW_A continuation bridges gutter",
                Rect::new(50.0 + indent, 120.0, 220.0, 12.0),
            ),
            RawLayoutBlock::new(
                "RIGHT_ROW_B first line",
                Rect::new(350.0, 120.0, 180.0, 12.0),
            ),
            RawLayoutBlock::new("LEFT_ROW_B first line", Rect::new(50.0, 140.0, 180.0, 12.0)),
            RawLayoutBlock::new(
                "RIGHT_ROW_B continuation bridges gutter",
                Rect::new(350.0 + indent, 140.0, 200.0, 12.0),
            ),
        ];

        let (result, snapshot) = analyze_layout_with_snapshot(blocks, 600.0, 800.0);
        let text = result
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();

        assert!(snapshot.column_splits >= 1);
        assert!(
            text.iter()
                .position(|value| *value == "LEFT_ROW_B first line")
                < text
                    .iter()
                    .position(|value| *value == "RIGHT_ROW_A first line")
        );
    }

    #[test]
    fn hanging_indent_does_not_false_split_single_column() {
        let heading = RawLayoutBlock::new("Section Title", Rect::new(220.0, 50.0, 160.0, 20.0));
        let para1 = RawLayoutBlock::new(
            "Opening line of a single-column paragraph.",
            Rect::new(80.0, 90.0, 440.0, 15.0),
        );
        let para2 = RawLayoutBlock::new(
            "Indented continuation of the same paragraph.",
            Rect::new(110.0, 110.0, 410.0, 15.0),
        );

        let (result, snapshot) =
            analyze_layout_with_snapshot(vec![heading, para1, para2], 600.0, 800.0);
        let text = result
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(snapshot.column_splits, 0);
        assert_eq!(text[0], "Section Title");
        assert_eq!(text[1], "Opening line of a single-column paragraph.");
        assert_eq!(text[2], "Indented continuation of the same paragraph.");
    }

    #[test]
    fn empirical_gutter_not_page_center() {
        let blocks = vec![
            RawLayoutBlock::new("RIGHT ONE", Rect::new(320.0, 90.0, 160.0, 12.0)),
            RawLayoutBlock::new("LEFT ONE", Rect::new(40.0, 90.0, 160.0, 12.0)),
            RawLayoutBlock::new("RIGHT TWO", Rect::new(320.0, 110.0, 160.0, 12.0)),
            RawLayoutBlock::new("LEFT TWO", Rect::new(40.0, 110.0, 160.0, 12.0)),
        ];

        let text = analyze_layout(blocks, 600.0, 800.0)
            .into_iter()
            .map(|block| block.text)
            .collect::<Vec<_>>();
        assert_eq!(text, vec!["LEFT ONE", "LEFT TWO", "RIGHT ONE", "RIGHT TWO"]);
    }

    #[test]
    fn single_cluster_band_reads_top_to_bottom() {
        let blocks = vec![
            RawLayoutBlock::new("second", Rect::new(90.0, 40.0, 440.0, 12.0)),
            RawLayoutBlock::new("first", Rect::new(60.0, 20.0, 460.0, 12.0)),
            RawLayoutBlock::new("third", Rect::new(80.0, 60.0, 450.0, 12.0)),
        ];
        let text = analyze_layout(blocks, 600.0, 800.0)
            .into_iter()
            .map(|block| block.text)
            .collect::<Vec<_>>();
        assert_eq!(text, vec!["first", "second", "third"]);
    }

    #[test]
    fn three_band_page_keeps_regions_in_reading_order() {
        let blocks = vec![
            RawLayoutBlock::new("FOOTER", Rect::new(40.0, 220.0, 520.0, 12.0)),
            RawLayoutBlock::new("RIGHT", Rect::new(330.0, 100.0, 180.0, 12.0)),
            RawLayoutBlock::new("TITLE", Rect::new(40.0, 20.0, 520.0, 16.0)),
            RawLayoutBlock::new("LEFT", Rect::new(40.0, 100.0, 180.0, 12.0)),
        ];
        let text = analyze_layout(blocks, 600.0, 800.0)
            .into_iter()
            .map(|block| block.text)
            .collect::<Vec<_>>();
        assert_eq!(text, vec!["TITLE", "LEFT", "RIGHT", "FOOTER"]);
    }

    #[test]
    fn metadata_row_classified_as_footer_not_deferred() {
        let blocks = vec![
            RawLayoutBlock::new(
                "Abstract-This is the full-width abstract opener spanning the page",
                Rect::new(50.0, 50.0, 500.0, 15.0),
            ),
            RawLayoutBlock::new("LEFT_ABSTRACT_ONE", Rect::new(50.0, 80.0, 200.0, 15.0)),
            RawLayoutBlock::new(
                "arXiv:1909.04694v4 [eess.SY] 18 Mar 2020",
                Rect::new(170.0, 100.0, 260.0, 12.0),
            ),
            RawLayoutBlock::new("LEFT_ABSTRACT_TWO", Rect::new(50.0, 120.0, 200.0, 15.0)),
            RawLayoutBlock::new("RIGHT_MOTIVATION_ONE", Rect::new(350.0, 80.0, 200.0, 15.0)),
            RawLayoutBlock::new("RIGHT_MOTIVATION_TWO", Rect::new(350.0, 120.0, 200.0, 15.0)),
        ];

        let (result, _) = analyze_layout_with_snapshot(blocks, 600.0, 800.0);
        let arxiv = result
            .iter()
            .find(|block| block.text.contains("arXiv:1909.04694"));
        assert!(arxiv.is_some(), "arxiv metadata");
        assert_eq!(arxiv.map(|block| block.block_type), Some(BlockType::Footer));

        let text = result
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        let left_two = text.iter().position(|value| *value == "LEFT_ABSTRACT_TWO");
        let right_one = text
            .iter()
            .position(|value| *value == "RIGHT_MOTIVATION_ONE");
        assert!(left_two.is_some(), "left abstract two");
        assert!(right_one.is_some(), "right motivation one");
        if let (Some(left_two), Some(right_one)) = (left_two, right_one) {
            assert!(
                left_two < right_one,
                "left column should precede right column"
            );
        }
    }

    #[test]
    fn header_and_footer_margin_classification() {
        let blocks = vec![
            RawLayoutBlock::new("Page 1", Rect::new(280.0, 12.0, 40.0, 10.0)),
            RawLayoutBlock::new("Body paragraph text.", Rect::new(50.0, 200.0, 500.0, 12.0)),
            RawLayoutBlock::new("Confidential", Rect::new(250.0, 740.0, 100.0, 12.0)),
        ];
        let result = analyze_layout(blocks, 600.0, 800.0);
        assert_eq!(result[0].block_type, BlockType::Header);
        assert_eq!(result[1].block_type, BlockType::Body);
        assert_eq!(result[2].block_type, BlockType::Footer);
    }

    #[test]
    fn dense_footer_band_splits_from_body() {
        let blocks = vec![
            RawLayoutBlock::new("Body line one", Rect::new(50.0, 100.0, 500.0, 14.0)),
            RawLayoutBlock::new("Body line two", Rect::new(50.0, 120.0, 500.0, 14.0)),
            RawLayoutBlock::new("Ref alpha", Rect::new(50.0, 740.0, 200.0, 8.0)),
            RawLayoutBlock::new("Ref beta", Rect::new(50.0, 752.0, 200.0, 8.0)),
            RawLayoutBlock::new("Ref gamma", Rect::new(50.0, 764.0, 200.0, 8.0)),
        ];
        let (_, snapshot) = analyze_layout_with_snapshot(blocks, 600.0, 800.0);
        assert!(
            snapshot.band_count >= 2,
            "expected footer band separate from body, got {} bands",
            snapshot.band_count
        );
    }

    #[test]
    fn structured_fragments_are_body_not_headings() {
        assert!(is_structured_fragment("Algorithm 1 CACC Car-Following"));
        assert!(is_structured_fragment("(12)"));
        assert!(is_structured_fragment("x = a + b / c ^ 2"));

        let result = analyze_layout(
            vec![
                RawLayoutBlock::new(
                    "Algorithm 1 CACC Car-Following",
                    Rect::new(50.0, 60.0, 300.0, 20.0),
                )
                .with_font_size(24.0),
                RawLayoutBlock::new("I. INTRODUCTION", Rect::new(50.0, 100.0, 300.0, 12.0))
                    .with_font_size(18.0),
                RawLayoutBlock::new(
                    "Prior research [6]-[8], [27] demonstrated the effect.",
                    Rect::new(50.0, 130.0, 350.0, 12.0),
                ),
                RawLayoutBlock::new(
                    "Ordinary body text establishes the page baseline.",
                    Rect::new(50.0, 160.0, 350.0, 12.0),
                ),
            ],
            600.0,
            800.0,
        );

        let algo = result
            .iter()
            .find(|block| block.text.contains("Algorithm 1"));
        assert!(algo.is_some(), "algorithm fragment");
        if let Some(algo) = algo {
            assert_eq!(algo.block_type, BlockType::Body);
            assert!(algo.fragment);
        }
        assert!(result
            .iter()
            .any(|block| block.text.contains("I. INTRODUCTION") && !block.fragment));
        assert!(result
            .iter()
            .any(|block| block.text.contains("Prior research")
                && block.block_type == BlockType::Body));
    }
}
