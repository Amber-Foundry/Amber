/// Collapses horizontal whitespace within a line while preserving word boundaries.
/// Newlines are handled by [`collapse_internal_whitespace_block`] so paragraph breaks
/// remain structural. Tabs follow the same single-space policy as ordinary spaces.
pub fn collapse_internal_whitespace_line(line: &str) -> String {
    let mut collapsed = String::with_capacity(line.len());
    let mut previous_was_space = false;

    for ch in line.trim().chars() {
        if ch.is_whitespace() || ch == '\u{00a0}' {
            if !previous_was_space {
                collapsed.push(' ');
                previous_was_space = true;
            }
        } else {
            collapsed.push(ch);
            previous_was_space = false;
        }
    }

    collapsed.trim_end().to_string()
}

/// Collapses horizontal whitespace per line without changing the number of newlines.
pub fn collapse_internal_whitespace_block(text: &str) -> String {
    text.split('\n')
        .map(collapse_internal_whitespace_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the first duplicate-space position, for focused whitespace assertions in tests.
pub fn assert_no_duplicate_spaces(text: &str) -> Result<(), String> {
    text.as_bytes()
        .windows(2)
        .position(|pair| pair == b"  ")
        .map_or(Ok(()), |index| {
            Err(format!("duplicate internal spaces at byte index {index}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_pdf_glyph_boundary_spaces() {
        assert_eq!(
            collapse_internal_whitespace_line("Density  Preserving"),
            "Density Preserving"
        );
    }

    #[test]
    fn preserves_single_spaces_and_normalizes_tabs() {
        assert_eq!(collapse_internal_whitespace_line("word word"), "word word");
        assert_eq!(collapse_internal_whitespace_line("a\t\tb"), "a b");
    }

    #[test]
    fn preserves_newline_count_and_trims_each_line() {
        assert_eq!(
            collapse_internal_whitespace_block(" line1 \n\n line2 "),
            "line1\n\nline2"
        );
    }

    #[test]
    fn accepts_valid_single_word_boundaries() {
        let text = collapse_internal_whitespace_line("Th is");
        assert_eq!(text, "Th is");
        assert_no_duplicate_spaces(&text).unwrap_or_else(|err| panic!("{err}"));
    }

    #[test]
    fn reports_duplicate_spaces() {
        assert!(assert_no_duplicate_spaces("one  two").is_err());
    }
}
