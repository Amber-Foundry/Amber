use crate::ipc_types::ChangesetCommitInput;
use crate::priority;
use crate::redacted;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashSet;
use std::path::Path;

fn resolve_effective_privacy(
    tx: &rusqlite::Transaction,
    vault_id: &str,
    privacy_override: Option<&str>,
) -> Result<String, String> {
    if let Some(tier) = privacy_override {
        return Ok(tier.to_string());
    }

    let vault_privacy: String = tx
        .query_row(
            "SELECT privacy_tier FROM vaults WHERE id = ?1 LIMIT 1;",
            [vault_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed to fetch vault privacy: {err}"))?;

    Ok(vault_privacy)
}

#[allow(clippy::too_many_arguments)]
fn insert_changeset_node(
    tx: &Transaction,
    vault_id: &str,
    title: &str,
    summary: &str,
    detail: Option<&str>,
    node_type: &str,
    tags: Option<&Vec<String>>,
    session_key: Option<&redacted::SessionKey>,
) -> Result<String, String> {
    let parent_vault_id: Option<String> = tx
        .query_row(
            "SELECT vault_id FROM sub_vaults WHERE id = ?1 LIMIT 1;",
            [vault_id],
            |row| row.get(0),
        )
        .ok();

    let (resolved_vault_id, resolved_sub_vault_id) = match parent_vault_id {
        Some(parent_id) => (parent_id, Some(vault_id.to_string())),
        None => (vault_id.to_string(), None),
    };

    let effective_privacy = resolve_effective_privacy(tx, &resolved_vault_id, None)?;
    let is_redacted = effective_privacy == "redacted";

    let encrypted_payload = if is_redacted {
        let key = session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
        Some(redacted::encrypt_json(
            &redacted::NodeSecretPayload {
                title: title.to_string(),
                summary: summary.to_string(),
                detail: detail.map(String::from),
                source: Some("agent_extract".to_string()),
                source_type: Some("agent_extract".to_string()),
            },
            key,
        )?)
    } else {
        None
    };

    let stored_title = if is_redacted {
        "[REDACTED]".to_string()
    } else {
        title.to_string()
    };

    let stored_summary = if is_redacted {
        "[Metadata Locked]".to_string()
    } else {
        summary.to_string()
    };

    let node_id = crate::generate_id(tx, "node")?;
    let priority_json = priority::DEFAULT_PRIORITY_JSON;

    tx.execute(
        "INSERT INTO nodes (
            id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
            privacy_tier, priority, meta, encrypted_payload
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'agent_extract', 'agent_extract', NULL, ?8, '{}', ?9);",
        params![
            node_id,
            resolved_vault_id,
            resolved_sub_vault_id,
            node_type,
            stored_title,
            stored_summary,
            if is_redacted { None } else { detail },
            priority_json,
            encrypted_payload
        ],
    )
    .map_err(|err| format!("Failed to insert changeset node: {err}"))?;

    if let Some(tag_list) = tags {
        for tag_name in tag_list {
            let clean_name = tag_name.trim();
            if clean_name.is_empty() {
                continue;
            }

            let tag_id = match tx.query_row(
                "SELECT id FROM tags WHERE name = ?1;",
                [clean_name],
                |row| row.get::<_, String>(0),
            ) {
                Ok(id) => id,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    let new_id = crate::generate_id(tx, "tag")?;
                    tx.execute(
                        "INSERT INTO tags (id, name, color) VALUES (?1, ?2, NULL);",
                        params![new_id, clean_name],
                    )
                    .map_err(|err| format!("Failed inserting tag: {err}"))?;
                    new_id
                }
                Err(err) => return Err(format!("Failed querying tag: {err}")),
            };

            tx.execute(
                "INSERT OR IGNORE INTO node_tags (node_id, tag_id) VALUES (?1, ?2);",
                params![&node_id, &tag_id],
            )
            .map_err(|err| format!("Failed inserting node tag: {err}"))?;
        }
    }

    Ok(node_id)
}

#[allow(clippy::too_many_arguments)]
fn update_changeset_node(
    tx: &Transaction,
    node_id: &str,
    vault_id: &str,
    title: &str,
    summary: &str,
    detail: Option<&str>,
    node_type: &str,
    tags: Option<&Vec<String>>,
    session_key: Option<&redacted::SessionKey>,
) -> Result<(), String> {
    let parent_vault_id: Option<String> = tx
        .query_row(
            "SELECT vault_id FROM sub_vaults WHERE id = ?1 LIMIT 1;",
            [vault_id],
            |row| row.get(0),
        )
        .ok();

    let (resolved_vault_id, resolved_sub_vault_id) = match parent_vault_id {
        Some(parent_id) => (parent_id, Some(vault_id.to_string())),
        None => (vault_id.to_string(), None),
    };

    let effective_privacy = resolve_effective_privacy(tx, &resolved_vault_id, None)?;
    let is_redacted = effective_privacy == "redacted";

    let encrypted_payload = if is_redacted {
        let key = session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
        Some(redacted::encrypt_json(
            &redacted::NodeSecretPayload {
                title: title.to_string(),
                summary: summary.to_string(),
                detail: detail.map(String::from),
                source: Some("agent_extract".to_string()),
                source_type: Some("agent_extract".to_string()),
            },
            key,
        )?)
    } else {
        None
    };

    let stored_title = if is_redacted {
        "[REDACTED]".to_string()
    } else {
        title.to_string()
    };

    let stored_summary = if is_redacted {
        "[Metadata Locked]".to_string()
    } else {
        summary.to_string()
    };

    tx.execute(
        "UPDATE nodes
         SET vault_id = ?2,
             sub_vault_id = ?3,
             node_type = ?4,
             title = ?5,
             summary = ?6,
             detail = ?7,
             version = version + 1,
             updated_at = datetime('now'),
             encrypted_payload = ?8
         WHERE id = ?1 AND deleted_at IS NULL;",
        params![
            node_id,
            resolved_vault_id,
            resolved_sub_vault_id,
            node_type,
            stored_title,
            stored_summary,
            if is_redacted { None } else { detail },
            encrypted_payload
        ],
    )
    .map_err(|err| format!("Failed updating node: {err}"))?;

    tx.execute("DELETE FROM node_tags WHERE node_id = ?1;", [node_id])
        .map_err(|err| format!("Failed clearing node tags: {err}"))?;

    if let Some(tag_list) = tags {
        for tag_name in tag_list {
            let clean_name = tag_name.trim();
            if clean_name.is_empty() {
                continue;
            }

            let tag_id = match tx.query_row(
                "SELECT id FROM tags WHERE name = ?1;",
                [clean_name],
                |row| row.get::<_, String>(0),
            ) {
                Ok(id) => id,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    let new_id = crate::generate_id(tx, "tag")?;
                    tx.execute(
                        "INSERT INTO tags (id, name, color) VALUES (?1, ?2, NULL);",
                        params![new_id, clean_name],
                    )
                    .map_err(|err| format!("Failed inserting tag: {err}"))?;
                    new_id
                }
                Err(err) => return Err(format!("Failed querying tag: {err}")),
            };

            tx.execute(
                "INSERT OR IGNORE INTO node_tags (node_id, tag_id) VALUES (?1, ?2);",
                params![&node_id, &tag_id],
            )
            .map_err(|err| format!("Failed inserting node tag: {err}"))?;
        }
    }

    Ok(())
}

pub fn commit_changeset_transaction(
    conn: &mut Connection,
    input: &ChangesetCommitInput,
    db_path: &Path,
    session_key: Option<redacted::SessionKey>,
) -> Result<bool, String> {
    // 1. Redacted Lock Check
    for item_action in &input.item_actions {
        if item_action.action == "accept" || item_action.action == "edit" {
            let parsed_props: Option<serde_json::Value> =
                if let Some(ref edited) = item_action.edited_data {
                    Some(edited.clone())
                } else {
                    let proposed_data_str: Option<String> = conn
                        .query_row(
                            "SELECT proposed_data FROM changeset_items WHERE id = ?1 LIMIT 1;",
                            [&item_action.item_id],
                            |row| row.get(0),
                        )
                        .ok();
                    proposed_data_str.and_then(|s| serde_json::from_str(&s).ok())
                };

            if let Some(props) = parsed_props {
                let target_vault_id = props
                    .get("vaultId")
                    .or_else(|| props.get("vault_id"))
                    .and_then(|v| v.as_str());

                if let Some(vid) = target_vault_id {
                    let target_tier: String = conn
                        .query_row(
                            "SELECT privacy_tier FROM vaults WHERE id = ?1 LIMIT 1;",
                            [vid],
                            |row| row.get(0),
                        )
                        .unwrap_or_else(|_| "open".to_string());

                    if target_tier == "redacted" && session_key.is_none() {
                        return Err("VAULT_LOCKED".to_string());
                    }
                }
            }
        }
    }

    // 2. Take pre-write database checkpoint
    if !input.item_actions.is_empty() {
        let _ = crate::minimal_pre_write_backup(conn, db_path, "changeset")?;
    }

    // 3. Begin atomic transaction scoping
    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed starting changeset commit transaction: {err}"))?;

    let mut accepted_diff = 0i64;
    let mut dismissed_diff = 0i64;

    for item_action in &input.item_actions {
        let current_status: String = tx
            .query_row(
                "SELECT status FROM changeset_items WHERE id = ?1 LIMIT 1;",
                [&item_action.item_id],
                |row| row.get(0),
            )
            .map_err(|err| {
                format!(
                    "Failed fetching status for changeset item '{}': {err}",
                    item_action.item_id
                )
            })?;

        if current_status != "pending" {
            return Err(format!(
                "Changeset item '{}' is already resolved (status: '{}')",
                item_action.item_id, current_status
            ));
        }

        match item_action.action.as_str() {
            "dismiss" => {
                tx.execute(
                    "UPDATE changeset_items SET status = 'dismissed', reviewed_at = datetime('now') WHERE id = ?1;",
                    [&item_action.item_id],
                )
                .map_err(|err| format!("Failed dismissing changeset item: {err}"))?;
                dismissed_diff += 1;
            }
            "accept" | "edit" => {
                let (item_type, proposed_data, target_node_id, merge_with_id): (
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                ) = tx
                    .query_row(
                        "SELECT item_type, proposed_data, target_node_id, merge_with_id FROM changeset_items WHERE id = ?1 LIMIT 1;",
                        [&item_action.item_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .map_err(|err| format!("Failed fetching changeset item: {err}"))?;

                let parsed_props = if let Some(ref edited) = item_action.edited_data {
                    edited.clone()
                } else {
                    serde_json::from_str(&proposed_data)
                        .map_err(|err| format!("Failed to parse proposed properties: {err}"))?
                };

                let title = parsed_props
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let summary = parsed_props
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let detail = parsed_props.get("detail").and_then(|v| v.as_str());
                let node_type = parsed_props
                    .get("nodeType")
                    .or_else(|| parsed_props.get("node_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("concept");
                let vault_id = parsed_props
                    .get("vaultId")
                    .or_else(|| parsed_props.get("vault_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("vault_root_graph");
                let tags = parsed_props
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|val| val.as_str().map(String::from))
                            .collect::<Vec<String>>()
                    });

                match item_type.as_str() {
                    "add" => {
                        let _new_node_id = insert_changeset_node(
                            &tx,
                            vault_id,
                            title,
                            summary,
                            detail,
                            node_type,
                            tags.as_ref(),
                            session_key.as_ref(),
                        )?;
                    }
                    "update" => {
                        if let Some(ref nid) = target_node_id {
                            update_changeset_node(
                                &tx,
                                nid,
                                vault_id,
                                title,
                                summary,
                                detail,
                                node_type,
                                tags.as_ref(),
                                session_key.as_ref(),
                            )?;
                        }
                    }
                    "merge" => {
                        if let Some(ref mid) = merge_with_id {
                            // 1. Fetch current node details, tags, and encrypted payload
                            let (ex_detail, ex_vault_id, mut ex_title, mut ex_summary, ex_node_type, encrypted_payload): (
                                Option<String>,
                                String,
                                String,
                                String,
                                String,
                                Option<String>,
                            ) = tx
                                .query_row(
                                    "SELECT detail, vault_id, title, summary, node_type, encrypted_payload FROM nodes WHERE id = ?1;",
                                    [mid],
                                    |row| {
                                        Ok((
                                            row.get(0)?,
                                            row.get(1)?,
                                            row.get(2)?,
                                            row.get(3)?,
                                            row.get(4)?,
                                            row.get(5)?,
                                        ))
                                    },
                                )
                                .map_err(|err| format!("Failed fetching node for merge: {err}"))?;

                            let mut decrypted_detail = ex_detail;
                            if let Some(ref enc_val) = encrypted_payload {
                                if !enc_val.trim().is_empty() {
                                    let key =
                                        session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
                                    let payload: redacted::NodeSecretPayload =
                                        redacted::decrypt_json(enc_val, &key)?;
                                    ex_title = payload.title;
                                    ex_summary = payload.summary;
                                    decrypted_detail = payload.detail;
                                }
                            }

                            // 2. Append details
                            let mut merged_detail = decrypted_detail.unwrap_or_default();
                            if let Some(new_det) = detail {
                                if !new_det.trim().is_empty() {
                                    if !merged_detail.is_empty() {
                                        merged_detail.push_str("\n\n");
                                    }
                                    merged_detail.push_str(new_det.trim());
                                }
                            }

                            // 3. Union tags
                            let mut merged_tags = HashSet::new();
                            let mut stmt = tx
                                .prepare("SELECT t.name FROM node_tags nt JOIN tags t ON nt.tag_id = t.id WHERE nt.node_id = ?1;")
                                .map_err(|err| format!("Failed querying current tags: {err}"))?;
                            let rows = stmt
                                .query_map([mid], |row| row.get::<_, String>(0))
                                .map_err(|err| format!("Failed fetching current tags: {err}"))?;
                            for r in rows.flatten() {
                                merged_tags.insert(r);
                            }
                            if let Some(ref new_tags) = tags {
                                for t in new_tags {
                                    merged_tags.insert(t.clone());
                                }
                            }
                            let merged_tags_vec: Vec<String> = merged_tags.into_iter().collect();

                            update_changeset_node(
                                &tx,
                                mid,
                                &ex_vault_id,
                                &ex_title,
                                &ex_summary,
                                if merged_detail.is_empty() {
                                    None
                                } else {
                                    Some(&merged_detail)
                                },
                                &ex_node_type,
                                Some(&merged_tags_vec),
                                session_key.as_ref(),
                            )?;
                        }
                    }
                    "delete" => {
                        if let Some(ref nid) = target_node_id {
                            tx.execute(
                                "UPDATE nodes SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1;",
                                [nid],
                            )
                            .map_err(|err| format!("Failed soft deleting node: {err}"))?;
                        }
                    }
                    "repoint_door" | "orphan_alert" => {
                        let door_id: Option<String> = tx
                            .query_row(
                                "SELECT door_id FROM changeset_items WHERE id = ?1 LIMIT 1;",
                                [&item_action.item_id],
                                |row| row.get(0),
                            )
                            .ok()
                            .flatten();

                        let did = door_id.ok_or_else(|| {
                            format!(
                                "Missing door_id for changeset item '{}' of type '{}'",
                                item_action.item_id, item_type
                            )
                        })?;

                        let nid = target_node_id.as_ref().ok_or_else(|| {
                            format!(
                                "Missing target_node_id for changeset item '{}' of type '{}'",
                                item_action.item_id, item_type
                            )
                        })?;

                        tx.execute(
                            "UPDATE doors SET target_node_id = ?1, status = 'active', updated_at = datetime('now') WHERE id = ?2;",
                            params![nid, did],
                        )
                        .map_err(|err| format!("Failed repointing door: {err}"))?;

                        // Backlink triggers will auto-sync backlinks
                    }
                    _ => {}
                }

                tx.execute(
                    "UPDATE changeset_items SET status = 'accepted', reviewed_at = datetime('now') WHERE id = ?1;",
                    [&item_action.item_id],
                )
                .map_err(|err| format!("Failed accepting changeset item: {err}"))?;
                accepted_diff += 1;
            }
            _ => {}
        }
    }

    // 4. Update parent changeset counts and status
    tx.execute(
        "UPDATE changesets
         SET accepted_count = accepted_count + ?2,
             dismissed_count = dismissed_count + ?3
         WHERE id = ?1;",
        params![input.changeset_id, accepted_diff, dismissed_diff],
    )
    .map_err(|err| format!("Failed updating parent changeset counts: {err}"))?;

    let (item_count, accepted_count, dismissed_count): (i64, i64, i64) = tx
        .query_row(
            "SELECT item_count, accepted_count, dismissed_count FROM changesets WHERE id = ?1 LIMIT 1;",
            [&input.changeset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|err| format!("Failed fetching resolved counts for changeset: {err}"))?;

    let resolved_status = if accepted_count + dismissed_count >= item_count {
        if accepted_count == item_count {
            "accepted"
        } else if dismissed_count == item_count {
            "dismissed"
        } else {
            "partial"
        }
    } else {
        "pending"
    };

    tx.execute(
        "UPDATE changesets
         SET status = ?2,
             reviewed_at = datetime('now')
         WHERE id = ?1;",
        params![input.changeset_id, resolved_status],
    )
    .map_err(|err| format!("Failed final status update on parent changeset: {err}"))?;

    tx.commit()
        .map_err(|err| format!("Failed committing changeset transaction: {err}"))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc_types::{ChangesetCommitInput, ItemReviewAction};
    use crate::redacted;
    use std::error::Error;

    fn setup_test_db() -> Result<Connection, Box<dyn Error>> {
        let conn = Connection::open_in_memory()?;
        let ddl = "
            CREATE TABLE IF NOT EXISTS vaults (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                privacy_tier TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sub_vaults (
                id TEXT PRIMARY KEY,
                vault_id TEXT REFERENCES vaults(id),
                privacy_tier TEXT
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                vault_id TEXT REFERENCES vaults(id)
            );
            CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                vault_id TEXT REFERENCES vaults(id),
                sub_vault_id TEXT,
                node_type TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                detail TEXT,
                source TEXT,
                source_type TEXT,
                privacy_tier TEXT,
                priority TEXT,
                version INTEGER NOT NULL DEFAULT 1,
                deleted_at TEXT,
                updated_at TEXT,
                meta TEXT DEFAULT '{}',
                encrypted_payload TEXT
            );
            CREATE TABLE IF NOT EXISTS changesets (
                id TEXT PRIMARY KEY,
                session_id TEXT REFERENCES sessions(id),
                status TEXT NOT NULL DEFAULT 'pending',
                item_count INTEGER NOT NULL DEFAULT 0,
                accepted_count INTEGER NOT NULL DEFAULT 0,
                dismissed_count INTEGER NOT NULL DEFAULT 0,
                model_used TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                reviewed_at TEXT
            );
            CREATE TABLE IF NOT EXISTS changeset_items (
                id TEXT PRIMARY KEY,
                changeset_id TEXT NOT NULL REFERENCES changesets(id),
                item_type TEXT NOT NULL,
                target_node_id TEXT,
                proposed_data TEXT NOT NULL DEFAULT '{}',
                existing_data TEXT DEFAULT '{}',
                similarity REAL,
                merge_with_id TEXT,
                door_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                reviewed_at TEXT,
                sort_order INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS node_tags (
                node_id TEXT REFERENCES nodes(id),
                tag_id TEXT REFERENCES tags(id),
                PRIMARY KEY (node_id, tag_id)
            );
            CREATE TABLE IF NOT EXISTS tags (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT
            );
        ";
        conn.execute_batch(ddl)?;
        Ok(conn)
    }

    #[test]
    fn test_commit_merge_redacted_node() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed redacted vault and session
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_credentials', 'Credentials', 'redacted');",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, vault_id) VALUES ('session_redacted', 'vault_credentials');",
            [],
        )?;

        // Encrypt seed payload
        let key = [0_u8; 32];
        let secret_payload = redacted::NodeSecretPayload {
            title: "Super Secret Title".to_string(),
            summary: "Super Secret Summary".to_string(),
            detail: Some("Super Secret Detail".to_string()),
            source: Some("agent_extract".to_string()),
            source_type: Some("agent_extract".to_string()),
        };
        let encrypted_payload = redacted::encrypt_json(&secret_payload, &key)?;

        // Insert node with placeholder values in cleartext and real values in encrypted_payload
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, encrypted_payload)
             VALUES ('node_secret', 'vault_credentials', 'concept', '[REDACTED]', '[Metadata Locked]', NULL, ?1);",
            [encrypted_payload],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_redacted', 'session_redacted', 'pending', 1);",
            [],
        )?;

        // Seed merge item
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, merge_with_id, proposed_data, status)
             VALUES ('item_merge', 'cs_redacted', 'merge', 'node_secret',
                     '{\"title\":\"Ignored Proposed Title\",\"summary\":\"Ignored Proposed Summary\",\"detail\":\"Additional Secret Info\",\"vaultId\":\"vault_credentials\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_redacted".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_merge".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Commit merge transaction
        let ok = commit_changeset_transaction(&mut conn, &input, db_path, Some(key))?;
        assert!(ok);

        // Retrieve node back
        let (title, summary, detail, encrypted_payload): (
            String,
            String,
            Option<String>,
            Option<String>,
        ) = conn.query_row(
            "SELECT title, summary, detail, encrypted_payload FROM nodes WHERE id = 'node_secret';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        // Verify cleartext placeholders are still present
        assert_eq!(title, "[REDACTED]");
        assert_eq!(summary, "[Metadata Locked]");
        assert!(detail.is_none());

        // Decrypt the newly merged payload
        let enc_str = encrypted_payload.ok_or("encrypted_payload is missing")?;
        let decrypted: redacted::NodeSecretPayload = redacted::decrypt_json(&enc_str, &key)?;

        // Assert title and summary are preserved, and detail is successfully appended!
        assert_eq!(decrypted.title, "Super Secret Title");
        assert_eq!(decrypted.summary, "Super Secret Summary");
        assert_eq!(
            decrypted.detail,
            Some("Super Secret Detail\n\nAdditional Secret Info".to_string())
        );
        Ok(())
    }

    #[test]
    fn test_commit_merge_redacted_node_locked_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed redacted vault and session
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_credentials', 'Credentials', 'redacted');",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, vault_id) VALUES ('session_redacted', 'vault_credentials');",
            [],
        )?;

        // Insert node with placeholder values in cleartext
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, encrypted_payload)
             VALUES ('node_secret', 'vault_credentials', 'concept', '[REDACTED]', '[Metadata Locked]', NULL, 'some-payload');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_redacted', 'session_redacted', 'pending', 1);",
            [],
        )?;

        // Seed merge item
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, merge_with_id, proposed_data, status)
             VALUES ('item_merge', 'cs_redacted', 'merge', 'node_secret',
                     '{\"detail\":\"Additional Info\",\"vaultId\":\"vault_credentials\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_redacted".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_merge".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Try to commit without a session key - should fail with VAULT_LOCKED!
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        match result {
            Err(err) => assert_eq!(err, "VAULT_LOCKED"),
            Ok(_) => panic!("Expected error VAULT_LOCKED, but got Ok"),
        }
        Ok(())
    }

    #[test]
    fn test_commit_edit_to_redacted_vault_locked_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vaults
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_credentials', 'Credentials', 'redacted');",
            [],
        )?;

        // Seed changeset and changeset item (originally targeting open vault)
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_edit', NULL, 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_edit', 'cs_edit', 'add', '{\"title\":\"Add Item\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        // Action is 'edit' and redirects to redacted vault
        let input = ChangesetCommitInput {
            changeset_id: "cs_edit".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_edit".to_string(),
                action: "edit".to_string(),
                edited_data: Some(serde_json::json!({
                    "title": "Add Item",
                    "vaultId": "vault_credentials"
                })),
            }],
        };

        // Try to commit without session key - should fail with VAULT_LOCKED because of the edit!
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        match result {
            Err(err) => assert_eq!(err, "VAULT_LOCKED"),
            Ok(_) => panic!("Expected error VAULT_LOCKED, but got Ok"),
        }
        Ok(())
    }

    #[test]
    fn test_commit_edit_away_from_redacted_vault_succeeds() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vaults
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_credentials', 'Credentials', 'redacted');",
            [],
        )?;

        // Seed changeset and changeset item (originally targeting redacted vault)
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_edit', NULL, 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_edit', 'cs_edit', 'add', '{\"title\":\"Add Item\",\"vaultId\":\"vault_credentials\"}', 'pending');",
            [],
        )?;

        // Action is 'edit' and redirects to open vault
        let input = ChangesetCommitInput {
            changeset_id: "cs_edit".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_edit".to_string(),
                action: "edit".to_string(),
                edited_data: Some(serde_json::json!({
                    "title": "Add Item",
                    "vaultId": "vault_open"
                })),
            }],
        };

        // Try to commit without session key - should succeed because it was edited away from the redacted vault!
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        assert!(
            result.is_ok(),
            "Expected Ok, but got Err: {:?}",
            result.err()
        );

        // Verify the node is actually created in the open vault
        let (vault_id, title): (String, String) = conn.query_row(
            "SELECT vault_id, title FROM nodes WHERE title = 'Add Item' LIMIT 1;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(vault_id, "vault_open");
        assert_eq!(title, "Add Item");

        Ok(())
    }

    #[test]
    fn test_commit_item_already_resolved_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vaults and changeset
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_resolved', NULL, 'pending', 1);",
            [],
        )?;

        // Seed item with status 'accepted' (already resolved)
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_resolved', 'cs_resolved', 'add', '{\"title\":\"Should Fail\",\"vaultId\":\"vault_open\"}', 'accepted');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_resolved".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_resolved".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Try to commit - should fail because status is 'accepted'
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        match result {
            Err(err) => assert!(err.contains("already resolved")),
            Ok(_) => panic!("Expected error due to already resolved item, but got Ok"),
        }

        // Verify the node was NOT created (transaction rolled back or aborted)
        let count: i64 = conn.query_row(
            "SELECT COUNT(1) FROM nodes WHERE title = 'Should Fail';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);

        Ok(())
    }

    #[test]
    fn test_commit_accept_with_edited_data_succeeds() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vaults and changeset
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_accept_edit', NULL, 'pending', 1);",
            [],
        )?;

        // Seed item with original proposed data
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_accept_edit', 'cs_accept_edit', 'add', '{\"title\":\"Original Title\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        // Action is 'accept' but edited_data is populated with different values
        let input = ChangesetCommitInput {
            changeset_id: "cs_accept_edit".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_accept_edit".to_string(),
                action: "accept".to_string(),
                edited_data: Some(serde_json::json!({
                    "title": "Edited Title",
                    "vaultId": "vault_open"
                })),
            }],
        };

        // Try to commit - should succeed and use the edited title
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None)?;
        assert!(result);

        // Verify the node is actually created with 'Edited Title', NOT 'Original Title'
        let title: String =
            conn.query_row("SELECT title FROM nodes LIMIT 1;", [], |row| row.get(0))?;
        assert_eq!(title, "Edited Title");

        Ok(())
    }

    #[test]
    fn test_commit_node_to_subvault_resolves_correct_parent() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed parent vault
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_parent', 'Parent Vault', 'open');",
            [],
        )?;

        // Seed sub-vault mapping
        conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, privacy_tier) VALUES ('vault_sub', 'vault_parent', 'open');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, session_id, status, item_count) VALUES ('cs_subvault', NULL, 'pending', 1);",
            [],
        )?;

        // Seed changeset item targeting sub-vault
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_subvault', 'cs_subvault', 'add', '{\"title\":\"Subvault Item\",\"vaultId\":\"vault_sub\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_subvault".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_subvault".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Commit transaction
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None)?;
        assert!(result);

        // Verify the node was created with:
        // vault_id = 'vault_parent' (parent vault)
        // sub_vault_id = 'vault_sub' (sub vault)
        let (vault_id, sub_vault_id): (String, Option<String>) = conn.query_row(
            "SELECT vault_id, sub_vault_id FROM nodes WHERE title = 'Subvault Item' LIMIT 1;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        assert_eq!(vault_id, "vault_parent");
        assert_eq!(sub_vault_id, Some("vault_sub".to_string()));

        Ok(())
    }
}
