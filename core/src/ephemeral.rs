use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// An in-memory, session-scoped chunk of an attached document with its computed embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EphemeralChunk {
    pub chunk_index: usize,
    pub text: String,
    pub token_count: usize,
    pub heading_context: Option<String>,
    pub source_page_indices: Vec<usize>,
    pub embedding: Vec<f32>,
    pub ocr_confidence: Option<f32>,
    pub tables_unstructured: bool,
}

/// Metadata summary for an attached document stored in the ephemeral chunk store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EphemeralAttachmentSummary {
    pub session_id: String,
    pub attachment_id: String,
    pub source_name: String,
    pub file_path: String,
    pub total_chunks: usize,
    pub total_tokens: usize,
    pub page_count: usize,
    pub ocr_confidence: Option<f32>,
    pub prompt_injection_flagged: bool,
    pub needs_ocr_models: bool,
    pub embeddings_computed: bool,
}

/// Thread-safe in-memory store for attached-document chunks + embeddings.
/// Keyed strictly by `(session_id, attachment_id)`. No DB persistence.
#[derive(Debug, Clone, Default)]
pub struct EphemeralChunkStore {
    chunks: HashMap<(String, String), Vec<EphemeralChunk>>,
    summaries: HashMap<(String, String), EphemeralAttachmentSummary>,
    compute_counts: HashMap<(String, String), usize>,
}

impl EphemeralChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store chunks and summary for an attached document. Increments compute count for tracking.
    pub fn insert_attachment(
        &mut self,
        summary: EphemeralAttachmentSummary,
        chunks: Vec<EphemeralChunk>,
    ) {
        let key = (summary.session_id.clone(), summary.attachment_id.clone());
        let current_count = self.compute_counts.get(&key).copied().unwrap_or(0);
        self.compute_counts.insert(key.clone(), current_count + 1);
        self.summaries.insert(key.clone(), summary);
        self.chunks.insert(key, chunks);
    }

    /// Retrieve chunks for a specific session + attachment.
    pub fn get_attachment_chunks(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Option<&Vec<EphemeralChunk>> {
        self.chunks
            .get(&(session_id.to_string(), attachment_id.to_string()))
    }

    /// Retrieve all chunks for all attachments belonging to a session.
    pub fn get_session_chunks(&self, session_id: &str) -> Vec<EphemeralChunk> {
        let mut session_chunks = Vec::new();
        for ((s_id, _), chunks) in &self.chunks {
            if s_id == session_id {
                session_chunks.extend(chunks.clone());
            }
        }
        session_chunks.sort_by_key(|c| c.chunk_index);
        session_chunks
    }

    /// Retrieve summary metadata for a specific attachment.
    pub fn get_attachment_summary(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Option<&EphemeralAttachmentSummary> {
        self.summaries
            .get(&(session_id.to_string(), attachment_id.to_string()))
    }

    /// Remove a specific attachment from memory when detached.
    pub fn remove_attachment(&mut self, session_id: &str, attachment_id: &str) -> bool {
        let key = (session_id.to_string(), attachment_id.to_string());
        let existed = self.chunks.remove(&key).is_some();
        self.summaries.remove(&key);
        self.compute_counts.remove(&key);
        existed
    }

    /// Clear all attachments for a session when the session ends or is switched.
    pub fn clear_session(&mut self, session_id: &str) -> usize {
        let keys_to_remove: Vec<(String, String)> = self
            .chunks
            .keys()
            .filter(|(s_id, _)| s_id == session_id)
            .cloned()
            .collect();

        let removed_count = keys_to_remove.len();
        for key in keys_to_remove {
            self.chunks.remove(&key);
            self.summaries.remove(&key);
            self.compute_counts.remove(&key);
        }
        removed_count
    }

    /// Clear all stored data across all sessions.
    pub fn clear_all(&mut self) {
        self.chunks.clear();
        self.summaries.clear();
        self.compute_counts.clear();
    }

    /// Check if a specific attachment exists in the store.
    pub fn has_attachment(&self, session_id: &str, attachment_id: &str) -> bool {
        self.chunks
            .contains_key(&(session_id.to_string(), attachment_id.to_string()))
    }

    /// Total number of attachments currently held in memory.
    pub fn total_attachments(&self) -> usize {
        self.chunks.len()
    }

    /// Total number of chunks stored across all active attachments.
    pub fn total_chunks(&self) -> usize {
        self.chunks.values().map(|c| c.len()).sum()
    }

    /// Return how many times embeddings were computed for a specific attachment.
    pub fn embedding_compute_count(&self, session_id: &str, attachment_id: &str) -> usize {
        self.compute_counts
            .get(&(session_id.to_string(), attachment_id.to_string()))
            .copied()
            .unwrap_or(0)
    }

    /// Return total tokens across all active attachments for a session.
    pub fn get_session_total_tokens(&self, session_id: &str) -> usize {
        self.summaries
            .iter()
            .filter(|((s_id, _), _)| s_id == session_id)
            .map(|(_, s)| s.total_tokens)
            .sum()
    }

    /// Return total chunks held for a session.
    pub fn get_session_total_chunks(&self, session_id: &str) -> usize {
        self.chunks
            .iter()
            .filter(|((s_id, _), _)| s_id == session_id)
            .map(|(_, c)| c.len())
            .sum()
    }

    /// Get all chunks for a session ordered by original chunk index.
    pub fn get_all_session_chunks(&self, session_id: &str) -> Vec<EphemeralChunk> {
        let mut all_chunks: Vec<(usize, EphemeralChunk)> = Vec::new();
        for ((s_id, _), chunks) in &self.chunks {
            if s_id == session_id {
                for c in chunks {
                    all_chunks.push((c.chunk_index, c.clone()));
                }
            }
        }
        all_chunks.sort_by_key(|(idx, _)| *idx);
        all_chunks.into_iter().map(|(_, c)| c).collect()
    }

    /// Get chunks for a session capped to a max token budget (e.g. 3200 tokens).
    /// If total chunks exceed the cap, samples evenly across the document to provide broad coverage without blowing the context budget.
    pub fn get_budget_capped_session_chunks(
        &self,
        session_id: &str,
        max_tokens: usize,
    ) -> Vec<EphemeralChunk> {
        let all_chunks = self.get_all_session_chunks(session_id);
        if all_chunks.is_empty() {
            return Vec::new();
        }

        let total_tokens: usize = all_chunks.iter().map(|c| c.token_count).sum();
        if total_tokens <= max_tokens || all_chunks.len() <= 10 {
            return all_chunks;
        }

        // Evenly sample up to 10 chunks across the full document range
        let target_count = 10;
        let mut sampled = Vec::new();
        let step = (all_chunks.len() - 1) as f32 / (target_count - 1) as f32;

        for i in 0..target_count {
            let idx = (i as f32 * step).round() as usize;
            if idx < all_chunks.len()
                && !sampled
                    .iter()
                    .any(|c: &EphemeralChunk| c.chunk_index == all_chunks[idx].chunk_index)
            {
                sampled.push(all_chunks[idx].clone());
            }
        }
        sampled
    }

    /// Evaluate if retrieval should be bypassed for a session turn.
    /// Returns (bypass: bool, reason: &str).
    pub fn should_bypass_retrieval(
        &self,
        session_id: &str,
        user_prompt: &str,
    ) -> (bool, &'static str) {
        let total_tokens = self.get_session_total_tokens(session_id);
        let total_chunks = self.get_session_total_chunks(session_id);

        if total_chunks == 0 {
            return (false, "No ephemeral chunks in store");
        }

        // Small-document bypass: if full text fits in context budget (<= 1500 tokens or <= 3 chunks), bypass retrieval
        if total_tokens <= 1500 || total_chunks <= 3 {
            return (
                true,
                "Small document fits fully in context budget — retrieval bypassed",
            );
        }

        // Broad-question handling: detect whole-document summarization phrasing
        if is_broad_summarization_query(user_prompt) {
            return (
                true,
                "Broad document request detected — sending full budgeted document text",
            );
        }

        (false, "Selective top-K smart retrieval active")
    }

    /// Perform hybrid vector + TF-IDF keyword search over all attached chunks in a session.
    /// Returns up to `top_k` most relevant EphemeralChunk items ranked by descending match score.
    pub fn query_session_chunks(
        &self,
        session_id: &str,
        query_text: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Vec<EphemeralChunk> {
        let mut session_chunks: Vec<(&(String, String), &EphemeralChunk)> = Vec::new();
        for (key, chunks) in &self.chunks {
            if key.0 == session_id {
                for c in chunks {
                    session_chunks.push((key, c));
                }
            }
        }

        if session_chunks.is_empty() {
            return Vec::new();
        }

        let query_tokens = normalize_search_tokens(query_text);
        let total_docs = session_chunks.len() as f32;

        // Calculate IDF for each query token
        let mut token_idf = std::collections::HashMap::new();
        for qtok in &query_tokens {
            let doc_freq = session_chunks
                .iter()
                .filter(|(_, c)| {
                    let c_tokens = normalize_search_tokens(&c.text);
                    let h_tokens =
                        normalize_search_tokens(c.heading_context.as_deref().unwrap_or(""));
                    c_tokens.contains(qtok) || h_tokens.contains(qtok)
                })
                .count() as f32;

            let idf = (total_docs / (doc_freq + 1.0)).ln().max(0.1);
            token_idf.insert(qtok.clone(), idf);
        }

        let mut scored_chunks: Vec<(f32, EphemeralChunk)> = Vec::new();

        for (key, chunk) in session_chunks {
            // Adaptive Weighting per Attachment: If real semantic embeddings are available for this specific attachment,
            // balance vector + keyword (0.4 / 0.6). If fallback bag-of-words vectors were used, rely 100% on pure keyword ranking.
            let chunk_embeddings_computed = self
                .summaries
                .get(key)
                .map(|s| s.embeddings_computed)
                .unwrap_or(false);

            let (vector_weight, keyword_weight) = if chunk_embeddings_computed {
                (0.4f32, 0.6f32)
            } else {
                (0.0f32, 1.0f32)
            };

            let vector_score = cosine_similarity(query_embedding, &chunk.embedding);

            let chunk_tokens = normalize_search_tokens(&chunk.text);
            let heading_tokens =
                normalize_search_tokens(chunk.heading_context.as_deref().unwrap_or(""));

            let mut keyword_score = 0.0f32;
            for qtok in &query_tokens {
                let idf = token_idf.get(qtok).cloned().unwrap_or(0.1);
                let in_chunk = chunk_tokens.contains(qtok);
                let in_heading = heading_tokens.contains(qtok);

                if in_heading {
                    keyword_score += idf * 4.0;
                } else if in_chunk {
                    keyword_score += idf * 1.5;
                }
            }

            let combined_score = vector_score * vector_weight + keyword_score * keyword_weight;
            scored_chunks.push((combined_score, chunk.clone()));
        }

        scored_chunks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let k = top_k.max(1);
        scored_chunks
            .into_iter()
            .take(k)
            .map(|(_, chunk)| chunk)
            .collect()
    }
}

/// Helper to perform lightweight English suffix stemming (plurals, past tense, gerunds).
fn stem_search_token(w: &str) -> Option<String> {
    if w.len() <= 3 {
        return None;
    }
    if w.ends_with("ies") && w.len() > 4 {
        return Some(format!("{}y", &w[..w.len() - 3]));
    }
    if w.ends_with("es") && w.len() > 3 {
        let stem_before_es = &w[..w.len() - 2];
        // Strip 2 chars ("es") ONLY if preceded by sibilants (s, x, z, ch, sh) e.g. "boxes" -> "box", "watches" -> "watch"
        if stem_before_es.ends_with("ch")
            || stem_before_es.ends_with("sh")
            || stem_before_es.ends_with('x')
            || stem_before_es.ends_with('z')
            || stem_before_es.ends_with("ss")
        {
            return Some(stem_before_es.to_string());
        }
        // Otherwise for words with silent 'e' + 's' ("crimes", "notes", "cases"), strip trailing 's' -> "crime", "note", "case"
        return Some(w[..w.len() - 1].to_string());
    }
    if w.ends_with('s') && !w.ends_with("ss") {
        return Some(w[..w.len() - 1].to_string());
    }
    if w.ends_with("ing") && w.len() > 4 {
        return Some(w[..w.len() - 3].to_string());
    }
    if w.ends_with("ed") && w.len() > 3 {
        return Some(w[..w.len() - 2].to_string());
    }
    None
}

/// Normalize text into clean search tokens by filtering out standard English stop words and generating stems.
pub fn normalize_search_tokens(text: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let stop_words = [
        "what", "does", "the", "document", "mention", "about", "is", "are", "and", "for", "with",
        "this", "that", "from", "have", "has", "can", "you", "tell", "me", "show", "find", "was",
        "were", "where", "which", "who", "when", "how", "why", "been", "being",
    ];

    for word in text.split_whitespace() {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();

        if clean.len() >= 2 && !stop_words.contains(&clean.as_str()) {
            if let Some(stemmed) = stem_search_token(&clean) {
                if stemmed.len() >= 2 && !stop_words.contains(&stemmed.as_str()) {
                    set.insert(stemmed);
                }
            }
            set.insert(clean);
        }
    }
    set
}

/// Check if user prompt contains whole-document or broad summarization keywords.
pub fn is_broad_summarization_query(user_prompt: &str) -> bool {
    let raw_lower = user_prompt.to_lowercase();

    // 1. Normalized Text (Punctuation-Agnostic with Apostrophe Stripping)
    // Strip apostrophes directly ("bird's" -> "birds"), then replace non-alphanumeric with spaces ("tl;dr" -> "tl dr")
    let strip_apostrophe = raw_lower.replace('\'', "");
    let clean_alpha: String = strip_apostrophe
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let tokens: Vec<&str> = clean_alpha.split_whitespace().collect();
    let collapsed = tokens.join(" ");

    // 2. Explicit Whole-Document Scope (Absolute Highest Precedence)
    // Phrases like "entire document", "whole file", "all sections", "cover to cover" explicitly request
    // the ENTIRE document, overriding any localized section mention.
    let explicit_whole_doc_phrases = [
        "entire document",
        "whole document",
        "full document",
        "entire file",
        "whole file",
        "full file",
        "all sections",
        "all pages",
        "every section",
        "whole thing",
        "entire text",
        "cover to cover",
        "beginning to end",
        "start to finish",
    ];

    if explicit_whole_doc_phrases
        .iter()
        .any(|phrase| collapsed.contains(phrase) || raw_lower.contains(phrase))
    {
        return true;
    }

    // 3. Localized Narrow Section Guard:
    // If the user specifies a localized section/chapter/page (e.g. "chapter 3", "chapter3", "section 2", "page 15", "line 12", "line12"),
    // return false so smart Top-K vector retrieval pinpoints that specific section instead of whole-document sampling!
    let localized_prefix_targets = [
        "chapter",
        "section",
        "page",
        "paragraph",
        "part",
        "article",
        "line",
    ];
    let localized_exact_targets = ["this function", "closure error", "bug in"];

    let has_localized_token = tokens.iter().any(|tok| {
        localized_prefix_targets.iter().any(|prefix| {
            if let Some(suffix) = tok.strip_prefix(prefix) {
                // Match "chapter", "chapter3", "line12", but NOT "participation", "partner", "pagination", or "lineage"
                suffix.is_empty() || suffix.chars().all(|c| c.is_numeric())
            } else {
                false
            }
        })
    });
    let has_localized_exact = localized_exact_targets
        .iter()
        .any(|target| raw_lower.contains(target));

    if has_localized_token || has_localized_exact {
        return false;
    }

    // 4. Broad Summarization Intent Terms:
    // Now that localized section requests (e.g. "summarize section 2") have been guarded by Step 3,
    // general broad summary requests ("summarize", "overview", "tldr", "executive summary", "overview of the code") return true.
    let broad_summary_terms = [
        "summarize",
        "summary",
        "overview",
        "synopsis",
        "digest",
        "tldr",
        "tl dr",
        "executive summary",
        "table of contents",
        "toc",
        "nutshell",
    ];

    for term in &broad_summary_terms {
        if collapsed.contains(term) || raw_lower.contains(term) {
            return true;
        }
    }

    // 5. Ambiguous Terms (require explicit document noun scope, e.g. "takeaways from the pdf")
    let ambiguous_terms = ["outline", "takeaway", "takeaways", "highlights", "recap"];
    let doc_scope_nouns = [
        "document",
        "documents",
        "file",
        "files",
        "pdf",
        "pdfs",
        "manual",
        "manuals",
        "presentation",
        "presentations",
        "paper",
        "papers",
        "text",
        "texts",
        "book",
        "books",
        "code",
        "codebase",
    ];

    let has_ambiguous = tokens.iter().any(|tok| ambiguous_terms.contains(tok));
    let has_doc_scope = tokens.iter().any(|tok| doc_scope_nouns.contains(tok));

    if has_ambiguous && has_doc_scope {
        return true;
    }

    // 6. Additional Broad Scope Phrases (Punctuation-Agnostic)
    let broad_scope_phrases = [
        "big picture",
        "birds eye",
        "bird eye",
        "what is this about",
        "what is this document about",
        "what is the document about",
        "what does this document say",
        "what does the document say",
        "explain the document",
    ];

    broad_scope_phrases
        .iter()
        .any(|phrase| collapsed.contains(phrase) || raw_lower.contains(phrase))
}

/// Compute cosine similarity between two vector slices.
pub fn cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
    if v1.len() != v2.len() || v1.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm1 = 0.0f32;
    let mut norm2 = 0.0f32;
    for (a, b) in v1.iter().zip(v2.iter()) {
        dot += a * b;
        norm1 += a * a;
        norm2 += b * b;
    }
    let denom = (norm1.sqrt() * norm2.sqrt()).max(1e-9);
    dot / denom
}

/// Helper to compute a normalized 384-dimensional term-frequency vector as a fallback
/// when the ONNX embedding model is unavailable.
pub fn compute_fallback_text_vector(text: &str) -> Vec<f32> {
    if text.trim().is_empty() {
        return vec![0.0f32; 384];
    }
    let mut vec = vec![0.0f32; 384];
    let words: Vec<&str> = text.split_whitespace().collect();
    for word in words {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if clean.len() < 2 {
            continue;
        }
        let byte_sum: usize = clean.bytes().map(|b| b as usize).sum();
        let dim = (clean.len() * 37 + byte_sum * 13) % 384;
        vec[dim] += 1.0;
    }
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for x in &mut vec {
        *x /= norm;
    }
    vec
}

/// Compute 384-dimensional query embedding vector using bundled GIST ONNX model (or normalized fallback).
pub fn embed_query(query: &str) -> Vec<f32> {
    use crate::embed::engine::EmbedEngine;

    if query.trim().is_empty() {
        return vec![0.0f32; 384];
    }

    if let Ok(engine) =
        crate::embed::BundledEmbedEngine::new(crate::embed::bundled::DEFAULT_BUNDLED_MODEL_ID, 384)
    {
        if let Ok(vectors) = engine.embed(&[query.to_string()]) {
            if let Some(vec) = vectors.into_iter().next() {
                return vec;
            }
        }
    }

    compute_fallback_text_vector(query)
}

/// Fallback chunker that splits assembled markdown text into ~350-token ImportChunkSpec blocks per page/paragraph.
pub fn fallback_chunk_markdown_text(
    markdown_text: &str,
    total_pages: usize,
    target_tokens: usize,
) -> Vec<crate::ingest::job::ImportChunkSpec> {
    let pages: Vec<&str> = markdown_text.split("\n\n--- PAGE_BREAK ---\n\n").collect();
    let mut specs = Vec::new();
    let mut chunk_idx = 0;

    for (page_idx, page_content) in pages.iter().enumerate() {
        let paragraphs: Vec<&str> = page_content
            .split("\n\n")
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();

        let mut current_text = String::new();
        let mut current_tokens = 0;
        let mut current_heading: Option<String> = None;

        for para in paragraphs {
            if para.starts_with('#') {
                let h = para.trim_start_matches('#').trim().to_string();
                if !h.is_empty() {
                    current_heading = Some(h);
                }
            }

            let para_tokens = crate::llm::assembler::count_tokens(para);
            if current_tokens + para_tokens > target_tokens && !current_text.is_empty() {
                specs.push(crate::ingest::job::ImportChunkSpec {
                    chunk_index: chunk_idx,
                    text: current_text.clone(),
                    token_count: current_tokens,
                    heading_context: current_heading.clone(),
                    chunk_type: "import".to_string(),
                    ocr_confidence: None,
                    tables_unstructured: false,
                    source_page_indices: vec![page_idx],
                });
                chunk_idx += 1;
                current_text.clear();
                current_tokens = 0;
            }

            if !current_text.is_empty() {
                current_text.push_str("\n\n");
            }
            current_text.push_str(para);
            current_tokens += para_tokens;
        }

        if !current_text.is_empty() {
            specs.push(crate::ingest::job::ImportChunkSpec {
                chunk_index: chunk_idx,
                text: current_text,
                token_count: current_tokens,
                heading_context: current_heading,
                chunk_type: "import".to_string(),
                ocr_confidence: None,
                tables_unstructured: false,
                source_page_indices: vec![page_idx],
            });
            chunk_idx += 1;
        }
    }

    if specs.is_empty() && !markdown_text.trim().is_empty() {
        let tokens = crate::llm::assembler::count_tokens(markdown_text);
        specs.push(crate::ingest::job::ImportChunkSpec {
            chunk_index: 0,
            text: markdown_text.to_string(),
            token_count: tokens,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: (0..total_pages.max(1)).collect(),
        });
    }

    specs
}

/// Compute 384-dimensional embeddings for a list of ImportChunkSpec items.
/// Returns (vectors, embeddings_computed).
/// Uses the bundled GIST model if available (embeddings_computed = true),
/// with a normalized fallback when ONNX model files are missing (embeddings_computed = false).
pub fn compute_ephemeral_chunk_embeddings(
    chunks: &[crate::ingest::job::ImportChunkSpec],
) -> (Vec<Vec<f32>>, bool) {
    use crate::embed::engine::EmbedEngine;

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    if texts.is_empty() {
        return (Vec::new(), false);
    }

    if let Ok(engine) =
        crate::embed::BundledEmbedEngine::new(crate::embed::bundled::DEFAULT_BUNDLED_MODEL_ID, 384)
    {
        if let Ok(vectors) = engine.embed(&texts) {
            return (vectors, true);
        }
    }

    // Fallback: Generate term-matching normalized 384-dim vectors for test/environments without ONNX model files
    let fallback_vectors = texts
        .iter()
        .map(|text| compute_fallback_text_vector(text))
        .collect();
    (fallback_vectors, false)
}

static GLOBAL_EPHEMERAL_STORE: OnceLock<Arc<RwLock<EphemeralChunkStore>>> = OnceLock::new();

/// Access the global singleton instance of the EphemeralChunkStore.
pub fn get_ephemeral_store() -> &'static Arc<RwLock<EphemeralChunkStore>> {
    GLOBAL_EPHEMERAL_STORE.get_or_init(|| Arc::new(RwLock::new(EphemeralChunkStore::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stem_search_token() {
        assert_eq!(stem_search_token("crimes"), Some("crime".to_string()));
        assert_eq!(stem_search_token("documents"), Some("document".to_string()));
        assert_eq!(stem_search_token("boxes"), Some("box".to_string()));
        assert_eq!(stem_search_token("watches"), Some("watch".to_string()));
        assert_eq!(stem_search_token("cases"), Some("case".to_string()));

        let crimes_tokens = normalize_search_tokens("high crimes and misdemeanors");
        let crime_tokens = normalize_search_tokens("crime rate");
        assert!(
            crimes_tokens.contains("crime"),
            "normalize_search_tokens('crimes') must generate stem 'crime'"
        );
        assert!(
            crime_tokens.contains("crime"),
            "normalize_search_tokens('crime') must generate 'crime'"
        );
    }

    #[test]
    fn test_step_1_normalization_and_punctuation() {
        assert!(is_broad_summarization_query(
            "Give me a bird's eye view of the file"
        ));
        assert!(is_broad_summarization_query("Give me a tl;dr of the text"));
        assert!(is_broad_summarization_query("What is the TL;DR?"));
    }

    #[test]
    fn test_step_2_explicit_whole_document_scope_precedence() {
        assert!(is_broad_summarization_query(
            "Please summarize the entire document"
        ));
        assert!(is_broad_summarization_query("Overview of the whole file"));
        assert!(is_broad_summarization_query("Cover to cover recap"));
        // Explicit scope overrides narrow section/code words
        assert!(is_broad_summarization_query(
            "Can you summarize the entire document? I need to understand error handling in section 2."
        ));
    }

    #[test]
    fn test_step_3_localized_narrow_section_guard() {
        assert!(!is_broad_summarization_query("Please summarize chapter 3."));
        assert!(!is_broad_summarization_query(
            "Give me an overview of section 2."
        ));
        assert!(!is_broad_summarization_query("Summarize chapter3"));
        assert!(!is_broad_summarization_query(
            "Can you summarize this chapter."
        ));
        assert!(!is_broad_summarization_query("Explain page 15"));

        // Words containing prefixes (participation, pagination, lineage) must NOT trigger narrow guard
        assert!(is_broad_summarization_query(
            "Summarize the standard participation model"
        ));
        assert!(is_broad_summarization_query(
            "Give me a summary of pagination in this architecture"
        ));
        assert!(is_broad_summarization_query(
            "Overview of the lineage tracking file"
        ));
    }

    #[test]
    fn test_step_4_unambiguous_summary_intent() {
        assert!(is_broad_summarization_query(
            "Can you give me an overview of the code?"
        ));
        assert!(is_broad_summarization_query("Give me a summary"));
        assert!(is_broad_summarization_query(
            "Show me the executive summary"
        ));
        assert!(is_broad_summarization_query("Table of contents"));
    }

    #[test]
    fn test_step_5_ambiguous_terms_with_doc_nouns() {
        assert!(is_broad_summarization_query(
            "Give me takeaways from these documents"
        ));
        assert!(is_broad_summarization_query("Highlights of the pdf"));
        assert!(is_broad_summarization_query("Recap of the codebase"));

        // Without doc scope nouns, ambiguous terms with narrow targets return false
        assert!(!is_broad_summarization_query(
            "Can you outline a solution for fixing this Rust closure error?"
        ));
    }

    #[test]
    fn test_step_6_additional_broad_phrases() {
        assert!(is_broad_summarization_query("What is this document about?"));
        assert!(is_broad_summarization_query("Explain the document"));
        assert!(is_broad_summarization_query("What does the document say?"));
    }
}
