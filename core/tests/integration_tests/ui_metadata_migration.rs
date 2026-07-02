use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;

fn migration_file_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("db")
        .join("migrations")
        .join(file_name)
}

fn apply_migration(conn: &Connection, file_name: &str) {
    let migration_sql = match fs::read_to_string(migration_file_path(file_name)) {
        Ok(value) => value,
        Err(err) => panic!("failed to read migration file {file_name}: {err}"),
    };
    if let Err(err) = conn.execute_batch(&migration_sql) {
        panic!("failed to execute migration {file_name}: {err}");
    }
}

fn exec_sql(conn: &Connection, sql: &str, context: &str) {
    if let Err(err) = conn.execute(sql, []) {
        panic!("{context}: {err}");
    }
}

#[test]
fn ui_metadata_backfill_repairs_existing_vault_rows() {
    let conn = match Connection::open_in_memory() {
        Ok(value) => value,
        Err(err) => panic!("failed to open in-memory sqlite: {err}"),
    };
    if let Err(err) = conn.pragma_update(None, "foreign_keys", "ON") {
        panic!("failed to enable foreign keys: {err}");
    }

    apply_migration(&conn, "0001_schema_v1.sql");

    exec_sql(
        &conn,
        "INSERT INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES ('vault_legacy', 'Legacy Vault', NULL, NULL, 'open', 'standard', 0, '{}');",
        "failed to insert legacy vault",
    );
    exec_sql(
        &conn,
        "INSERT INTO sub_vaults (id, vault_id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES ('sub_legacy', 'vault_legacy', 'Legacy Subvault', NULL, NULL, 'open', 'standard', 0, '{}');",
        "failed to insert legacy sub-vault",
    );

    apply_migration(&conn, "0004_vault_ui_metadata.sql");

    exec_sql(
        &conn,
        "UPDATE vaults SET ui_metadata = NULL WHERE id = 'vault_legacy';",
        "failed to null out legacy vault ui_metadata",
    );
    exec_sql(
        &conn,
        "UPDATE sub_vaults SET ui_metadata = NULL WHERE id = 'sub_legacy';",
        "failed to null out legacy sub-vault ui_metadata",
    );

    apply_migration(&conn, "0006_vault_ui_metadata_backfill.sql");

    let vault_ui_metadata: String = match conn.query_row(
        "SELECT ui_metadata FROM vaults WHERE id = 'vault_legacy';",
        [],
        |row| row.get(0),
    ) {
        Ok(value) => value,
        Err(err) => panic!("failed to read vault ui_metadata: {err}"),
    };
    let sub_vault_ui_metadata: String = match conn.query_row(
        "SELECT ui_metadata FROM sub_vaults WHERE id = 'sub_legacy';",
        [],
        |row| row.get(0),
    ) {
        Ok(value) => value,
        Err(err) => panic!("failed to read sub-vault ui_metadata: {err}"),
    };

    assert_eq!(vault_ui_metadata, "{}");
    assert_eq!(sub_vault_ui_metadata, "{}");
}
