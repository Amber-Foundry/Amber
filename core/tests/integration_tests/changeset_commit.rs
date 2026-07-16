use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use amber_lib::enforce_backup_retention;
use amber_lib::ipc_types::{ChangesetCommitInput, ItemReviewAction};
use amber_lib::memory_agent::changeset::{
    ChangesetItemType, PendingChangeset, PendingChangesetItem,
};
use amber_lib::memory_agent::{
    commit_changeset_transaction, list_changeset_items, list_pending_changesets,
    list_resolved_changesets, persist_changeset,
};

static TEMP_DB_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn get_temp_db_path() -> Result<PathBuf, Box<dyn Error>> {
    let mut dir = std::env::temp_dir();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let count = TEMP_DB_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.push(format!(
        "mindvault_test_commit_{}_{}_{}",
        std::process::id(),
        now,
        count
    ));
    fs::create_dir_all(&dir)?;
    dir.push("test.db");
    Ok(dir)
}

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

fn setup_test_db() -> Result<(Connection, PathBuf), Box<dyn Error>> {
    let db_path = get_temp_db_path()?;
    let conn = Connection::open(&db_path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    apply_migrations(&conn)?;

    // Seed core vaults
    conn.execute(
        "INSERT OR IGNORE INTO vaults (id, name, privacy_tier) VALUES ('vault_personal', 'Personal', 'open');",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO vaults (id, name, privacy_tier) VALUES ('vault_work', 'Work', 'local_only');",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO vaults (id, name, privacy_tier) VALUES ('vault_credentials', 'Credentials', 'redacted');",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO vaults (id, name, privacy_tier) VALUES ('vault_learning', 'Learning', 'open');",
        [],
    )?;

    // Seed a session
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, vault_id) VALUES ('test-session', 'vault_personal');",
        [],
    )?;

    Ok((conn, db_path))
}

#[test]
fn test_changeset_commit_enforces_backup() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    let pending = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("test-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data:
                r#"{"title":"Rust fact","summary":"Systems language","vaultId":"vault_personal"}"#
                    .to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("test-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let backups_dir = parent_dir.join("backups");
    assert!(!backups_dir.exists() || fs::read_dir(&backups_dir)?.count() == 0);

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    assert!(
        backups_dir.exists(),
        "Backups directory should have been created"
    );
    let backup_files = fs::read_dir(&backups_dir)?.count();
    assert_eq!(backup_files, 1, "Expected exactly 1 backup file");

    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_add_creates_node() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    let pending = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("test-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{"title":"Rust Programming","summary":"High performance","detail":"Systems programming language","vaultId":"vault_personal"}"#.to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("test-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    let (title, summary, detail, source_type): (String, String, Option<String>, String) = conn.query_row(
        "SELECT title, summary, detail, source_type FROM nodes WHERE source_type = 'agent_extract' LIMIT 1;",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(title, "Rust Programming");
    assert_eq!(summary, "High performance");
    assert_eq!(detail, Some("Systems programming language".to_string()));
    assert_eq!(source_type, "agent_extract");

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_update_upserts_node() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    // Pre-insert a node
    conn.execute(
        "INSERT INTO nodes (id, vault_id, title, summary, detail, node_type, version) VALUES ('node_123', 'vault_personal', 'Old Title', 'Old Summary', 'Old Detail', 'concept', 1);",
        [],
    )?;

    let pending = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("test-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Update,
            target_node_id: Some("node_123".to_string()),
            proposed_data: r#"{"title":"New Title","summary":"New Summary","detail":"New Detail","vaultId":"vault_personal"}"#.to_string(),
            existing_data: Some(r#"{"title":"Old Title"}"#.to_string()),
            similarity: Some(0.85),
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("test-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    let (title, summary, detail, version): (String, String, Option<String>, i64) = conn.query_row(
        "SELECT title, summary, detail, version FROM nodes WHERE id = 'node_123';",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(title, "New Title");
    assert_eq!(summary, "New Summary");
    assert_eq!(detail, Some("New Detail".to_string()));
    assert_eq!(version, 2);

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_merge_unions_data() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    // Pre-insert a node
    conn.execute(
        "INSERT INTO nodes (id, vault_id, title, summary, detail, node_type) VALUES ('node_merge', 'vault_personal', 'Rust Lang', 'Systems language', 'Existing Detail', 'concept');",
        [],
    )?;
    // Pre-insert tag 'systems' and map it
    conn.execute(
        "INSERT INTO tags (id, name) VALUES ('tag_systems', 'systems');",
        [],
    )?;
    conn.execute(
        "INSERT INTO node_tags (node_id, tag_id) VALUES ('node_merge', 'tag_systems');",
        [],
    )?;

    let pending = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("test-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Merge,
            target_node_id: None,
            proposed_data: r#"{"title":"Rust Lang","summary":"Systems language","detail":"Additional Detail","vaultId":"vault_personal","tags":["systems","programming"]}"#.to_string(),
            existing_data: Some(r#"{"title":"Rust Lang"}"#.to_string()),
            similarity: Some(0.99),
            merge_with_id: Some("node_merge".to_string()),
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("test-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    let detail: Option<String> = conn.query_row(
        "SELECT detail FROM nodes WHERE id = 'node_merge';",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        detail,
        Some("Existing Detail\n\nAdditional Detail".to_string())
    );

    // Verify tags union (systems, programming)
    let mut stmt = conn.prepare("SELECT t.name FROM node_tags nt JOIN tags t ON nt.tag_id = t.id WHERE nt.node_id = 'node_merge' ORDER BY t.name ASC;")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(
        names,
        vec!["programming".to_string(), "systems".to_string()]
    );

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_delete_sets_deleted_at() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    // Pre-insert node
    conn.execute(
        "INSERT INTO nodes (id, vault_id, title, summary, detail, node_type) VALUES ('node_delete', 'vault_personal', 'To Delete', 'Summary', 'Detail', 'concept');",
        [],
    )?;

    let pending = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("test-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Delete,
            target_node_id: Some("node_delete".to_string()),
            proposed_data: r#"{"title":"To Delete","vaultId":"vault_personal"}"#.to_string(),
            existing_data: Some(r#"{"title":"To Delete"}"#.to_string()),
            similarity: Some(1.0),
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("test-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    let deleted_at: Option<String> = conn.query_row(
        "SELECT deleted_at FROM nodes WHERE id = 'node_delete';",
        [],
        |row| row.get(0),
    )?;
    assert!(deleted_at.is_some());

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_repoint_door_resolves_orphan() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    // Pre-insert two nodes
    conn.execute("INSERT INTO nodes (id, vault_id, title, summary, detail, node_type) VALUES ('node_src', 'vault_personal', 'Source', 'Summary', 'Detail', 'concept');", [])?;
    conn.execute("INSERT INTO nodes (id, vault_id, title, summary, detail, node_type) VALUES ('node_dest', 'vault_personal', 'Dest', 'Summary', 'Detail', 'concept');", [])?;

    // Pre-insert door with orphan properties
    conn.execute(
        "INSERT INTO doors (id, source_node_id, target_node_id, status, orphan_reason, orphan_since)
         VALUES ('door_test', 'node_src', NULL, 'orphaned', 'target_deleted', '2026-06-04 12:00:00');",
        [],
    )?;

    // Persist changeset
    let changeset_id = "cs_orphan_test".to_string();
    conn.execute(
        "INSERT INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count) VALUES (?1, 'test-session', 'pending', 1, 0, 0);",
        [&changeset_id],
    )?;
    let item_id = "csi_orphan_test".to_string();
    conn.execute(
        "INSERT INTO changeset_items (id, changeset_id, item_type, door_id, target_node_id, proposed_data, status) VALUES (?1, ?2, 'repoint_door', 'door_test', 'node_dest', '{}', 'pending');",
        [&item_id, &changeset_id],
    )?;

    let input = ChangesetCommitInput {
        changeset_id,
        item_actions: vec![ItemReviewAction {
            item_id,
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    let (target_node_id, status, orphan_reason, orphan_since): (Option<String>, String, Option<String>, Option<String>) = conn.query_row(
        "SELECT target_node_id, status, orphan_reason, orphan_since FROM doors WHERE id = 'door_test';",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(target_node_id, Some("node_dest".to_string()));
    assert_eq!(status, "active");
    assert!(orphan_reason.is_none());
    assert!(orphan_since.is_none());

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_transaction_rollback() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    let pending = PendingChangeset {
        session_id: "test-session".to_string(),
        model_used: Some("test-model".to_string()),
        items: vec![
            PendingChangesetItem {
                item_type: ChangesetItemType::Add,
                target_node_id: None,
                proposed_data:
                    r#"{"title":"Should Rollback","summary":"Summary","vaultId":"vault_personal"}"#
                        .to_string(),
                existing_data: None,
                similarity: None,
                merge_with_id: None,
            },
            PendingChangesetItem {
                item_type: ChangesetItemType::Add,
                target_node_id: None,
                proposed_data:
                    r#"{"title":"Broken Item","summary":"Summary","vaultId":"non_existent_vault"}"#
                        .to_string(),
                existing_data: None,
                similarity: None,
                merge_with_id: None,
            },
        ],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("test-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![
            ItemReviewAction {
                item_id: items[0].id.clone(),
                action: "accept".to_string(),
                edited_data: None,
            },
            ItemReviewAction {
                item_id: items[1].id.clone(),
                action: "accept".to_string(),
                edited_data: None,
            },
        ],
    };

    let result = commit_changeset_transaction(&mut conn, &input, &db_path, None);
    assert!(
        result.is_err(),
        "Expected failure due to foreign key constraint"
    );

    // Verify that the first node was NOT inserted
    let count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM nodes WHERE title = 'Should Rollback';",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 0, "Changes should have been rolled back!");

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_list_resolved_changesets_coverage() -> Result<(), Box<dyn Error>> {
    let (conn, db_path) = setup_test_db()?;

    // Create three changesets
    let cs_id_1 = "cs_resolved_1".to_string();
    let cs_id_2 = "cs_resolved_2".to_string();
    let cs_id_3 = "cs_pending_3".to_string();

    conn.execute("INSERT INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count, reviewed_at) VALUES (?1, 'test-session', 'accepted', 1, 1, 0, datetime('now', '-10 seconds'));", [&cs_id_1])?;
    conn.execute("INSERT INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count, reviewed_at) VALUES (?1, 'test-session', 'dismissed', 1, 0, 1, datetime('now'));", [&cs_id_2])?;
    conn.execute("INSERT INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count) VALUES (?1, 'test-session', 'pending', 1, 0, 0);", [&cs_id_3])?;

    let resolved = list_resolved_changesets(&conn)?;
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].id, cs_id_2);
    assert_eq!(resolved[1].id, cs_id_1);

    let pending = list_pending_changesets(&conn)?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, cs_id_3);

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_enforce_backup_retention_disk_cap() -> Result<(), Box<dyn Error>> {
    let temp_dir = std::env::temp_dir();
    let test_backups_dir = temp_dir.join(format!(
        "test_backups_cap_{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&test_backups_dir)?;

    let file_3 = test_backups_dir.join("mindvault-pre-changeset-10.db");
    fs::write(&file_3, vec![0; 30 * 1024 * 1024])?; // 30 MB (Oldest)

    std::thread::sleep(std::time::Duration::from_millis(20));
    let file_2 = test_backups_dir.join("mindvault-pre-changeset-20.db");
    fs::write(&file_2, vec![0; 30 * 1024 * 1024])?; // 30 MB (Middle)

    std::thread::sleep(std::time::Duration::from_millis(20));
    let file_1 = test_backups_dir.join("mindvault-pre-changeset-30.db");
    fs::write(&file_1, vec![0; 30 * 1024 * 1024])?; // 30 MB (Newest)

    // Total size: 90 MB. Since 90 MB > 50 MB, retention should prune File 2 and File 3, keeping only File 1 (Newest)!
    enforce_backup_retention(&test_backups_dir, 10)?;

    assert!(file_1.exists(), "Newest file should be preserved!");
    assert!(
        !file_2.exists(),
        "Middle file should be trimmed due to 50 MB cap!"
    );
    assert!(
        !file_3.exists(),
        "Oldest file should be trimmed due to 50 MB cap!"
    );

    let _ = fs::remove_dir_all(&test_backups_dir);
    Ok(())
}

#[test]
fn test_enforce_backup_retention_contiguous_preservation() -> Result<(), Box<dyn Error>> {
    let temp_dir = std::env::temp_dir();
    let test_backups_dir = temp_dir.join(format!(
        "test_backups_contiguous_{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&test_backups_dir)?;

    let file_3 = test_backups_dir.join("mindvault-pre-changeset-10.db");
    fs::write(&file_3, vec![0; 10 * 1024 * 1024])?; // 10 MB (Oldest, would fit under 50MB if skip was allowed)

    std::thread::sleep(std::time::Duration::from_millis(20));
    let file_2 = test_backups_dir.join("mindvault-pre-changeset-20.db");
    fs::write(&file_2, vec![0; 30 * 1024 * 1024])?; // 30 MB (Middle)

    std::thread::sleep(std::time::Duration::from_millis(20));
    let file_1 = test_backups_dir.join("mindvault-pre-changeset-30.db");
    fs::write(&file_1, vec![0; 30 * 1024 * 1024])?; // 30 MB (Newest)

    // Under the new contiguous policy:
    // file_1 (30MB) is kept.
    // file_2 (30MB) exceeds 50MB limit when added to file_1.
    // Since file_2 exceeds, file_2 AND all subsequent (file_3) are deleted.
    // Result: only file_1 exists. file_3 is NOT preserved despite being only 10MB!
    enforce_backup_retention(&test_backups_dir, 10)?;

    assert!(file_1.exists(), "Newest file should be preserved!");
    assert!(
        !file_2.exists(),
        "Middle file should be trimmed due to limit!"
    );
    assert!(
        !file_3.exists(),
        "Oldest file should be trimmed to maintain a contiguous sequence of recent backups!"
    );

    let _ = fs::remove_dir_all(&test_backups_dir);
    Ok(())
}

#[test]
fn test_changeset_commit_repoint_door_missing_fields_fail() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    // Pre-insert dummy node and door to satisfy foreign key constraints
    conn.execute("INSERT INTO nodes (id, vault_id, title, summary, detail, node_type) VALUES ('some_node', 'vault_personal', 'Title', 'Summary', 'Detail', 'concept');", [])?;
    conn.execute("INSERT INTO doors (id, source_node_id, target_node_id, status) VALUES ('some_door', 'some_node', NULL, 'orphaned');", [])?;

    // 1. Missing door_id case
    let changeset_id = "cs_missing_door".to_string();
    conn.execute(
        "INSERT INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count) VALUES (?1, 'test-session', 'pending', 1, 0, 0);",
        [&changeset_id],
    )?;
    let item_id_1 = "csi_missing_door".to_string();
    // target_node_id is provided but door_id is NULL
    conn.execute(
        "INSERT INTO changeset_items (id, changeset_id, item_type, door_id, target_node_id, proposed_data, status) VALUES (?1, ?2, 'repoint_door', NULL, 'some_node', '{}', 'pending');",
        [&item_id_1, &changeset_id],
    )?;

    let input_1 = ChangesetCommitInput {
        changeset_id: changeset_id.clone(),
        item_actions: vec![ItemReviewAction {
            item_id: item_id_1,
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let result_1 = commit_changeset_transaction(&mut conn, &input_1, &db_path, None);
    assert!(result_1.is_err(), "Expected failure due to missing door_id");
    let err_msg_1 = match result_1 {
        Err(e) => e,
        Ok(_) => return Err("Expected Err, got Ok".into()),
    };
    assert!(err_msg_1.contains("Missing door_id"));

    // 2. Missing target_node_id case
    let item_id_2 = "csi_missing_target".to_string();
    // door_id is provided but target_node_id is NULL
    conn.execute(
        "INSERT INTO changeset_items (id, changeset_id, item_type, door_id, target_node_id, proposed_data, status) VALUES (?1, ?2, 'repoint_door', 'some_door', NULL, '{}', 'pending');",
        [&item_id_2, &changeset_id],
    )?;

    let input_2 = ChangesetCommitInput {
        changeset_id,
        item_actions: vec![ItemReviewAction {
            item_id: item_id_2,
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let result_2 = commit_changeset_transaction(&mut conn, &input_2, &db_path, None);
    assert!(
        result_2.is_err(),
        "Expected failure due to missing target_node_id"
    );
    let err_msg_2 = match result_2 {
        Err(e) => e,
        Ok(_) => return Err("Expected Err, got Ok".into()),
    };
    assert!(err_msg_2.contains("Missing target_node_id"));

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_import_job_committed_when_changeset_resolves() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    conn.execute(
        "INSERT INTO sessions (id, vault_id, scope_json) VALUES ('import-job-001', 'vault_learning', '[]');",
        [],
    )?;

    let pending = PendingChangeset {
        session_id: "import-job-001".to_string(),
        model_used: Some("import-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data:
                r#"{"title":"Imported note","summary":"From PDF","vaultId":"vault_learning"}"#
                    .to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("import-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    conn.execute(
        "INSERT INTO import_jobs (id, import_type, source_name, target_vault_id, status, changeset_id, node_count)
         VALUES ('job-001', 'pdf', 'sample.pdf', 'vault_learning', 'staged', ?1, 1);",
        [&cs_id],
    )?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id.clone(),
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    let import_status: String = conn.query_row(
        "SELECT status FROM import_jobs WHERE id = 'job-001';",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(import_status, "committed");

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_ingest_changeset_commit_enforces_backup() -> Result<(), Box<dyn Error>> {
    let (mut conn, db_path) = setup_test_db()?;

    conn.execute(
        "INSERT INTO sessions (id, vault_id, scope_json) VALUES ('import-job-backup', 'vault_learning', '[]');",
        [],
    )?;

    let pending = PendingChangeset {
        session_id: "import-job-backup".to_string(),
        model_used: Some("import-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{
                "title":"Traffic sign fact",
                "summary":"Imported from academic PDF",
                "vaultId":"vault_learning",
                "source":"report.pdf",
                "sourceType":"pdf_import"
            }"#
            .to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("import-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    let input = ChangesetCommitInput {
        changeset_id: cs_id,
        item_actions: vec![ItemReviewAction {
            item_id: items[0].id.clone(),
            action: "accept".to_string(),
            edited_data: None,
        }],
    };

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let backups_dir = parent_dir.join("backups");
    assert!(!backups_dir.exists() || fs::read_dir(&backups_dir)?.count() == 0);

    let ok = commit_changeset_transaction(&mut conn, &input, &db_path, None)?;
    assert!(ok);

    assert!(
        backups_dir.exists(),
        "Backups directory should have been created for ingest changeset commit"
    );
    let backup_files = fs::read_dir(&backups_dir)?.count();
    assert_eq!(backup_files, 1, "Expected exactly 1 backup file");

    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}

#[test]
fn test_ingest_changeset_cross_vault_anomaly() -> Result<(), Box<dyn Error>> {
    let (conn, db_path) = setup_test_db()?;

    conn.execute(
        "INSERT INTO sessions (id, vault_id, scope_json) VALUES ('import-job-anomaly', 'vault_learning', '[]');",
        [],
    )?;

    let pending = PendingChangeset {
        session_id: "import-job-anomaly".to_string(),
        model_used: Some("import-model".to_string()),
        items: vec![PendingChangesetItem {
            item_type: ChangesetItemType::Add,
            target_node_id: None,
            proposed_data: r#"{
                "title":"Password hint",
                "summary":"Should not land in learning vault",
                "vaultId":"vault_credentials"
            }"#
            .to_string(),
            existing_data: None,
            similarity: None,
            merge_with_id: None,
        }],
    };
    let cs_id = persist_changeset(&conn, &pending, Some("import-model"))?;
    let items = list_changeset_items(&conn, &cs_id)?;

    assert_eq!(items.len(), 1);
    assert!(
        items[0].cross_vault_anomaly,
        "Import session scoped to open vault should flag redacted target"
    );
    let warning = items[0]
        .anomaly_warning
        .as_deref()
        .ok_or("expected cross-vault warning text")?;
    assert!(warning.contains("Security Warning"));
    assert!(warning.contains("REDACTED") || warning.contains("Credentials"));

    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let _ = fs::remove_dir_all(parent_dir);
    Ok(())
}
