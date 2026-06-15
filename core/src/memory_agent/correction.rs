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
        start = abs_pos + 1;
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
        let prev_lower = prev.to_lowercase();
        for word in prev_lower.split_whitespace() {
            // Check if current message negates a specific word/phrase from the previous message
            if message_lower.contains(&format!("not {}", word))
                || message_lower.contains(&format!("no, {}", word))
                || message_lower.contains(&format!("no {}", word))
            {
                return Some(CorrectionSignal::Negation {
                    negated_fragment: word.to_string(),
                });
            }
        }
    }

    // Check for contradictions with pending proposed data (title and summary only)
    for pending_raw in pending_proposed_data {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(pending_raw) {
            if let Some(title) = val.get("title").and_then(|t| t.as_str()) {
                let title_lower = title.to_lowercase();
                if message_lower.contains(&format!("not {}", title_lower))
                    || message_lower.contains(&format!("{} is wrong", title_lower))
                {
                    return Some(CorrectionSignal::ChangesetContradiction {
                        contradicted_field: title.to_string(),
                    });
                }
            }
            if let Some(summary) = val.get("summary").and_then(|s| s.as_str()) {
                let summary_lower = summary.to_lowercase();
                if message_lower.contains(&format!("not {}", summary_lower))
                    || message_lower.contains(&format!("{} is wrong", summary_lower))
                {
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
