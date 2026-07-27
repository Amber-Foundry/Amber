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

    /// Perform hybrid vector + TF-IDF keyword search over all attached chunks in a session.
    /// Returns up to `top_k` most relevant EphemeralChunk items ranked by descending match score.
    pub fn query_session_chunks(
        &self,
        session_id: &str,
        query_text: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Vec<EphemeralChunk> {
        let mut session_chunks: Vec<&EphemeralChunk> = Vec::new();
        for ((s_id, _), chunks) in &self.chunks {
            if s_id == session_id {
                session_chunks.extend(chunks);
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
                .filter(|c| {
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

        for chunk in session_chunks {
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

            let combined_score = vector_score * 0.2 + keyword_score * 0.8;
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

/// Normalize text into clean search tokens by filtering out standard English stop words.
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
            set.insert(clean);
        }
    }
    set
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
/// Uses the bundled GIST model if available, with a normalized fallback when ONNX model files are missing.
pub fn compute_ephemeral_chunk_embeddings(
    chunks: &[crate::ingest::job::ImportChunkSpec],
) -> Vec<Vec<f32>> {
    use crate::embed::engine::EmbedEngine;

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    if texts.is_empty() {
        return Vec::new();
    }

    if let Ok(engine) =
        crate::embed::BundledEmbedEngine::new(crate::embed::bundled::DEFAULT_BUNDLED_MODEL_ID, 384)
    {
        if let Ok(vectors) = engine.embed(&texts) {
            return vectors;
        }
    }

    // Fallback: Generate term-matching normalized 384-dim vectors for test/environments without ONNX model files
    texts
        .iter()
        .map(|text| compute_fallback_text_vector(text))
        .collect()
}

static GLOBAL_EPHEMERAL_STORE: OnceLock<Arc<RwLock<EphemeralChunkStore>>> = OnceLock::new();

/// Access the global singleton instance of the EphemeralChunkStore.
pub fn get_ephemeral_store() -> &'static Arc<RwLock<EphemeralChunkStore>> {
    GLOBAL_EPHEMERAL_STORE.get_or_init(|| Arc::new(RwLock::new(EphemeralChunkStore::new())))
}
