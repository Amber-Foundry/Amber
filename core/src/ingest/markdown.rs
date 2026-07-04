use crate::ingest::layout::{BlockType, TextBlock};

/// Assembles a sequential vector of layout `TextBlock`s into a clean Markdown document string.
pub fn assemble_markdown(blocks: &[TextBlock]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut prev_block_type: Option<BlockType> = None;

    for block in blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

        match block.block_type {
            BlockType::Heading(level) => {
                let hashes = "#".repeat((level.clamp(1, 6)) as usize);
                if prev_block_type.is_some() {
                    out.push_str("\n\n");
                }
                out.push_str(&format!("{hashes} {text}"));
            }
            BlockType::ListItem => {
                if prev_block_type == Some(BlockType::ListItem) {
                    out.push('\n');
                } else if prev_block_type.is_some() {
                    out.push_str("\n\n");
                }
                out.push_str(&format_list_item(text));
            }
            BlockType::Table => {
                if prev_block_type.is_some() {
                    out.push_str("\n\n");
                }
                out.push_str(text);
            }
            BlockType::Body => {
                if prev_block_type == Some(BlockType::Body) || prev_block_type.is_some() {
                    out.push_str("\n\n");
                }
                out.push_str(text);
            }
        }

        prev_block_type = Some(block.block_type);
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
