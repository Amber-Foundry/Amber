use crate::embed::{cosine_similarity, EmbedEngine};
use rusqlite::Connection;
use std::collections::HashSet;

pub const STOPWORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "aren't",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can't",
    "cannot",
    "could",
    "couldn't",
    "did",
    "didn't",
    "do",
    "does",
    "doesn't",
    "doing",
    "don't",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "had",
    "hadn't",
    "has",
    "hasn't",
    "have",
    "haven't",
    "having",
    "he",
    "he'd",
    "he'll",
    "he's",
    "her",
    "here",
    "here's",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "how's",
    "i",
    "i'd",
    "i'll",
    "i'm",
    "i've",
    "if",
    "in",
    "into",
    "is",
    "isn't",
    "it",
    "it's",
    "its",
    "itself",
    "let's",
    "me",
    "more",
    "most",
    "mustn't",
    "my",
    "myself",
    "no",
    "nor",
    "not",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "ought",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "same",
    "shan't",
    "she",
    "she'd",
    "she'll",
    "she's",
    "should",
    "shouldn't",
    "so",
    "some",
    "such",
    "than",
    "that",
    "that's",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "there's",
    "these",
    "they",
    "they'd",
    "they'll",
    "they're",
    "they've",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "wasn't",
    "we",
    "we'd",
    "we'll",
    "we're",
    "we've",
    "were",
    "weren't",
    "what",
    "what's",
    "when",
    "when's",
    "where",
    "where's",
    "which",
    "while",
    "who",
    "who's",
    "whom",
    "why",
    "why's",
    "with",
    "won't",
    "would",
    "wouldn't",
    "you",
    "you'd",
    "you'll",
    "you're",
    "you've",
    "your",
    "yours",
    "yourself",
    "yourselves",
];

pub const SIMILARITY_DUPLICATE: f64 = 0.85; // >85% → update proposal
pub const SIMILARITY_FLAG: f64 = 0.50; // 50–85% → duplicate flag

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimilarityClass {
    Update,
    DuplicateFlag,
    New,
}

pub fn tokenize(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .filter(|word| STOPWORDS.binary_search(&word.as_str()).is_err())
        .collect()
}

/// Compute the Jaccard similarity (|intersection| / |union|) of the tokenized sets.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a = tokenize(a);
    let set_b = tokenize(b);
    jaccard_similarity_pretokenized(&set_a, &set_b)
}

/// Compute Jaccard similarity from pre-tokenized sets, avoiding redundant tokenization
pub fn jaccard_similarity_pretokenized(set_a: &HashSet<String>, set_b: &HashSet<String>) -> f64 {
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }

    let intersection_size = set_a.intersection(set_b).count() as f64;
    let union_size = (set_a.len() + set_b.len()) as f64 - intersection_size;

    intersection_size / union_size
}

fn cosine_via_embed(
    candidate_text: &str,
    existing_text: &str,
    engine: &dyn EmbedEngine,
) -> Option<f64> {
    let texts = vec![candidate_text.to_string(), existing_text.to_string()];
    let embeddings = engine.embed(&texts).ok()?;
    if embeddings.len() != 2 {
        return None;
    }

    Some(cosine_similarity(&embeddings[0], &embeddings[1]))
}

/// Compare pre-combined text blocks (e.g. title + summary).
///
/// Uses embedding cosine similarity when an engine is available, falling back to
/// Jaccard token overlap when embeddings are unavailable.
pub fn compute_text_similarity(
    _conn: &Connection,
    candidate_text: &str,
    existing_text: &str,
    engine: Option<&dyn EmbedEngine>,
) -> f64 {
    engine
        .and_then(|eng| cosine_via_embed(candidate_text, existing_text, eng))
        .unwrap_or_else(|| jaccard_similarity(candidate_text, existing_text))
}

/// Classify a similarity score into its matching action classification.
pub fn classify_similarity(score: f64) -> SimilarityClass {
    if score > SIMILARITY_DUPLICATE {
        SimilarityClass::Update
    } else if score >= SIMILARITY_FLAG {
        SimilarityClass::DuplicateFlag
    } else {
        SimilarityClass::New
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::EmbedError;

    struct FakeEmbedEngine;

    impl EmbedEngine for FakeEmbedEngine {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("semantic match") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                })
                .collect())
        }

        fn model_id(&self) -> &str {
            "fake-model"
        }

        fn dims(&self) -> usize {
            2
        }
    }

    fn setup_conn() -> Connection {
        match Connection::open_in_memory() {
            Ok(conn) => conn,
            Err(err) => panic!("Failed to open test DB: {err}"),
        }
    }

    #[test]
    fn test_identical_strings() {
        let conn = setup_conn();
        let text = "MindVault is a local first secure personal knowledge base";
        let score = compute_text_similarity(&conn, text, text, None);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_disjoint_strings() {
        let conn = setup_conn();
        let a = "apple orange banana";
        let b = "computer keyboard mouse";
        let score = compute_text_similarity(&conn, a, b, None);
        assert!(score < f64::EPSILON);
    }

    #[test]
    fn test_partial_overlap() {
        let conn = setup_conn();
        // "learning Rust is fun" -> tokenize -> {"learning", "rust", "fun"} (3 words)
        // "learning Python is fun" -> tokenize -> {"learning", "python", "fun"} (3 words)
        // Intersection -> {"learning", "fun"} (2 words)
        // Union -> {"learning", "rust", "python", "fun"} (4 words)
        // Expected Jaccard -> 2 / 4 = 0.50
        let score = compute_text_similarity(
            &conn,
            "learning Rust is fun",
            "learning Python is fun",
            None,
        );
        assert!((score - 0.50).abs() < f64::EPSILON);
    }

    #[test]
    fn test_empty_inputs() {
        let conn = setup_conn();
        assert!(compute_text_similarity(&conn, "", "test", None) < f64::EPSILON);
        assert!(compute_text_similarity(&conn, "test", "", None) < f64::EPSILON);
        assert!(compute_text_similarity(&conn, "", "", None) < f64::EPSILON);
    }

    #[test]
    fn test_case_insensitivity() {
        let conn = setup_conn();
        let score = compute_text_similarity(&conn, "LEARNING RUST", "learning rust", None);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_stopword_filtering() {
        let conn = setup_conn();
        // "about a the learning" -> stopwords filtered -> {"learning"}
        // "learning" -> {"learning"}
        // Expected Jaccard -> 1.0
        let score = compute_text_similarity(&conn, "about a the learning", "learning", None);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_text_similarity_uses_cosine_when_engine_available() {
        let conn = setup_conn();
        let engine = FakeEmbedEngine;

        let score = compute_text_similarity(
            &conn,
            "semantic match candidate",
            "semantic match existing",
            Some(&engine),
        );

        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_contractions_stay_whole_before_stopword_filtering() {
        let tokens = tokenize("don't isn't i've");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_stopwords_remain_sorted_for_binary_search() {
        assert!(STOPWORDS.windows(2).all(|window| window[0] <= window[1]));
    }

    #[test]
    fn test_classification_buckets() {
        assert_eq!(classify_similarity(0.90), SimilarityClass::Update);
        assert_eq!(classify_similarity(0.85), SimilarityClass::DuplicateFlag);
        assert_eq!(classify_similarity(0.50), SimilarityClass::DuplicateFlag);
        assert_eq!(classify_similarity(0.49), SimilarityClass::New);
    }
}
