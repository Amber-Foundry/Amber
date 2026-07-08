use crate::ingest::layout::{BlockType, TextBlock};
use crate::ocr::engine::Rect;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct IngestBlock {
    pub formatted_text: String,
    pub block_type: BlockType,
    pub confidence: Option<f32>,
    pub page_index: usize,
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

    for block in blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

        if block.block_type == BlockType::Body {
            if let Some(ref mut pending) = pending_body {
                if should_merge_body_line(pending.bbox.as_ref(), block.bbox.as_ref()) {
                    append_body_line(&mut pending.text, text);
                    pending.bbox = union_optional_rects(pending.bbox.take(), block.bbox.clone());
                    if let Some(confidence) = block.confidence {
                        pending.confidence_sum += confidence;
                        pending.confidence_count += 1;
                    }
                    continue;
                }
            }

            flush_pending_body(&mut pending_body, &mut out, page_index);
            pending_body = Some(PendingBody::new(text, block.bbox.clone(), block.confidence));
            continue;
        }

        flush_pending_body(&mut pending_body, &mut out, page_index);

        let formatted = match block.block_type {
            BlockType::Heading(level) => {
                let hashes = "#".repeat((level.clamp(1, 6)) as usize);
                format!("{hashes} {text}")
            }
            BlockType::ListItem => format_list_item(text),
            BlockType::Table | BlockType::Body => text.to_string(),
        };

        out.push(IngestBlock {
            formatted_text: formatted,
            block_type: block.block_type,
            confidence: block.confidence,
            page_index,
        });
    }

    flush_pending_body(&mut pending_body, &mut out, page_index);
    out
}

struct PendingBody {
    text: String,
    bbox: Option<Rect>,
    confidence_sum: f32,
    confidence_count: usize,
}

impl PendingBody {
    fn new(text: &str, bbox: Option<Rect>, confidence: Option<f32>) -> Self {
        Self {
            text: text.to_string(),
            bbox,
            confidence_sum: confidence.unwrap_or(0.0),
            confidence_count: usize::from(confidence.is_some()),
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
            formatted_text: pending.text,
            block_type: BlockType::Body,
            confidence,
            page_index,
        });
    }
}

fn should_merge_body_line(prev: Option<&Rect>, next: Option<&Rect>) -> bool {
    let (Some(prev), Some(next)) = (prev, next) else {
        return true;
    };

    let vertical_gap = next.y - (prev.y + prev.height);
    let line_height = prev.height.max(next.height).max(1.0);
    let same_column =
        horizontal_overlap_ratio(prev, next) > 0.15 || (prev.x - next.x).abs() <= line_height * 3.0;

    vertical_gap >= -line_height * 0.5 && vertical_gap <= line_height * 1.8 && same_column
}

fn append_body_line(current: &mut String, next: &str) {
    let next_starts_lowercase = next.chars().next().is_some_and(char::is_lowercase);
    if current.ends_with('-') && next_starts_lowercase {
        current.pop();
        current.push_str(next);
    } else if next_starts_lowercase && starts_with_word_continuation(next) {
        current.push_str(next);
    } else {
        if !current.chars().last().is_some_and(char::is_whitespace) {
            current.push(' ');
        }
        current.push_str(next);
    }
}

fn starts_with_word_continuation(text: &str) -> bool {
    let first_word = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_ascii_lowercase();

    matches!(
        first_word.as_str(),
        "ate" | "bers" | "cations" | "cies" | "sequence" | "ment" | "tion" | "sion"
    )
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
            },
            IngestBlock {
                formatted_text: "- Item 2 on Page 1".to_string(),
                block_type: BlockType::ListItem,
                confidence: None,
                page_index: 1,
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
            },
            TextBlock {
                text: "ate and House of Representatives.".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 22.0, 160.0, 10.0)),
                confidence: None,
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
    fn test_markdown_assembler_joins_capitalized_word_fragments() {
        let blocks = vec![
            TextBlock {
                text: "The House shall be composed of Mem".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 10.0, 180.0, 10.0)),
                confidence: None,
            },
            TextBlock {
                text: "bers chosen every second Year".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 22.0, 160.0, 10.0)),
                confidence: None,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(
            ingest_blocks[0].formatted_text,
            "The House shall be composed of Members chosen every second Year"
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
            },
            TextBlock {
                text: "of the United States".to_string(),
                block_type: BlockType::Body,
                bbox: Some(Rect::new(10.0, 22.0, 120.0, 10.0)),
                confidence: None,
            },
        ];

        let ingest_blocks = assemble_markdown_blocks(&blocks, 0);

        assert_eq!(
            ingest_blocks[0].formatted_text,
            "CONSTITUTION of the United States"
        );
    }
}
