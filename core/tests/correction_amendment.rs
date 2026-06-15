use std::error::Error;
use std::fs;
use std::path::PathBuf;

use rusqlite::{params, Connection};

use mindvault_lib::memory_agent::{
    amend_or_create_changeset, detect_correction_signal, list_changeset_items,
    list_pending_changesets, mark_extraction_complete, persist_changeset, should_extract,
    should_extract_correction, CandidateAction, CandidateNode, ChangesetItemType, CorrectionSignal,
    PendingChangeset, PendingChangesetItem,
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

fn create_test_session(conn: &Connection, id: &str, vault_id: &str) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO sessions (id, vault_id) VALUES (?1, ?2);",
        params![id, vault_id],
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
        params![id, session_id, role, content],
    )?;
    Ok(())
}

#[test]
fn test_detect_explicit_correction_phrase() {
    let phrases = vec!["actually", "wait,", "i meant"];
    for phrase in phrases {
        let msg = format!("{} it should be green", phrase);
        let signal = detect_correction_signal(&msg, None, &[]);
        assert!(
            matches!(signal, Some(CorrectionSignal::ExplicitPhrase { .. })),
            "Expected ExplicitPhrase for msg: {}",
            msg
        );
    }
}

#[test]
fn test_detect_negation_of_previous_message() {
    let previous_message = "My favorite color is blue";
    let current_message = "not blue, it's green";
    let signal = detect_correction_signal(current_message, Some(previous_message), &[]);
    assert_eq!(
        signal,
        Some(CorrectionSignal::Negation {
            negated_fragment: "blue".to_string()
        })
    );
}

#[test]
fn test_detect_changeset_contradiction() -> Result<(), Box<dyn Error>> {
    let pending_data = vec![r#"{"title": "Blue Theme"}"#.to_string()];

    // 1. Direct contradiction check without using explicit phrases (which would trigger early return)
    let signal = detect_correction_signal("Blue Theme is wrong", None, &pending_data);
    assert_eq!(
        signal,
        Some(CorrectionSignal::ChangesetContradiction {
            contradicted_field: "Blue Theme".to_string()
        })
    );

    // 2. Direct contradiction check with "not"
    let signal_not = detect_correction_signal("not Blue Theme", None, &pending_data);
    assert_eq!(
        signal_not,
        Some(CorrectionSignal::ChangesetContradiction {
            contradicted_field: "Blue Theme".to_string()
        })
    );

    // 3. User requested feed message: "actually it should be green theme"
    // Note: This message contains "actually" which triggers the ExplicitPhrase scan early.
    let signal_actually =
        detect_correction_signal("actually it should be green theme", None, &pending_data);
    assert_eq!(
        signal_actually,
        Some(CorrectionSignal::ExplicitPhrase {
            phrase: "actually".to_string()
        })
    );

    Ok(())
}

#[test]
fn test_no_false_positive_on_neutral_message() {
    let signal = detect_correction_signal("tell me about rust", None, &[]);
    assert!(signal.is_none());
}

#[test]
fn test_should_extract_correction_bypasses_debounce() -> Result<(), Box<dyn Error>> {
    let conn = setup_test_db()?;
    let session_id = "default-session";
    create_test_session(&conn, session_id, "vault_learning")?;

    // Insert 3 messages (minimum threshold for correction check)
    insert_test_message(&conn, "msg_1", session_id, "user", "Hello")?;
    insert_test_message(&conn, "msg_2", session_id, "assistant", "Hi")?;
    insert_test_message(&conn, "msg_3", session_id, "user", "Let's study Rust")?;

    // Mark extraction complete at 3 messages (this starts the debounce)
    mark_extraction_complete(&conn, 3)?;

    // should_extract must return false because debounce is active
    let ready = should_extract(&conn, session_id)?;
    assert!(!ready);

    // should_extract_correction must return true (correction message bypasses debounce)
    let ready_correction = should_extract_correction(&conn, session_id, "actually I meant Go")?;
    assert!(ready_correction);

    Ok(())
}

#[test]
fn test_should_extract_correction_filters_by_session() -> Result<(), Box<dyn Error>> {
    let mut conn = setup_test_db()?;
    let session_a = "session-A";
    let session_b = "session-B";

    // Setup vaults and sessions
    let _ = conn.execute(
        "INSERT OR IGNORE INTO vaults (id, name, privacy_tier) VALUES ('vault-root', 'Vault Root', 'open');",
        [],
    );
    create_test_session(&conn, session_a, "vault-root")?;
    create_test_session(&conn, session_b, "vault-root")?;

    // Create a pending changeset for session_b containing "Blue Theme"
    let pending_items = vec![PendingChangesetItem {
        item_type: ChangesetItemType::Add,
        target_node_id: None,
        proposed_data: r#"{"title":"Blue Theme","summary":"This is a beautiful blue theme"}"#
            .to_string(),
        existing_data: None,
        similarity: None,
        merge_with_id: None,
    }];
    let pending_changeset = PendingChangeset {
        session_id: session_b.to_string(),
        model_used: Some("llama3".to_string()),
        items: pending_items,
    };

    let tx = conn.transaction()?;
    persist_changeset(&tx, &pending_changeset, Some("llama3"))?;
    tx.commit()?;

    // Insert 3 messages in session_a
    insert_test_message(&conn, "msg_a1", session_a, "user", "Hello")?;
    insert_test_message(&conn, "msg_a2", session_a, "assistant", "Hi")?;
    insert_test_message(&conn, "msg_a3", session_a, "user", "Some query")?;

    // Insert 3 messages in session_b
    insert_test_message(&conn, "msg_b1", session_b, "user", "Hello")?;
    insert_test_message(&conn, "msg_b2", session_b, "assistant", "Hi")?;
    insert_test_message(&conn, "msg_b3", session_b, "user", "Some other query")?;

    // If we check session_a for "Blue Theme is wrong", it should return FALSE,
    // because "Blue Theme" is in session_b's pending changesets, not session_a's.
    let ready_a = should_extract_correction(&conn, session_a, "Blue Theme is wrong")?;
    assert!(
        !ready_a,
        "Session A should not be contaminated by Session B's changesets"
    );

    // If we check session_b for "Blue Theme is wrong", it should return TRUE.
    let ready_b = should_extract_correction(&conn, session_b, "Blue Theme is wrong")?;
    assert!(
        ready_b,
        "Session B should detect contradiction on its own pending changeset"
    );

    Ok(())
}

#[test]
fn test_amend_existing_changeset_in_place() -> Result<(), Box<dyn Error>> {
    let mut conn = setup_test_db()?;
    let session_id = "test-session";
    let model = "granite4.1:3b";

    // Create pending changeset with one item
    let pending_items = vec![PendingChangesetItem {
        item_type: ChangesetItemType::Add,
        target_node_id: None,
        proposed_data:
            r#"{"title":"Blue Theme","summary":"This is a beautiful blue theme summary"}"#
                .to_string(),
        existing_data: None,
        similarity: None,
        merge_with_id: None,
    }];
    let pending_changeset = PendingChangeset {
        session_id: session_id.to_string(),
        model_used: Some(model.to_string()),
        items: pending_items,
    };

    let tx = conn.transaction()?;
    let cs_id = persist_changeset(&tx, &pending_changeset, Some(model))?;
    tx.commit()?;

    // Get the original item ID
    let items_before = list_changeset_items(&conn, &cs_id)?;
    assert_eq!(items_before.len(), 1);
    let original_item_id = items_before[0].id.clone();

    // Corrected candidate with similarity > 0.5 (Jaccard = 6/8 = 0.75)
    let candidates = vec![CandidateNode {
        title: "Green Theme".to_string(),
        summary: "This is a beautiful green theme summary".to_string(),
        detail: None,
        node_type: Some("concept".to_string()),
        target_vault_key: Some("personal".to_string()),
        tags: None,
        confidence: 0.95,
        action: CandidateAction::Add,
    }];

    let correction_signal = CorrectionSignal::ExplicitPhrase {
        phrase: "actually".to_string(),
    };

    let (returned_cs_id, amended) = amend_or_create_changeset(
        &mut conn,
        &candidates,
        session_id,
        model,
        &correction_signal,
    )?;

    // Assert original changeset_items row was updated in-place, not duplicated
    assert_eq!(returned_cs_id, cs_id);
    assert!(amended);

    let items_after = list_changeset_items(&conn, &cs_id)?;
    assert_eq!(items_after.len(), 1);
    assert_eq!(items_after[0].id, original_item_id);

    // Verify _amended metadata is present in proposed_data
    let parsed_data: serde_json::Value = serde_json::from_str(&items_after[0].proposed_data)?;
    assert!(parsed_data.get("_amended").is_some());
    let amended_meta = parsed_data
        .get("_amended")
        .ok_or("expected _amended metadata")?;
    assert!(amended_meta.get("similarity").is_some());
    assert!(amended_meta.get("reason").is_some());

    Ok(())
}

#[test]
fn test_amend_creates_new_when_no_pending_exists() -> Result<(), Box<dyn Error>> {
    let mut conn = setup_test_db()?;
    let session_id = "test-session";
    let model = "granite4.1:3b";

    let candidates = vec![CandidateNode {
        title: "Green Theme".to_string(),
        summary: "This is a beautiful green theme summary".to_string(),
        detail: None,
        node_type: Some("concept".to_string()),
        target_vault_key: Some("personal".to_string()),
        tags: None,
        confidence: 0.95,
        action: CandidateAction::Add,
    }];
    let correction_signal = CorrectionSignal::ExplicitPhrase {
        phrase: "actually".to_string(),
    };

    let (cs_id, amended) = amend_or_create_changeset(
        &mut conn,
        &candidates,
        session_id,
        model,
        &correction_signal,
    )?;

    // Assert a new changeset is created normally
    assert!(!amended);

    let pending_changesets = list_pending_changesets(&conn)?;
    assert_eq!(pending_changesets.len(), 1);
    assert_eq!(pending_changesets[0].id, cs_id);

    Ok(())
}

#[test]
fn test_amend_appends_genuinely_new_candidate() -> Result<(), Box<dyn Error>> {
    let mut conn = setup_test_db()?;
    let session_id = "test-session";
    let model = "granite4.1:3b";

    // Create pending changeset with item A
    let pending_items = vec![PendingChangesetItem {
        item_type: ChangesetItemType::Add,
        target_node_id: None,
        proposed_data:
            r#"{"title":"Blue Theme","summary":"This is a beautiful blue theme summary"}"#
                .to_string(),
        existing_data: None,
        similarity: None,
        merge_with_id: None,
    }];
    let pending_changeset = PendingChangeset {
        session_id: session_id.to_string(),
        model_used: Some(model.to_string()),
        items: pending_items,
    };

    let tx = conn.transaction()?;
    let cs_id = persist_changeset(&tx, &pending_changeset, Some(model))?;
    tx.commit()?;

    // Unrelated candidate B (similarity = 0.0)
    let candidates = vec![CandidateNode {
        title: "Baking Cakes".to_string(),
        summary: "How to bake chocolate cakes".to_string(),
        detail: None,
        node_type: Some("concept".to_string()),
        target_vault_key: Some("learning".to_string()),
        tags: None,
        confidence: 0.9,
        action: CandidateAction::Add,
    }];

    let correction_signal = CorrectionSignal::ExplicitPhrase {
        phrase: "actually".to_string(),
    };

    let (returned_cs_id, amended) = amend_or_create_changeset(
        &mut conn,
        &candidates,
        session_id,
        model,
        &correction_signal,
    )?;

    // Assert B is inserted as a new row and item_count is incremented
    assert_eq!(returned_cs_id, cs_id);
    assert!(amended);

    let items = list_changeset_items(&conn, &cs_id)?;
    assert_eq!(items.len(), 2);

    let count: i64 = conn.query_row(
        "SELECT item_count FROM changesets WHERE id = ?1;",
        params![cs_id],
        |row| row.get(0),
    )?;
    assert_eq!(count, 2);

    Ok(())
}

#[test]
fn test_force_extract_minimum_message_threshold() -> Result<(), Box<dyn Error>> {
    tauri::async_runtime::block_on(async {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_force_extract.db");
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
        }

        let session_id = "default-session";
        {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            apply_migrations(&conn)?;
            create_test_session(&conn, session_id, "vault_learning")?;

            // Insert fewer than 3 messages (e.g. 2 messages)
            insert_test_message(&conn, "msg_1", session_id, "user", "Hello")?;
            insert_test_message(&conn, "msg_2", session_id, "assistant", "Hi")?;
        }

        // Call memory_extract_force via test_helper_memory_extract_force
        let result = mindvault_lib::test_helper_memory_extract_force(
            "ollama".to_string(),
            "http://localhost:11434".to_string(),
            "granite".to_string(),
            db_path.clone(),
        )
        .await;

        // Assert it returns an error
        assert!(result.is_err());
        let err_msg = result.err().ok_or("expected error result")?;
        assert!(
            err_msg.contains("at least 3 messages"),
            "Expected 'at least 3 messages' error, got: {}",
            err_msg
        );

        // Clean up
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
        }

        Ok(())
    })
}
