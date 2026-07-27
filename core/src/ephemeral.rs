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

    // Fallback: Generate normalized 384-dim vectors for test/environments without ONNX model files
    texts
        .iter()
        .enumerate()
        .map(|(idx, text)| {
            let mut vec = vec![0.0f32; 384];
            let val = (idx + text.len()) as f32 + 1.0;
            vec[0] = val;
            let norm = vec[0].abs().max(1e-6);
            vec[0] /= norm;
            vec
        })
        .collect()
}

static GLOBAL_EPHEMERAL_STORE: OnceLock<Arc<RwLock<EphemeralChunkStore>>> = OnceLock::new();

/// Access the global singleton instance of the EphemeralChunkStore.
pub fn get_ephemeral_store() -> &'static Arc<RwLock<EphemeralChunkStore>> {
    GLOBAL_EPHEMERAL_STORE.get_or_init(|| Arc::new(RwLock::new(EphemeralChunkStore::new())))
}
