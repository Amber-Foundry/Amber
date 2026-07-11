use crate::ingest::layout::{BlockType, TextBlock};
use crate::ingest::text::collapse_internal_whitespace_block;
use crate::ocr::engine::Rect;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct IngestBlock {
    pub formatted_text: String,
    pub block_type: BlockType,
    pub confidence: Option<f32>,
    pub page_index: usize,
    #[serde(default)]
    pub fragment: bool,
}

/// Assembles a sequential vector of layout `TextBlock`s into a clean Markdown document string.
pub fn assemble_markdown(blocks: &[TextBlock]) -> String {
    let ingest_blocks = assemble_markdown_blocks(blocks, 0);
    join_ingest_blocks(&ingest_blocks)
}

/// Formats text blocks independently into structured `IngestBlock` representations.
pub fn assemble_markdown_blocks(blocks: &[TextBlock], page_index: usize) -> Vec<IngestBlock> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut pending_body: Option<PendingBody> = None;
    let coalesced_blocks = coalesce_consecutive_headings(blocks);

    for block in &coalesced_blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

        if block.block_type == BlockType::Body {
            if let Some(ref mut pending) = pending_body {
                if should_merge_body_line(
                    pending.bbox.as_ref(),
                    pending.last_bbox.as_ref(),
                    block.bbox.as_ref(),
                ) {
                    append_body_line(
                        &mut pending.text,
                        text,
                        pending.last_bbox.as_ref(),
                        block.bbox.as_ref(),
                        &pending.last_text,
                    );
                    pending.last_text = text.to_string();
                    pending.last_bbox = block.bbox.clone();
                    pending.bbox = union_optional_rects(pending.bbox.take(), block.bbox.clone());
                    pending.fragment |= block.fragment;
                    if let Some(confidence) = block.confidence {
                        pending.confidence_sum += confidence;
                        pending.confidence_count += 1;
                    }
                    continue;
                }
            }

            flush_pending_body(&mut pending_body, &mut out, page_index);
            pending_body = Some(PendingBody::new(
                text,
                block.bbox.clone(),
                block.confidence,
                block.fragment,
            ));
            continue;
        }

        flush_pending_body(&mut pending_body, &mut out, page_index);

        if matches!(block.block_type, BlockType::Header | BlockType::Footer) {
            out.push(IngestBlock {
                formatted_text: collapse_internal_whitespace_block(text),
                block_type: block.block_type,
                confidence: block.confidence,
                page_index,
                fragment: block.fragment,
            });
            continue;
        }

        let formatted = match block.block_type {
            BlockType::Heading(level) => {
                let hashes = "#".repeat((level.clamp(1, 6)) as usize);
                format!("{hashes} {text}")
            }
            BlockType::ListItem => format_list_item(text),
            BlockType::Header | BlockType::Footer => text.to_string(),
            BlockType::Table | BlockType::Body => text.to_string(),
        };

        out.push(IngestBlock {
            formatted_text: collapse_internal_whitespace_block(&formatted),
            block_type: block.block_type,
            confidence: block.confidence,
            page_index,
            fragment: block.fragment,
        });
    }

    flush_pending_body(&mut pending_body, &mut out, page_index);
    out
}

struct PendingBody {
    text: String,
    bbox: Option<Rect>,
    /// Last fragment only — used for mid-word gap detection (not the union bbox).
    last_bbox: Option<Rect>,
    last_text: String,
    confidence_sum: f32,
    confidence_count: usize,
    fragment: bool,
}

impl PendingBody {
    fn new(text: &str, bbox: Option<Rect>, confidence: Option<f32>, fragment: bool) -> Self {
        Self {
            text: text.to_string(),
            bbox: bbox.clone(),
            last_bbox: bbox,
            last_text: text.to_string(),
            confidence_sum: confidence.unwrap_or(0.0),
            confidence_count: usize::from(confidence.is_some()),
            fragment,
        }
    }

    fn confidence(&self) -> Option<f32> {
        if self.confidence_count > 0 {
            Some(self.confidence_sum / self.confidence_count as f32)
        } else {
            None
        }
    }
}

fn flush_pending_body(
    pending_body: &mut Option<PendingBody>,
    out: &mut Vec<IngestBlock>,
    page_index: usize,
) {
    if let Some(pending) = pending_body.take() {
        let confidence = pending.confidence();
        out.push(IngestBlock {
            formatted_text: collapse_internal_whitespace_block(&pending.text),
            block_type: BlockType::Body,
            confidence,
            page_index,
            fragment: pending.fragment,
        });
    }
}

fn should_merge_body_line(
    union_prev: Option<&Rect>,
    last_prev: Option<&Rect>,
    next: Option<&Rect>,
) -> bool {
    let (Some(union_prev), Some(next)) = (union_prev, next) else {
        return false;
    };

    // Prefer the last fragment for same-line detection so mid-word splits are merged
    // even when the pending union bbox already spans prior wrapped lines.
    if let Some(last_prev) = last_prev {
        let line_height = last_prev.height.max(next.height).max(1.0);
        if (next.y - last_prev.y).abs() <= line_height * 0.5 {
            let gap = next.x - (last_prev.x + last_prev.width);
            if gap > line_height * 3.0 {
                return false;
            }
            return next.x >= last_prev.x - line_height * 0.25;
        }
    }

    // Use the last visual fragment, not the union bounding box: a merged paragraph can
    // span many lines, and treating its total height as a line height wrongly joins the
    // next column or an unrelated block below it.
    let reference = last_prev.unwrap_or(union_prev);
    let vertical_gap = next.y - (reference.y + reference.height);
    let line_height = reference.height.max(next.height).max(1.0);
    let same_column = horizontal_overlap_ratio(reference, next) > 0.15
        || (reference.x - next.x).abs() <= line_height * 3.0;

    vertical_gap >= -line_height * 0.5 && vertical_gap <= line_height * 1.8 && same_column
}

fn append_body_line(
    current: &mut String,
    next: &str,
    prev_bbox: Option<&Rect>,
    next_bbox: Option<&Rect>,
    prev_text: &str,
) {
    let next = next.trim_start();
    let next_starts_lowercase = next.chars().next().is_some_and(char::is_lowercase);
    if current.ends_with('-')
        && next_starts_lowercase
        && (is_mid_word_split(prev_bbox, next_bbox, prev_text, next)
            || is_line_break_hyphen(prev_bbox, next_bbox))
    {
        current.pop();
        current.push_str(next);
    } else if is_mid_word_split(prev_bbox, next_bbox, prev_text, next) {
        // Same-line fragments whose boxes sit tighter than a normal word gap.
        current.push_str(next);
    } else {
        let trimmed_len = current.trim_end().len();
        current.truncate(trimmed_len);
        if !current.is_empty() && !next.chars().next().is_some_and(char::is_whitespace) {
            current.push(' ');
        }
        current.push_str(next);
    }
}

/// True when `next` continues the same word as `prev` based on horizontal gap,
/// matching the threshold used when merging PDF text objects on a visual line.
fn is_mid_word_split(
    prev: Option<&Rect>,
    next: Option<&Rect>,
    prev_text: &str,
    next_text: &str,
) -> bool {
    let (Some(prev), Some(next)) = (prev, next) else {
        return false;
    };

    let line_height = prev.height.max(next.height).max(1.0);
    // Mid-word splits only occur between fragments on the same visual line.
    if (next.y - prev.y).abs() > line_height * 0.5 {
        return false;
    }

    let gap = next.x - (prev.x + prev.width);
    let prev_char_width = average_char_width(prev_text, prev.width);
    let next_char_width = average_char_width(next_text, next.width);
    let space_threshold = prev_char_width
        .min(next_char_width)
        .mul_add(0.5, 0.0)
        .max(1.0);

    // Allow tiny negative gaps from measurement noise; anything larger is overlap/wrap.
    gap <= space_threshold && gap >= -space_threshold * 0.25
}

fn is_line_break_hyphen(prev: Option<&Rect>, next: Option<&Rect>) -> bool {
    let (Some(prev), Some(next)) = (prev, next) else {
        return false;
    };

    let line_height = prev.height.max(next.height).max(1.0);
    if (next.y - prev.y).abs() <= line_height * 0.5 {
        return false;
    }

    let vertical_gap = next.y - (prev.y + prev.height);
    horizontal_overlap_ratio(prev, next) > 0.15
        && vertical_gap >= -line_height * 0.5
        && vertical_gap <= line_height * 1.8
}

fn average_char_width(text: &str, width: f32) -> f32 {
    let char_count = text.chars().filter(|ch| !ch.is_whitespace()).count().max(1) as f32;
    (width / char_count).max(0.1)
}

fn horizontal_overlap_ratio(a: &Rect, b: &Rect) -> f32 {
    let left = a.x.max(b.x);
    let right = (a.x + a.width).min(b.x + b.width);
    let overlap = (right - left).max(0.0);
    overlap / a.width.min(b.width).max(1.0)
}

fn union_optional_rects(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            let right = (a.x + a.width).max(b.x + b.width);
            let bottom = (a.y + a.height).max(b.y + b.height);
            Some(Rect::new(x, y, right - x, bottom - y))
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn coalesce_consecutive_headings(blocks: &[TextBlock]) -> Vec<TextBlock> {
    let median_line_height = median_line_height(blocks);
    let mut out: Vec<TextBlock> = Vec::with_capacity(blocks.len());

    for block in blocks {
        if let Some(previous) = out.last_mut() {
            if should_merge_headings(previous, block, median_line_height) {
                previous.text = format!("{} {}", previous.text.trim(), block.text.trim());
                previous.bbox = union_optional_rects(previous.bbox.take(), block.bbox.clone());
                if let (Some(previous_confidence), Some(next_confidence)) =
                    (previous.confidence, block.confidence)
                {
                    previous.confidence = Some((previous_confidence + next_confidence) / 2.0);
                }
                continue;
            }
        }
        out.push(block.clone());
    }
    out
}

fn should_merge_headings(prev: &TextBlock, next: &TextBlock, median_line_height: f32) -> bool {
    let (BlockType::Heading(prev_level), BlockType::Heading(next_level)) =
        (prev.block_type, next.block_type)
    else {
        return false;
    };
    if prev_level != next_level || prev.fragment || next.fragment {
        return false;
    }
    let (Some(prev_bbox), Some(next_bbox)) = (prev.bbox.as_ref(), next.bbox.as_ref()) else {
        return false;
    };

    let vertical_gap = next_bbox.y - (prev_bbox.y + prev_bbox.height);
    let heading_line_height = median_line_height
        .max(prev_bbox.height)
        .max(next_bbox.height)
        .max(1.0);
    if vertical_gap >= heading_line_height * 1.2 {
        return false;
    }

    let same_visual_line = (next_bbox.y - prev_bbox.y).abs() <= heading_line_height * 0.25;
    !(same_visual_line && horizontal_overlap_ratio(prev_bbox, next_bbox) == 0.0)
}

fn median_line_height(blocks: &[TextBlock]) -> f32 {
    let mut heights: Vec<f32> = blocks
        .iter()
        .filter_map(|block| block.bbox.as_ref().map(|bbox| bbox.height))
        .filter(|height| *height > 0.0)
        .collect();
    if heights.is_empty() {
        return 1.0;
    }
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    heights[(heights.len() - 1) / 2]
}

/// Joins structured blocks into a formatted Markdown string, using `\n` for consecutive list items
/// on the same page, and `\n\n` for general paragraph or page boundaries.
pub fn join_ingest_blocks(blocks: &[IngestBlock]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut prev_block: Option<&IngestBlock> = None;

    for block in blocks {
        if let Some(prev) = prev_block {
            if prev.page_index != block.page_index {
                // Page transition: always break with double newline
                out.push_str("\n\n");
            } else if prev.block_type == BlockType::ListItem
                && block.block_type == BlockType::ListItem
            {
                // Consecutive list items on same page: single newline
                out.push('\n');
            } else {
                out.push_str("\n\n");
            }
        }

        out.push_str(&block.formatted_text);
        prev_block = Some(block);
    }

    out.trim().to_string()
}

/// Formats a list item string into standard GFM Markdown list item syntax.
fn format_list_item(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with('•')
        || trimmed.starts_with('-')
        || trimmed.starts_with('*')
        || trimmed.starts_with('◦')
        || trimmed.starts_with('▪')
    {
        let content = trimmed.trim_start_matches(|c| "•-*◦▪ ".contains(c));
        format!("- {}", content.trim())
    } else {
        // If it starts with digits like "1. ", preserve or format as number list
        format!("- {trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_assembler_basic() {
        let blocks = vec![
            TextBlock::new("CONSTITUTION OF THE UNITED STATES", BlockType::Heading(1)),
            TextBlock::new("We the People of the United States...", BlockType::Body),
            TextBlock::new("ARTICLE I", BlockType::Heading(2)),
            TextBlock::new(
                "• Section 1. All legislative powers...",
                BlockType::ListItem,
            ),
            TextBlock::new(
                "• Section 2. The House of Representatives...",
                BlockType::ListItem,
            ),
        ];

        let markdown = assemble_markdown(&blocks);

        let expected = "# CONSTITUTION OF THE UNITED STATES\n\n\
                        We the People of the United States...\n\n\
                        ## ARTICLE I\n\n\
                        - Section 1. All legislative powers...\n\
                        - Section 2. The House of Representatives...";

        assert_eq!(markdown, expected);
    }

    #[test]
    fn test_markdown_assembler_list_items_cross_pages() {
        let blocks = vec![
            IngestBlock {
                formatted_text: "- Item 1 on Page 0".to_string(),
                block_type: BlockType::ListItem,
                confidence: None,
                page_index: 0,
                fragment: false,
            },
            IngestBlock {
                formatted_text: "- Item 2 on Page 1".to_string(),
                block_type: BlockType::ListItem,
                confidence: None,
                page_index: 1,
                fragment: false,
            },
        ];

        let result = join_ingest_blocks(&blocks);
        // Should be joined with double newlines "\n\n" since they are on different pages
        assert_eq!(result, "- Item 1 on Page 0\n\n- Item 2 on Page 1");
    }

    #[test]
    fn test_markdown_assembler_merges_body_lines() {
        let blocks = vec![
            TextBlock {
                text: "Congress of the United States, which shall consist of a Sen-".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 220.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "ate and House of Representatives.".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 22.0, 160.0, 10.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(ingest_blocks.len(), 1);
        assert_eq!(
            ingest_blocks[0].formatted_text,
            "Congress of the United States, which shall consist of a Senate and House of Representatives."
        );
    }

    #[test]
    fn test_markdown_assembler_joins_same_line_word_fragments_by_gap() {
        // "Mem" + "bers" sit flush on one line (gap << inter-word space).
        let blocks = vec![
            TextBlock {
                text: "The House shall be composed of Mem".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 180.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "bers chosen every second Year".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(190.2, 10.1, 160.0, 10.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(
            ingest_blocks[0].formatted_text,
            "The House shall be composed of Members chosen every second Year"
        );
    }

    #[test]
    fn test_markdown_assembler_joins_novel_word_fragments_by_gap() {
        // Novel split not in any vocabulary — gap alone must join it.
        let blocks = vec![
            TextBlock {
                text: "Gov".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 18.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "ernment of the United States".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(28.2, 10.0, 150.0, 10.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(
            ingest_blocks[0].formatted_text,
            "Government of the United States"
        );
    }

    #[test]
    fn test_markdown_assembler_keeps_normal_line_wrap_space() {
        let blocks = vec![
            TextBlock {
                text: "CONSTITUTION".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 90.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "of the United States".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 22.0, 120.0, 10.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(
            ingest_blocks[0].formatted_text,
            "CONSTITUTION of the United States"
        );
    }

    #[test]
    fn test_markdown_assembler_does_not_merge_when_next_bbox_missing() {
        let blocks = vec![
            TextBlock {
                text: "First paragraph block.".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 120.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "Unrelated second block.".to_string(),
                block_type: BlockType::Body,
                bbox: None,
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(ingest_blocks.len(), 2);
        assert_eq!(ingest_blocks[0].formatted_text, "First paragraph block.");
        assert_eq!(ingest_blocks[1].formatted_text, "Unrelated second block.");
    }

    #[test]
    fn test_markdown_assembler_keeps_same_line_columns_separate() {
        let blocks = vec![
            TextBlock {
                text: "Left column".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 20.0, 60.0, 8.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "Right column".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(220.0, 20.0, 70.0, 8.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(ingest_blocks.len(), 2);
        assert_eq!(ingest_blocks[0].formatted_text, "Left column");
        assert_eq!(ingest_blocks[1].formatted_text, "Right column");
    }

    #[test]
    fn test_markdown_assembler_keeps_same_line_word_space() {
        // Same line, but gap looks like a normal inter-word space.
        let blocks = vec![
            TextBlock {
                text: "United".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 42.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "States".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(58.0, 10.0, 40.0, 10.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(ingest_blocks[0].formatted_text, "United States");
    }

    #[test]
    fn test_markdown_assembler_preserves_compound_hyphen() {
        let blocks = vec![
            TextBlock {
                text: "multi-".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 36.0, 10.0)),
                confidence: None,
                fragment: false,
            },
            TextBlock {
                text: "level".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(58.0, 10.0, 40.0, 10.0)),
                confidence: None,
                fragment: false,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(ingest_blocks[0].formatted_text, "multi- level");
    }

    fn heading(text: &str, level: u8, y: f32, x: f32) -> TextBlock {
        TextBlock {
            text: text.to_string(),
            block_type: BlockType::Heading(level),
            bbox: Some(Rect::new(x, y, 180.0, 14.0)),
            confidence: None,
            fragment: false,
        }
    }

    #[test]
    fn markdown_sink_collapses_duplicate_spaces_without_changing_paragraphs() {
        let blocks = vec![TextBlock {
            text: "Density  Preserving".to_string(),
            block_type: BlockType::Body,
            bbox: None,
            confidence: None,
            fragment: false,
        }];
        let markdown = assemble_markdown(&blocks);
        assert_eq!(markdown, "Density Preserving");
        crate::ingest::text::assert_no_duplicate_spaces(&markdown)
            .unwrap_or_else(|err| panic!("{err}"));
    }

    #[test]
    fn merge_three_line_title() {
        let blocks = vec![
            heading("Title Line One", 1, 10.0, 50.0),
            heading("Title Line Two", 1, 26.0, 50.0),
            heading("Title Line Three", 1, 42.0, 50.0),
        ];
        let assembled = assemble_markdown_blocks(&blocks, 0);
        assert_eq!(assembled.len(), 1);
        assert_eq!(
            assembled[0].formatted_text,
            "# Title Line One Title Line Two Title Line Three"
        );
    }

    #[test]
    fn heading_merge_rejects_different_levels_distant_and_side_by_side_titles() {
        let different_levels = assemble_markdown_blocks(
            &[
                heading("First", 1, 10.0, 50.0),
                heading("Second", 2, 26.0, 50.0),
            ],
            0,
        );
        assert_eq!(different_levels.len(), 2);

        let distant = assemble_markdown_blocks(
            &[
                heading("First", 1, 10.0, 50.0),
                heading("Second", 1, 200.0, 50.0),
            ],
            0,
        );
        assert_eq!(distant.len(), 2);

        let side_by_side = assemble_markdown_blocks(
            &[
                heading("Left", 1, 10.0, 50.0),
                heading("Right", 1, 10.0, 350.0),
            ],
            0,
        );
        assert_eq!(side_by_side.len(), 2);
    }
}
