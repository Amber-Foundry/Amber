use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;

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
    Ok(())
}
