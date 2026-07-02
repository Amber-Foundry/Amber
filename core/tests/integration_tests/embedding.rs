use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_db_path(label: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(std::env::temp_dir().join(format!("amber_{label}_{nanos}.sqlite")))
}

#[test]
fn embedding_get_status_returns_seeded_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("embedding_status")?;
    amber_lib::test_helper_init_embedding_db(db_path.clone())?;

    let status = amber_lib::test_helper_embedding_get_status(db_path.clone())?;

    assert_eq!(status.model, "avsolatorio/GIST-small-Embedding-v0");
    assert_eq!(status.tier, "light");
    assert_eq!(status.backend, "onnx");
    assert_eq!(status.coverage_percent, 0.0);
    assert!(!status.reembed_in_progress);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn embedding_reembed_cancel_sets_active_cancel_token() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("embedding_cancel")?;

    let cancelled = amber_lib::test_helper_embedding_reembed_cancel(db_path.clone())?;

    assert!(cancelled);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_storage_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("storage_round_trip")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;
    let conn = rusqlite::Connection::open(&db_path)?;

    // Insert vault and node to satisfy foreign keys
    conn.execute(
        "INSERT INTO vaults (id, name) VALUES ('vault_test', 'Test Vault');",
        [],
    )?;
    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
         VALUES ('node_test', 'vault_test', 'concept', 'Test Title', 'Test Summary', 'Test Detail');",
        [],
    )?;

    let row = mindvault_lib::embed::EmbeddingRow {
        node_id: "node_test".to_string(),
        chunk_index: 0,
        chunk_type: "primary".to_string(),
        model: "avsolatorio/GIST-small-Embedding-v0".to_string(),
        embedding: vec![1.5, 2.5, 3.5],
        computed_at: "2026-06-23T12:00:00Z".to_string(),
    };

    mindvault_lib::embed::storage::upsert_embedding(&conn, &row)?;

    let read_opt = mindvault_lib::embed::storage::get_primary_embedding(
        &conn,
        "node_test",
        "avsolatorio/GIST-small-Embedding-v0",
    )?;

    assert!(read_opt.is_some());
    if let Some(read_vec) = read_opt {
        assert_eq!(read_vec, vec![1.5, 2.5, 3.5]);
    }

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_chunking_long_detail() -> Result<(), Box<dyn std::error::Error>> {
    let title = "My Note Title";
    let summary = "This is the summary of my note.";
    let detail = "This is a sentence. ".repeat(100);

    let config = mindvault_lib::embed::TierConfig {
        model_id: "test-model".to_string(),
        params_m: 100,
        dims: 128,
        max_tokens: 256,
        onnx_size_mb: 50,
        chunk_target_tokens: vec![128, 30],
        chunk_overlap_tokens: vec![0, 5],
        rules: serde_json::Value::Null,
        fallback_model_id: None,
    };

    let chunks =
        mindvault_lib::embed::chunking::chunk_node_text(title, summary, Some(&detail), &config);

    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].chunk_type, "primary");
    assert_eq!(chunks[0].chunk_index, 0);

    let detail_chunks: Vec<_> = chunks.iter().filter(|c| c.chunk_type == "detail").collect();
    assert!(detail_chunks.len() > 1);

    for (i, chunk) in detail_chunks.iter().enumerate() {
        assert_eq!(chunk.chunk_index, (i + 1) as i32);
    }

    Ok(())
}

#[test]
fn test_cosine_search_ranking() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("cosine_search")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;
    let conn = rusqlite::Connection::open(&db_path)?;

    conn.execute(
        "INSERT INTO vaults (id, name) VALUES ('vault_test', 'Test Vault');",
        [],
    )?;
    for i in 1..=3 {
        conn.execute(
            &format!(
                "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
                      VALUES ('node_{i}', 'vault_test', 'concept', 'Title', 'Summary', 'Detail');"
            ),
            [],
        )?;
    }

    // Node 1: orthogonal (cosine similarity 0.0)
    mindvault_lib::embed::storage::upsert_embedding(
        &conn,
        &mindvault_lib::embed::EmbeddingRow {
            node_id: "node_1".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "fake-model".to_string(),
            embedding: vec![0.0, 1.0],
            computed_at: "time".to_string(),
        },
    )?;

    // Node 2: identical (cosine similarity 1.0)
    mindvault_lib::embed::storage::upsert_embedding(
        &conn,
        &mindvault_lib::embed::EmbeddingRow {
            node_id: "node_2".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "fake-model".to_string(),
            embedding: vec![1.0, 0.0],
            computed_at: "time".to_string(),
        },
    )?;

    // Node 3: 45 degrees (cosine similarity should be ~0.707)
    let val_45 = std::f32::consts::FRAC_1_SQRT_2;
    mindvault_lib::embed::storage::upsert_embedding(
        &conn,
        &mindvault_lib::embed::EmbeddingRow {
            node_id: "node_3".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "fake-model".to_string(),
            embedding: vec![val_45, val_45],
            computed_at: "time".to_string(),
        },
    )?;

    let query_vector = vec![1.0, 0.0];
    let matches =
        mindvault_lib::embed::find_top_n_similar(&conn, &query_vector, "fake-model", 3, None)?;

    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].0.id, "node_2");
    assert!((matches[0].1 - 1.0).abs() < 1e-4);

    assert_eq!(matches[1].0.id, "node_3");
    assert!((matches[1].1 - (val_45 as f64)).abs() < 1e-3);

    assert_eq!(matches[2].0.id, "node_1");
    assert!(matches[2].1.abs() < 1e-4);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_compute_text_similarity_falls_back_without_engine() -> Result<(), Box<dyn std::error::Error>>
{
    let score = mindvault_lib::memory_agent::similarity::compute_text_similarity(
        "apple banana",
        "banana orange",
        None,
    );

    assert!((score - 0.3333).abs() < 1e-3);
    Ok(())
}

#[test]
fn test_model_migration_invalidates_old_vectors() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("model_migration")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;
    let mut conn = rusqlite::Connection::open(&db_path)?;

    conn.execute(
        "INSERT INTO vaults (id, name) VALUES ('vault_test', 'Test Vault');",
        [],
    )?;
    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
         VALUES ('node_test', 'vault_test', 'concept', 'Title', 'Summary', 'Detail');",
        [],
    )?;

    mindvault_lib::embed::storage::upsert_embedding(
        &conn,
        &mindvault_lib::embed::EmbeddingRow {
            node_id: "node_test".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "old-model".to_string(),
            embedding: vec![1.0, 2.0],
            computed_at: "time".to_string(),
        },
    )?;

    let rows = mindvault_lib::embed::storage::get_embeddings_for_model(&conn, "old-model")?;
    assert_eq!(rows.len(), 1);

    struct MockEngine;
    impl mindvault_lib::embed::EmbedEngine for MockEngine {
        fn embed(
            &self,
            texts: &[String],
        ) -> Result<Vec<Vec<f32>>, mindvault_lib::embed::EmbedError> {
            Ok(texts.iter().map(|_| vec![0.5, 0.5]).collect())
        }
        fn model_id(&self) -> &str {
            "new-model"
        }
        fn dims(&self) -> usize {
            2
        }
    }

    let engine = MockEngine;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    mindvault_lib::embed::job::embed_all_nodes(
        &mut conn,
        &engine,
        &cancel,
        Some("old-model"),
        false,
    );

    let old_rows = mindvault_lib::embed::storage::get_embeddings_for_model(&conn, "old-model")?;
    assert!(old_rows.is_empty());

    let new_rows = mindvault_lib::embed::storage::get_embeddings_for_model(&conn, "new-model")?;
    assert_eq!(new_rows.len(), 2);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_reembed_cancel() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("reembed_cancel")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;
    let mut conn = rusqlite::Connection::open(&db_path)?;

    conn.execute(
        "INSERT INTO vaults (id, name) VALUES ('vault_test', 'Test Vault');",
        [],
    )?;
    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
         VALUES ('node_1', 'vault_test', 'concept', 'Title 1', 'Summary 1', NULL);",
        [],
    )?;
    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
         VALUES ('node_2', 'vault_test', 'concept', 'Title 2', 'Summary 2', NULL);",
        [],
    )?;

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    struct MockCancelEngine {
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl mindvault_lib::embed::EmbedEngine for MockCancelEngine {
        fn embed(
            &self,
            texts: &[String],
        ) -> Result<Vec<Vec<f32>>, mindvault_lib::embed::EmbedError> {
            // Signal cancellation mid-run
            self.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(texts.iter().map(|_| vec![0.5, 0.5]).collect())
        }
        fn model_id(&self) -> &str {
            "new-model"
        }
        fn dims(&self) -> usize {
            2
        }
    }

    let engine = MockCancelEngine {
        cancel: cancel.clone(),
    };
    let result =
        mindvault_lib::embed::job::embed_all_nodes(&mut conn, &engine, &cancel, None, false);

    assert!(matches!(
        result,
        mindvault_lib::embed::EmbedJobResult::Cancelled
    ));

    // Verify only the first node got embedded before cancellation halted the job
    let rows = mindvault_lib::embed::storage::get_embeddings_for_model(&conn, "new-model")?;
    assert_eq!(rows.len(), 1);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_invalidation_trigger_on_title_change() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("trigger_invalidation")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;
    let conn = rusqlite::Connection::open(&db_path)?;

    conn.execute(
        "INSERT INTO vaults (id, name) VALUES ('vault_test', 'Test Vault');",
        [],
    )?;
    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
         VALUES ('node_test', 'vault_test', 'concept', 'Old Title', 'Summary', 'Detail');",
        [],
    )?;

    mindvault_lib::embed::storage::upsert_embedding(
        &conn,
        &mindvault_lib::embed::EmbeddingRow {
            node_id: "node_test".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "avsolatorio/GIST-small-Embedding-v0".to_string(),
            embedding: vec![1.0, 2.0],
            computed_at: "time".to_string(),
        },
    )?;

    let opt = mindvault_lib::embed::storage::get_primary_embedding(
        &conn,
        "node_test",
        "avsolatorio/GIST-small-Embedding-v0",
    )?;
    assert!(opt.is_some());

    conn.execute(
        "UPDATE nodes SET title = 'New Title' WHERE id = 'node_test';",
        [],
    )?;

    let opt_after = mindvault_lib::embed::storage::get_primary_embedding(
        &conn,
        "node_test",
        "avsolatorio/GIST-small-Embedding-v0",
    )?;
    assert!(opt_after.is_none());

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_settings_set_validation_and_auto_align() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("settings_set_val")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;

    // 1. Validate local_model_endpoint
    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "local_model_endpoint".to_string(),
        "\"not-a-valid-url\"".to_string(),
    );
    match res {
        Err(err) => assert!(err.contains("Invalid local_model_endpoint URL")),
        Ok(_) => panic!("Expected settings_set to fail for invalid URL"),
    }

    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "local_model_endpoint".to_string(),
        "\"ftp://localhost\"".to_string(),
    );
    match res {
        Err(err) => assert!(err.contains("local_model_endpoint must be an HTTP or HTTPS URL")),
        Ok(_) => panic!("Expected settings_set to fail for non-HTTP URL"),
    }

    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "local_model_endpoint".to_string(),
        "\"http://localhost:11434\"".to_string(),
    );
    assert!(res.is_ok());

    // 2. Validate embedding.backend
    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "embedding.backend".to_string(),
        "\"invalid-backend\"".to_string(),
    );
    match res {
        Err(err) => assert!(err.contains("Unsupported embedding backend")),
        Ok(_) => panic!("Expected settings_set to fail for invalid backend"),
    }

    // Switch to ollama. It should auto-align model to nomic-embed-text.
    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "embedding.backend".to_string(),
        "\"ollama\"".to_string(),
    );
    assert!(res.is_ok());

    // Check that model was auto-aligned to nomic-embed-text
    let conn = rusqlite::Connection::open(&db_path)?;
    let settings = mindvault_lib::embed::get_embedding_settings(&conn)?;
    assert_eq!(settings.backend, "ollama");
    assert_eq!(settings.model, "nomic-embed-text");

    // Try setting model to something else while backend is ollama. It should fail.
    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "embedding.model".to_string(),
        "\"avsolatorio/GIST-small-Embedding-v0\"".to_string(),
    );
    match res {
        Err(err) => assert!(err.contains("is not valid for Ollama")),
        Ok(_) => panic!("Expected settings_set to fail for model backend mismatch"),
    }

    // Switch back to onnx. It should auto-align model to the light tier model.
    let res = mindvault_lib::test_helper_settings_set(
        db_path.clone(),
        "embedding.backend".to_string(),
        "\"onnx\"".to_string(),
    );
    assert!(res.is_ok());

    let settings = mindvault_lib::embed::get_embedding_settings(&conn)?;
    assert_eq!(settings.backend, "onnx");
    assert_eq!(settings.model, "avsolatorio/GIST-small-Embedding-v0");

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn test_privacy_tier_change_clears_stale_embeddings() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("privacy_tier_invalidation")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;
    let conn = rusqlite::Connection::open(&db_path)?;

    conn.execute(
        "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_test', 'Test Vault', 'open');",
        [],
    )?;
    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, privacy_tier)
         VALUES ('node_test', 'vault_test', 'concept', 'Title', 'Summary', 'Detail', 'open');",
        [],
    )?;

    mindvault_lib::embed::storage::upsert_embedding(
        &conn,
        &mindvault_lib::embed::EmbeddingRow {
            node_id: "node_test".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "avsolatorio/GIST-small-Embedding-v0".to_string(),
            embedding: vec![1.0, 2.0],
            computed_at: "time".to_string(),
        },
    )?;

    let before = mindvault_lib::embed::storage::get_primary_embedding(
        &conn,
        "node_test",
        "avsolatorio/GIST-small-Embedding-v0",
    )?;
    assert!(before.is_some());

    conn.execute(
        "UPDATE nodes SET privacy_tier = 'locked' WHERE id = 'node_test';",
        [],
    )?;

    let after = mindvault_lib::embed::storage::get_primary_embedding(
        &conn,
        "node_test",
        "avsolatorio/GIST-small-Embedding-v0",
    )?;
    assert!(
        after.is_none(),
        "privacy_tier change should invalidate stored embeddings"
    );

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}
