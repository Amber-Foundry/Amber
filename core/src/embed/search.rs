use crate::embed::storage::deserialize_f32_vec;
use rusqlite::Connection;

/// Compute cosine similarity between two f32 slices.
///
/// Formula: A . B / (||A|| * ||B||)
/// Returns 0.0 if vectors are empty, have different lengths, or either has zero norm.
/// Clamps results between [-1.0, 1.0].
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot_product = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;

    for (&x, &y) in a.iter().zip(b.iter()) {
        let x_f64 = x as f64;
        let y_f64 = y as f64;
        dot_product += x_f64 * y_f64;
        norm_a += x_f64 * x_f64;
        norm_b += y_f64 * y_f64;
    }

    let norm_a = norm_a.sqrt();
    let norm_b = norm_b.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    let raw_score = dot_product / (norm_a * norm_b);
    if raw_score.is_nan() {
        return 0.0;
    }

    raw_score.clamp(-1.0, 1.0)
}

use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct DbNode {
    pub id: String,
    pub vault_id: String,
    pub title: String,
    pub summary: String,
    pub node_type: String,
}

pub fn expand_vault_scope(
    conn: &Connection,
    vaults: &HashSet<String>,
) -> Result<HashSet<String>, String> {
    if vaults.is_empty() {
        return Ok(HashSet::new());
    }

    let has_parent_col: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info('vaults') WHERE name = 'parent_vault_id'
            );",
            [],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed checking schema for parent_vault_id column: {err}"))?;

    if !has_parent_col {
        return Ok(vaults.clone());
    }

    let max_depth = crate::MAX_VAULT_NESTING_DEPTH;
    let placeholders = vec!["?"; vaults.len()].join(", ");
    let query = format!(
        "WITH RECURSIVE vault_tree AS (
            SELECT id, 0 AS depth
            FROM vaults
            WHERE id IN ({placeholders}) AND deleted_at IS NULL
            UNION ALL
            SELECT v.id, vt.depth + 1
            FROM vaults v
            JOIN vault_tree vt ON v.parent_vault_id = vt.id
            WHERE v.deleted_at IS NULL AND vt.depth < {max_depth}
        )
        SELECT DISTINCT id FROM vault_tree;"
    );

    let mut stmt = conn
        .prepare(&query)
        .map_err(|err| format!("Failed preparing vault scope expansion query: {err}"))?;

    let params_refs: Vec<&dyn rusqlite::ToSql> =
        vaults.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params_refs), |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| format!("Failed expanding vault scope: {err}"))?;

    let mut expanded = vaults.clone();
    for r in rows {
        let v_id = r.map_err(|err| format!("Failed reading expanded vault ID: {err}"))?;
        expanded.insert(v_id);
    }
    Ok(expanded)
}

/// Find the top N similar nodes using cosine similarity over their primary embeddings.
///
/// Only compares primary chunks (`chunk_type = 'primary'` and `chunk_index = 0`)
/// for the specified `model` name. It filters out soft-deleted nodes and archived nodes.
/// If `n` is 0, it returns an empty vector.
pub fn find_top_n_similar(
    conn: &Connection,
    query_vector: &[f32],
    model: &str,
    n: usize,
    vaults: Option<&HashSet<String>>,
) -> Result<Vec<(DbNode, f64)>, String> {
    if n == 0 || query_vector.is_empty() {
        return Ok(Vec::new());
    }

    let expanded_storage;
    let target_vaults = if let Some(v_set) = vaults {
        if v_set.is_empty() {
            return Ok(Vec::new());
        }
        expanded_storage = expand_vault_scope(conn, v_set)?;
        if expanded_storage.is_empty() {
            return Ok(Vec::new());
        }
        Some(&expanded_storage)
    } else {
        None
    };

    let (query_str, params_vec) = if let Some(vaults) = target_vaults {
        let placeholders = vec!["?"; vaults.len()].join(", ");
        let query = format!(
            "SELECT n.id, n.vault_id, n.title, n.summary, n.node_type, ne.embedding,
                    n.sub_vault_id,
                    COALESCE(o.privacy_tier, n.privacy_tier) AS node_privacy_tier
             FROM node_embeddings ne
             JOIN nodes n ON ne.node_id = n.id
             LEFT JOIN privacy_overrides o ON n.id = o.node_id
             LEFT JOIN vaults v ON n.vault_id = v.id
             LEFT JOIN vaults sv ON n.sub_vault_id = sv.id
             WHERE ne.chunk_type = 'primary'
               AND ne.chunk_index = 0
               AND ne.model = ?
               AND n.deleted_at IS NULL
               AND n.is_archived = 0
               AND (v.id IS NULL OR v.deleted_at IS NULL)
               AND (sv.id IS NULL OR sv.deleted_at IS NULL)
               AND n.vault_id IN ({});",
            placeholders
        );
        let mut p = vec![model.to_string()];
        p.extend(vaults.iter().cloned());
        (query, p)
    } else {
        let query = "SELECT n.id, n.vault_id, n.title, n.summary, n.node_type, ne.embedding,
                    n.sub_vault_id,
                    COALESCE(o.privacy_tier, n.privacy_tier) AS node_privacy_tier
             FROM node_embeddings ne
             JOIN nodes n ON ne.node_id = n.id
             LEFT JOIN privacy_overrides o ON n.id = o.node_id
             LEFT JOIN vaults v ON n.vault_id = v.id
             LEFT JOIN vaults sv ON n.sub_vault_id = sv.id
             WHERE ne.chunk_type = 'primary'
               AND ne.chunk_index = 0
               AND ne.model = ?
               AND n.deleted_at IS NULL
               AND n.is_archived = 0
               AND (v.id IS NULL OR v.deleted_at IS NULL)
               AND (sv.id IS NULL OR sv.deleted_at IS NULL);"
            .to_string();
        (query, vec![model.to_string()])
    };

    let mut stmt = conn
        .prepare(&query_str)
        .map_err(|err| format!("Failed to prepare search statement: {}", err))?;

    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect();

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params_refs), |row| {
            let node = DbNode {
                id: row.get(0)?,
                vault_id: row.get(1)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                node_type: row.get(4)?,
            };
            let embedding_bytes: Vec<u8> = row.get(5)?;
            let sub_vault_id: Option<String> = row.get(6)?;
            let node_privacy_tier: Option<String> = row.get(7)?;
            Ok((node, embedding_bytes, sub_vault_id, node_privacy_tier))
        })
        .map_err(|err| format!("Failed to execute search query: {}", err))?;

    let mut candidates = Vec::new();
    for row_res in rows {
        let (node, bytes, sub_vault_id, node_privacy_tier) =
            row_res.map_err(|err| format!("Failed to read row: {}", err))?;

        let effective = crate::resolve_node_effective_privacy(
            conn,
            &node.vault_id,
            sub_vault_id.as_deref(),
            node_privacy_tier.as_deref(),
        )?;

        if crate::privacy::embedding_should_skip(&effective) {
            continue;
        }

        match deserialize_f32_vec(&bytes) {
            Ok(vec) => {
                if vec.len() == query_vector.len() {
                    let score = cosine_similarity(query_vector, &vec);
                    if !score.is_nan() {
                        candidates.push((node, score));
                    }
                } else {
                    eprintln!(
                        "Dimension mismatch for node {}: query has {}, stored has {}",
                        node.id,
                        query_vector.len(),
                        vec.len()
                    );
                }
            }
            Err(err) => {
                eprintln!(
                    "Failed to deserialize embedding for node {}: {}",
                    node.id, err
                );
            }
        }
    }

    // Sort descending by similarity score, with a deterministic secondary sort alphabetically by node id.
    if candidates.len() > n {
        candidates.select_nth_unstable_by(n, |a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        candidates[0..n].sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        candidates.truncate(n);
    } else {
        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
    }

    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::storage::{setup_test_db, upsert_embedding, EmbeddingRow};

    #[test]
    fn test_cosine_similarity() {
        // Identical vectors -> 1.0
        let a = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);

        // Opposite vectors -> -1.0
        let b = vec![-1.0, -2.0, -3.0];
        assert!((cosine_similarity(&a, &b) - (-1.0)).abs() < 1e-6);

        // Orthogonal vectors -> 0.0
        let c1 = vec![1.0, 0.0];
        let c2 = vec![0.0, 1.0];
        assert!((cosine_similarity(&c1, &c2) - 0.0).abs() < 1e-6);

        // Clamping checks
        let large_a = vec![1.00001, 0.0];
        let large_b = vec![1.00002, 0.0];
        let sim = cosine_similarity(&large_a, &large_b);
        assert!((-1.0..=1.0).contains(&sim));

        // Division-by-zero or empty checks -> 0.0
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[]), 0.0);
        assert_eq!(cosine_similarity(&[0.0], &[0.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[test]
    fn test_find_top_n_similar_behavior() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";

        // Seed embeddings:
        // node_test_1: [1.0, 0.0, 0.0]
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        // node_test_2: [0.0, 1.0, 0.0]
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_2".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![0.0, 1.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        // Query vector: [1.0, 0.1, 0.0] (Very similar to node_test_1, less similar to node_test_2)
        let query = vec![1.0, 0.1, 0.0];
        let results = find_top_n_similar(&conn, &query, model, 10, None)?;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.id, "node_test_1");
        assert!(results[0].1 > results[1].1);

        // Limit truncation test: retrieve only 1 result
        let results_limited = find_top_n_similar(&conn, &query, model, 1, None)?;
        assert_eq!(results_limited.len(), 1);
        assert_eq!(results_limited[0].0.id, "node_test_1");

        // Soft-deleted node filter test
        conn.execute(
            "UPDATE nodes SET deleted_at = datetime('now') WHERE id = 'node_test_1';",
            [],
        )?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;
        let results_after_delete = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert_eq!(results_after_delete.len(), 1);
        assert_eq!(results_after_delete[0].0.id, "node_test_2");

        // Archived node filter test
        conn.execute(
            "UPDATE nodes SET deleted_at = NULL, is_archived = 1 WHERE id = 'node_test_1';",
            [],
        )?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;
        let results_after_archive = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert_eq!(results_after_archive.len(), 1);
        assert_eq!(results_after_archive[0].0.id, "node_test_2");

        // Detail chunk/non-primary chunk exclusion test:
        // Upsert a detail chunk (chunk_type = 'detail') with high similarity for a new node
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('node_test_3', 'vault_test', 'concept', 'Test Node 3', 'Test summary 3', 'Test detail 3', 'test', 'manual', '{}', '{}');",
            [],
        )?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_3".to_string(),
                chunk_index: 0,
                chunk_type: "detail".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.1, 0.0],
                computed_at: "time".to_string(),
            },
        )?;
        // Also upsert a primary chunk with chunk_index = 1 for the same node
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_3".to_string(),
                chunk_index: 1,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.1, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        // Search with query and assert that node_test_3 (which only has non-primary chunks) does not appear
        let results_detail_excluded = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert!(!results_detail_excluded
            .iter()
            .any(|(node, _)| node.id == "node_test_3"));

        // Zero-embedding node test:
        // Insert a node with NO embedding rows
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('node_test_4', 'vault_test', 'concept', 'Test Node 4', 'Test summary 4', 'Test detail 4', 'test', 'manual', '{}', '{}');",
            [],
        )?;

        // Search and assert that node_test_4 does not appear in results
        let results_zero_emb = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert!(!results_zero_emb
            .iter()
            .any(|(node, _)| node.id == "node_test_4"));

        // Restore node_test_1 so it is active again
        conn.execute(
            "UPDATE nodes SET is_archived = 0 WHERE id = 'node_test_1';",
            [],
        )?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_1".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        // Vault-filtered similarity test
        // 1. Create a second vault: vault_other
        conn.execute(
            "INSERT INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
             VALUES ('vault_other', 'Other Vault', 'vault', 'Fixture Vault', 'open', 'standard', 0, '{}');",
            [],
        )?;

        // 2. Insert a node in vault_other: node_test_5
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('node_test_5', 'vault_other', 'concept', 'Test Node 5', 'Test summary 5', 'Test detail 5', 'test', 'manual', '{}', '{}');",
            [],
        )?;

        // 3. Upsert a primary chunk for node_test_5 that is MORE similar to the query than node_test_1
        // Query is [1.0, 0.1, 0.0]
        // node_test_1 is [1.0, 0.0, 0.0] (sim ~ 0.995)
        // node_test_5 is [1.0, 0.09, 0.0] (sim ~ 0.9999)
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "node_test_5".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.09, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        // 4. Query without vault filter -> node_test_5 should be ranked first because it is more similar
        let results_unfiltered = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert_eq!(results_unfiltered[0].0.id, "node_test_5");

        // 5. Query with vault filter for vault_test -> node_test_5 must be excluded, and node_test_1 should be first
        let mut allowed_vaults = HashSet::new();
        allowed_vaults.insert("vault_test".to_string());
        let results_filtered = find_top_n_similar(&conn, &query, model, 10, Some(&allowed_vaults))?;
        assert!(!results_filtered
            .iter()
            .any(|(node, _)| node.id == "node_test_5"));
        assert_eq!(results_filtered[0].0.id, "node_test_1");

        // 6. Query with empty vault set -> should return an empty result immediately due to empty guard
        let results_empty_vaults =
            find_top_n_similar(&conn, &query, model, 10, Some(&HashSet::new()))?;
        assert!(results_empty_vaults.is_empty());

        Ok(())
    }

    #[test]
    fn test_find_top_n_similar_excludes_soft_deleted_vaults(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";
        let query = vec![1.0, 0.0, 0.0];

        // 1. Soft-deleted parent vault test
        conn.execute(
            "INSERT INTO vaults (id, name, deleted_at) VALUES ('v_del', 'Deleted Vault', datetime('now'));",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
             VALUES ('n_del_v', 'v_del', 'concept', 'Title', 'Summary', 'Detail');",
            [],
        )?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "n_del_v".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        let results_del_v = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert!(!results_del_v.iter().any(|(node, _)| node.id == "n_del_v"));

        // 2. Soft-deleted sub-vault test
        conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, name, deleted_at) VALUES ('sv_del', 'vault_test', 'Deleted Sub Vault', datetime('now'));",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, sub_vault_id, node_type, title, summary, detail)
             VALUES ('n_del_sv', 'vault_test', 'sv_del', 'concept', 'Title', 'Summary', 'Detail');",
            [],
        )?;
        upsert_embedding(
            &conn,
            &EmbeddingRow {
                node_id: "n_del_sv".to_string(),
                chunk_index: 0,
                chunk_type: "primary".to_string(),
                model: model.to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                computed_at: "time".to_string(),
            },
        )?;

        let results_del_sv = find_top_n_similar(&conn, &query, model, 10, None)?;
        assert!(!results_del_sv.iter().any(|(node, _)| node.id == "n_del_sv"));

        Ok(())
    }

    #[test]
    fn test_n_level_deep_nested_vector_search_retrieval() -> Result<(), Box<dyn std::error::Error>>
    {
        let conn = setup_test_db()?;
        let model = "test-model";
        let query = vec![1.0, 0.0, 0.0];

        // Create 3-level deep vault hierarchy: Root -> Level1 -> Level2 -> Level3
        conn.execute(
            "INSERT INTO vaults (id, name) VALUES ('v_root', 'Root Vault');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, parent_vault_id, name) VALUES ('v_l1', 'v_root', 'Level 1');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, parent_vault_id, name) VALUES ('v_l2', 'v_l1', 'Level 2');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, parent_vault_id, name) VALUES ('v_l3', 'v_l2', 'Level 3');",
            [],
        )?;

        // Create nodes at each nesting level
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_root', 'v_root', 'concept', 'Root Note', 'Sum');", [])?;
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_l3', 'v_l3', 'concept', 'Deep Note', 'Sum');", [])?;

        for n_id in ["n_root", "n_l3"] {
            upsert_embedding(
                &conn,
                &EmbeddingRow {
                    node_id: n_id.to_string(),
                    chunk_index: 0,
                    chunk_type: "primary".to_string(),
                    model: model.to_string(),
                    embedding: vec![1.0, 0.0, 0.0],
                    computed_at: "time".to_string(),
                },
            )?;
        }

        // Test 1: Search scoped to Root Vault must find both Root Note AND Deep Note (Level 3)
        let root_scope = HashSet::from(["v_root".to_string()]);
        let results = find_top_n_similar(&conn, &query, model, 10, Some(&root_scope))?;

        assert_eq!(
            results.len(),
            2,
            "Search scoped to root must return both root note and deep descendant note"
        );
        let result_ids: HashSet<String> = results.into_iter().map(|(n, _)| n.id).collect();
        assert!(result_ids.contains("n_root"));
        assert!(result_ids.contains("n_l3"));

        // Test 2: Search scoped specifically to Level 2 must find Deep Note (Level 3) but NOT Root Note
        let l2_scope = HashSet::from(["v_l2".to_string()]);
        let results_l2 = find_top_n_similar(&conn, &query, model, 10, Some(&l2_scope))?;

        assert_eq!(results_l2.len(), 1);
        assert_eq!(results_l2[0].0.id, "n_l3");

        Ok(())
    }

    #[test]
    fn test_find_top_n_similar_nested_vault_waterfall_privacy_filtering(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        let model = "test-model";
        let query = vec![1.0, 0.0, 0.0];

        // 1. Create a 5-level nested vault hierarchy
        // v_lvl0 (open) -> v_lvl1 (open) -> v_lvl2 (redacted) -> v_lvl3 (open) -> v_lvl4 (open)
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('v_lvl0', 'Level 0', 'open');",
            [],
        )?;
        conn.execute("INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES ('v_lvl1', 'v_lvl0', 'Level 1', 'open');", [])?;
        conn.execute("INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES ('v_lvl2', 'v_lvl1', 'Level 2', 'redacted');", [])?;
        conn.execute("INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES ('v_lvl3', 'v_lvl2', 'Level 3', 'open');", [])?;
        conn.execute("INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES ('v_lvl4', 'v_lvl3', 'Level 4', 'open');", [])?;

        // 2. Insert nodes at various depths
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_lvl0', 'v_lvl0', 'concept', 'L0 Note', 'Sum');", [])?;
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_lvl1', 'v_lvl1', 'concept', 'L1 Note', 'Sum');", [])?;
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_lvl3', 'v_lvl3', 'concept', 'L3 Note under Redacted Parent', 'Sum');", [])?;
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_lvl4', 'v_lvl4', 'concept', 'L4 Note under Redacted Ancestor', 'Sum');", [])?;

        // Insert a node in v_lvl1 with a direct privacy_overrides record setting it to redacted
        conn.execute("INSERT INTO nodes (id, vault_id, node_type, title, summary) VALUES ('n_lvl1_override', 'v_lvl1', 'concept', 'L1 Overridden Note', 'Sum');", [])?;
        conn.execute("INSERT INTO privacy_overrides (node_id, privacy_tier) VALUES ('n_lvl1_override', 'redacted');", [])?;

        // Upsert embeddings for all nodes
        for n_id in ["n_lvl0", "n_lvl1", "n_lvl3", "n_lvl4", "n_lvl1_override"] {
            upsert_embedding(
                &conn,
                &EmbeddingRow {
                    node_id: n_id.to_string(),
                    chunk_index: 0,
                    chunk_type: "primary".to_string(),
                    model: model.to_string(),
                    embedding: vec![1.0, 0.0, 0.0],
                    computed_at: "time".to_string(),
                },
            )?;
        }

        // Test: Vector search scoped to root vault v_lvl0
        let root_scope = HashSet::from(["v_lvl0".to_string()]);
        let results = find_top_n_similar(&conn, &query, model, 10, Some(&root_scope))?;

        let result_ids: HashSet<String> = results.into_iter().map(|(n, _)| n.id).collect();

        // Open nodes in open vaults MUST be present
        assert!(
            result_ids.contains("n_lvl0"),
            "L0 open node must be returned"
        );
        assert!(
            result_ids.contains("n_lvl1"),
            "L1 open node must be returned"
        );

        // Nodes in/under v_lvl2 (redacted) MUST be excluded by waterfall privacy
        assert!(
            !result_ids.contains("n_lvl3"),
            "L3 node under redacted parent must be excluded"
        );
        assert!(
            !result_ids.contains("n_lvl4"),
            "L4 node under redacted ancestor must be excluded"
        );

        // Node with privacy override = redacted MUST be excluded
        assert!(
            !result_ids.contains("n_lvl1_override"),
            "Node with redacted override must be excluded"
        );

        Ok(())
    }
}
