use crate::ocr::engine::Rect;

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

/// True when `text` begins with an opening quote followed by a word character.
fn is_opening_quote_fragment(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some('"' | '\'') => chars.next().is_some_and(|c| c.is_alphanumeric()),
        _ => false,
    }
}

/// True when `text` should join without a leading space (e.g. `,` or `.` fragments).
pub(crate) fn attaches_to_previous_word(text: &str) -> bool {
    let trimmed = text.trim();
    if is_opening_quote_fragment(trimmed) {
        return false;
    }
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

/// True when `next` continues a decimal numeral after `prev` (e.g. `2.` + `5` → `2.5`).
pub(crate) fn is_decimal_continuation(prev: &str, next: &str) -> bool {
    let prev = prev.trim_end();
    let next = next.trim_start();
    prev.ends_with('.')
        && prev
            .strip_suffix('.')
            .and_then(|s| s.chars().last())
            .is_some_and(|c| c.is_ascii_digit())
        && next.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// True when typography requires a space after `prev` before `next` (e.g. `,` + `world`).
fn requires_following_word_space(prev: &str, next: &str) -> bool {
    let prev = prev.trim_end();
    let next = next.trim_start();
    let last = match prev.chars().last() {
        Some(c) => c,
        None => return false,
    };
    if !REQUIRES_SPACE_AFTER.contains(last) {
        return false;
    }
    if !next.chars().next().is_some_and(|c| c.is_alphanumeric()) {
        return false;
    }
    if last == '.' && is_decimal_continuation(prev, next) {
        return false;
    }
    true
}

fn letter_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphanumeric()).count()
}

fn boundary_prev_token(prev_text: &str) -> &str {
    prev_text.split_whitespace().last().unwrap_or(prev_text)
}

fn boundary_next_token(next_text: &str) -> &str {
    next_text.split_whitespace().next().unwrap_or(next_text)
}

fn average_char_width(text: &str, width: f32) -> f32 {
    let char_count = text.chars().filter(|ch| !ch.is_whitespace()).count().max(1) as f32;
    (width / char_count).max(0.1)
}

/// True when `text` is a multi-letter alphanumeric token (word-like fragment).
fn is_word_like_token(text: &str) -> bool {
    let trimmed = text.trim();
    letter_count(trimmed) >= 2 && trimmed.chars().all(|c| c.is_alphanumeric())
}

/// True when both sides are short single- or two-letter alphanumeric fragments.
fn is_glyph_like_pair(prev: &str, next: &str) -> bool {
    let prev_letters = letter_count(prev);
    let next_letters = letter_count(next);
    prev_letters <= 2
        && next_letters <= 2
        && prev.trim().chars().all(|c| c.is_alphanumeric())
        && next.trim().chars().all(|c| c.is_alphanumeric())
}

/// Kerning-scale gap: gaps at or below this are treated as same-word letter spacing.
pub(crate) fn kerning_gap_threshold(
    font_size: f32,
    _prev_bbox: Option<&Rect>,
    _next_bbox: Option<&Rect>,
) -> f32 {
    font_size.mul_add(0.08, 0.0).max(1.0)
}

/// Word-scale gap: gaps above this normally separate words (~¼ em).
pub(crate) fn word_gap_threshold(font_size: f32) -> f32 {
    font_size.mul_add(0.25, 0.0).max(1.0)
}

/// True when `next` continues the same word as `prev` based on horizontal gap.
pub(crate) fn is_mid_word_split(
    prev_bbox: Option<&Rect>,
    next_bbox: Option<&Rect>,
    prev_text: &str,
    next_text: &str,
) -> bool {
    let (Some(prev), Some(next)) = (prev_bbox, next_bbox) else {
        return false;
    };

    let line_height = prev.height.max(next.height).max(1.0);
    if (next.y - prev.y).abs() > line_height * 0.5 {
        return false;
    }

    let gap = next.x - (prev.x + prev.width);
    let prev_token = boundary_prev_token(prev_text);
    let next_token = boundary_next_token(next_text);
    let prev_letters = letter_count(prev_token);
    let next_letters = letter_count(next_token);

    let prev_char_width = average_char_width(prev_text, prev.width);
    let next_char_width = average_char_width(next_text, next.width);
    let space_threshold = prev_char_width
        .min(next_char_width)
        .mul_add(0.5, 0.0)
        .max(1.0);

    if gap <= space_threshold && gap >= -space_threshold * 0.25 {
        if prev_letters >= 5 && next_letters >= 2 {
            return false;
        }
        if prev_letters >= 3 && next_letters >= 2 {
            let font_size = line_height;
            return gap < word_gap_threshold(font_size);
        }
        return true;
    }

    if prev_letters >= 2 && next_letters >= 2 {
        let font_size = line_height;
        let word_thr = word_gap_threshold(font_size);
        let kern_thr = kerning_gap_threshold(font_size, Some(prev), Some(next));
        if gap > kern_thr && gap < word_thr {
            if prev_letters >= 3 && next_letters >= 3 {
                return true;
            }
            return false;
        }
    }

    false
}

/// Geometry-first join between pdfium text objects on the same visual line.
pub(crate) struct InterObjectJoinContext<'a> {
    pub prev_text: &'a str,
    pub next_text: &'a str,
    pub gap: f32,
    pub font_size: f32,
    pub prev_bbox: Option<&'a Rect>,
    pub next_bbox: Option<&'a Rect>,
}

/// Two-tier word-boundary join: kerning cap vs word cap, with punctuation guards.
pub(crate) fn should_insert_inter_object_space(ctx: &InterObjectJoinContext<'_>) -> bool {
    // Word-final marker from per-glyph Word/Docs exports (trailing space in object_text)
    if ctx
        .prev_text
        .chars()
        .last()
        .is_some_and(char::is_whitespace)
    {
        let next_trim = ctx.next_text.trim_start();
        if !next_trim.is_empty()
            && !attaches_to_previous_word(next_trim)
            && !attaches_to_following_word(ctx.prev_text.trim_end())
        {
            let prev_trim = ctx.prev_text.trim_end();
            let kern_thr = kerning_gap_threshold(ctx.font_size, ctx.prev_bbox, ctx.next_bbox);
            if letter_count(prev_trim) <= 2
                && is_glyph_like_pair(prev_trim, next_trim)
                && ctx.gap <= kern_thr
            {
                // Phantom trailing space on same-word glyph pair (e.g. narrow "i " + "s").
            } else if letter_count(prev_trim) >= 3 && letter_count(next_trim) >= 3 {
                // Longer fragments (e.g. Mem + bers) — defer to geometry.
            } else {
                return true;
            }
        }
    }

    let prev = ctx.prev_text.trim_end();
    let next = ctx.next_text.trim_start();
    if prev.is_empty() || next.is_empty() {
        return false;
    }
    if attaches_to_previous_word(next) || attaches_to_following_word(prev) {
        return false;
    }
    if is_decimal_continuation(prev, next) {
        return false;
    }
    if requires_following_word_space(prev, next) {
        return true;
    }

    let word_thr = word_gap_threshold(ctx.font_size);
    let kern_thr = kerning_gap_threshold(ctx.font_size, ctx.prev_bbox, ctx.next_bbox);

    let prev_bbox_ref = ctx.prev_bbox;
    let next_bbox_ref = ctx.next_bbox;
    let mid_word = is_mid_word_split(prev_bbox_ref, next_bbox_ref, prev, next);

    if is_glyph_like_pair(prev, next) && ctx.gap > word_thr {
        return true;
    }

    if is_word_like_token(prev)
        && is_word_like_token(next)
        && !mid_word
        && ctx.gap > kern_thr
        && ctx.gap <= word_thr
    {
        return true;
    }

    if mid_word {
        return false;
    }

    if is_glyph_like_pair(prev, next) {
        if ctx.gap <= kern_thr {
            return false;
        }
        if ctx.gap <= word_thr {
            if let (Some(pb), Some(nb)) = (prev_bbox_ref, next_bbox_ref) {
                let prev_cw = average_char_width(prev, pb.width);
                let next_cw = average_char_width(next, nb.width);
                let tight = prev_cw.min(next_cw).mul_add(0.5, 0.0).max(1.0);
                if ctx.gap <= tight {
                    return false;
                }
            } else {
                return false;
            }
        }
    }

    if (is_punctuation_only(prev) || is_punctuation_only(next)) && ctx.gap <= word_thr {
        return false;
    }

    ctx.gap > word_thr
}

/// Fail when `left` and `right` appear glued without a word boundary (fixture assertions).
pub fn assert_words_not_run_together(text: &str, left: &str, right: &str) -> Result<(), String> {
    let glued = format!("{left}{right}");
    if text.contains(&glued) {
        return Err(format!("found run-together pattern '{glued}'"));
    }
    Ok(())
}

/// True when text contains `word ,` or `( word` style spacing errors.
pub(crate) fn has_spurious_punctuation_spacing(text: &str) -> bool {
    assert_no_space_before_punctuation(text).is_err()
}

/// Fail on `word ,` or `word .` patterns (focused extraction-hygiene assertions).
pub fn assert_no_space_before_punctuation(text: &str) -> Result<(), String> {
    for (idx, window) in text.as_bytes().windows(2).enumerate() {
        if window[0] == b' ' {
            let rest = &text[idx + 1..];
            if is_opening_quote_fragment(rest) {
                continue;
            }
            if ATTACHES_PREVIOUS.as_bytes().contains(&window[1]) {
                return Err(format!(
                    "space before punctuation '{}' at byte index {}",
                    window[1] as char,
                    idx + 1
                ));
            }
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
    use crate::ocr::engine::Rect;

    fn typographic_join(prev: &str, next: &str, gap: f32, threshold: f32) -> bool {
        should_insert_inter_object_space(&InterObjectJoinContext {
            prev_text: prev,
            next_text: next,
            gap,
            font_size: threshold / 0.25,
            prev_bbox: None,
            next_bbox: None,
        })
    }

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
        assert!(!typographic_join("Hello", ",", 5.0, 2.0));
        assert!(typographic_join("Hello,", "world", 1.0, 3.0));
        assert!(!typographic_join("word", ",", 5.0, 2.0));
        assert!(!typographic_join("word", ".", 5.0, 2.0));
        assert!(typographic_join("word", "next", 5.0, 2.0));
        assert!(!typographic_join("(", "word", 5.0, 2.0));
        assert!(!typographic_join("end", ")", 5.0, 2.0));
    }

    #[test]
    fn typographic_space_before_opening_quote() {
        let gap = 12.0_f32 * 0.3;
        assert!(typographic_join("said", "\"Hello", gap, 3.0));
        assert!(typographic_join("said", "'Hello", gap, 3.0));
        assert!(!typographic_join("word", "\"", 2.0, 3.0));
        assert!(assert_no_space_before_punctuation("He said \"Hello\"").is_ok());
    }

    #[test]
    fn typographic_decimal_point_no_space() {
        let gap = 12.0_f32 * 0.3;
        assert!(!typographic_join("2.", "5", gap, 3.0));
        assert!(!typographic_join("0.", "0012", gap, 3.0));
        assert!(typographic_join("end.", "Next", gap, 3.0));
        assert!(typographic_join("Fig.", "2", gap, 3.0));
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

    #[test]
    fn intra_word_short_fragments_no_space() {
        let ctx = InterObjectJoinContext {
            prev_text: "M",
            next_text: "a",
            gap: 3.0,
            font_size: 12.0,
            prev_bbox: Some(&Rect::new(10.0, 20.0, 8.0, 12.0)),
            next_bbox: Some(&Rect::new(19.0, 20.0, 7.0, 12.0)),
        };
        assert!(!should_insert_inter_object_space(&ctx));
    }

    #[test]
    fn inter_word_tokens_insert_space() {
        let gap = 12.0_f32 * 0.24;
        let prev_bbox = Rect::new(10.0, 20.0, 14.0, 12.0);
        let next_bbox = Rect::new(10.0 + 14.0 + gap, 20.0, 22.0, 12.0);
        let ctx = InterObjectJoinContext {
            prev_text: "of",
            next_text: "this",
            gap,
            font_size: 12.0,
            prev_bbox: Some(&prev_bbox),
            next_bbox: Some(&next_bbox),
        };
        assert!(should_insert_inter_object_space(&ctx));
    }

    #[test]
    fn mid_word_split_no_space() {
        let ctx = InterObjectJoinContext {
            prev_text: "Mem",
            next_text: "bers",
            gap: 0.2,
            font_size: 8.0,
            prev_bbox: Some(&Rect::new(10.0, 20.0, 18.0, 8.0)),
            next_bbox: Some(&Rect::new(28.2, 20.1, 24.0, 8.0)),
        };
        assert!(!should_insert_inter_object_space(&ctx));
    }

    #[test]
    fn word_gap_threshold_at_quarter_em() {
        assert_eq!(word_gap_threshold(12.0), 3.0);
        assert_eq!(word_gap_threshold(4.0), 1.0);
    }

    #[test]
    fn word_final_trailing_space_marks_boundary() {
        let ctx = InterObjectJoinContext {
            prev_text: "d ",
            next_text: "w",
            gap: 2.0,
            font_size: 12.0,
            prev_bbox: Some(&Rect::new(10.0, 20.0, 6.0, 12.0)),
            next_bbox: Some(&Rect::new(18.0, 20.0, 7.0, 12.0)),
        };
        assert!(should_insert_inter_object_space(&ctx));
    }

    #[test]
    fn single_glyph_pair_large_gap_inserts_space() {
        let gap = 12.0_f32 * 0.3;
        let ctx = InterObjectJoinContext {
            prev_text: "d",
            next_text: "w",
            gap,
            font_size: 12.0,
            prev_bbox: Some(&Rect::new(10.0, 20.0, 6.0, 12.0)),
            next_bbox: Some(&Rect::new(10.0 + 6.0 + gap, 20.0, 7.0, 12.0)),
        };
        assert!(should_insert_inter_object_space(&ctx));
    }
}
