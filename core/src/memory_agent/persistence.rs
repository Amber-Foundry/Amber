use crate::memory_agent::changeset::{ChangesetItemType, PendingChangeset};
use rusqlite::{params, Connection};

/// Persists a built changeset and its pending items into SQLite.
/// Returns the generated changeset ID.
pub fn persist_changeset(
    conn: &Connection,
    changeset: &PendingChangeset,
    model_used: Option<&str>,
) -> Result<String, String> {
    // 1. Generate unique changeset ID
    let changeset_id = crate::generate_id(conn, "cs")?;

    // 2. Insert Changeset
    conn.execute(
        "INSERT INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count, model_used, created_at)
         VALUES (?1, ?2, 'pending', ?3, 0, 0, ?4, datetime('now'));",
        params![
            changeset_id,
            changeset.session_id,
            changeset.items.len() as i64,
            model_used,
        ],
    )
    .map_err(|err| format!("Failed to insert changeset: {err}"))?;

    // 3. Insert Changeset Items
    for (i, item) in changeset.items.iter().enumerate() {
        let item_id = crate::generate_id(conn, "csi")?;
        let item_type_str = match item.item_type {
            ChangesetItemType::Add => "add",
            ChangesetItemType::Update => "update",
            ChangesetItemType::Merge => "merge",
            ChangesetItemType::Delete => "delete",
        };

        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, status, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9);",
            params![
                item_id,
                changeset_id,
                item_type_str,
                item.target_node_id,
                item.proposed_data,
                item.existing_data,
                item.similarity,
                item.merge_with_id,
                i as i64, // sort_order
            ],
        )
        .map_err(|err| format!("Failed to insert changeset item: {err}"))?;
    }

    Ok(changeset_id)
}

/// Counts total pending changeset items.
pub fn count_pending_items(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM changeset_items WHERE status = 'pending';",
        [],
        |row| row.get(0),
    )
    .map_err(|err| format!("Failed counting pending changeset items: {err}"))
}

fn map_changeset_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::ipc_types::Changeset> {
    let id: String = row.get(0)?;
    let session_id: Option<String> = row.get(1)?;
    let status: String = row.get(2)?;
    let item_count: i64 = row.get(3)?;
    let accepted_count: i64 = row.get(4)?;
    let dismissed_count: i64 = row.get(5)?;
    let model_used: Option<String> = row.get(6)?;
    let created_at: String = row.get(7)?;
    let reviewed_at: Option<String> = row.get(8)?;

    let item_type: Option<String> = row.get(9)?;
    let door_id: Option<String> = row.get(10)?;
    let proposed_data: Option<String> = row.get(11)?;

    let summary = if let Some(item_type) = item_type {
        let item_type_lower = item_type.to_lowercase();
        let primary_title = if item_type_lower == "repoint_door"
            || item_type_lower == "orphan_alert"
        {
            format!(
                "Repoint door #{}",
                door_id.unwrap_or_else(|| "unknown".to_string())
            )
        } else if let Some(ref data_str) = proposed_data {
            let parsed: serde_json::Value = serde_json::from_str(data_str).unwrap_or_default();
            parsed
                .get("title")
                .or_else(|| parsed.get("summary"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Proposal #{}", id.chars().take(8).collect::<String>()))
        } else {
            format!("Proposal #{}", id.chars().take(8).collect::<String>())
        };

        if item_count > 1 {
            let others = item_count - 1;
            Some(format!(
                "{} & {} other{}",
                primary_title,
                others,
                if others > 1 { "s" } else { "" }
            ))
        } else {
            Some(primary_title)
        }
    } else {
        Some(format!(
            "Empty Changeset #{}",
            id.chars().take(8).collect::<String>()
        ))
    };

    Ok(crate::ipc_types::Changeset {
        id,
        session_id,
        status,
        item_count,
        accepted_count,
        dismissed_count,
        model_used,
        created_at,
        reviewed_at,
        summary,
    })
}

/// Lists all pending changesets ordered by creation time descending.
pub fn list_pending_changesets(
    conn: &Connection,
) -> Result<Vec<crate::ipc_types::Changeset>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.session_id, c.status, c.item_count, c.accepted_count, c.dismissed_count, c.model_used, c.created_at, c.reviewed_at,
                    ci.item_type, ci.door_id, ci.proposed_data
             FROM changesets c
             LEFT JOIN changeset_items ci ON ci.id = (
                 SELECT ci2.id
                 FROM changeset_items ci2
                 WHERE ci2.changeset_id = c.id
                 ORDER BY CASE WHEN ci2.item_type IN ('add', 'update', 'merge', 'ADD', 'UPDATE', 'MERGE') THEN 0 ELSE 1 END ASC,
                          ci2.sort_order ASC,
                          ci2.id ASC
                 LIMIT 1
             )
             WHERE c.status = 'pending'
             ORDER BY c.created_at DESC;",
        )
        .map_err(|err| format!("Failed to prepare list pending changesets query: {err}"))?;

    let rows = stmt
        .query_map([], map_changeset_row)
        .map_err(|err| format!("Failed to execute list pending changesets query: {err}"))?;

    let mut list = Vec::new();
    for r in rows {
        list.push(r.map_err(|e| format!("Failed decoding changeset row: {e}"))?);
    }
    Ok(list)
}

/// Lists all items in a changeset ordered by sort order.
fn privacy_tier_value(tier: &str) -> i32 {
    match tier.trim().to_lowercase().as_str() {
        "open" => 0,
        "local_only" => 1,
        "local" => 1,
        "locked" => 2,
        "redacted" => 3,
        _ => 0,
    }
}

/// Lists all items in a changeset ordered by sort order.
pub fn list_changeset_items(
    conn: &Connection,
    changeset_id: &str,
) -> Result<Vec<crate::ipc_types::ChangesetItem>, String> {
    // 1. Resolve source vault sensitivity
    let source_info: Option<(String, String)> = conn
        .query_row(
            "SELECT v.name, v.privacy_tier FROM changesets c
             JOIN sessions s ON c.session_id = s.id
             JOIN vaults v ON s.vault_id = v.id
             WHERE c.id = ?1;",
            [changeset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let (source_vault_name, source_tier) = match source_info {
        Some((name, tier)) => (Some(name), tier),
        None => (None, "open".to_string()),
    };
    let source_val = privacy_tier_value(&source_tier);

    // 2. Preload all vault privacy tiers into a HashMap to avoid N+1 queries
    let mut vault_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    {
        let mut vault_stmt = conn
            .prepare(
                "SELECT id, name, privacy_tier FROM vaults
                 UNION ALL
                 SELECT sv.id, sv.name, COALESCE(sv.privacy_tier, v.privacy_tier) AS privacy_tier
                 FROM sub_vaults sv
                 JOIN vaults v ON sv.vault_id = v.id;",
            )
            .map_err(|err| format!("Failed to prepare vault preload query: {err}"))?;
        let vault_rows = vault_stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let tier: String = row.get(2)?;
                Ok((id, name, tier))
            })
            .map_err(|err| format!("Failed to execute vault preload query: {err}"))?;
        for (id, name, tier) in vault_rows.flatten() {
            vault_map.insert(id, (name, tier));
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, door_id, status, reviewed_at, sort_order
             FROM changeset_items
             WHERE changeset_id = ?1
             ORDER BY sort_order ASC, id ASC;",
        )
        .map_err(|err| format!("Failed to prepare list changeset items query: {err}"))?;

    let rows = stmt
        .query_map([changeset_id], |row| {
            let id: String = row.get(0)?;
            let c_id: String = row.get(1)?;
            let item_type: String = row.get(2)?;
            let target_node_id: Option<String> = row.get(3)?;
            let proposed_data: String = row.get(4)?;
            let existing_data: Option<String> = row.get(5)?;
            let similarity: Option<f64> = row.get(6)?;
            let merge_with_id: Option<String> = row.get(7)?;
            let door_id: Option<String> = row.get(8)?;
            let status: String = row.get(9)?;
            let reviewed_at: Option<String> = row.get(10)?;
            let sort_order: i64 = row.get(11)?;

            Ok((
                id,
                c_id,
                item_type,
                target_node_id,
                proposed_data,
                existing_data,
                similarity,
                merge_with_id,
                door_id,
                status,
                reviewed_at,
                sort_order,
            ))
        })
        .map_err(|err| format!("Failed to execute list changeset items query: {err}"))?;

    let mut list = Vec::new();
    for r in rows {
        let (
            id,
            c_id,
            item_type,
            target_node_id,
            proposed_data,
            existing_data,
            similarity,
            merge_with_id,
            door_id,
            status,
            reviewed_at,
            sort_order,
        ) = r.map_err(|e| format!("Failed decoding changeset item row: {e}"))?;

        // 3. Parse target vault sensitivity from proposed_data, look up from preloaded map
        let proposed_json: serde_json::Value =
            serde_json::from_str(&proposed_data).unwrap_or_else(|_| serde_json::json!({}));
        let target_vault_id = proposed_json
            .get("vaultId")
            .or_else(|| proposed_json.get("vault_id"))
            .and_then(|v| v.as_str());

        let (target_vault_name, target_tier) =
            match target_vault_id.and_then(|vid| vault_map.get(vid)) {
                Some((name, tier)) => (Some(name.clone()), tier.clone()),
                None => (None, "open".to_string()),
            };
        let target_val = privacy_tier_value(&target_tier);

        // 4. Compute cross-vault anomaly
        let cross_vault_anomaly = target_val > source_val;
        let anomaly_warning = if cross_vault_anomaly {
            Some(format!(
                "Security Warning: Slated for {} ({}) but source conversation was scoped to {} ({}). Review carefully.",
                target_vault_name.unwrap_or_else(|| target_vault_id.unwrap_or("target").to_string()),
                target_tier.to_uppercase(),
                source_vault_name.as_deref().unwrap_or("source"),
                source_tier.to_uppercase()
            ))
        } else {
            None
        };

        list.push(crate::ipc_types::ChangesetItem {
            id,
            changeset_id: c_id,
            item_type,
            target_node_id,
            proposed_data,
            existing_data,
            similarity,
            merge_with_id,
            door_id,
            status,
            reviewed_at,
            sort_order,
            cross_vault_anomaly,
            anomaly_warning,
        });
    }
    Ok(list)
}

/// Lists all resolved changesets (status IN ('accepted', 'dismissed', 'partial')) ordered by reviewed time descending.
pub fn list_resolved_changesets(
    conn: &Connection,
) -> Result<Vec<crate::ipc_types::Changeset>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.session_id, c.status, c.item_count, c.accepted_count, c.dismissed_count, c.model_used, c.created_at, c.reviewed_at,
                    ci.item_type, ci.door_id, ci.proposed_data
             FROM changesets c
             LEFT JOIN changeset_items ci ON ci.id = (
                 SELECT ci2.id
                 FROM changeset_items ci2
                 WHERE ci2.changeset_id = c.id
                 ORDER BY CASE WHEN ci2.item_type IN ('add', 'update', 'merge', 'ADD', 'UPDATE', 'MERGE') THEN 0 ELSE 1 END ASC,
                          ci2.sort_order ASC,
                          ci2.id ASC
                 LIMIT 1
             )
             WHERE c.status IN ('accepted', 'dismissed', 'partial')
             ORDER BY c.reviewed_at DESC;",
        )
        .map_err(|err| format!("Failed to prepare list resolved changesets query: {err}"))?;

    let rows = stmt
        .query_map([], map_changeset_row)
        .map_err(|err| format!("Failed to execute list resolved changesets query: {err}"))?;

    let mut list = Vec::new();
    for r in rows {
        list.push(r.map_err(|e| format!("Failed decoding changeset row: {e}"))?);
    }
    Ok(list)
}

/// Fetches a single changeset by ID with its summary.
pub fn get_changeset_by_id(
    conn: &Connection,
    changeset_id: &str,
) -> Result<crate::ipc_types::Changeset, String> {
    conn.query_row(
        "SELECT c.id, c.session_id, c.status, c.item_count, c.accepted_count, c.dismissed_count, c.model_used, c.created_at, c.reviewed_at,
                ci.item_type, ci.door_id, ci.proposed_data
         FROM changesets c
         LEFT JOIN changeset_items ci ON ci.id = (
             SELECT ci2.id
             FROM changeset_items ci2
             WHERE ci2.changeset_id = c.id
             ORDER BY CASE WHEN ci2.item_type IN ('add', 'update', 'merge', 'ADD', 'UPDATE', 'MERGE') THEN 0 ELSE 1 END ASC,
                      ci2.sort_order ASC,
                      ci2.id ASC
             LIMIT 1
         )
         WHERE c.id = ?1 LIMIT 1;",
        [changeset_id],
        map_changeset_row,
    )
    .map_err(|err| format!("Failed to fetch changeset by ID: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_agent::changeset::PendingChangesetItem;

    fn setup_test_db() -> Connection {
        let conn = match Connection::open_in_memory() {
            Ok(c) => c,
            Err(e) => panic!("Failed to open in-memory DB: {e}"),
        };
        let ddl = "
            CREATE TABLE IF NOT EXISTS vaults (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                privacy_tier TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sub_vaults (
                id TEXT PRIMARY KEY,
                vault_id TEXT NOT NULL REFERENCES vaults(id),
                name TEXT NOT NULL,
                privacy_tier TEXT
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                vault_id TEXT REFERENCES vaults(id)
            );
            CREATE TABLE IF NOT EXISTS changesets (
                id TEXT PRIMARY KEY,
                session_id TEXT REFERENCES sessions(id),
                status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'dismissed', 'partial')),
                item_count INTEGER NOT NULL DEFAULT 0,
                accepted_count INTEGER NOT NULL DEFAULT 0,
                dismissed_count INTEGER NOT NULL DEFAULT 0,
                model_used TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                reviewed_at TEXT
            );
            CREATE TABLE IF NOT EXISTS changeset_items (
                id TEXT PRIMARY KEY,
                changeset_id TEXT NOT NULL REFERENCES changesets(id) ON DELETE CASCADE,
                item_type TEXT NOT NULL CHECK (item_type IN ('add', 'update', 'merge', 'delete', 'repoint_door', 'orphan_alert')),
                target_node_id TEXT,
                proposed_data TEXT NOT NULL DEFAULT '{}',
                existing_data TEXT DEFAULT '{}',
                similarity REAL,
                merge_with_id TEXT,
                door_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'dismissed', 'edited')),
                reviewed_at TEXT,
                sort_order INTEGER DEFAULT 0
            );
        ";
        if let Err(e) = conn.execute_batch(ddl) {
            panic!("Failed to create DDL: {e}");
        }
        // Insert dummy vault and session to satisfy foreign key constraints
        let _ = conn.execute(
            "INSERT OR IGNORE INTO vaults (id, name, privacy_tier) VALUES ('vault-root', 'Vault Root', 'open');",
            [],
        );
        let _ = conn.execute(
            "INSERT OR IGNORE INTO sessions (id, vault_id) VALUES ('session-123', 'vault-root');",
            [],
        );
        conn
    }

    #[test]
    fn test_persist_and_retrieve_changesets() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db();

        // 1. Initial counts: should be empty
        assert_eq!(count_pending_items(&conn)?, 0);
        assert!(list_pending_changesets(&conn)?.is_empty());

        // 2. Prepare a mock PendingChangeset
        let pending = PendingChangeset {
            session_id: "session-123".to_string(),
            model_used: None,
            items: vec![
                PendingChangesetItem {
                    item_type: ChangesetItemType::Add,
                    target_node_id: None,
                    proposed_data: "{\"title\":\"Rust fact\"}".to_string(),
                    existing_data: None,
                    similarity: None,
                    merge_with_id: None,
                },
                PendingChangesetItem {
                    item_type: ChangesetItemType::Update,
                    target_node_id: Some("node_abc".to_string()),
                    proposed_data: "{\"title\":\"Acme Corp update\"}".to_string(),
                    existing_data: Some("{\"title\":\"Acme Corp\"}".to_string()),
                    similarity: Some(0.92),
                    merge_with_id: None,
                },
            ],
        };

        // 3. Persist changeset
        let cs_id = persist_changeset(&conn, &pending, Some("llama3"))?;
        assert!(cs_id.starts_with("cs_"));

        // 4. Verify counts & list changesets
        assert_eq!(count_pending_items(&conn)?, 2);
        let changesets = list_pending_changesets(&conn)?;
        assert_eq!(changesets.len(), 1);
        assert_eq!(changesets[0].id, cs_id);
        assert_eq!(changesets[0].session_id, Some("session-123".to_string()));
        assert_eq!(changesets[0].status, "pending");
        assert_eq!(changesets[0].item_count, 2);
        assert_eq!(changesets[0].model_used, Some("llama3".to_string()));
        assert_eq!(
            changesets[0].summary,
            Some("Rust fact & 1 other".to_string())
        );

        // 5. Verify list changeset items
        let items = list_changeset_items(&conn, &cs_id)?;
        assert_eq!(items.len(), 2);

        assert_eq!(items[0].changeset_id, cs_id);
        assert_eq!(items[0].item_type, "add");
        assert_eq!(items[0].target_node_id, None);
        assert_eq!(items[0].proposed_data, "{\"title\":\"Rust fact\"}");
        assert_eq!(items[0].existing_data, None);
        assert_eq!(items[0].similarity, None);
        assert_eq!(items[0].sort_order, 0);

        assert_eq!(items[1].changeset_id, cs_id);
        assert_eq!(items[1].item_type, "update");
        assert_eq!(items[1].target_node_id, Some("node_abc".to_string()));
        assert_eq!(items[1].proposed_data, "{\"title\":\"Acme Corp update\"}");
        assert_eq!(
            items[1].existing_data,
            Some("{\"title\":\"Acme Corp\"}".to_string())
        );
        assert_eq!(items[1].similarity, Some(0.92));
        assert_eq!(items[1].sort_order, 1);

        Ok(())
    }

    #[test]
    fn test_sub_vault_privacy_resolution_and_warning() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db();

        // Insert restricted vault and sub-vaults
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault-restricted', 'Vault Restricted', 'locked');",
            [],
        )?;
        conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, name, privacy_tier) VALUES ('sub-null-tier', 'vault-restricted', 'Sub Null Tier', NULL);",
            [],
        )?;
        conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, name, privacy_tier) VALUES ('sub-open-tier', 'vault-restricted', 'Sub Open Tier', 'open');",
            [],
        )?;

        // Prepare a changeset where source is open (from session-123, which is mapped to vault-root with tier open)
        let pending = PendingChangeset {
            session_id: "session-123".to_string(),
            model_used: None,
            items: vec![
                PendingChangesetItem {
                    item_type: ChangesetItemType::Add,
                    target_node_id: None,
                    proposed_data:
                        "{\"title\":\"Targeting sub-null-tier\",\"vaultId\":\"sub-null-tier\"}"
                            .to_string(),
                    existing_data: None,
                    similarity: None,
                    merge_with_id: None,
                },
                PendingChangesetItem {
                    item_type: ChangesetItemType::Add,
                    target_node_id: None,
                    proposed_data:
                        "{\"title\":\"Targeting sub-open-tier\",\"vaultId\":\"sub-open-tier\"}"
                            .to_string(),
                    existing_data: None,
                    similarity: None,
                    merge_with_id: None,
                },
            ],
        };

        let cs_id = persist_changeset(&conn, &pending, Some("llama3"))?;
        let items = list_changeset_items(&conn, &cs_id)?;
        assert_eq!(items.len(), 2);

        // Item 0 targets sub-null-tier which inherits 'locked' (value 2). Source is 'open' (value 0).
        // Since 2 > 0, it must trigger a Security Warning.
        assert!(items[0].anomaly_warning.is_some());
        let warning_text = items[0]
            .anomaly_warning
            .as_ref()
            .ok_or("Expected anomaly warning to be present")?;
        assert!(warning_text.contains("Security Warning: Slated for Sub Null Tier (LOCKED)"));

        // Item 1 targets sub-open-tier which overrides parent and has 'open' (value 0). Source is 'open' (value 0).
        // Since 0 is not > 0, it must NOT trigger a Security Warning.
        assert!(items[1].anomaly_warning.is_none());

        Ok(())
    }
}
