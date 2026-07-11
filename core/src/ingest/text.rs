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

/// Punctuation that typographically attaches to the preceding word (no space before).
const ATTACHES_PREVIOUS: &str = ",.;:!?%)]}'\"";

/// Opening punctuation that typographically attaches to the following word (no space after).
const ATTACHES_FOLLOWING: &str = "([{'\"";

/// Punctuation after which a word space is required before the next alphanumeric fragment.
const REQUIRES_SPACE_AFTER: &str = ",.;:!?";

/// True when `text` is only attaching punctuation (per-glyph PDF objects).
pub(crate) fn is_punctuation_only(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| ATTACHES_PREVIOUS.contains(c))
}

/// True when `text` should join without a leading space (e.g. `,` or `.` fragments).
pub(crate) fn attaches_to_previous_word(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed
        .chars()
        .next()
        .is_some_and(|c| ATTACHES_PREVIOUS.contains(c))
}

/// True when `text` should join without a trailing space (e.g. `(` fragments).
pub(crate) fn attaches_to_following_word(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed
        .chars()
        .last()
        .is_some_and(|c| ATTACHES_FOLLOWING.contains(c))
}

/// True when typography requires a space after `prev` before `next` (e.g. `,` + `world`).
fn requires_following_word_space(prev: &str, next: &str) -> bool {
    let prev = prev.trim_end();
    let next = next.trim_start();
    prev.chars()
        .last()
        .is_some_and(|c| REQUIRES_SPACE_AFTER.contains(c))
        && next.chars().next().is_some_and(|c| c.is_alphanumeric())
}

/// Geometry-first join: insert a word space only when gap exceeds threshold and typography allows.
pub(crate) fn should_insert_typographic_space(
    prev_text: &str,
    next_text: &str,
    gap: f32,
    threshold: f32,
) -> bool {
    let prev = prev_text.trim_end();
    let next = next_text.trim_start();
    if prev.is_empty() || next.is_empty() {
        return false;
    }
    if attaches_to_previous_word(next) || attaches_to_following_word(prev) {
        return false;
    }
    if requires_following_word_space(prev, next) {
        return true;
    }
    if (is_punctuation_only(prev) || is_punctuation_only(next)) && gap <= threshold {
        return false;
    }
    gap > threshold
}

/// True when text contains `word ,` or `( word` style spacing errors.
pub(crate) fn has_spurious_punctuation_spacing(text: &str) -> bool {
    assert_no_space_before_punctuation(text).is_err()
}

/// Fail on `word ,` or `word .` patterns (focused extraction-hygiene assertions).
pub fn assert_no_space_before_punctuation(text: &str) -> Result<(), String> {
    for (idx, window) in text.as_bytes().windows(2).enumerate() {
        if window[0] == b' ' && ATTACHES_PREVIOUS.as_bytes().contains(&window[1]) {
            return Err(format!(
                "space before punctuation '{}' at byte index {}",
                window[1] as char,
                idx + 1
            ));
        }
    }
    for (idx, window) in text.as_bytes().windows(2).enumerate() {
        if ATTACHES_FOLLOWING.as_bytes().contains(&window[0]) && window[1] == b' ' {
            return Err(format!(
                "space after opening punctuation '{}' at byte index {}",
                window[0] as char,
                idx + 1
            ));
        }
    }
    Ok(())
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

    #[test]
    fn typographic_space_before_comma() {
        assert!(!should_insert_typographic_space("Hello", ",", 5.0, 2.0));
        assert!(should_insert_typographic_space("Hello,", "world", 1.0, 3.0));
        assert!(!should_insert_typographic_space("word", ",", 5.0, 2.0));
        assert!(!should_insert_typographic_space("word", ".", 5.0, 2.0));
        assert!(should_insert_typographic_space("word", "next", 5.0, 2.0));
        assert!(!should_insert_typographic_space("(", "word", 5.0, 2.0));
        assert!(!should_insert_typographic_space("end", ")", 5.0, 2.0));
    }

    #[test]
    fn punctuation_only_objects() {
        assert!(is_punctuation_only(","));
        assert!(is_punctuation_only("."));
        assert!(!is_punctuation_only("word"));
    }

    #[test]
    fn rejects_space_before_punctuation() {
        assert!(assert_no_space_before_punctuation("Hello, world.").is_ok());
        assert!(assert_no_space_before_punctuation("word , next").is_err());
        assert!(assert_no_space_before_punctuation("( word").is_err());
    }
}
