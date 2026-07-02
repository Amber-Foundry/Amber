use std::fs;
use std::path::PathBuf;

use rusqlite::{params, Connection};

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

#[test]
fn onboarding_default_vaults_exist_and_support_node_foreign_keys() {
    let conn = match Connection::open_in_memory() {
        Ok(value) => value,
        Err(err) => panic!("failed to connect in-memory sqlite: {err}"),
    };

    if let Err(err) = conn.pragma_update(None, "foreign_keys", "ON") {
        panic!("failed to enable foreign keys: {err}");
    }

    apply_migration(&conn, "0001_schema_v1.sql");
    apply_migration(&conn, "0003_onboarding_default_vaults.sql");

    let expected_vault_ids = [
        "vault_personal",
        "vault_work",
        "vault_learning",
        "vault_health",
        "vault_finance",
    ];

    for vault_id in expected_vault_ids {
        let count = match conn.query_row(
            "SELECT COUNT(1) FROM vaults WHERE id = ?1;",
            [vault_id],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(value) => value,
            Err(err) => panic!("failed checking vault {vault_id}: {err}"),
        };
        assert!(count == 1, "expected inserted vault id: {vault_id}");
    }

    let sort_order_pairs: Vec<(String, i64)> = {
        let mut statement = match conn.prepare(
            "SELECT id, sort_order
             FROM vaults
             WHERE id IN ('vault_personal', 'vault_work', 'vault_learning', 'vault_health', 'vault_finance')
             ORDER BY sort_order ASC;",
        ) {
            Ok(value) => value,
            Err(err) => panic!("failed preparing sort order query: {err}"),
        };
        let rows = match statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            Ok(value) => value,
            Err(err) => panic!("failed querying sort order rows: {err}"),
        };
        let mut values = Vec::new();
        for row in rows {
            values.push(match row {
                Ok(value) => value,
                Err(err) => panic!("failed decoding sort order row: {err}"),
            });
        }
        values
    };

    assert_eq!(
        sort_order_pairs,
        vec![
            ("vault_personal".to_string(), 2),
            ("vault_work".to_string(), 3),
            ("vault_learning".to_string(), 4),
            ("vault_health".to_string(), 5),
            ("vault_finance".to_string(), 6),
        ],
        "expected stable onboarding vault sort order values",
    );

    let inserted = match conn.execute(
        "INSERT INTO nodes (id, vault_id, title, summary) VALUES (?1, ?2, ?3, ?4);",
        params![
            "node_onboarding_smoke",
            "vault_personal",
            "Onboarding Test",
            "FK target exists"
        ],
    ) {
        Ok(value) => value,
        Err(err) => panic!("failed inserting node into onboarding vault: {err}"),
    };
    assert_eq!(inserted, 1, "expected node insert into onboarding vault");
}
