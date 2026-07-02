use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use amber_lib::ipc_types::OnboardingNodeCommitInput;
use amber_lib::{execute_onboarding_commit, minimal_pre_write_backup};
use rusqlite::Connection;

fn get_temp_db_path() -> Result<PathBuf, Box<dyn Error>> {
    let mut dir = std::env::temp_dir();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    dir.push(format!("mindvault_test_{}", now));
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

#[test]
fn test_onboarding_commit_and_backup() -> Result<(), Box<dyn Error>> {
    let db_path = get_temp_db_path()?;
    let conn = Connection::open(&db_path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // 1. Setup Database
    apply_migrations(&conn)?;

    conn.execute(
        "UPDATE vaults SET deleted_at = datetime('now') WHERE id = 'vault_personal';",
        [],
    )?;

    // 2. Prepare proposals
    let proposals = vec![OnboardingNodeCommitInput {
        vault_id: "vault_personal".to_string(),
        title: "Test Note".to_string(),
        summary: "Test summary".to_string(),
        detail: Some("Test detail".to_string()),
        node_type: Some("concept".to_string()),
        source_type: Some("onboarding".to_string()),
        tags: Some(vec!["test-tag".to_string()]),
    }];

    // 3. Execute commit
    let result = execute_onboarding_commit(&proposals, &db_path);
    assert!(result.is_ok(), "Commit failed: {:?}", result);

    // 4. Verify node insertion and tags
    let count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM nodes WHERE title = 'Test Note';",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1, "Node should be inserted");

    // Verify tag insertion
    let tag_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM tags WHERE name = 'test-tag';",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tag_count, 1, "Tag should be inserted");

    // 5. Verify onboarding_complete setting
    let setting: String = conn.query_row(
        "SELECT value FROM settings WHERE key = 'onboarding_complete';",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(setting, "true", "Setting should be true");

    // 6. Verify vault revival
    let vault_deleted_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM vaults WHERE id = 'vault_personal' AND deleted_at IS NOT NULL;",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        vault_deleted_count, 0,
        "Vault should be revived (deleted_at IS NULL)"
    );

    // 7. Verify backup file creation
    let parent_dir = db_path.parent().ok_or("No parent dir")?;
    let backups_dir = parent_dir.join("backups");
    assert!(backups_dir.exists(), "Backups directory should exist");
    let mut backup_files = 0;
    for entry in fs::read_dir(&backups_dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("mindvault-pre-onboarding-commit-")
        {
            backup_files += 1;
        }
    }
    assert_eq!(backup_files, 1, "Should have created exactly 1 backup file");

    // 8. Test minimal_pre_write_backup directly
    let backup_path = minimal_pre_write_backup(&conn, &db_path, "manual-test")?;
    assert!(backup_path.exists(), "Backup file should be created");
    assert!(backup_path
        .file_name()
        .ok_or("No file name")?
        .to_string_lossy()
        .starts_with("mindvault-pre-manual-test-"));

    // 9. Test backup retention (should keep only 10 total)
    // We already have 2 backups (1 from execute_onboarding_commit, 1 from manual-test).
    // Let's create 10 more, for a total of 12. The retention policy should trim it to 10.
    for i in 0..10 {
        // Sleep slightly to ensure distinct modification times
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = minimal_pre_write_backup(&conn, &db_path, &format!("retention-{}", i))?;
    }

    let mut final_backup_files = 0;
    for entry in fs::read_dir(&backups_dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("mindvault-pre-")
        {
            final_backup_files += 1;
        }
    }
    assert_eq!(
        final_backup_files, 10,
        "Should have retained exactly 10 backup files"
    );

    let _ = fs::remove_dir_all(parent_dir);

    Ok(())
}
