use crate::ingest::layout::{BlockType, TextBlock};

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

    for block in blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

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
    out
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
}
