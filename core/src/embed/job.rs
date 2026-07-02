use crate::embed::config;
use crate::embed::storage::{
    delete_embeddings_for_model, delete_node_embeddings, upsert_embedding, EmbeddingRow,
};
use crate::embed::{chunk_node_text, EmbedEngine, EmbedError, TierConfig};
use rusqlite::{Connection, OptionalExtension};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedJobResult {
    Completed { nodes_embedded: u64 },
    Cancelled,
    Failed(String),
}

pub struct EmbedJobHandle {
    pub cancel: Arc<AtomicBool>,
}

#[allow(clippy::too_many_arguments)]
pub fn stored_text_columns_changed(
    current_title: &str,
    current_summary: &str,
    current_detail: Option<&str>,
    current_is_encrypted: bool,
    should_encrypt: bool,
    current_effective_privacy: &str,
    next_effective_privacy: &str,
    next_title: &str,
    next_summary: &str,
    next_detail: Option<&str>,
) -> bool {
    let privacy_tier_changed = current_effective_privacy != next_effective_privacy;
    let encryption_state_changed = should_encrypt != current_is_encrypted;
    let content_changed = !should_encrypt
        && (next_title != current_title
            || next_summary != current_summary
            || next_detail != current_detail);
    privacy_tier_changed || encryption_state_changed || content_changed
}

fn is_local_endpoint(endpoint: &str) -> bool {
    let url = match reqwest::Url::parse(endpoint) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let host_clean = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host_clean.parse::<std::net::IpAddr>() {
        ip.is_loopback()
    } else {
        false
    }
}

pub fn embed_node_with_config(
    conn: &mut Connection,
    node_id: &str,
    engine: &dyn EmbedEngine,
    chunk_config: &TierConfig,
    cancel: &AtomicBool,
    _is_unlocked: bool,
) -> Result<bool, EmbedError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(EmbedError::Cancelled);
    }

    let node = conn
        .query_row(
            "SELECT title, summary, detail, vault_id, sub_vault_id, privacy_tier
             FROM nodes
             WHERE id = ?1 AND deleted_at IS NULL
             LIMIT 1;",
            [node_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|err| {
            EmbedError::InferenceFailed(format!("database failed loading node {node_id}: {err}"))
        })?;

    let Some((title, mut summary, mut detail, vault_id, sub_vault_id, privacy_tier)) = node else {
        return Ok(false);
    };

    let settings = config::get_embedding_settings(conn).map_err(|err| {
        EmbedError::InferenceFailed(format!("embedding settings read failed: {err}"))
    })?;

    let mut title = title;

    let effective_tier = crate::resolve_node_effective_privacy(
        conn,
        &vault_id,
        sub_vault_id.as_deref(),
        privacy_tier.as_deref(),
    )
    .map_err(|err| {
        EmbedError::InferenceFailed(format!("failed to resolve effective privacy: {err}"))
    })?;

    if crate::privacy::embedding_should_skip(&effective_tier) {
        let _ = delete_node_embeddings(conn, node_id);
        return Ok(false);
    }

    if settings.backend.eq_ignore_ascii_case("ollama")
        && crate::privacy::embedding_blocks_on_remote_ollama(&effective_tier)
    {
        let endpoint = config::get_local_model_endpoint(conn).map_err(|err| {
            EmbedError::InferenceFailed(format!("failed to read local model endpoint: {err}"))
        })?;
        if !is_local_endpoint(&endpoint) {
            let _ = delete_node_embeddings(conn, node_id);
            return Ok(false);
        }
    }

    if crate::privacy::embedding_uses_stub(&effective_tier) {
        let stub = crate::privacy::generate_pointer_stub(&title, node_id);
        title = stub;
        summary = String::new();
        detail = None;
    }

    let chunks = chunk_node_text(&title, &summary, detail.as_deref(), chunk_config);

    let computed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut rows = Vec::with_capacity(chunks.len());

    for chunk in chunks {
        if cancel.load(Ordering::Relaxed) {
            return Err(EmbedError::Cancelled);
        }

        let vectors = engine.embed(std::slice::from_ref(&chunk.text))?;
        if vectors.len() != 1 {
            return Err(EmbedError::InferenceFailed(format!(
                "embedding output count mismatch for node {node_id} chunk {}:{}: expected 1, got {}",
                chunk.chunk_type,
                chunk.chunk_index,
                vectors.len()
            )));
        }

        let embedding = vectors.into_iter().next().ok_or_else(|| {
            EmbedError::InferenceFailed(format!(
                "embedding output missing for node {node_id} chunk {}:{}",
                chunk.chunk_type, chunk.chunk_index
            ))
        })?;

        if embedding.len() != engine.dims() {
            return Err(EmbedError::InferenceFailed(format!(
                "embedding dimension mismatch for node {node_id} chunk {}:{}: expected {}, got {}",
                chunk.chunk_type,
                chunk.chunk_index,
                engine.dims(),
                embedding.len()
            )));
        }

        rows.push(EmbeddingRow {
            node_id: node_id.to_string(),
            chunk_index: chunk.chunk_index,
            chunk_type: chunk.chunk_type,
            model: engine.model_id().to_string(),
            embedding,
            computed_at: computed_at.clone(),
        });
    }

    let tx = conn.transaction().map_err(|err| {
        EmbedError::InferenceFailed(format!(
            "database failed starting embedding write transaction for node {node_id}: {err}"
        ))
    })?;
    delete_node_embeddings(&tx, node_id).map_err(|err| {
        EmbedError::InferenceFailed(format!(
            "database failed deleting old embeddings for node {node_id}: {err}"
        ))
    })?;
    for row in rows {
        upsert_embedding(&tx, &row).map_err(|err| {
            EmbedError::InferenceFailed(format!(
                "database failed upserting embedding for node {node_id}: {err}"
            ))
        })?;
    }
    tx.commit().map_err(|err| {
        EmbedError::InferenceFailed(format!(
            "database failed committing embedding write for node {node_id}: {err}"
        ))
    })?;

    Ok(true)
}

pub fn embed_node(
    conn: &mut Connection,
    node_id: &str,
    engine: &dyn EmbedEngine,
    cancel: &AtomicBool,
    is_unlocked: bool,
) -> Result<bool, EmbedError> {
    let settings = config::get_embedding_settings(conn).map_err(|err| {
        EmbedError::InferenceFailed(format!("embedding config read failed: {err}"))
    })?;
    let chunk_config = config::chunking_config_for_settings(&settings).map_err(|err| {
        EmbedError::InferenceFailed(format!("embedding chunk config failed: {err}"))
    })?;
    embed_node_with_config(conn, node_id, engine, &chunk_config, cancel, is_unlocked)
}

pub fn embed_all_nodes(
    conn: &mut Connection,
    engine: &dyn EmbedEngine,
    cancel: &AtomicBool,
    old_model_id: Option<&str>,
    is_unlocked: bool,
) -> EmbedJobResult {
    if cancel.load(Ordering::Relaxed) {
        return EmbedJobResult::Cancelled;
    }

    let settings = match config::get_embedding_settings(conn) {
        Ok(s) => s,
        Err(err) => return EmbedJobResult::Failed(format!("embedding config read failed: {err}")),
    };
    let chunk_config = match config::chunking_config_for_settings(&settings) {
        Ok(c) => c,
        Err(err) => return EmbedJobResult::Failed(format!("embedding chunk config failed: {err}")),
    };

    if let Some(model) = old_model_id.filter(|model| !model.trim().is_empty()) {
        if cancel.load(Ordering::Relaxed) {
            return EmbedJobResult::Cancelled;
        }
        if let Err(err) = delete_embeddings_for_model(conn, model) {
            return EmbedJobResult::Failed(format!(
                "failed deleting old model embeddings for {model}: {err}"
            ));
        }
    }

    let node_ids = match list_active_node_ids(conn) {
        Ok(ids) => ids,
        Err(err) => return EmbedJobResult::Failed(err),
    };

    let mut nodes_embedded = 0_u64;
    for node_id in node_ids {
        if cancel.load(Ordering::Relaxed) {
            return EmbedJobResult::Cancelled;
        }

        // Resumable check: if an up-to-date primary embedding already exists for this model, skip it
        match crate::embed::storage::get_primary_embedding(conn, &node_id, engine.model_id()) {
            Ok(Some(_)) => {
                continue;
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("[embed] Error checking existing embedding for node {node_id}: {err}");
            }
        }

        match embed_node_with_config(conn, &node_id, engine, &chunk_config, cancel, is_unlocked) {
            Ok(true) => nodes_embedded += 1,
            Ok(false) => {}
            Err(EmbedError::Cancelled) => return EmbedJobResult::Cancelled,
            Err(err) => {
                eprintln!(
                    "[embed] Failed to embed node {node_id} (skipping and continuing): {err}"
                );
            }
        }
    }

    let completed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Err(err) = config::set_embedding_last_computed_at(conn, &completed_at) {
        return EmbedJobResult::Failed(format!("failed writing embedding.last_computed_at: {err}"));
    }

    EmbedJobResult::Completed { nodes_embedded }
}

fn list_active_node_ids(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id
             FROM nodes
             WHERE deleted_at IS NULL
             ORDER BY updated_at ASC, id ASC;",
        )
        .map_err(|err| format!("failed preparing active node list: {err}"))?;

    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("failed querying active node list: {err}"))?;

    let mut node_ids = Vec::new();
    for row in rows {
        node_ids.push(row.map_err(|err| format!("failed reading active node id: {err}"))?);
    }
    Ok(node_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::storage::{get_embeddings_for_model, get_primary_embedding, setup_test_db};
    use rusqlite::params;
    use std::sync::atomic::AtomicUsize;

    const TEST_MODEL: &str = "avsolatorio/GIST-small-Embedding-v0";
    const TEST_DIMS: usize = 384;

    struct FakeEmbedEngine {
        model_id: String,
        dims: usize,
        calls: AtomicUsize,
        cancel_after_call: Option<usize>,
        cancel: Option<Arc<AtomicBool>>,
        fail_after_call: Option<usize>,
        wrong_dims: bool,
        wrong_count: bool,
        inputs: std::sync::Mutex<Vec<String>>,
    }

    impl FakeEmbedEngine {
        fn new() -> Self {
            Self {
                model_id: TEST_MODEL.to_string(),
                dims: TEST_DIMS,
                calls: AtomicUsize::new(0),
                cancel_after_call: None,
                cancel: None,
                fail_after_call: None,
                wrong_dims: false,
                wrong_count: false,
                inputs: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl EmbedEngine for FakeEmbedEngine {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            if let Ok(mut guard) = self.inputs.lock() {
                guard.extend(texts.iter().cloned());
            }

            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if self.fail_after_call == Some(call) {
                return Err(EmbedError::InferenceFailed("boom".to_string()));
            }
            if self.cancel_after_call == Some(call) {
                if let Some(cancel) = &self.cancel {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            if self.wrong_count {
                return Ok(Vec::new());
            }

            let dims = if self.wrong_dims {
                self.dims.saturating_sub(1)
            } else {
                self.dims
            };
            Ok(texts
                .iter()
                .map(|text| {
                    let seed = text.len() as f32;
                    vec![seed; dims]
                })
                .collect())
        }

        fn model_id(&self) -> &str {
            &self.model_id
        }

        fn dims(&self) -> usize {
            self.dims
        }
    }

    fn setup_job_db() -> Result<Connection, Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        config::seed_embedding_defaults(&conn)?;
        Ok(conn)
    }

    #[test]
    fn test_embed_node_writes_primary_and_detail() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let embedded = embed_node(&mut conn, "node_test_1", &engine, &cancel, false)?;

        assert!(embedded);
        let rows = get_embeddings_for_model(&conn, TEST_MODEL)?;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.chunk_type == "primary"));
        assert!(rows.iter().any(|row| row.chunk_type == "detail"));
        Ok(())
    }

    #[test]
    fn test_embed_node_missing_returns_noop() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let embedded = embed_node(&mut conn, "missing_node", &engine, &cancel, false)?;

        assert!(!embedded);
        assert_eq!(engine.calls(), 0);
        Ok(())
    }

    #[test]
    fn test_embed_node_replaces_stale_chunks_after_shrink() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut conn = setup_job_db()?;
        conn.execute(
            "UPDATE nodes SET detail = ?2 WHERE id = ?1;",
            params!["node_test_1", "This is a sentence. ".repeat(250)],
        )?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let first = embed_node(&mut conn, "node_test_1", &engine, &cancel, false)?;
        assert!(first);
        let first_rows = get_embeddings_for_model(&conn, TEST_MODEL)?;
        assert!(first_rows.len() > 2);

        conn.execute(
            "UPDATE nodes SET detail = NULL WHERE id = ?1;",
            ["node_test_1"],
        )?;
        let second = embed_node(&mut conn, "node_test_1", &engine, &cancel, false)?;
        assert!(second);
        let second_rows = get_embeddings_for_model(&conn, TEST_MODEL)?;
        assert_eq!(second_rows.len(), 1);
        assert_eq!(second_rows[0].chunk_type, "primary");
        Ok(())
    }

    #[test]
    fn test_embed_all_nodes_completed() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        assert_eq!(result, EmbedJobResult::Completed { nodes_embedded: 2 });
        let rows = get_embeddings_for_model(&conn, TEST_MODEL)?;
        assert_eq!(rows.len(), 4);
        Ok(())
    }

    #[test]
    fn test_embed_all_nodes_skips_deleted() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        conn.execute(
            "UPDATE nodes SET deleted_at = datetime('now') WHERE id = ?1;",
            ["node_test_2"],
        )?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        assert_eq!(result, EmbedJobResult::Completed { nodes_embedded: 1 });
        Ok(())
    }

    #[test]
    fn test_embed_all_nodes_mid_batch_cancellation() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        conn.execute("UPDATE nodes SET detail = NULL;", [])?;
        let cancel = Arc::new(AtomicBool::new(false));
        let engine = FakeEmbedEngine {
            cancel_after_call: Some(1),
            cancel: Some(Arc::clone(&cancel)),
            ..FakeEmbedEngine::new()
        };

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        assert_eq!(result, EmbedJobResult::Cancelled);
        let rows = get_embeddings_for_model(&conn, TEST_MODEL)?;
        assert_eq!(rows.len(), 1);
        Ok(())
    }

    #[test]
    fn test_embed_node_chunk_level_cancellation() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        conn.execute(
            "UPDATE nodes SET detail = ?2 WHERE id = ?1;",
            params!["node_test_1", "This is a sentence. ".repeat(250)],
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        let engine = FakeEmbedEngine {
            cancel_after_call: Some(1),
            cancel: Some(Arc::clone(&cancel)),
            ..FakeEmbedEngine::new()
        };

        let result = embed_node(&mut conn, "node_test_1", &engine, &cancel, false);

        assert!(matches!(result, Err(EmbedError::Cancelled)));
        let rows = get_embeddings_for_model(&conn, TEST_MODEL)?;
        assert_eq!(rows.len(), 0);
        Ok(())
    }

    #[test]
    fn test_old_model_deleted_before_reembed() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: "old-model".to_string(),
                embedding: vec![1.0],
                computed_at: "old".to_string(),
            },
        )?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, Some("old-model"), false);

        assert_eq!(result, EmbedJobResult::Completed { nodes_embedded: 2 });
        assert_eq!(get_embeddings_for_model(&conn, "old-model")?.len(), 0);
        assert!(!get_embeddings_for_model(&conn, TEST_MODEL)?.is_empty());
        Ok(())
    }

    #[test]
    fn test_same_model_reembed_none_preserves_unprocessed_rows_on_cancel(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        conn.execute("UPDATE nodes SET detail = NULL;", [])?;
        // Seed only node_test_2 so node_test_1 is processed and triggers cancel
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_2".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: TEST_MODEL.to_string(),
                embedding: vec![9.0; TEST_DIMS],
                computed_at: "old".to_string(),
            },
        )?;

        let cancel = Arc::new(AtomicBool::new(false));
        let engine = FakeEmbedEngine {
            cancel_after_call: Some(1),
            cancel: Some(Arc::clone(&cancel)),
            ..FakeEmbedEngine::new()
        };

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        assert_eq!(result, EmbedJobResult::Cancelled);
        let unprocessed = get_primary_embedding(&conn, "node_test_2", TEST_MODEL)?
            .ok_or("expected unprocessed node embedding to remain")?;
        assert_eq!(unprocessed, vec![9.0; TEST_DIMS]);
        Ok(())
    }

    #[test]
    fn test_inference_failure_preserves_existing_embedding(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: TEST_MODEL.to_string(),
                embedding: vec![42.0; TEST_DIMS],
                computed_at: "old".to_string(),
            },
        )?;
        let engine = FakeEmbedEngine {
            fail_after_call: Some(1),
            ..FakeEmbedEngine::new()
        };
        let cancel = AtomicBool::new(false);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        assert_eq!(result, EmbedJobResult::Completed { nodes_embedded: 0 });
        let existing = get_primary_embedding(&conn, "node_test_1", TEST_MODEL)?
            .ok_or("expected existing embedding")?;
        assert_eq!(existing, vec![42.0; TEST_DIMS]);
        Ok(())
    }

    #[test]
    fn test_embed_all_nodes_resumable() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        // Update details to NULL to ensure exactly 1 chunk per node
        conn.execute("UPDATE nodes SET detail = NULL;", [])?;

        // Seed node_test_1 with embedding
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: TEST_MODEL.to_string(),
                embedding: vec![7.0; TEST_DIMS],
                computed_at: "old".to_string(),
            },
        )?;

        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(false);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        // node_test_1 is skipped, node_test_2 is embedded.
        assert_eq!(result, EmbedJobResult::Completed { nodes_embedded: 1 });
        // The engine should only have been called once (for node_test_2's single primary chunk)
        assert_eq!(engine.calls(), 1);

        // node_test_1 embedding should still be the old one
        let val = get_primary_embedding(&conn, "node_test_1", TEST_MODEL)?
            .ok_or("expected primary embedding for node_test_1")?;
        assert_eq!(val, vec![7.0; TEST_DIMS]);

        Ok(())
    }

    #[test]
    fn test_embed_all_nodes_continues_on_failure() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        // Update details to NULL to ensure exactly 1 chunk per node
        conn.execute("UPDATE nodes SET detail = NULL;", [])?;

        // Both nodes have no embeddings.
        // We make the first call fail.
        let engine = FakeEmbedEngine {
            fail_after_call: Some(1),
            ..FakeEmbedEngine::new()
        };
        let cancel = AtomicBool::new(false);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        // The first node fails, but the second one is still embedded.
        assert_eq!(result, EmbedJobResult::Completed { nodes_embedded: 1 });
        assert_eq!(engine.calls(), 2); // Both nodes (1 chunk each) were processed

        Ok(())
    }

    #[test]
    fn test_embed_all_nodes_pre_cancelled() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        let engine = FakeEmbedEngine::new();
        let cancel = AtomicBool::new(true);

        let result = embed_all_nodes(&mut conn, &engine, &cancel, None, false);

        assert_eq!(result, EmbedJobResult::Cancelled);
        assert_eq!(engine.calls(), 0);
        Ok(())
    }

    #[test]
    fn test_wrong_vector_count_and_dims_fail() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        let cancel = AtomicBool::new(false);
        let wrong_count = FakeEmbedEngine {
            wrong_count: true,
            ..FakeEmbedEngine::new()
        };
        let count_result = embed_node(&mut conn, "node_test_1", &wrong_count, &cancel, false);
        assert!(matches!(count_result, Err(EmbedError::InferenceFailed(_))));

        let wrong_dims = FakeEmbedEngine {
            wrong_dims: true,
            ..FakeEmbedEngine::new()
        };
        let dims_result = embed_node(&mut conn, "node_test_1", &wrong_dims, &cancel, false);
        assert!(matches!(dims_result, Err(EmbedError::InferenceFailed(_))));
        Ok(())
    }

    #[test]
    fn test_stored_text_columns_changed() {
        // Scenario 1: Open stays open, no changes
        assert!(!stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "open",
            "open",
            "Title",
            "Summary",
            Some("Detail")
        ));

        // Scenario 2: Open stays open, text changes
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "open",
            "open",
            "Title 2",
            "Summary",
            Some("Detail")
        ));
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "open",
            "open",
            "Title",
            "Summary 2",
            Some("Detail")
        ));
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "open",
            "open",
            "Title",
            "Summary",
            Some("Detail 2")
        ));
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "open",
            "open",
            "Title",
            "Summary",
            None
        ));

        // Scenario 3: Open to redacted transition
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            true,
            "open",
            "redacted",
            "Title",
            "Summary",
            Some("Detail")
        ));

        // Scenario 4: Redacted to open transition
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            true,
            false,
            "redacted",
            "open",
            "Title",
            "Summary",
            Some("Detail")
        ));

        // Scenario 5: Redacted stays redacted
        assert!(!stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            true,
            true,
            "redacted",
            "redacted",
            "Title",
            "Summary",
            Some("Detail")
        ));
        // Even if cleartext changes, stored columns remain "[REDACTED]"
        assert!(!stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            true,
            true,
            "redacted",
            "redacted",
            "Changed Title",
            "Changed Summary",
            Some("Changed Detail")
        ));

        // Scenario 6: Privacy tier changes (open -> locked), text remains same
        assert!(stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "open",
            "locked",
            "Title",
            "Summary",
            Some("Detail")
        ));

        // Scenario 7: Privacy tier stays locked, text remains same
        assert!(!stored_text_columns_changed(
            "Title",
            "Summary",
            Some("Detail"),
            false,
            false,
            "locked",
            "locked",
            "Title",
            "Summary",
            Some("Detail")
        ));
    }

    #[test]
    fn test_is_local_endpoint_helper() {
        assert!(is_local_endpoint("http://localhost:11434"));
        assert!(is_local_endpoint("http://127.0.0.1:11434"));
        assert!(is_local_endpoint("http://[::1]:11434"));
        assert!(!is_local_endpoint("http://example.com:11434"));
        assert!(!is_local_endpoint("http://google.com"));
    }

    #[test]
    fn test_ollama_egress_privacy_filtering() -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_job_db()?;
        let cancel = AtomicBool::new(false);

        // Configure backend = ollama, and a remote endpoint URL
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, scope) VALUES ('embedding.backend', '\"ollama\"', 'global');",
            [],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, scope) VALUES ('embedding.model', '\"nomic-embed-text\"', 'global');",
            [],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, scope) VALUES ('local_model_endpoint', '\"http://example.com\"', 'global');",
            [],
        )?;

        // 1. Set up a remote destination and a local_only node.
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('v_local', 'Local Vault', 'local_only');",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('n_local_only', 'v_local', 'concept', 'Sensitive Title', 'Sensitive Summary', 'Sensitive Detail', 'test', 'manual', '{}', '{}');",
            [],
        )?;

        // Insert a dummy embedding for n_local_only to verify stale vector deletion on skip
        let dummy_row = EmbeddingRow {
            node_id: "n_local_only".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "nomic-embed-text".to_string(),
            embedding: vec![0.1; 768],
            computed_at: "now".to_string(),
        };
        crate::embed::storage::upsert_embedding(&conn, &dummy_row)?;
        assert!(crate::embed::storage::get_primary_embedding(
            &conn,
            "n_local_only",
            "nomic-embed-text"
        )?
        .is_some());

        // Try embedding n_local_only. It should return Ok(false) and skip it.
        let engine = FakeEmbedEngine {
            model_id: "nomic-embed-text".to_string(),
            dims: 768,
            ..FakeEmbedEngine::new()
        };
        let is_embedded = embed_node(&mut conn, "n_local_only", &engine, &cancel, false)?;
        assert!(!is_embedded);
        assert_eq!(engine.calls(), 0);
        // Verify the stale embedding was deleted
        assert!(crate::embed::storage::get_primary_embedding(
            &conn,
            "n_local_only",
            "nomic-embed-text"
        )?
        .is_none());

        // 2. Set up a remote destination and a locked node, with is_unlocked = false.
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('v_locked', 'Locked Vault', 'locked');",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('n_locked', 'v_locked', 'concept', 'Secret Title', 'Secret Summary', 'Secret Detail', 'test', 'manual', '{}', '{}');",
            [],
        )?;

        // Try embedding n_locked when is_unlocked = false.
        // It should embed the stub.
        let is_embedded = embed_node(&mut conn, "n_locked", &engine, &cancel, false)?;
        assert!(is_embedded);
        assert_eq!(engine.calls(), 1);

        let inputs = match engine.inputs.lock() {
            Ok(guard) => guard,
            Err(_) => panic!("Failed to lock inputs"),
        };
        assert_eq!(inputs.len(), 1);
        let stub = crate::privacy::generate_pointer_stub("Secret Title", "n_locked");
        assert!(inputs[0].contains(&stub));
        assert!(!inputs[0].contains("Secret Summary"));
        assert!(!inputs[0].contains("Secret Detail"));

        // 3. Set up a redacted node and verify it is skipped and deletes stale vectors.
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('v_redacted', 'Redacted Vault', 'redacted');",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('n_redacted', 'v_redacted', 'concept', 'Redacted Title', 'Redacted Summary', 'Redacted Detail', 'test', 'manual', '{}', '{}');",
            [],
        )?;
        let dummy_row_redacted = EmbeddingRow {
            node_id: "n_redacted".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "nomic-embed-text".to_string(),
            embedding: vec![0.1; 768],
            computed_at: "now".to_string(),
        };
        crate::embed::storage::upsert_embedding(&conn, &dummy_row_redacted)?;
        let is_embedded_redacted = embed_node(&mut conn, "n_redacted", &engine, &cancel, false)?;
        assert!(!is_embedded_redacted);
        assert!(crate::embed::storage::get_primary_embedding(
            &conn,
            "n_redacted",
            "nomic-embed-text"
        )?
        .is_none());

        Ok(())
    }
}
