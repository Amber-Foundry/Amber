use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingRow {
    pub node_id: String,
    pub chunk_index: i32,
    pub chunk_type: String, // "primary" | "detail" | "import"
    pub model: String,
    pub embedding: Vec<f32>,
    pub computed_at: String,
}

pub fn serialize_f32_vec(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for &val in vec {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

pub fn deserialize_f32_vec(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "Invalid byte array length for f32 vector: expected multiple of 4, got {}",
            bytes.len()
        ));
    }
    let mut vec = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr = chunk
            .try_into()
            .map_err(|_| "Failed to slice byte chunk".to_string())?;
        vec.push(f32::from_le_bytes(arr));
    }
    Ok(vec)
}

pub fn upsert_embedding(conn: &Connection, row: &EmbeddingRow) -> Result<(), String> {
    conn.execute(
        "INSERT INTO node_embeddings (node_id, chunk_index, chunk_type, model, embedding, computed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (node_id, chunk_index, chunk_type) DO UPDATE SET
             model = excluded.model,
             embedding = excluded.embedding,
             computed_at = excluded.computed_at;",
        params![
            row.node_id,
            row.chunk_index,
            row.chunk_type,
            row.model,
            serialize_f32_vec(&row.embedding),
            row.computed_at,
        ],
    )
    .map_err(|err| format!("Failed to upsert embedding: {}", err))?;
    Ok(())
}

pub fn get_primary_embedding(
    conn: &Connection,
    node_id: &str,
    model: &str,
) -> Result<Option<Vec<f32>>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT embedding FROM node_embeddings
             WHERE node_id = ?1 AND model = ?2 AND chunk_index = 0 AND chunk_type = 'primary'
             LIMIT 1;",
        )
        .map_err(|err| format!("Failed to prepare select query: {}", err))?;

    let val = stmt
        .query_row(params![node_id, model], |row| row.get::<_, Vec<u8>>(0))
        .optional()
        .map_err(|err| format!("Failed to fetch embedding: {}", err))?;

    match val {
        Some(bytes) => Ok(Some(deserialize_f32_vec(&bytes)?)),
        None => Ok(None),
    }
}

pub fn get_embeddings_for_model(
    conn: &Connection,
    model: &str,
) -> Result<Vec<EmbeddingRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT node_id, chunk_index, chunk_type, model, embedding, computed_at
             FROM node_embeddings
             WHERE model = ?1;",
        )
        .map_err(|err| format!("Failed to prepare list query: {}", err))?;

    let rows = stmt
        .query_map(params![model], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|err| format!("Query failed: {}", err))?;

    let mut results = Vec::new();
    for r in rows {
        let (node_id, chunk_index, chunk_type, model, bytes, computed_at) =
            r.map_err(|err| format!("Failed to read row: {}", err))?;
        results.push(EmbeddingRow {
            node_id,
            chunk_index,
            chunk_type,
            model,
            embedding: deserialize_f32_vec(&bytes)?,
            computed_at,
        });
    }
    Ok(results)
}

pub fn delete_node_embeddings(conn: &Connection, node_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM node_embeddings WHERE node_id = ?1;",
        params![node_id],
    )
    .map_err(|err| format!("Failed to delete node embeddings: {}", err))?;
    Ok(())
}

pub fn delete_embeddings_for_model(conn: &Connection, model: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM node_embeddings WHERE model = ?1;",
        params![model],
    )
    .map_err(|err| format!("Failed to delete embeddings for model: {}", err))?;
    Ok(())
}

pub fn count_coverage(conn: &Connection, model: &str) -> Result<(i64, i64), String> {
    let total_nodes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes n
             JOIN vaults v ON n.vault_id = v.id
             LEFT JOIN sub_vaults sv ON n.sub_vault_id = sv.id
             WHERE n.deleted_at IS NULL
               AND v.deleted_at IS NULL
               AND COALESCE(n.privacy_tier, '') != 'redacted'
               AND COALESCE(sv.privacy_tier, '') != 'redacted'
               AND COALESCE(v.privacy_tier, '') != 'redacted';",
            [],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed to count nodes: {}", err))?;

    let embedded_nodes: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT ne.node_id) FROM node_embeddings ne
             JOIN nodes n ON ne.node_id = n.id
             JOIN vaults v ON n.vault_id = v.id
             LEFT JOIN sub_vaults sv ON n.sub_vault_id = sv.id
             WHERE ne.model = ?1
               AND ne.chunk_index = 0
               AND ne.chunk_type = 'primary'
               AND n.deleted_at IS NULL
               AND v.deleted_at IS NULL
               AND COALESCE(n.privacy_tier, '') != 'redacted'
               AND COALESCE(sv.privacy_tier, '') != 'redacted'
               AND COALESCE(v.privacy_tier, '') != 'redacted';",
            params![model],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed to count covered nodes: {}", err))?;

    Ok((embedded_nodes, total_nodes))
}

#[cfg(test)]
pub(crate) fn setup_test_db() -> Result<Connection, Box<dyn std::error::Error>> {
    use std::fs;
    use std::path::PathBuf;

    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("db")
            .join("migrations")
    }

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

    let dir = migrations_dir();
    if !dir.exists() {
        return Err(format!("migrations directory does not exist: {}", dir.display()).into());
    }

    let entries = fs::read_dir(&dir)?;
    let mut migrations = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("failed to get file name for path: {}", path.display()))?;

        if !file_name.ends_with(".sql") {
            continue;
        }

        let (version_text, name_rest) = file_name.split_once('_').ok_or_else(|| {
            format!("migration file must follow '<version>_<name>.sql': {file_name}")
        })?;

        let version = version_text
            .parse::<i64>()
            .map_err(|_| format!("migration version must be numeric: {file_name}"))?;

        let name = name_rest.trim_end_matches(".sql").to_string();
        migrations.push((version, name, path));
    }

    migrations.sort_by_key(|migration| migration.0);

    for (version, name, path) in migrations {
        let sql = fs::read_to_string(&path)?;
        conn.execute_batch(&sql)
            .map_err(|err| format!("migration {version}_{name} failed: {err}"))?;
    }

    conn.execute(
        "INSERT INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES ('vault_test', 'Test Vault', 'vault', 'Fixture Vault', 'open', 'standard', 0, '{}');",
        [],
    )?;

    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
         VALUES ('node_test_1', 'vault_test', 'concept', 'Test Node 1', 'Test summary', 'Test detail', 'test', 'manual', '{}', '{}');",
        [],
    )?;

    conn.execute(
        "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
         VALUES ('node_test_2', 'vault_test', 'concept', 'Test Node 2', 'Test summary 2', 'Test detail 2', 'test', 'manual', '{}', '{}');",
        [],
    )?;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f32_serialization_validation() -> Result<(), Box<dyn std::error::Error>> {
        let original_vector = vec![0.1f32, -0.2f32, 3.1f32, 42.0f32];
        let serialized = serialize_f32_vec(&original_vector);
        assert_eq!(serialized.len(), 16);

        let deserialized = deserialize_f32_vec(&serialized)?;
        assert_eq!(deserialized, original_vector);

        // Invalid length check (not multiple of 4)
        let corrupt_data = vec![1, 2, 3];
        let result = deserialize_f32_vec(&corrupt_data);
        assert!(result.is_err());
        let err_msg = match result {
            Err(e) => e,
            Ok(_) => return Err("Expected error, got Ok".into()),
        };
        assert!(err_msg.contains("Invalid byte array length"));
        Ok(())
    }

    #[test]
    fn test_metadata_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let row = EmbeddingRow {
            node_id: "node_test_1".to_string(),
            chunk_index: 1,
            chunk_type: "detail".to_string(),
            model: "test-model-a".to_string(),
            embedding: vec![1.0, 2.0, 3.0],
            computed_at: "2026-06-19T12:00:00Z".to_string(),
        };

        upsert_embedding(&conn, &row)?;

        let all = get_embeddings_for_model(&conn, "test-model-a")?;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].node_id, row.node_id);
        assert_eq!(all[0].chunk_index, row.chunk_index);
        assert_eq!(all[0].chunk_type, row.chunk_type);
        assert_eq!(all[0].model, row.model);
        assert_eq!(all[0].embedding, row.embedding);
        assert_eq!(all[0].computed_at, row.computed_at);

        Ok(())
    }

    #[test]
    fn test_upsert_overwrite() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let row1 = EmbeddingRow {
            node_id: "node_test_1".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "test-model-a".to_string(),
            embedding: vec![1.0, 1.0],
            computed_at: "t1".to_string(),
        };
        upsert_embedding(&conn, &row1)?;

        let row2 = EmbeddingRow {
            node_id: "node_test_1".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: "test-model-a".to_string(),
            embedding: vec![2.0, 2.0],
            computed_at: "t2".to_string(),
        };
        upsert_embedding(&conn, &row2)?;

        let embedding = get_primary_embedding(&conn, "node_test_1", "test-model-a")?
            .ok_or("Expected embedding to be present")?;
        assert_eq!(embedding, vec![2.0, 2.0]);

        let all = get_embeddings_for_model(&conn, "test-model-a")?;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].computed_at, "t2");

        Ok(())
    }

    #[test]
    fn test_multi_chunk_delete() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";

        let primary_row = EmbeddingRow {
            node_id: "node_test_1".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: model.to_string(),
            embedding: vec![0.5],
            computed_at: "time".to_string(),
        };
        let detail_row = EmbeddingRow {
            node_id: "node_test_1".to_string(),
            chunk_index: 1,
            chunk_type: "detail".to_string(),
            model: model.to_string(),
            embedding: vec![0.9],
            computed_at: "time".to_string(),
        };

        upsert_embedding(&conn, &primary_row)?;
        upsert_embedding(&conn, &detail_row)?;

        let all_before = get_embeddings_for_model(&conn, model)?;
        assert_eq!(all_before.len(), 2);

        delete_node_embeddings(&conn, "node_test_1")?;

        let all_after = get_embeddings_for_model(&conn, model)?;
        assert_eq!(all_after.len(), 0);

        Ok(())
    }

    #[test]
    fn test_delete_embeddings_for_model() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;

        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: "model-x".to_string(),
                embedding: vec![1.0],
                computed_at: "time".to_string(),
            },
        )?;

        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_2".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: "model-y".to_string(),
                embedding: vec![2.0],
                computed_at: "time".to_string(),
            },
        )?;

        delete_embeddings_for_model(&conn, "model-x")?;

        let all_x = get_embeddings_for_model(&conn, "model-x")?;
        assert_eq!(all_x.len(), 0);

        let all_y = get_embeddings_for_model(&conn, "model-y")?;
        assert_eq!(all_y.len(), 1);

        Ok(())
    }

    #[test]
    fn test_count_coverage_behavior() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";

        // Coverage start state: 0 / 2 non-deleted nodes
        let (num, den) = count_coverage(&conn, model)?;
        assert_eq!(num, 0);
        assert_eq!(den, 2);

        // 1. Add a primary embedding for node_test_1
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0],
                computed_at: "time".to_string(),
            },
        )?;

        let (num, den) = count_coverage(&conn, model)?;
        assert_eq!(num, 1);
        assert_eq!(den, 2);

        // 2. Add only detail chunks for node_test_2 (should NOT increment numerator)
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_2".to_string(),
                chunk_index: 1,
                chunk_type: "detail".to_string(),
                model: model.to_string(),
                embedding: vec![2.0],
                computed_at: "time".to_string(),
            },
        )?;

        let (num, den) = count_coverage(&conn, model)?;
        assert_eq!(num, 1);
        assert_eq!(den, 2);

        // 3. Soft-delete node_test_1 (should remove it from both numerator and denominator)
        conn.execute(
            "UPDATE nodes SET deleted_at = datetime('now') WHERE id = 'node_test_1';",
            [],
        )?;

        let (num, den) = count_coverage(&conn, model)?;
        // Only node_test_2 is active (denominator = 1), and it has no primary embedding (numerator = 0)
        assert_eq!(num, 0);
        assert_eq!(den, 1);

        Ok(())
    }

    #[test]
    fn test_count_coverage_excludes_redacted_nodes() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";

        // Initial setup has 2 open nodes (total_nodes = 2)
        let (_num, den) = count_coverage(&conn, model)?;
        assert_eq!(den, 2);

        // 1. Mark node_test_1 as redacted -> denominator should become 1
        conn.execute(
            "UPDATE nodes SET privacy_tier = 'redacted' WHERE id = 'node_test_1';",
            [],
        )?;
        let (_num, den) = count_coverage(&conn, model)?;
        assert_eq!(den, 1);

        // 2. Mark parent vault_test as redacted -> denominator should become 0
        conn.execute(
            "UPDATE vaults SET privacy_tier = 'redacted' WHERE id = 'vault_test';",
            [],
        )?;
        let (_num, den) = count_coverage(&conn, model)?;
        assert_eq!(den, 0);

        // 3. Reset privacy_tier and soft-delete vault_test -> coverage count should be 0
        conn.execute(
            "UPDATE vaults SET privacy_tier = 'open', deleted_at = datetime('now') WHERE id = 'vault_test';",
            [],
        )?;
        let (num, den) = count_coverage(&conn, model)?;
        assert_eq!(num, 0);
        assert_eq!(den, 0);

        Ok(())
    }

    #[test]
    fn test_trigger_invalidation_on_update() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";
        let row = EmbeddingRow {
            node_id: "node_test_1".to_string(),
            chunk_index: 0,
            chunk_type: "primary".to_string(),
            model: model.to_string(),
            embedding: vec![1.0, 2.0, 3.0],
            computed_at: "time".to_string(),
        };

        // 1. Initial insert of embedding
        upsert_embedding(&conn, &row)?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(embeddings.len(), 1);

        // 2. Update detail to NULL (transition from text to NULL)
        conn.execute(
            "UPDATE nodes SET detail = NULL WHERE id = 'node_test_1';",
            [],
        )?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(
            embeddings.len(),
            0,
            "Embedding should be deleted when detail changes to NULL"
        );

        // 3. Re-insert embedding
        upsert_embedding(&conn, &row)?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(embeddings.len(), 1);

        // 4. Update detail from NULL to text
        conn.execute(
            "UPDATE nodes SET detail = 'New Detail' WHERE id = 'node_test_1';",
            [],
        )?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(
            embeddings.len(),
            0,
            "Embedding should be deleted when detail changes from NULL to text"
        );

        // 5. Re-insert embedding
        upsert_embedding(&conn, &row)?;

        // 6. Test non-triggering update (node_type)
        conn.execute(
            "UPDATE nodes SET node_type = 'fact' WHERE id = 'node_test_1';",
            [],
        )?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(
            embeddings.len(),
            1,
            "Embedding should NOT be deleted when non-triggering column is updated"
        );

        // 7. Update title
        conn.execute(
            "UPDATE nodes SET title = 'New Title' WHERE id = 'node_test_1';",
            [],
        )?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(
            embeddings.len(),
            0,
            "Embedding should be deleted when title changes"
        );

        // 8. Re-insert embedding
        upsert_embedding(&conn, &row)?;

        // 9. Update summary
        conn.execute(
            "UPDATE nodes SET summary = 'New Summary' WHERE id = 'node_test_1';",
            [],
        )?;
        let embeddings = get_embeddings_for_model(&conn, model)?;
        assert_eq!(
            embeddings.len(),
            0,
            "Embedding should be deleted when summary changes"
        );

        Ok(())
    }
}
