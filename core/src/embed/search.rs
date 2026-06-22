use crate::embed::storage::deserialize_f32_vec;
use rusqlite::{params, Connection};

/// Compute cosine similarity between two f32 slices.
///
/// Formula: A . B / (||A|| * ||B||)
/// Returns 0.0 if vectors are empty, have different lengths, or either has zero norm.
/// Clamps results between [-1.0, 1.0].
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (&x, &y) in a.iter().zip(b.iter()) {
        dot_product += x * y;
        norm_a += x * x;
        norm_b += y * y;
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

    (raw_score as f64).clamp(-1.0, 1.0)
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
) -> Result<Vec<(String, f64)>, String> {
    if n == 0 || query_vector.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ne.node_id, ne.embedding
             FROM node_embeddings ne
             JOIN nodes n ON ne.node_id = n.id
             WHERE ne.chunk_type = 'primary'
               AND ne.chunk_index = 0
               AND ne.model = ?1
               AND n.deleted_at IS NULL
               AND n.is_archived = 0;",
        )
        .map_err(|err| format!("Failed to prepare search statement: {}", err))?;

    let rows = stmt
        .query_map(params![model], |row| {
            let node_id: String = row.get(0)?;
            let embedding_bytes: Vec<u8> = row.get(1)?;
            Ok((node_id, embedding_bytes))
        })
        .map_err(|err| format!("Failed to execute search query: {}", err))?;

    let mut candidates = Vec::new();
    for row_res in rows {
        let (node_id, bytes) = row_res.map_err(|err| format!("Failed to read row: {}", err))?;

        match deserialize_f32_vec(&bytes) {
            Ok(vec) => {
                if vec.len() == query_vector.len() {
                    let score = cosine_similarity(query_vector, &vec);
                    if !score.is_nan() {
                        candidates.push((node_id, score));
                    }
                } else {
                    eprintln!(
                        "Dimension mismatch for node {}: query has {}, stored has {}",
                        node_id,
                        query_vector.len(),
                        vec.len()
                    );
                }
            }
            Err(err) => {
                eprintln!(
                    "Failed to deserialize embedding for node {}: {}",
                    node_id, err
                );
            }
        }
    }

    // Sort descending by similarity score, with a deterministic secondary sort alphabetically by node_id.
    if candidates.len() > n {
        candidates.select_nth_unstable_by(n, |a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        candidates[0..n].sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        candidates.truncate(n);
    } else {
        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
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
        let results = find_top_n_similar(&conn, &query, model, 10)?;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "node_test_1");
        assert!(results[0].1 > results[1].1);

        // Limit truncation test: retrieve only 1 result
        let results_limited = find_top_n_similar(&conn, &query, model, 1)?;
        assert_eq!(results_limited.len(), 1);
        assert_eq!(results_limited[0].0, "node_test_1");

        // Soft-deleted node filter test
        conn.execute(
            "UPDATE nodes SET deleted_at = datetime('now') WHERE id = 'node_test_1';",
            [],
        )?;
        let results_after_delete = find_top_n_similar(&conn, &query, model, 10)?;
        assert_eq!(results_after_delete.len(), 1);
        assert_eq!(results_after_delete[0].0, "node_test_2");

        // Archived node filter test
        conn.execute(
            "UPDATE nodes SET deleted_at = NULL, is_archived = 1 WHERE id = 'node_test_1';",
            [],
        )?;
        let results_after_archive = find_top_n_similar(&conn, &query, model, 10)?;
        assert_eq!(results_after_archive.len(), 1);
        assert_eq!(results_after_archive[0].0, "node_test_2");

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
        let results_detail_excluded = find_top_n_similar(&conn, &query, model, 10)?;
        assert!(!results_detail_excluded
            .iter()
            .any(|(id, _)| id == "node_test_3"));

        // Zero-embedding node test:
        // Insert a node with NO embedding rows
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority, meta)
             VALUES ('node_test_4', 'vault_test', 'concept', 'Test Node 4', 'Test summary 4', 'Test detail 4', 'test', 'manual', '{}', '{}');",
            [],
        )?;

        // Search and assert that node_test_4 does not appear in results
        let results_zero_emb = find_top_n_similar(&conn, &query, model, 10)?;
        assert!(!results_zero_emb.iter().any(|(id, _)| id == "node_test_4"));

        Ok(())
    }
}
