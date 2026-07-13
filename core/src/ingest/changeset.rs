use rusqlite::{params, Connection};

use crate::embed::EmbedEngine;
use crate::memory_agent::parser::CandidateNode;

fn import_session_id(job_id: &str) -> String {
    format!("import-{job_id}")
}

fn ensure_import_session(
    conn: &Connection,
    job_id: &str,
    target_vault_id: Option<&str>,
) -> Result<String, String> {
    let session_id = import_session_id(job_id);
    let vault_id = target_vault_id.unwrap_or("vault_learning");
    conn.execute(
        "INSERT INTO sessions (id, vault_id, scope_json)
         VALUES (?1, ?2, '[]')
         ON CONFLICT(id) DO UPDATE SET vault_id = excluded.vault_id;",
        params![session_id, vault_id],
    )
    .map_err(|err| format!("Failed to ensure import session: {err}"))?;
    Ok(session_id)
}

/// Build and persist a pending changeset from import extraction candidates.
///
/// Returns `Ok(None)` when `candidates` is empty (no changeset row is created).
pub fn finalize_import_changeset(
    conn: &mut Connection,
    job_id: &str,
    target_vault_id: Option<&str>,
    candidates: &[CandidateNode],
    model_used: Option<&str>,
    embed_engine: Option<&dyn EmbedEngine>,
) -> Result<Option<(String, i32)>, String> {
    if candidates.is_empty() {
        return Ok(None);
    }

    let session_id = ensure_import_session(conn, job_id, target_vault_id)?;
    let pending = crate::memory_agent::changeset::build_changeset(
        conn,
        candidates,
        &session_id,
        embed_engine,
    )?;
    let item_count = pending.items.len() as i32;
    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed to start import changeset transaction: {err}"))?;
    let changeset_id =
        crate::memory_agent::persistence::persist_changeset(&tx, &pending, model_used)?;
    tx.commit()
        .map_err(|err| format!("Failed to commit import changeset transaction: {err}"))?;
    Ok(Some((changeset_id, item_count)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_agent::changeset::ProposedNodeData;
    use crate::memory_agent::parser::{CandidateAction, CandidateNode};
    use rusqlite::Connection;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory()
            .unwrap_or_else(|err| panic!("expected in-memory sqlite connection: {err}"));
        conn.execute_batch(
            "CREATE TABLE vaults (id TEXT PRIMARY KEY, deleted_at TEXT);
             CREATE TABLE sub_vaults (id TEXT PRIMARY KEY, vault_id TEXT, deleted_at TEXT);
             CREATE TABLE sessions (id TEXT PRIMARY KEY, vault_id TEXT, scope_json TEXT NOT NULL DEFAULT '[]');
             CREATE TABLE nodes (
                id TEXT PRIMARY KEY,
                vault_id TEXT NOT NULL,
                sub_vault_id TEXT,
                node_type TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                detail TEXT,
                version INTEGER NOT NULL DEFAULT 1,
                is_archived INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT
             );
             CREATE TABLE node_embeddings (
                node_id TEXT NOT NULL,
                chunk_index INTEGER NOT NULL DEFAULT 0,
                chunk_type TEXT NOT NULL DEFAULT 'primary',
                model TEXT NOT NULL,
                embedding BLOB NOT NULL,
                computed_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (node_id, chunk_index, chunk_type)
             );
             CREATE TABLE changesets (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                status TEXT NOT NULL,
                item_count INTEGER NOT NULL DEFAULT 0,
                accepted_count INTEGER NOT NULL DEFAULT 0,
                dismissed_count INTEGER NOT NULL DEFAULT 0,
                model_used TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                reviewed_at TEXT
             );
             CREATE TABLE changeset_items (
                id TEXT PRIMARY KEY,
                changeset_id TEXT NOT NULL,
                item_type TEXT NOT NULL,
                target_node_id TEXT,
                proposed_data TEXT NOT NULL,
                existing_data TEXT,
                similarity REAL,
                merge_with_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                sort_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE id_sequences (prefix TEXT PRIMARY KEY, next_val INTEGER NOT NULL);",
        )
        .unwrap_or_else(|err| panic!("expected test schema: {err}"));
        conn.execute(
            "INSERT INTO vaults (id) VALUES ('vault_learning'), ('vault_root_graph'), ('vault_custom_user');",
            [],
        )
        .unwrap_or_else(|err| panic!("expected vault seed: {err}"));
        conn
    }

    fn sample_candidate(target_vault_key: Option<String>) -> CandidateNode {
        CandidateNode {
            title: "Imported fact".to_string(),
            summary: "From PDF extraction".to_string(),
            detail: None,
            node_type: Some("fact".to_string()),
            target_vault_key,
            tags: None,
            confidence: 0.9,
            action: CandidateAction::Add,
            source: Some("sample.pdf".to_string()),
            source_type: Some("pdf_import".to_string()),
            meta: None,
        }
    }

    #[test]
    fn finalize_import_changeset_empty_candidates_skips_persist() {
        let mut conn = setup_test_db();
        let result = finalize_import_changeset(
            &mut conn,
            "job-empty",
            Some("vault_learning"),
            &[],
            None,
            None,
        )
        .unwrap_or_else(|err| panic!("expected finalize to succeed: {err}"));
        assert!(result.is_none());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM changesets;", [], |row| row.get(0))
            .unwrap_or_else(|err| panic!("expected changeset count: {err}"));
        assert_eq!(count, 0);
    }

    #[test]
    fn finalize_import_changeset_onboarding_vault_round_trip() {
        let mut conn = setup_test_db();
        let candidates = vec![sample_candidate(Some("learning".to_string()))];
        let (changeset_id, item_count) = finalize_import_changeset(
            &mut conn,
            "job-learn",
            Some("vault_learning"),
            &candidates,
            Some("test-model"),
            None,
        )
        .unwrap_or_else(|err| panic!("expected finalize to succeed: {err}"))
        .unwrap_or_else(|| panic!("expected changeset id"));

        assert_eq!(item_count, 1);
        let session_vault: String = conn
            .query_row(
                "SELECT vault_id FROM sessions WHERE id = 'import-job-learn';",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|err| panic!("expected import session: {err}"));
        assert_eq!(session_vault, "vault_learning");

        let proposed_data: String = conn
            .query_row(
                "SELECT proposed_data FROM changeset_items WHERE changeset_id = ?1 LIMIT 1;",
                [&changeset_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|err| panic!("expected changeset item: {err}"));
        let proposed: ProposedNodeData = serde_json::from_str(&proposed_data)
            .unwrap_or_else(|err| panic!("expected proposed JSON: {err}"));
        assert_eq!(proposed.vault_id.as_deref(), Some("vault_learning"));
    }

    #[test]
    fn finalize_import_changeset_custom_vault_falls_back_to_root_graph() {
        let mut conn = setup_test_db();
        let candidates = vec![sample_candidate(None)];
        let (changeset_id, _) = finalize_import_changeset(
            &mut conn,
            "job-custom",
            Some("vault_custom_user"),
            &candidates,
            None,
            None,
        )
        .unwrap_or_else(|err| panic!("expected finalize to succeed: {err}"))
        .unwrap_or_else(|| panic!("expected changeset id"));

        let proposed_data: String = conn
            .query_row(
                "SELECT proposed_data FROM changeset_items WHERE changeset_id = ?1 LIMIT 1;",
                [&changeset_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|err| panic!("expected changeset item: {err}"));
        let proposed: ProposedNodeData = serde_json::from_str(&proposed_data)
            .unwrap_or_else(|err| panic!("expected proposed JSON: {err}"));
        assert_eq!(proposed.vault_id.as_deref(), Some("vault_root_graph"));
    }
}
