use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("db")
        .join("migrations")
}

fn apply_migrations(conn: &Connection) {
    let dir = migrations_dir();
    if !dir.exists() {
        panic!("migrations directory does not exist: {}", dir.display());
    }

    let entries = fs::read_dir(&dir).unwrap_or_else(|err| {
        panic!(
            "failed to read migrations directory {}: {err}",
            dir.display()
        )
    });

    let mut migrations = Vec::new();

    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read migration entry: {err}"));
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("failed to get file name for path: {}", path.display()));

        if !file_name.ends_with(".sql") {
            continue;
        }

        let (version_text, name_rest) = file_name.split_once('_').unwrap_or_else(|| {
            panic!("migration file must follow '<version>_<name>.sql': {file_name}")
        });

        let version = version_text
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("migration version must be numeric: {file_name}"));

        let name = name_rest.trim_end_matches(".sql").to_string();
        migrations.push((version, name, path));
    }

    migrations.sort_by_key(|migration| migration.0);

    for (version, name, path) in migrations {
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        if let Err(err) = conn.execute_batch(&sql) {
            panic!("migration {version}_{name} failed: {err}");
        }
    }
}

fn assert_tables_exist(conn: &Connection) {
    let required_tables = [
        "vaults",
        "sub_vaults",
        "nodes",
        "node_embeddings",
        "tags",
        "node_tags",
        "doors",
        "backlinks",
        "changesets",
        "changeset_items",
        "snapshots",
        "snapshot_nodes",
        "sessions",
        "session_messages",
        "routing_feedback",
        "import_jobs",
        "privacy_overrides",
        "settings",
        "schema_migrations",
    ];

    for table in required_tables {
        let exists = match conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = ?1;",
            [table],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(value) => value,
            Err(err) => panic!("failed to query sqlite_master for table: {err}"),
        };
        assert!(exists > 0, "missing table: {table}");
    }
}

fn assert_indexes_exist(conn: &Connection) {
    let required_indexes = [
        "idx_vaults_privacy",
        "idx_vaults_deleted",
        "idx_sub_vaults_vault",
        "idx_nodes_vault",
        "idx_nodes_sub_vault",
        "idx_nodes_type",
        "idx_nodes_deleted",
        "idx_nodes_archived",
        "idx_nodes_accessed",
        "idx_node_tags_tag",
        "idx_doors_source",
        "idx_doors_target",
        "idx_doors_status",
        "idx_backlinks_target",
        "idx_backlinks_source",
        "idx_changesets_status",
        "idx_changeset_items_changeset",
        "idx_changeset_items_status",
        "idx_changeset_items_target",
        "idx_snapshots_vault",
        "idx_snapshots_version",
        "idx_sessions_vault",
        "idx_session_msgs_sess",
        "idx_routing_feedback_vault",
        "idx_routing_feedback_type",
        "idx_node_embeddings_model",
    ];

    for index in required_indexes {
        let exists = match conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'index' AND name = ?1;",
            [index],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(value) => value,
            Err(err) => panic!("failed to query sqlite_master for index: {err}"),
        };
        assert!(exists > 0, "missing index: {index}");
    }
}

fn assert_foreign_keys_exist(conn: &Connection) {
    let fk_expectations: [(&str, &[&str]); 13] = [
        ("sub_vaults", &["vaults"]),
        ("nodes", &["vaults", "sub_vaults"]),
        ("node_embeddings", &["nodes"]),
        ("node_tags", &["nodes", "tags"]),
        ("doors", &["nodes", "vaults"]),
        ("backlinks", &["nodes", "doors"]),
        ("changeset_items", &["changesets", "nodes", "doors"]),
        ("snapshots", &["vaults", "changesets"]),
        ("sessions", &["vaults"]),
        ("session_messages", &["sessions"]),
        ("routing_feedback", &["sessions", "vaults"]),
        ("import_jobs", &["vaults", "changesets"]),
        ("privacy_overrides", &["nodes"]),
    ];

    for (table, expected_targets) in fk_expectations {
        let pragma_sql = format!("PRAGMA foreign_key_list({table});");
        let mut statement = match conn.prepare(&pragma_sql) {
            Ok(value) => value,
            Err(err) => panic!("failed to prepare pragma query: {err}"),
        };
        let fk_rows = match statement.query_map([], |row| row.get::<_, String>(2)) {
            Ok(value) => value,
            Err(err) => panic!("failed to query foreign keys: {err}"),
        };

        let mut target_counts: HashMap<String, usize> = HashMap::new();
        for target_table in fk_rows {
            let target_table = match target_table {
                Ok(value) => value,
                Err(err) => panic!("failed to decode foreign key row: {err}"),
            };
            *target_counts.entry(target_table).or_insert(0) += 1;
        }

        for expected_target in expected_targets {
            let exists = target_counts.get(*expected_target).copied().unwrap_or(0) > 0;
            assert!(
                exists,
                "missing foreign key on {table} referencing {expected_target}"
            );
        }
    }
}

fn assert_composite_pk_exists(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let mut stmt = conn.prepare("PRAGMA table_info(node_embeddings);")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
    })?;

    let mut pk_columns = Vec::new();
    for r in rows {
        let (name, pk) = r?;
        if pk > 0 {
            pk_columns.push(name);
        }
    }

    assert!(
        pk_columns.contains(&"node_id".to_string()),
        "node_id is not part of the primary key"
    );
    assert!(
        pk_columns.contains(&"chunk_index".to_string()),
        "chunk_index is not part of the primary key"
    );
    assert!(
        pk_columns.contains(&"chunk_type".to_string()),
        "chunk_type is not part of the primary key"
    );
    assert_eq!(
        pk_columns.len(),
        3,
        "primary key should consist of exactly 3 columns (node_id, chunk_index, chunk_type)"
    );
    Ok(())
}

fn assert_invalidation_trigger_covers_fields(
    conn: &Connection,
) -> Result<(), Box<dyn std::error::Error>> {
    let (name, sql): (String, String) = conn
        .query_row(
            "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_invalidate_embedding_on_update';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

    assert_eq!(name, "trg_invalidate_embedding_on_update");
    let sql_upper = sql.to_uppercase();
    assert!(
        sql_upper.contains("NEW.TITLE != OLD.TITLE")
            || sql_upper.contains("NEW.TITLE <> OLD.TITLE")
            || sql_upper.contains("NEW.TITLE IS NOT OLD.TITLE"),
        "Trigger missing title change check"
    );
    assert!(
        sql_upper.contains("NEW.SUMMARY != OLD.SUMMARY")
            || sql_upper.contains("NEW.SUMMARY <> OLD.SUMMARY")
            || sql_upper.contains("NEW.SUMMARY IS NOT OLD.SUMMARY"),
        "Trigger missing summary change check"
    );
    assert!(
        sql_upper.contains("NEW.DETAIL != OLD.DETAIL")
            || sql_upper.contains("NEW.DETAIL <> OLD.DETAIL")
            || sql_upper.contains("NEW.DETAIL IS NOT OLD.DETAIL"),
        "Trigger missing detail change check"
    );
    assert!(
        sql_upper.contains("NEW.PRIVACY_TIER != OLD.PRIVACY_TIER")
            || sql_upper.contains("NEW.PRIVACY_TIER <> OLD.PRIVACY_TIER")
            || sql_upper.contains("NEW.PRIVACY_TIER IS NOT OLD.PRIVACY_TIER"),
        "Trigger missing privacy_tier change check"
    );
    assert!(
        sql_upper.contains("NEW.VAULT_ID != OLD.VAULT_ID")
            || sql_upper.contains("NEW.VAULT_ID <> OLD.VAULT_ID")
            || sql_upper.contains("NEW.VAULT_ID IS NOT OLD.VAULT_ID"),
        "Trigger missing vault_id change check"
    );
    assert!(
        sql_upper.contains("NEW.SUB_VAULT_ID != OLD.SUB_VAULT_ID")
            || sql_upper.contains("NEW.SUB_VAULT_ID <> OLD.SUB_VAULT_ID")
            || sql_upper.contains("NEW.SUB_VAULT_ID IS NOT OLD.SUB_VAULT_ID"),
        "Trigger missing sub_vault_id change check"
    );
    assert!(
        sql_upper.contains("NEW.DELETED_AT != OLD.DELETED_AT")
            || sql_upper.contains("NEW.DELETED_AT <> OLD.DELETED_AT")
            || sql_upper.contains("NEW.DELETED_AT IS NOT OLD.DELETED_AT"),
        "Trigger missing deleted_at change check"
    );

    // Verify the vault update trigger
    let (v_name, v_sql): (String, String) = conn
        .query_row(
            "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_invalidate_embedding_on_vault_update';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
    assert_eq!(v_name, "trg_invalidate_embedding_on_vault_update");
    let v_sql_upper = v_sql.to_uppercase();
    assert!(
        v_sql_upper.contains("NEW.PRIVACY_TIER IS NOT OLD.PRIVACY_TIER")
            || v_sql_upper.contains("NEW.PRIVACY_TIER != OLD.PRIVACY_TIER")
            || v_sql_upper.contains("NEW.PRIVACY_TIER <> OLD.PRIVACY_TIER"),
        "Vault trigger missing privacy_tier check"
    );

    // Verify the sub-vault update trigger
    let (sv_name, sv_sql): (String, String) = conn
        .query_row(
            "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_invalidate_embedding_on_sub_vault_update';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
    assert_eq!(sv_name, "trg_invalidate_embedding_on_sub_vault_update");
    let sv_sql_upper = sv_sql.to_uppercase();
    assert!(
        sv_sql_upper.contains("NEW.PRIVACY_TIER IS NOT OLD.PRIVACY_TIER")
            || sv_sql_upper.contains("NEW.PRIVACY_TIER != OLD.PRIVACY_TIER")
            || sv_sql_upper.contains("NEW.PRIVACY_TIER <> OLD.PRIVACY_TIER"),
        "Sub-vault trigger missing privacy_tier check"
    );

    Ok(())
}

fn assert_import_jobs_metadata_columns_exist(
    conn: &Connection,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stmt = conn.prepare("PRAGMA table_info(import_jobs);")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;

    let mut columns = HashMap::new();
    for r in rows {
        let (name, col_type) = r?;
        columns.insert(name, col_type);
    }

    let required_columns = [
        ("total_pages", "INTEGER"),
        ("digital_pages", "INTEGER"),
        ("ocr_pages", "INTEGER"),
        ("hybrid_pages", "INTEGER"),
        ("avg_ocr_confidence", "REAL"),
        ("rasterization_dpi", "INTEGER"),
        ("tables_detected_unpreserved", "INTEGER"),
        ("extraction_path", "TEXT"),
    ];

    for (col_name, expected_type) in required_columns {
        let col_type = columns.get(col_name).unwrap_or_else(|| {
            panic!("missing column {col_name} on import_jobs table");
        });
        assert_eq!(
            col_type.to_uppercase(),
            expected_type,
            "column {col_name} has unexpected type: {col_type}"
        );
    }

    Ok(())
}

#[test]
fn test_manual_migration_0009_inspect() -> Result<(), Box<dyn std::error::Error>> {
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

    apply_migrations(&conn);

    println!("\n========================================");
    println!("    MIGRATION 0009: IMPORT_JOBS SCHEMA   ");
    println!("========================================");

    let mut stmt = conn.prepare("PRAGMA table_info(import_jobs);")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    for r in rows {
        let (cid, name, col_type, dflt) = r?;
        println!(
            "Col #{:<2} {:<28} type: {:<8} default: {:?}",
            cid,
            name,
            col_type,
            dflt.unwrap_or_else(|| "NULL".to_string())
        );
    }

    // Insert a dummy job record testing all 8 new metadata fields
    conn.execute(
        r#"
        INSERT INTO import_jobs (
            id, import_type, source_name, status, total_pages, digital_pages,
            ocr_pages, hybrid_pages, avg_ocr_confidence, rasterization_dpi,
            tables_detected_unpreserved, extraction_path
        ) VALUES (
            'job-test-001', 'pdf', 'sample_doc.pdf', 'staged', 10, 8, 2, 0,
            0.985, 300, 1, 'hybrid'
        );
        "#,
        [],
    )?;

    let (id, status, pages, conf, path): (String, String, i64, f64, String) = conn.query_row(
        "SELECT id, status, total_pages, avg_ocr_confidence, extraction_path FROM import_jobs WHERE id = 'job-test-001';",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;

    println!("----------------------------------------");
    println!("Successfully inserted & queried sample import_job record:");
    println!("  Job ID: {id}");
    println!("  Status: {status}");
    println!("  Total Pages: {pages}");
    println!("  Avg OCR Confidence: {:.1}%", conf * 100.0);
    println!("  Extraction Path: {path}");
    println!("========================================\n");

    Ok(())
}

#[test]
fn schema_integrity_migration_has_tables_indexes_and_foreign_keys(
) -> Result<(), Box<dyn std::error::Error>> {
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

    apply_migrations(&conn);
    assert_tables_exist(&conn);
    assert_indexes_exist(&conn);
    assert_foreign_keys_exist(&conn);
    assert_composite_pk_exists(&conn)?;
    assert_invalidation_trigger_covers_fields(&conn)?;
    assert_import_jobs_metadata_columns_exist(&conn)?;
    Ok(())
}

fn load_migration_files() -> Vec<(i64, String, PathBuf)> {
    let dir = migrations_dir();
    let entries = fs::read_dir(&dir).unwrap_or_else(|err| {
        panic!(
            "failed to read migrations directory {}: {err}",
            dir.display()
        )
    });

    let mut migrations = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read migration entry: {err}"));
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("failed to get file name for path: {}", path.display()));

        if !file_name.ends_with(".sql") {
            continue;
        }

        let (version_text, _) = file_name.split_once('_').unwrap_or_else(|| {
            panic!("migration file must follow '<version>_<name>.sql': {file_name}")
        });
        let version = version_text
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("migration version must be numeric: {file_name}"));

        migrations.push((version, file_name.to_string(), path));
    }

    migrations.sort_by_key(|(version, _, _)| *version);
    migrations
}

fn apply_migrations_through_version(conn: &Connection, max_version: i64) {
    for (version, name, path) in load_migration_files() {
        if version > max_version {
            break;
        }
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        if let Err(err) = conn.execute_batch(&sql) {
            panic!("migration {version}_{name} failed: {err}");
        }
        if let Err(err) = conn.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![version, name],
        ) {
            panic!("failed to record migration {version}_{name}: {err}");
        }
    }
}

/// Pre-0008 0007: chunk schema from 0007, trigger without privacy/vault columns.
const LEGACY_0007_NODE_EMBEDDINGS_SQL: &str = r#"
DROP TABLE IF EXISTS node_embeddings;

CREATE TABLE node_embeddings (
    node_id     TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    chunk_type  TEXT NOT NULL DEFAULT 'primary'
                CHECK (chunk_type IN ('primary', 'detail', 'import')),
    model       TEXT NOT NULL,
    embedding   BLOB NOT NULL,
    computed_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (node_id, chunk_index, chunk_type)
);

CREATE INDEX IF NOT EXISTS idx_node_embeddings_model ON node_embeddings(model);

DROP TRIGGER IF EXISTS trg_invalidate_embedding_on_update;

CREATE TRIGGER trg_invalidate_embedding_on_update
AFTER UPDATE ON nodes
WHEN NEW.title IS NOT OLD.title
   OR NEW.summary IS NOT OLD.summary
   OR NEW.detail IS NOT OLD.detail
BEGIN
    DELETE FROM node_embeddings WHERE node_id = NEW.id;
END;
"#;

#[test]
fn migration_0008_upgrades_legacy_0007_embedding_triggers() -> Result<(), Box<dyn std::error::Error>>
{
    let temp_dir = std::env::temp_dir().join(format!(
        "amber-migration-0008-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| format!("system clock before UNIX epoch: {err}"))?
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)?;
    let db_path = temp_dir.join("legacy.db");

    {
        let conn = Connection::open(&db_path)?;
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
        apply_migrations_through_version(&conn, 6);

        conn.execute_batch(LEGACY_0007_NODE_EMBEDDINGS_SQL)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (7, '0007_node_embeddings_chunks.sql')",
            [],
        )?;

        let legacy_trigger_sql: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_invalidate_embedding_on_update'",
            [],
            |row| row.get(0),
        )?;
        assert!(
            !legacy_trigger_sql.contains("privacy_tier"),
            "legacy 0007 trigger should not reference privacy_tier"
        );
    }

    amber_lib::test_helper_run_migrations(db_path.clone())?;

    let conn = Connection::open(&db_path)?;
    let upgraded_trigger_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_invalidate_embedding_on_update'",
        [],
        |row| row.get(0),
    )?;
    let upgraded_upper = upgraded_trigger_sql.to_uppercase();
    assert!(
        upgraded_upper.contains("PRIVACY_TIER")
            && (upgraded_upper.contains("NEW.PRIVACY_TIER != OLD.PRIVACY_TIER")
                || upgraded_upper.contains("NEW.PRIVACY_TIER <> OLD.PRIVACY_TIER")
                || upgraded_upper.contains("NEW.PRIVACY_TIER IS NOT OLD.PRIVACY_TIER")),
        "0008 upgrade should add privacy_tier to node invalidation trigger"
    );

    let applied_0008: i64 = conn.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE version = 8",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(applied_0008, 1, "migration 0008 should be recorded");

    fs::remove_dir_all(temp_dir).ok();
    Ok(())
}
