use std::error::Error;
use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;

use amber_lib::memory_agent::{
    build_changeset, count_pending_items, list_changeset_items, list_pending_changesets,
    mark_extraction_complete, parse_candidates_from_llm_output, persist_changeset, should_extract,
    CandidateAction, CandidateNode, ChangesetItemType, PendingChangeset, PendingChangesetItem,
};

fn apply_migrations(conn: &Connection) -> Result<(), Box<dyn Error>> {
    let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("db")
        .join("migrations");

    let mut paths: Vec<PathBuf> = fs::read_dir(&migrations_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();

    paths.sort();

    for path in paths {
        let sql = fs::read_to_string(&path)?;
        conn.execute_batch(&sql)?;
    }
    Ok(())
}

fn setup_test_db() -> Result<Connection, Box<dyn Error>> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    )?;

    apply_migrations(&conn)?;

    Ok(conn)
}

fn create_test_node(
    conn: &Connection,
    id: &str,
    vault_id: &str,
    title: &str,
    summary: &str,
    detail: &str,
) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO nodes (id, vault_id, sub_vault_id, title, summary, detail, node_type, is_archived) VALUES (?1, ?2, NULL, ?3, ?4, ?5, 'concept', 0);",
        rusqlite::params![id, vault_id, title, summary, detail],
    )?;
    Ok(())
}

fn create_test_session(conn: &Connection, id: &str, vault_id: &str) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO sessions (id, vault_id) VALUES (?1, ?2);",
        rusqlite::params![id, vault_id],
    )?;
    Ok(())
}

fn insert_test_message(
    conn: &Connection,
    id: &str,
    session_id: &str,
    role: &str,
    content: &str,
) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO session_messages (id, session_id, role, content) VALUES (?1, ?2, ?3, ?4);",
        rusqlite::params![id, session_id, role, content],
    )?;
    Ok(())
}

#[test]
fn test_extraction_parser_golden_output() -> Result<(), Box<dyn Error>> {
    let raw_output = r#"{
  "candidates": [
    {
      "action": "add",
      "title": "React and TypeScript",
      "summary": "Building modern frontend applications with type safety.",
      "detail": "TypeScript provides static type-checking to catch bugs early.",
      "node_type": "concept",
      "target_vault_key": "learning",
      "tags": ["programming", "web"],
      "confidence": 0.98
    }
  ]
}"#;

    let parsed = parse_candidates_from_llm_output(raw_output)
        .map_err(|err| format!("Failed LLM parsing: {err}"))?;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].title, "React and TypeScript");
    assert_eq!(
        parsed[0].summary,
        "Building modern frontend applications with type safety."
    );
    assert_eq!(
        parsed[0].detail.as_deref(),
        Some("TypeScript provides static type-checking to catch bugs early.")
    );
    assert_eq!(parsed[0].node_type.as_deref(), Some("concept"));
    assert_eq!(parsed[0].target_vault_key.as_deref(), Some("learning"));
    assert_eq!(
        parsed[0].tags,
        Some(vec!["programming".to_string(), "web".to_string()])
    );
    assert_eq!(parsed[0].confidence, 0.98);
    assert_eq!(parsed[0].action, CandidateAction::Add);

    Ok(())
}

#[test]
fn test_jaccard_dedup_classifies_correctly() -> Result<(), Box<dyn Error>> {
    let conn = setup_test_db()?;
    create_test_node(
        &conn,
        "node_1",
        "vault_learning",
        "Machine Learning",
        "Introduction to ML algorithms.",
        "Covers regression and classification.",
    )?;

    let candidates = vec![
        // 1. Identical match (Update bucket)
        CandidateNode {
            title: "Machine Learning".to_string(),
            summary: "Introduction to ML algorithms.".to_string(),
            detail: Some("Covers regression and classification.".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Add,

            source: None,
            source_type: None,
            meta: None,
        },
        // 2. Divergent details Jaccard check (Merge zone upgraded to Update)
        CandidateNode {
            title: "Machine Learning Advanced".to_string(),
            summary: "Advanced ML algorithms.".to_string(),
            detail: Some("Covers deep neural networks.".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.95,
            action: CandidateAction::Add,

            source: None,
            source_type: None,
            meta: None,
        },
        // 3. Completely new concept (Add bucket)
        CandidateNode {
            title: "Baking Cakes".to_string(),
            summary: "How to bake chocolate cakes.".to_string(),
            detail: None,
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Add,

            source: None,
            source_type: None,
            meta: None,
        },
        // 4. Delete request matching node_1 (Delete bucket)
        CandidateNode {
            title: "Machine Learning".to_string(),
            summary: "Introduction to ML algorithms.".to_string(),
            detail: None,
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Delete,

            source: None,
            source_type: None,
            meta: None,
        },
    ];

    let changeset = build_changeset(&conn, &candidates, "test-session", None)
        .map_err(|err| format!("Failed compiling changeset: {err}"))?;

    assert_eq!(changeset.items.len(), 4);

    // Assert correct Jaccard classifications
    assert_eq!(changeset.items[0].item_type, ChangesetItemType::Update);
    assert_eq!(
        changeset.items[0].target_node_id,
        Some("node_1".to_string())
    );

    assert_eq!(changeset.items[1].item_type, ChangesetItemType::Update);
    assert!(changeset.items[1]
        .proposed_data
        .contains("substantialChange"));

    assert_eq!(changeset.items[2].item_type, ChangesetItemType::Add);
    assert_eq!(changeset.items[2].target_node_id, None);

    assert_eq!(changeset.items[3].item_type, ChangesetItemType::Delete);
    assert_eq!(
        changeset.items[3].target_node_id,
        Some("node_1".to_string())
    );

    Ok(())
}

#[test]
fn test_changeset_persistence_round_trip() -> Result<(), Box<dyn Error>> {
    let mut conn = setup_test_db()?;
    create_test_node(
        &conn,
        "node_1",
        "vault_learning",
        "Acme",
        "Acme summary",
        "Acme details",
    )?;

    let pending_items = vec![
        PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{"title":"Unique Title","summary":"Unique summary"}"#.to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        },
        PendingChangesetItem {
            item_type: ChangesetItemType::Update,
            target_node_id: Some("node_1".to_string()),
            proposed_data: r#"{"title":"Acme update"}"#.to_string(),
            existing_data: Some(r#"{"title":"Acme"}"#.to_string()),
            similarity: Some(0.92),
            merge_with_id: None,
        },
    ];

    let pending_changeset = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("granite4.1:3b".to_string()),
        items: pending_items,
    };

    let tx = conn.transaction()?;
    let cs_id = persist_changeset(&tx, &pending_changeset, Some("granite4.1:3b"))
        .map_err(|err| format!("Failed to persist: {err}"))?;
    tx.commit()?;

    // Query pending changesets
    let pending_changesets =
        list_pending_changesets(&conn).map_err(|err| format!("Failed listing: {err}"))?;
    assert_eq!(pending_changesets.len(), 1);
    assert_eq!(pending_changesets[0].id, cs_id);
    assert_eq!(
        pending_changesets[0].session_id.as_deref(),
        Some("test-session")
    );
    assert_eq!(pending_changesets[0].status, "pending");
    assert_eq!(pending_changesets[0].item_count, 2);
    assert_eq!(
        pending_changesets[0].model_used.as_deref(),
        Some("granite4.1:3b")
    );

    // Query changeset items
    let items =
        list_changeset_items(&conn, &cs_id).map_err(|err| format!("Failed list items: {err}"))?;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].changeset_id, cs_id);
    assert_eq!(items[0].item_type, "add");
    assert_eq!(items[1].changeset_id, cs_id);
    assert_eq!(items[1].item_type, "update");

    Ok(())
}

#[test]
fn test_pending_count_reflects_items() -> Result<(), Box<dyn Error>> {
    let mut conn = setup_test_db()?;

    let pending_items = vec![
        PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{"title":"Item 1"}"#.to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        },
        PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{"title":"Item 2"}"#.to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        },
        PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{"title":"Item 3"}"#.to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        },
    ];

    let pending_changeset = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("granite4.1:3b".to_string()),
        items: pending_items,
    };

    let tx = conn.transaction()?;
    persist_changeset(&tx, &pending_changeset, Some("granite4.1:3b"))
        .map_err(|err| format!("Failed to persist: {err}"))?;
    tx.commit()?;

    let count = count_pending_items(&conn).map_err(|err| format!("Failed counting: {err}"))?;
    assert_eq!(count, 3);

    Ok(())
}

#[test]
fn test_empty_conversation_skips_extraction() -> Result<(), Box<dyn Error>> {
    let conn = setup_test_db()?;
    create_test_session(&conn, "default-session", "vault_learning")?;

    let session_id = "default-session";

    // 1. Empty chat history (should return false)
    assert!(!should_extract(&conn, session_id).map_err(|err| format!("Check failed: {err}"))?);

    // 2. Add 2 messages (should return false, less than 6 threshold)
    insert_test_message(&conn, "msg_1", session_id, "user", "Hello")?;
    insert_test_message(&conn, "msg_2", session_id, "assistant", "Hi")?;
    assert!(!should_extract(&conn, session_id).map_err(|err| format!("Check failed: {err}"))?);

    // 3. Add 4 more messages (total 6)
    insert_test_message(&conn, "msg_3", session_id, "user", "I need help")?;
    insert_test_message(&conn, "msg_4", session_id, "assistant", "Sure, what's up?")?;
    insert_test_message(&conn, "msg_5", session_id, "user", "Let's learn cooking")?;
    insert_test_message(&conn, "msg_6", session_id, "assistant", "Cooking is great!")?;

    // 4. Trigger threshold should now be met successfully (true)
    assert!(should_extract(&conn, session_id).map_err(|err| format!("Check failed: {err}"))?);

    // 5. Mark extraction complete
    mark_extraction_complete(&conn, 6).map_err(|err| format!("Mark failed: {err}"))?;

    // 6. Immediate check fails due to time debounce (120 seconds)
    assert!(!should_extract(&conn, session_id).map_err(|err| format!("Check failed: {err}"))?);

    Ok(())
}

#[test]
fn test_privacy_filtering_excludes_redacted_and_locked() -> Result<(), Box<dyn Error>> {
    tauri::async_runtime::block_on(async {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_privacy_pipeline.db");
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }

        let session_id = "default-session";
        {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            apply_migrations(&conn)?;
            create_test_session(&conn, session_id, "vault_learning")?;

            create_test_node(
                &conn,
                "node_redacted",
                "vault_personal",
                "My Secret Diary",
                "Private thoughts.",
                "Deeply personal secrets.",
            )?;
            conn.execute(
                "INSERT INTO privacy_overrides (node_id, privacy_tier) VALUES ('node_redacted', 'redacted');",
                [],
            )?;

            create_test_node(
                &conn,
                "node_locked",
                "vault_work",
                "Confidential Project",
                "Work in progress.",
                "Internal project secrets.",
            )?;
            conn.execute(
                "INSERT INTO privacy_overrides (node_id, privacy_tier) VALUES ('node_locked', 'locked');",
                [],
            )?;

            create_test_node(
                &conn,
                "node_open",
                "vault_personal",
                "Baking Bread",
                "How to bake sourdough bread.",
                "Step by step guide.",
            )?;

            // Message 1: Refers to locked node (should be filtered out)
            conn.execute(
                "INSERT INTO session_messages (id, session_id, role, content, node_refs) VALUES ('msg_1', ?1, 'user', 'Tell me about node_locked', '[\"node_locked\"]');",
                rusqlite::params![session_id],
            )?;

            // Message 2: Refers to redacted node (should be filtered out)
            conn.execute(
                "INSERT INTO session_messages (id, session_id, role, content, node_refs) VALUES ('msg_2', ?1, 'assistant', 'I cannot say much about node_redacted', '[\"node_redacted\"]');",
                rusqlite::params![session_id],
            )?;

            // Message 3: Refers to open node (retained)
            conn.execute(
                "INSERT INTO session_messages (id, session_id, role, content, node_refs) VALUES ('msg_3', ?1, 'user', 'Let us bake bread together.', '[\"node_open\"]');",
                rusqlite::params![session_id],
            )?;
        }

        // Now run extraction pipeline. Because msg_1 and msg_2 are filtered out, only msg_3 remains (< 3 messages).
        // It must return an Insufficient history error!
        let result = amber_lib::execute_memory_extraction_pipeline(
            "ollama".to_string(),
            "http://localhost:11434".to_string(),
            "granite".to_string(),
            db_path.clone(),
            None,
        )
        .await;

        let err_msg = result
            .err()
            .ok_or("Expected Insufficient chat history error, but got Ok")?;
        assert!(err_msg.contains("Insufficient chat history"));

        // Clean up
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }

        Ok(())
    })
}

#[test]
fn test_malformed_json_logging_graceful_recovery() -> Result<(), Box<dyn Error>> {
    let conn = setup_test_db()?;

    // 1. Log a few raw error responses
    amber_lib::log_memory_agent_error(&conn, "raw_error_1")?;
    amber_lib::log_memory_agent_error(&conn, "raw_error_2")?;
    amber_lib::log_memory_agent_error(&conn, "raw_error_3")?;
    amber_lib::log_memory_agent_error(&conn, "raw_error_4")?;
    amber_lib::log_memory_agent_error(&conn, "raw_error_5")?;
    amber_lib::log_memory_agent_error(&conn, "raw_error_6")?;

    // 2. Query the settings table for 'memory_agent_errors'
    let val_str: String = conn.query_row(
        "SELECT value FROM settings WHERE key = 'memory_agent_errors' LIMIT 1;",
        [],
        |row| row.get(0),
    )?;

    let errors: Vec<String> = serde_json::from_str(&val_str)?;

    // 3. Asserts
    assert_eq!(errors.len(), 5);
    assert_eq!(errors[0], "raw_error_2");
    assert_eq!(errors[4], "raw_error_6");

    Ok(())
}

fn spawn_mock_llm_server(response_body: String) -> Result<u16, Box<dyn Error>> {
    use std::io::Write;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Set a short read timeout to consume the entire request without blocking indefinitely
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(15)));
            let mut buf = [0; 4096];
            while let Ok(n) = std::io::Read::read(&mut stream, &mut buf) {
                if n == 0 {
                    break;
                }
            }

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    Ok(port)
}

#[test]
fn test_full_pipeline_graceful_recovery_under_malformed_llm_response() -> Result<(), Box<dyn Error>>
{
    tauri::async_runtime::block_on(async {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_malformed_full_pipeline.db");
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }

        let session_id = "default-session";
        {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            apply_migrations(&conn)?;
            create_test_session(&conn, session_id, "vault_learning")?;

            // Insert 3 completely safe messages to satisfy the 3-message check
            insert_test_message(&conn, "msg_1", session_id, "user", "Hello")?;
            insert_test_message(&conn, "msg_2", session_id, "assistant", "Hi, how are you?")?;
            insert_test_message(
                &conn,
                "msg_3",
                session_id,
                "user",
                "I need to learn cooking",
            )?;
        }

        // Spawn mock LLM server returning a valid Ollama JSON shell but containing malformed content
        let ollama_response =
            r#"{"message":{"role":"assistant","content":"This is raw non-JSON text output!"}}"#;
        let port = spawn_mock_llm_server(ollama_response.to_string())?;

        // Run pipeline pointing to our mock server
        let result = amber_lib::execute_memory_extraction_pipeline(
            "ollama".to_string(),
            format!("http://127.0.0.1:{}", port),
            "granite".to_string(),
            db_path.clone(),
            None,
        )
        .await;

        // Verify graceful recovery returned a success changeset with 0 items
        assert!(
            result.is_ok(),
            "Expected Ok, but got Err: {:?}",
            result.as_ref().err()
        );
        let changeset = result?;
        assert_eq!(changeset.item_count, 0);
        assert_eq!(changeset.status, "pending");

        // Verify the database contains the logged parse error
        {
            let conn = rusqlite::Connection::open(&db_path)?;
            let val_str: String = conn.query_row(
                "SELECT value FROM settings WHERE key = 'memory_agent_errors' LIMIT 1;",
                [],
                |row| row.get(0),
            )?;
            let errors: Vec<String> = serde_json::from_str(&val_str)?;
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0], "This is raw non-JSON text output!");
        }

        // Clean up
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }

        Ok(())
    })
}

#[test]
fn test_full_pipeline_successful_extraction_and_persistence() -> Result<(), Box<dyn Error>> {
    tauri::async_runtime::block_on(async {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_success_full_pipeline.db");
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }

        let session_id = "default-session";
        {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            apply_migrations(&conn)?;
            create_test_session(&conn, session_id, "vault_learning")?;

            // Insert 3 completely safe messages to satisfy the 3-message check
            insert_test_message(&conn, "msg_1", session_id, "user", "Hello")?;
            insert_test_message(&conn, "msg_2", session_id, "assistant", "Hi, how are you?")?;
            insert_test_message(
                &conn,
                "msg_3",
                session_id,
                "user",
                "I need to learn cooking",
            )?;
        }

        // Spawn mock LLM server returning a valid Ollama JSON shell containing actual candidate JSON
        let candidate_json = r#"{\n  \"candidates\": [\n    {\n      \"action\": \"add\",\n      \"title\": \"Baking Bread\",\n      \"summary\": \"How to bake sourdough bread.\",\n      \"node_type\": \"concept\",\n      \"target_vault_key\": \"personal\",\n      \"confidence\": 0.95\n    },\n    {\n      \"action\": \"add\",\n      \"title\": \"Rust Programming\",\n      \"summary\": \"Systems programming language.\",\n      \"node_type\": \"concept\",\n      \"target_vault_key\": \"learning\",\n      \"confidence\": 0.99\n    }\n  ]\n}"#;

        let ollama_response = format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}}}}",
            candidate_json
        );
        let port = spawn_mock_llm_server(ollama_response)?;

        // Run pipeline pointing to our mock server
        let result = amber_lib::execute_memory_extraction_pipeline(
            "ollama".to_string(),
            format!("http://127.0.0.1:{}", port),
            "granite".to_string(),
            db_path.clone(),
            None,
        )
        .await;

        // Verify successful extraction returned a changeset with 2 items
        assert!(
            result.is_ok(),
            "Expected Ok, but got Err: {:?}",
            result.as_ref().err()
        );
        let changeset = result?;
        assert_eq!(changeset.item_count, 2);
        assert_eq!(changeset.status, "pending");
        assert_eq!(changeset.model_used.as_deref(), Some("granite"));

        // Verify the database contains the persisted changeset and items
        {
            let conn = rusqlite::Connection::open(&db_path)?;
            let count = amber_lib::memory_agent::count_pending_items(&conn)?;
            assert_eq!(count, 2);

            let items = amber_lib::memory_agent::list_changeset_items(&conn, &changeset.id)?;
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].item_type, "add");
            assert!(items[0].proposed_data.contains("Baking Bread"));
            assert_eq!(items[1].item_type, "add");
            assert!(items[1].proposed_data.contains("Rust Programming"));
        }

        // Clean up
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }

        Ok(())
    })
}

struct TestEmbedEngine {
    model_id: String,
}

impl amber_lib::embed::EmbedEngine for TestEmbedEngine {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, amber_lib::embed::EmbedError> {
        let val_45 = std::f32::consts::FRAC_1_SQRT_2;
        Ok(texts
            .iter()
            .map(|text| {
                if text.contains("Update Candidate") {
                    vec![1.0, 0.0]
                } else if text.contains("Merge Candidate") {
                    vec![val_45, val_45]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        2
    }
}

#[test]
fn test_cosine_dedup_classifies_correctly() -> Result<(), Box<dyn Error>> {
    let conn = setup_test_db()?;
    create_test_node(
        &conn,
        "node_existing",
        "vault_learning",
        "Target Node",
        "Existing Summary",
        "Details",
    )?;

    // Manually seed primary embedding for node_existing
    conn.execute(
        "INSERT INTO node_embeddings (node_id, chunk_index, chunk_type, model, embedding, computed_at)
         VALUES ('node_existing', 0, 'primary', 'test-model', ?1, 'time');",
        rusqlite::params![amber_lib::embed::storage::serialize_f32_vec(&[1.0, 0.0])],
    )?;

    let engine = TestEmbedEngine {
        model_id: "test-model".to_string(),
    };

    let candidates = vec![
        // 1. High similarity (> 0.85) -> Update
        CandidateNode {
            title: "Update Candidate".to_string(),
            summary: "Will match vector [1.0, 0.0] with score 1.0".to_string(),
            detail: None,
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Add,

            source: None,
            source_type: None,
            meta: None,
        },
        // 2. Medium similarity (0.50 - 0.85) -> Merge
        CandidateNode {
            title: "Merge Candidate".to_string(),
            summary: "Will match vector [0.70710678, 0.70710678] with score 0.7071".to_string(),
            detail: None,
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Add,

            source: None,
            source_type: None,
            meta: None,
        },
        // 3. Low similarity (< 0.50) -> Add (New)
        CandidateNode {
            title: "Add Candidate".to_string(),
            summary: "Will match vector [0.0, 1.0] with score 0.0".to_string(),
            detail: None,
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Add,

            source: None,
            source_type: None,
            meta: None,
        },
    ];

    let changeset = build_changeset(&conn, &candidates, "test-session", Some(&engine))
        .map_err(|err| format!("Failed to build changeset: {err}"))?;

    assert_eq!(changeset.items.len(), 3);

    // High similarity candidate -> Update
    assert_eq!(changeset.items[0].item_type, ChangesetItemType::Update);
    assert_eq!(
        changeset.items[0].target_node_id,
        Some("node_existing".to_string())
    );

    // Medium similarity candidate -> Merge
    assert_eq!(changeset.items[1].item_type, ChangesetItemType::Merge);
    assert_eq!(
        changeset.items[1].merge_with_id,
        Some("node_existing".to_string())
    );

    // Low similarity candidate -> Add
    assert_eq!(changeset.items[2].item_type, ChangesetItemType::Add);
    assert_eq!(changeset.items[2].target_node_id, None);

    Ok(())
}
