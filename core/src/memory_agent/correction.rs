//! This module defines the logic for detecting correction signals in user messages.

fn contains_phrase_with_boundaries(message: &str, phrase: &str) -> bool {
    let msg_len = message.len();
    let phrase_len = phrase.len();
    if phrase_len == 0 {
        return true;
    }

    let mut start = 0;
    while let Some(pos) = message[start..].find(phrase) {
        let abs_pos = start + pos;
        let end_pos = abs_pos + phrase_len;

        let before_ok = if abs_pos == 0 {
            true
        } else {
            message[..abs_pos]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric())
        };

        let after_ok = if end_pos >= msg_len {
            true
        } else {
            message[end_pos..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
        };

        if before_ok && after_ok {
            return true;
        }
        let char_len = message[abs_pos..]
            .chars()
            .next()
            .map_or(1, |c| c.len_utf8());
        start = abs_pos + char_len;
    }
    false
}

/// Evaluates whether a user message contains correction signals.
/// Returns `Some(CorrectionSignal)` if detected, `None` otherwise.
pub fn detect_correction_signal(
    message: &str,
    previous_message: Option<&str>,
    pending_proposed_data: &[String],
) -> Option<CorrectionSignal> {
    let message_lower = message.to_lowercase();

    // 1. Explicit Phrase Scan
    for phrase in CORRECTION_PHRASES {
        if contains_phrase_with_boundaries(&message_lower, phrase) {
            return Some(CorrectionSignal::ExplicitPhrase {
                phrase: phrase.to_string(),
            });
        }
    }

    // 2. Direct Negation Scan
    if let Some(prev) = previous_message {
        use std::collections::HashSet;
        let mut prev_words = HashSet::new();
        for word in prev.to_lowercase().split_whitespace() {
            let clean_word = word.trim_matches(|c: char| !c.is_alphanumeric());
            if !clean_word.is_empty()
                && crate::memory_agent::similarity::STOPWORDS
                    .binary_search(&clean_word)
                    .is_err()
            {
                prev_words.insert(clean_word.to_string());
            }
        }

        let current_words: Vec<&str> = message_lower.split_whitespace().collect();
        for i in 0..current_words.len() {
            let clean_curr = current_words[i].trim_matches(|c: char| !c.is_alphanumeric());
            if clean_curr == "not" || clean_curr == "no" {
                if let Some(next_word) = current_words.get(i + 1) {
                    let clean_next = next_word.trim_matches(|c: char| !c.is_alphanumeric());
                    if prev_words.contains(clean_next) {
                        return Some(CorrectionSignal::Negation {
                            negated_fragment: clean_next.to_string(),
                        });
                    }
                }
            }
        }
    }

    // Check for contradictions with pending proposed data (title and summary only)
    for pending_raw in pending_proposed_data {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(pending_raw) {
            if let Some(title) = val.get("title").and_then(|t| t.as_str()) {
                let title_lower = title.to_lowercase();
                if contains_phrase_with_boundaries(&message_lower, &format!("not {}", title_lower))
                    || contains_phrase_with_boundaries(
                        &message_lower,
                        &format!("{} is wrong", title_lower),
                    )
                {
                    return Some(CorrectionSignal::ChangesetContradiction {
                        contradicted_field: title.to_string(),
                    });
                }
            }
            if let Some(summary) = val.get("summary").and_then(|s| s.as_str()) {
                let summary_lower = summary.to_lowercase();
                if contains_phrase_with_boundaries(
                    &message_lower,
                    &format!("not {}", summary_lower),
                ) || contains_phrase_with_boundaries(
                    &message_lower,
                    &format!("{} is wrong", summary_lower),
                ) {
                    return Some(CorrectionSignal::ChangesetContradiction {
                        contradicted_field: summary.to_string(),
                    });
                }
            }
        }
    }

    None
}

#[derive(Debug, Clone, PartialEq)]
pub enum CorrectionSignal {
    /// Explicit correction phrases: "actually," "wait," "I meant," "not X, Y," etc.
    ExplicitPhrase { phrase: String },
    /// Direct negation of a prior message value
    Negation { negated_fragment: String },
    /// Contradiction of a field in a pending changeset item
    ChangesetContradiction { contradicted_field: String },
}

const CORRECTION_PHRASES: &[&str] = &[
    "actually",
    "actually,",
    "wait,",
    "wait",
    "i meant",
    "not that",
    "correction",
    "correction:",
    "to clarify",
    "scratch that",
    "never mind",
    "nevermind",
    "no wait",
    "i was wrong",
    "let me correct",
    "that's wrong",
    "that's not right",
    "i misspoke",
];

/// Returns `true` if any correction signal is detected in the message.
pub fn has_correction_signal(
    message: &str,
    previous_message: Option<&str>,
    pending_proposed_data: &[String],
) -> bool {
    detect_correction_signal(message, previous_message, pending_proposed_data).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_phrase_with_boundaries_unicode_start() {
        // This test would trigger a panic with the original `start = abs_pos + 1` logic
        assert!(!contains_phrase_with_boundaries("a⚠️test", "⚠️test"));
    }

    #[test]
    fn test_negation_scan_ignores_punctuation() {
        let prev = "My favorite color is blue.";
        let current = "not blue";
        let signal = detect_correction_signal(current, Some(prev), &[]);
        assert_eq!(
            signal,
            Some(CorrectionSignal::Negation {
                negated_fragment: "blue".to_string()
            })
        );
    }

    #[test]
    fn test_negation_scan_respects_word_boundaries() {
        let prev = "My favorite color is blue.";
        let current = "not blueprint";
        let signal = detect_correction_signal(current, Some(prev), &[]);
        assert_eq!(signal, None);

        let current_ok = "not blue";
        let signal_ok = detect_correction_signal(current_ok, Some(prev), &[]);
        assert!(signal_ok.is_some());
    }

    #[test]
    fn test_negation_scan_ignores_stopwords() {
        let prev = "It is to be or not to be.";
        let current = "not to";
        let signal = detect_correction_signal(current, Some(prev), &[]);
        assert_eq!(signal, None);

        let current_neutral = "is it correct";
        let signal_neutral = detect_correction_signal(current_neutral, Some(prev), &[]);
        assert_eq!(signal_neutral, None);
    }
}
