use crate::ipc_types::ChangesetCommitInput;
use crate::priority;
use crate::redacted;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
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

fn insert_changeset_node(
    tx: &Transaction,
    vault_id: &str,
    proposed: &crate::memory_agent::changeset::ProposedNodeData,
    session_key: Option<&redacted::SessionKey>,
) -> Result<String, String> {
    let parent_vault_id: Option<String> = tx
        .query_row(
            "SELECT vault_id FROM sub_vaults WHERE id = ?1 LIMIT 1;",
            [vault_id],
            |row| row.get(0),
        )
        .ok();

    let (resolved_vault_id, resolved_sub_vault_id, sub_vault_privacy) = match parent_vault_id {
        Some(parent_id) => {
            let sv_privacy: Option<String> = tx
                .query_row(
                    "SELECT privacy_tier FROM sub_vaults WHERE id = ?1 LIMIT 1;",
                    [vault_id],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            (parent_id, Some(vault_id.to_string()), sv_privacy)
        }
        None => (vault_id.to_string(), None, None),
    };

    let effective_privacy =
        resolve_effective_privacy(tx, &resolved_vault_id, sub_vault_privacy.as_deref())?;
    let is_redacted = effective_privacy == "redacted";

    let source = proposed.source.as_deref().unwrap_or("agent_extract");
    let source_type = proposed.source_type.as_deref().unwrap_or("agent_extract");
    let meta_str = proposed
        .meta
        .as_ref()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "{}".to_string());

    let encrypted_payload = if is_redacted {
        let key = session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
        Some(redacted::encrypt_json(
            &redacted::NodeSecretPayload {
                title: proposed.title.to_string(),
                summary: proposed.summary.to_string(),
                detail: proposed.detail.clone(),
                source: Some(source.to_string()),
                source_type: Some(source_type.to_string()),
            },
            key,
        )?)
    } else {
        None
    };

    let stored_title = if is_redacted {
        "[REDACTED]".to_string()
    } else {
        proposed.title.to_string()
    };

    let stored_summary = if is_redacted {
        "[Metadata Locked]".to_string()
    } else {
        proposed.summary.to_string()
    };

    let node_id = crate::generate_id(tx, "node")?;
    let priority_json = priority::DEFAULT_PRIORITY_JSON;

    tx.execute(
        "INSERT INTO nodes (
            id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
            privacy_tier, priority, meta, encrypted_payload
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12);",
        params![
            node_id,
            resolved_vault_id,
            resolved_sub_vault_id,
            proposed.node_type.as_deref().unwrap_or("concept"),
            stored_title,
            stored_summary,
            if is_redacted {
                None
            } else {
                proposed.detail.as_deref()
            },
            source,
            source_type,
            priority_json,
            meta_str,
            encrypted_payload
        ],
    )
    .map_err(|err| format!("Failed to insert changeset node: {err}"))?;

    if let Some(tag_list) = &proposed.tags {
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

fn source_stem(source_name: &str) -> &str {
    Path::new(source_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(source_name)
}

fn strip_chunk_index_suffix(title: &str) -> String {
    if let Some(idx) = title.rfind(" (") {
        let suffix = &title[idx + 2..];
        if suffix.ends_with(')')
            && suffix
                .trim_end_matches(')')
                .chars()
                .all(|c| c.is_ascii_digit() || c == '/')
        {
            return title[..idx].to_string();
        }
    }
    title.to_string()
}

fn meta_chunk_index(meta: &serde_json::Value) -> Option<u64> {
    meta.get("chunk_index")
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)))
}

struct ImportChunkRef {
    id: String,
    title: String,
    summary: String,
    source: Option<String>,
    vault_id: String,
    sub_vault_id: Option<String>,
    meta: serde_json::Value,
    chunk_index: u64,
}

fn insert_door(
    tx: &Transaction,
    source_node_id: &str,
    target_node_id: &str,
    label: &str,
) -> Result<(), String> {
    let id = crate::generate_id(tx, "door")?;
    tx.execute(
        "INSERT INTO doors (id, source_node_id, target_node_id, target_vault_id, label)
         VALUES (?1, ?2, ?3, NULL, ?4);",
        params![id, source_node_id, target_node_id, label],
    )
    .map_err(|err| format!("Failed inserting import spine door: {err}"))?;
    Ok(())
}

struct ImportJobSpineRow {
    source_name: String,
    assembled_markdown: String,
    avg_ocr_confidence: f64,
    tables_detected_unpreserved: i64,
    extraction_path: Option<String>,
}

/// After an import changeset is fully reviewed, create a parent document node and wire
/// `section` (parent→chunk) + `next` (chunk→chunk) doors.
fn create_import_document_spine(
    tx: &Transaction,
    changeset_id: &str,
    session_key: Option<&redacted::SessionKey>,
) -> Result<(), String> {
    let import_jobs_exist = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'import_jobs' LIMIT 1;",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !import_jobs_exist {
        return Ok(());
    }

    let job: Option<ImportJobSpineRow> = tx
        .query_row(
            "SELECT COALESCE(source_name, ''), COALESCE(assembled_markdown, ''),
                    COALESCE(avg_ocr_confidence, 0.0), COALESCE(tables_detected_unpreserved, 0),
                    extraction_path
             FROM import_jobs
             WHERE changeset_id = ?1
             LIMIT 1;",
            [changeset_id],
            |row| {
                Ok(ImportJobSpineRow {
                    source_name: row.get(0)?,
                    assembled_markdown: row.get(1)?,
                    avg_ocr_confidence: row.get(2)?,
                    tables_detected_unpreserved: row.get(3)?,
                    extraction_path: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("Failed loading import job for spine: {err}"))?;

    let Some(ImportJobSpineRow {
        source_name,
        assembled_markdown,
        avg_ocr_confidence,
        tables_detected_unpreserved,
        extraction_path,
    }) = job
    else {
        return Ok(());
    };
    if assembled_markdown.trim().is_empty() {
        return Ok(());
    }

    let mut stmt = tx
        .prepare(
            "SELECT n.id, n.title, n.summary, n.source, n.vault_id, n.sub_vault_id, COALESCE(n.meta, '{}')
             FROM changeset_items ci
             INNER JOIN nodes n ON n.id = ci.target_node_id
             WHERE ci.changeset_id = ?1
               AND ci.status = 'accepted'
               AND ci.item_type = 'add'
               AND n.deleted_at IS NULL
               AND n.source_type = 'pdf_import';",
        )
        .map_err(|err| format!("Failed preparing import chunk query: {err}"))?;

    let rows = stmt
        .query_map([changeset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|err| format!("Failed querying accepted import chunks: {err}"))?;

    let mut chunks: Vec<ImportChunkRef> = Vec::new();
    for row in rows {
        let (id, title, summary, source, vault_id, sub_vault_id, meta_str) =
            row.map_err(|err| format!("Failed reading import chunk row: {err}"))?;
        let meta: serde_json::Value =
            serde_json::from_str(&meta_str).unwrap_or(serde_json::json!({}));
        if meta.get("import_role").and_then(|v| v.as_str()) == Some("document") {
            continue;
        }
        let Some(chunk_index) = meta_chunk_index(&meta) else {
            continue;
        };
        chunks.push(ImportChunkRef {
            id,
            title,
            summary,
            source,
            vault_id,
            sub_vault_id,
            meta,
            chunk_index,
        });
    }
    drop(stmt);

    if chunks.is_empty() {
        return Ok(());
    }

    chunks.sort_by_key(|c| c.chunk_index);
    let chunk_total = chunks.len();
    let write_vault = chunks[0]
        .sub_vault_id
        .clone()
        .unwrap_or_else(|| chunks[0].vault_id.clone());
    let source = chunks[0]
        .source
        .clone()
        .unwrap_or_else(|| source_name.clone());
    let parent_title = {
        let stripped = strip_chunk_index_suffix(&chunks[0].title);
        if stripped.trim().is_empty() {
            format!("{} · Document", source_stem(&source_name))
        } else {
            stripped
        }
    };
    let parent_summary = if chunks[0].summary.chars().count() > 120 {
        chunks[0].summary.chars().take(117).collect::<String>() + "..."
    } else if chunks[0].summary.trim().is_empty() {
        source_name.clone()
    } else {
        chunks[0].summary.clone()
    };

    let mut parent_meta = serde_json::json!({
        "import_role": "document",
        "chunk_total": chunk_total,
        "avg_ocr_confidence": avg_ocr_confidence as f32,
        "tables_unstructured": tables_detected_unpreserved > 0,
    });
    if let Some(path) = extraction_path.filter(|p| !p.trim().is_empty()) {
        if let Some(map) = parent_meta.as_object_mut() {
            map.insert("extraction_path".to_string(), serde_json::json!(path));
        }
    }
    let parent_proposed = crate::memory_agent::changeset::ProposedNodeData {
        title: parent_title,
        summary: parent_summary,
        detail: Some(assembled_markdown),
        node_type: Some("summary".to_string()),
        target_vault_key: None,
        vault_id: Some(write_vault.clone()),
        tags: Some(vec!["pdf_import".to_string()]),
        confidence: 1.0,
        action: crate::memory_agent::parser::CandidateAction::Add,
        substantial_change: None,
        source: Some(source),
        source_type: Some("pdf_import".to_string()),
        meta: Some(parent_meta),
    };
    let parent_id = insert_changeset_node(tx, &write_vault, &parent_proposed, session_key)?;

    for chunk in &chunks {
        let mut updated = chunk.meta.clone();
        if !updated.is_object() {
            updated = serde_json::json!({});
        }
        if let Some(map) = updated.as_object_mut() {
            map.insert("import_role".to_string(), serde_json::json!("chunk"));
            map.insert("document_id".to_string(), serde_json::json!(parent_id));
            map.insert("chunk_total".to_string(), serde_json::json!(chunk_total));
            map.insert(
                "chunk_index".to_string(),
                serde_json::json!(chunk.chunk_index),
            );
        }
        let meta_str = updated.to_string();
        tx.execute(
            "UPDATE nodes SET meta = ?2, updated_at = datetime('now') WHERE id = ?1;",
            params![chunk.id, meta_str],
        )
        .map_err(|err| format!("Failed stamping chunk document_id on {}: {err}", chunk.id))?;

        insert_door(tx, &parent_id, &chunk.id, "section")?;
    }

    for window in chunks.windows(2) {
        insert_door(tx, &window[0].id, &window[1].id, "next")?;
    }

    Ok(())
}

fn update_changeset_node(
    tx: &Transaction,
    node_id: &str,
    vault_id: &str,
    proposed: &crate::memory_agent::changeset::ProposedNodeData,
    session_key: Option<&redacted::SessionKey>,
) -> Result<(), String> {
    let parent_vault_id: Option<String> = tx
        .query_row(
            "SELECT vault_id FROM sub_vaults WHERE id = ?1 LIMIT 1;",
            [vault_id],
            |row| row.get(0),
        )
        .ok();

    let (resolved_vault_id, resolved_sub_vault_id, sub_vault_privacy) = match parent_vault_id {
        Some(parent_id) => {
            let sv_privacy: Option<String> = tx
                .query_row(
                    "SELECT privacy_tier FROM sub_vaults WHERE id = ?1 LIMIT 1;",
                    [vault_id],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            (parent_id, Some(vault_id.to_string()), sv_privacy)
        }
        None => (vault_id.to_string(), None, None),
    };

    let effective_privacy =
        resolve_effective_privacy(tx, &resolved_vault_id, sub_vault_privacy.as_deref())?;
    let is_redacted = effective_privacy == "redacted";

    let current_is_encrypted = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM nodes
                WHERE id = ?1 AND deleted_at IS NULL AND encrypted_payload IS NOT NULL AND encrypted_payload != ''
            );",
            [node_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("Failed checking redacted state for node {}: {err}", node_id))? > 0;

    if current_is_encrypted && !is_redacted && session_key.is_none() {
        return Err(
            "Unlock redacted content with your master password before changing the node to a non-redacted tier."
                .to_string(),
        );
    }

    let (existing_source, existing_source_type): (Option<String>, Option<String>) = tx
        .query_row(
            "SELECT source, source_type FROM nodes WHERE id = ?1 LIMIT 1;",
            [node_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|err| format!("Failed fetching node source fields: {err}"))?;
    let source = proposed
        .source
        .as_deref()
        .or(existing_source.as_deref())
        .unwrap_or("agent_extract");
    let source_type = proposed
        .source_type
        .as_deref()
        .or(existing_source_type.as_deref())
        .unwrap_or("agent_extract");
    let meta_str = match &proposed.meta {
        Some(meta) => meta.to_string(),
        None => tx
            .query_row(
                "SELECT COALESCE(meta, '{}') FROM nodes WHERE id = ?1 LIMIT 1;",
                [node_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("Failed fetching node meta: {err}"))?,
    };

    let encrypted_payload = if is_redacted {
        let key = session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
        Some(redacted::encrypt_json(
            &redacted::NodeSecretPayload {
                title: proposed.title.to_string(),
                summary: proposed.summary.to_string(),
                detail: proposed.detail.clone(),
                source: Some(source.to_string()),
                source_type: Some(source_type.to_string()),
            },
            key,
        )?)
    } else {
        None
    };

    let stored_title = if is_redacted {
        "[REDACTED]".to_string()
    } else {
        proposed.title.to_string()
    };

    let stored_summary = if is_redacted {
        "[Metadata Locked]".to_string()
    } else {
        proposed.summary.to_string()
    };

    let rows_affected = tx
        .execute(
            "UPDATE nodes
         SET vault_id = ?2,
             sub_vault_id = ?3,
             node_type = ?4,
             title = ?5,
             summary = ?6,
             detail = ?7,
             source = ?8,
             source_type = ?9,
             version = version + 1,
             updated_at = datetime('now'),
             meta = ?10,
             encrypted_payload = ?11
         WHERE id = ?1 AND deleted_at IS NULL;",
            params![
                node_id,
                resolved_vault_id,
                resolved_sub_vault_id,
                proposed.node_type.as_deref().unwrap_or("concept"),
                stored_title,
                stored_summary,
                if is_redacted {
                    None
                } else {
                    proposed.detail.as_deref()
                },
                source,
                source_type,
                meta_str,
                encrypted_payload
            ],
        )
        .map_err(|err| format!("Failed updating node: {err}"))?;

    if rows_affected == 0 {
        return Err(format!(
            "Node '{}' not found or already deleted (0 rows updated)",
            node_id
        ));
    }

    tx.execute("DELETE FROM node_tags WHERE node_id = ?1;", [node_id])
        .map_err(|err| format!("Failed clearing node tags: {err}"))?;

    if let Some(tag_list) = &proposed.tags {
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
            let parsed_props: Option<serde_json::Value> = if let Some(ref edited) =
                item_action.edited_data
            {
                Some(edited.clone())
            } else {
                let proposed_data_str: Option<String> = conn
                        .query_row(
                            "SELECT proposed_data FROM changeset_items WHERE id = ?1 AND changeset_id = ?2 AND status = 'pending' LIMIT 1;",
                            params![&item_action.item_id, &input.changeset_id],
                            |row| row.get(0),
                        )
                        .ok();
                proposed_data_str.and_then(|s| serde_json::from_str(&s).ok())
            };

            if let Some(props) = parsed_props {
                let mut target_vault_id = props
                    .get("vaultId")
                    .or_else(|| props.get("vault_id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let mut current_is_encrypted = false;

                let item_info: Option<(String, Option<String>)> = conn
                    .query_row(
                        "SELECT item_type, target_node_id FROM changeset_items WHERE id = ?1 AND changeset_id = ?2 AND status = 'pending' LIMIT 1;",
                        params![&item_action.item_id, &input.changeset_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok();

                if let Some((ref item_type, Some(ref node_id))) = item_info {
                    if item_type == "update" {
                        current_is_encrypted = conn
                            .query_row(
                                "SELECT EXISTS(
                                    SELECT 1
                                    FROM nodes
                                    WHERE id = ?1 AND deleted_at IS NULL AND encrypted_payload IS NOT NULL AND encrypted_payload != ''
                                );",
                                [node_id],
                                |row| row.get::<_, i64>(0),
                            )
                            .unwrap_or(0) > 0;

                        if target_vault_id.is_none() {
                            let current_vault: Option<String> = conn
                                .query_row(
                                    "SELECT COALESCE(sub_vault_id, vault_id) FROM nodes WHERE id = ?1 AND deleted_at IS NULL;",
                                    [node_id],
                                    |row| row.get(0),
                                )
                                .ok();
                            target_vault_id = current_vault;
                        }
                    }
                }

                if let Some(ref vid) = target_vault_id {
                    let target_tier: String = conn
                        .query_row(
                            "SELECT COALESCE(
                                (SELECT COALESCE(sv.privacy_tier, v.privacy_tier)
                                 FROM sub_vaults sv
                                 JOIN vaults v ON sv.vault_id = v.id
                                 WHERE sv.id = ?1),
                                (SELECT privacy_tier FROM vaults WHERE id = ?1)
                             );",
                            [vid],
                            |row| row.get(0),
                        )
                        .unwrap_or_else(|_| "open".to_string());

                    let is_target_redacted = target_tier == "redacted";
                    if is_target_redacted && session_key.is_none() {
                        return Err("VAULT_LOCKED".to_string());
                    }
                    if !is_target_redacted && current_is_encrypted && session_key.is_none() {
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

    let mut tag_stmt = tx
        .prepare("SELECT t.name FROM node_tags nt JOIN tags t ON nt.tag_id = t.id WHERE nt.node_id = ?1;")
        .map_err(|err| format!("Failed preparing tag fetch statement: {err}"))?;

    let mut accepted_diff = 0i64;
    let mut dismissed_diff = 0i64;

    for item_action in &input.item_actions {
        let (current_status, item_changeset_id): (String, String) = tx
            .query_row(
                "SELECT status, changeset_id FROM changeset_items WHERE id = ?1 LIMIT 1;",
                [&item_action.item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|err| {
                format!(
                    "Failed fetching status for changeset item '{}': {err}",
                    item_action.item_id
                )
            })?;

        if item_changeset_id != input.changeset_id {
            return Err(format!(
                "Changeset item '{}' does not belong to changeset '{}' (belongs to '{}')",
                item_action.item_id, input.changeset_id, item_changeset_id
            ));
        }

        if current_status != "pending" {
            return Err(format!(
                "Changeset item '{}' is already resolved (status: '{}')",
                item_action.item_id, current_status
            ));
        }

        match item_action.action.as_str() {
            "dismiss" => {
                let rows = tx.execute(
                    "UPDATE changeset_items SET status = 'dismissed', reviewed_at = datetime('now') WHERE id = ?1 AND changeset_id = ?2 AND status = 'pending';",
                    params![&item_action.item_id, &input.changeset_id],
                )
                .map_err(|err| format!("Failed dismissing changeset item: {err}"))?;
                if rows == 0 {
                    return Err(format!(
                        "Failed to dismiss changeset item '{}' (no rows affected)",
                        item_action.item_id
                    ));
                }
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
                        "SELECT item_type, proposed_data, target_node_id, merge_with_id FROM changeset_items WHERE id = ?1 AND changeset_id = ?2 AND status = 'pending' LIMIT 1;",
                        params![&item_action.item_id, &input.changeset_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .map_err(|err| format!("Failed fetching changeset item: {err}"))?;

                let parsed_props = if let Some(ref edited) = item_action.edited_data {
                    edited.clone()
                } else {
                    serde_json::from_str(&proposed_data)
                        .map_err(|err| format!("Failed to parse proposed properties: {err}"))?
                };

                let mut current_vault_id = None;
                let mut current_title = None;
                let mut current_summary = None;
                let mut current_detail = None;
                let mut current_node_type = None;
                let mut current_tags = None;

                if item_type == "update" {
                    if let Some(ref nid) = target_node_id {
                        // Load current node
                        let (v_id, sub_v_id, t, s, d, n_type, enc_payload): (
                            String,
                            Option<String>,
                            String,
                            String,
                            Option<String>,
                            String,
                            Option<String>,
                        ) = tx
                            .query_row(
                                "SELECT vault_id, sub_vault_id, title, summary, detail, node_type, encrypted_payload FROM nodes WHERE id = ?1 AND deleted_at IS NULL;",
                                [nid],
                                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
                            )
                            .map_err(|err| match err {
                                rusqlite::Error::QueryReturnedNoRows => {
                                    format!("Node '{}' not found or already deleted", nid)
                                }
                                _ => format!("Failed fetching node for update fallback: {err}"),
                            })?;

                        // If redacted, decrypt to get the original values
                        let (mut resolved_title, mut resolved_summary, mut resolved_detail) =
                            (t, s, d);
                        if let Some(ref enc_val) = enc_payload {
                            if !enc_val.trim().is_empty() {
                                let key = session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
                                let payload: redacted::NodeSecretPayload =
                                    redacted::decrypt_json(enc_val, &key)?;
                                resolved_title = payload.title;
                                resolved_summary = payload.summary;
                                resolved_detail = payload.detail;
                            }
                        }

                        current_vault_id = Some(sub_v_id.unwrap_or(v_id));
                        current_title = Some(resolved_title);
                        current_summary = Some(resolved_summary);
                        current_detail = resolved_detail;
                        current_node_type = Some(n_type);

                        // Load current tags
                        let tag_rows = tag_stmt
                            .query_map([nid], |row| row.get::<_, String>(0))
                            .map_err(|err| format!("Failed executing tag fetch: {err}"))?;
                        let mut tags_vec = vec![];
                        for tag_res in tag_rows {
                            tags_vec
                                .push(tag_res.map_err(|err| format!("Failed reading tag: {err}"))?);
                        }
                        current_tags = Some(tags_vec);
                    }
                }

                let title_str = if let Some(val) = parsed_props.get("title") {
                    val.as_str().unwrap_or("").to_string()
                } else {
                    current_title.unwrap_or_default()
                };
                let title = title_str.as_str();

                let summary_str = if let Some(val) = parsed_props.get("summary") {
                    val.as_str().unwrap_or("").to_string()
                } else {
                    current_summary.unwrap_or_default()
                };
                let summary = summary_str.as_str();

                let detail_str = if parsed_props.get("detail").is_some() {
                    parsed_props
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    current_detail
                };
                let detail = detail_str.as_deref();

                let node_type_str = if let Some(val) = parsed_props
                    .get("nodeType")
                    .or_else(|| parsed_props.get("node_type"))
                {
                    val.as_str().unwrap_or("concept").to_string()
                } else {
                    current_node_type.unwrap_or_else(|| "concept".to_string())
                };
                let node_type = node_type_str.as_str();

                let vault_id_str = if let Some(val) = parsed_props
                    .get("vaultId")
                    .or_else(|| parsed_props.get("vault_id"))
                {
                    val.as_str().unwrap_or("vault_root_graph").to_string()
                } else {
                    current_vault_id.unwrap_or_else(|| "vault_root_graph".to_string())
                };
                let vault_id = vault_id_str.as_str();

                let tags = if parsed_props.get("tags").is_some() {
                    parsed_props
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|val| val.as_str().map(String::from))
                                .collect::<Vec<String>>()
                        })
                } else {
                    current_tags
                };

                let source = parsed_props
                    .get("source")
                    .and_then(|v| v.as_str().map(String::from));
                let source_type = parsed_props
                    .get("sourceType")
                    .or_else(|| parsed_props.get("source_type"))
                    .and_then(|v| v.as_str().map(String::from));
                let meta = parsed_props.get("meta").cloned();

                match item_type.as_str() {
                    "add" => {
                        let proposed = crate::memory_agent::changeset::ProposedNodeData {
                            title: title.to_string(),
                            summary: summary.to_string(),
                            detail: detail.map(String::from),
                            node_type: Some(node_type.to_string()),
                            target_vault_key: None,
                            vault_id: Some(vault_id.to_string()),
                            tags: tags.clone(),
                            confidence: 1.0,
                            action: crate::memory_agent::parser::CandidateAction::Add,
                            substantial_change: None,
                            source,
                            source_type,
                            meta,
                        };
                        let new_node_id =
                            insert_changeset_node(&tx, vault_id, &proposed, session_key.as_ref())?;
                        // Persist created node id so a later resolve can build the import spine
                        // even when Accept is split across multiple commit calls.
                        tx.execute(
                            "UPDATE changeset_items
                             SET target_node_id = ?1
                             WHERE id = ?2 AND changeset_id = ?3;",
                            params![new_node_id, &item_action.item_id, &input.changeset_id],
                        )
                        .map_err(|err| {
                            format!("Failed linking accepted add to new node id: {err}")
                        })?;
                    }
                    "update" => {
                        let nid = target_node_id.as_ref().ok_or_else(|| {
                            format!(
                                "Missing target_node_id for changeset item '{}' of type 'update'",
                                item_action.item_id
                            )
                        })?;
                        let proposed = crate::memory_agent::changeset::ProposedNodeData {
                            title: title.to_string(),
                            summary: summary.to_string(),
                            detail: detail.map(String::from),
                            node_type: Some(node_type.to_string()),
                            target_vault_key: None,
                            vault_id: Some(vault_id.to_string()),
                            tags: tags.clone(),
                            confidence: 1.0,
                            action: crate::memory_agent::parser::CandidateAction::Update,
                            substantial_change: None,
                            source,
                            source_type,
                            meta,
                        };
                        update_changeset_node(&tx, nid, vault_id, &proposed, session_key.as_ref())?;
                    }
                    "merge" => {
                        let mid = merge_with_id.as_ref().ok_or_else(|| {
                            format!(
                                "Missing merge_with_id for changeset item '{}' of type 'merge'",
                                item_action.item_id
                            )
                        })?;
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
                                "SELECT detail, vault_id, title, summary, node_type, encrypted_payload FROM nodes WHERE id = ?1 AND deleted_at IS NULL;",
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
                                let key = session_key.ok_or_else(|| "VAULT_LOCKED".to_string())?;
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
                        let rows = tag_stmt
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

                        let proposed = crate::memory_agent::changeset::ProposedNodeData {
                            title: ex_title.clone(),
                            summary: ex_summary.clone(),
                            detail: if merged_detail.is_empty() {
                                None
                            } else {
                                Some(merged_detail.clone())
                            },
                            node_type: Some(ex_node_type.clone()),
                            target_vault_key: None,
                            vault_id: Some(ex_vault_id.clone()),
                            tags: Some(merged_tags_vec.clone()),
                            confidence: 1.0,
                            action: crate::memory_agent::parser::CandidateAction::Update,
                            substantial_change: None,
                            source: None,
                            source_type: None,
                            meta: None,
                        };
                        update_changeset_node(
                            &tx,
                            mid,
                            &ex_vault_id,
                            &proposed,
                            session_key.as_ref(),
                        )?;
                    }
                    "delete" => {
                        let nid = target_node_id.as_ref().ok_or_else(|| {
                            format!(
                                "Missing target_node_id for changeset item '{}' of type 'delete'",
                                item_action.item_id
                            )
                        })?;
                        let rows_affected = tx.execute(
                            "UPDATE nodes SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL;",
                            [nid],
                        )
                        .map_err(|err| format!("Failed soft deleting node: {err}"))?;
                        if rows_affected == 0 {
                            return Err(format!(
                                "Node '{}' not found or already deleted (0 rows affected during delete)",
                                nid
                            ));
                        }
                    }
                    "repoint_door" | "orphan_alert" => {
                        let door_id: Option<String> = tx
                            .query_row(
                                "SELECT door_id FROM changeset_items WHERE id = ?1 AND changeset_id = ?2 AND status = 'pending' LIMIT 1;",
                                params![&item_action.item_id, &input.changeset_id],
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

                        let rows_affected = tx
                            .execute(
                                "UPDATE doors
                             SET target_node_id = ?1,
                                 status = 'active',
                                 orphan_reason = NULL,
                                 orphan_since = NULL,
                                 updated_at = datetime('now')
                             WHERE id = ?2;",
                                params![nid, did],
                            )
                            .map_err(|err| format!("Failed repointing door: {err}"))?;
                        if rows_affected == 0 {
                            return Err(format!("Door '{}' not found (0 rows updated)", did));
                        }

                        // Backlink triggers will auto-sync backlinks
                    }
                    _ => {}
                }

                let rows = tx.execute(
                    "UPDATE changeset_items SET status = 'accepted', reviewed_at = datetime('now') WHERE id = ?1 AND changeset_id = ?2 AND status = 'pending';",
                    params![&item_action.item_id, &input.changeset_id],
                )
                .map_err(|err| format!("Failed accepting changeset item: {err}"))?;
                if rows == 0 {
                    return Err(format!(
                        "Failed to accept changeset item '{}' (no rows affected)",
                        item_action.item_id
                    ));
                }
                accepted_diff += 1;
            }
            _ => {
                return Err(format!(
                    "Unsupported action '{}' for changeset item '{}'",
                    item_action.action, item_action.item_id
                ));
            }
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

    if resolved_status != "pending" {
        let import_jobs_table_exists = tx
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'import_jobs' LIMIT 1;",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if import_jobs_table_exists {
            // Spine only when something was accepted; all-dismissed leaves no chunks.
            if resolved_status != "dismissed" {
                create_import_document_spine(&tx, &input.changeset_id, session_key.as_ref())?;
            }
            // DB CHECK still only allows committed (not dismissed). List/get map
            // committed+changeset.dismissed → status "dismissed" for the Job Log.
            tx.execute(
                "UPDATE import_jobs
                 SET status = 'committed',
                     completed_at = datetime('now')
                 WHERE changeset_id = ?1 AND status = 'staged';",
                [&input.changeset_id],
            )
            .map_err(|err| format!("Failed to commit linked import job: {err}"))?;
        }
    }

    drop(tag_stmt);

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
            CREATE TABLE IF NOT EXISTS doors (
                id TEXT PRIMARY KEY,
                source_node_id TEXT NOT NULL,
                target_node_id TEXT,
                target_vault_id TEXT,
                label TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                orphan_reason TEXT,
                orphan_since TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS import_jobs (
                id TEXT PRIMARY KEY,
                import_type TEXT NOT NULL DEFAULT 'pdf',
                source_name TEXT,
                target_vault_id TEXT,
                status TEXT NOT NULL,
                changeset_id TEXT,
                node_count INTEGER DEFAULT 0,
                error TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                assembled_markdown TEXT,
                avg_ocr_confidence REAL DEFAULT 0.0,
                tables_detected_unpreserved INTEGER DEFAULT 0,
                extraction_path TEXT
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

    #[test]
    fn test_commit_cross_changeset_item_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vault
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;

        // Seed changesets cs_a and cs_b
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_a', 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_b', 'pending', 1);",
            [],
        )?;

        // Seed item_b belonging to cs_b
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_b', 'cs_b', 'add', '{\"title\":\"Item B\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        // Try to commit cs_a but passing item_b
        let input = ChangesetCommitInput {
            changeset_id: "cs_a".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_b".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to cross-changeset item boundary")?;
        assert!(err_msg.contains("does not belong to changeset"));

        // Verify item_b is still pending
        let status: String = conn.query_row(
            "SELECT status FROM changeset_items WHERE id = 'item_b';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(status, "pending");

        // Verify no node was created
        let count: i64 = conn.query_row(
            "SELECT COUNT(1) FROM nodes WHERE title = 'Item B';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);

        Ok(())
    }

    #[test]
    fn test_commit_unsupported_action_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vault
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_a', 'pending', 1);",
            [],
        )?;

        // Seed changeset item
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_a', 'cs_a', 'add', '{\"title\":\"Item A\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        // Try to commit with unsupported action "destroy"
        let input = ChangesetCommitInput {
            changeset_id: "cs_a".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_a".to_string(),
                action: "destroy".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to unsupported action")?;
        assert!(err_msg.contains("Unsupported action 'destroy'"));

        // Verify item_a is still pending
        let status: String = conn.query_row(
            "SELECT status FROM changeset_items WHERE id = 'item_a';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(status, "pending");

        // Verify no node was created
        let count: i64 = conn.query_row(
            "SELECT COUNT(1) FROM nodes WHERE title = 'Item A';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);

        Ok(())
    }

    #[test]
    fn test_commit_update_missing_target_node_id_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_update', 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_update', 'cs_update', 'update', NULL, '{\"title\":\"Title\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_update".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_update".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to missing target_node_id")?;
        assert!(err_msg.contains("Missing target_node_id"));
        Ok(())
    }

    #[test]
    fn test_commit_merge_missing_merge_with_id_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_merge', 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, merge_with_id, proposed_data, status)
             VALUES ('item_merge', 'cs_merge', 'merge', NULL, '{\"title\":\"Title\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_merge".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_merge".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to missing merge_with_id")?;
        assert!(err_msg.contains("Missing merge_with_id"));
        Ok(())
    }

    #[test]
    fn test_commit_delete_missing_target_node_id_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_delete', 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_delete', 'cs_delete', 'delete', NULL, '{\"title\":\"Title\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_delete".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_delete".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to missing target_node_id")?;
        assert!(err_msg.contains("Missing target_node_id"));
        Ok(())
    }

    #[test]
    fn test_commit_node_to_redacted_subvault_under_open_parent_locked_fails(
    ) -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed parent vault (open)
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_parent', 'Parent Vault', 'open');",
            [],
        )?;

        // Seed sub-vault (redacted override)
        conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, privacy_tier) VALUES ('vault_sub', 'vault_parent', 'redacted');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_sub_redacted', 'pending', 1);",
            [],
        )?;

        // Seed changeset item targeting sub-vault
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_sub_redacted', 'cs_sub_redacted', 'add', '{\"title\":\"Secret Subvault Item\",\"vaultId\":\"vault_sub\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_sub_redacted".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_sub_redacted".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Try to commit without session key - should fail with VAULT_LOCKED
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        match result {
            Err(err) => assert_eq!(err, "VAULT_LOCKED"),
            Ok(_) => panic!("Expected error VAULT_LOCKED, but got Ok"),
        }

        Ok(())
    }

    #[test]
    fn test_commit_node_to_redacted_subvault_under_open_parent_unlocked_succeeds(
    ) -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed parent vault (open)
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_parent', 'Parent Vault', 'open');",
            [],
        )?;

        // Seed sub-vault (redacted override)
        conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, privacy_tier) VALUES ('vault_sub', 'vault_parent', 'redacted');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_sub_redacted', 'pending', 1);",
            [],
        )?;

        // Seed changeset item targeting sub-vault
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_sub_redacted', 'cs_sub_redacted', 'add', '{\"title\":\"Secret Subvault Item\",\"vaultId\":\"vault_sub\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_sub_redacted".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_sub_redacted".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Commit with session key - should succeed and encrypt the node title/summary
        let key = [0_u8; 32];
        let result = commit_changeset_transaction(&mut conn, &input, db_path, Some(key))?;
        assert!(result);

        // Verify the node was created under the correct vaults and is redacted
        let (vault_id, sub_vault_id, title, summary, encrypted_payload): (String, Option<String>, String, String, Option<String>) = conn.query_row(
            "SELECT vault_id, sub_vault_id, title, summary, encrypted_payload FROM nodes WHERE sub_vault_id = 'vault_sub' LIMIT 1;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;

        assert_eq!(vault_id, "vault_parent");
        assert_eq!(sub_vault_id, Some("vault_sub".to_string()));
        assert_eq!(title, "[REDACTED]");
        assert_eq!(summary, "[Metadata Locked]");
        assert!(encrypted_payload.is_some());

        Ok(())
    }

    #[test]
    fn test_commit_update_stale_target_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vault
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_update_stale', 'pending', 1);",
            [],
        )?;

        // Seed changeset item targeting a nonexistent node
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_update_stale', 'cs_update_stale', 'update', 'nonexistent_node', '{\"title\":\"Title\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_update_stale".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_update_stale".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to stale target node")?;
        assert!(err_msg.contains("not found or already deleted"));
        Ok(())
    }

    #[test]
    fn test_commit_delete_stale_target_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vault
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_delete_stale', 'pending', 1);",
            [],
        )?;

        // Seed changeset item targeting a nonexistent node
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_delete_stale', 'cs_delete_stale', 'delete', 'nonexistent_node', '{\"title\":\"Title\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_delete_stale".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_delete_stale".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        let err_msg = result
            .err()
            .ok_or("Expected error due to stale target node")?;
        assert!(err_msg.contains("not found or already deleted"));
        Ok(())
    }

    #[test]
    fn test_commit_update_preserves_omitted_fields() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vault
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;

        // Seed node
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, priority)
             VALUES ('node_update', 'vault_open', 'custom_type', 'Original Title', 'Original Summary', 'Original Detail', '{}');",
            [],
        )?;

        // Seed tags
        conn.execute(
            "INSERT INTO tags (id, name) VALUES ('tag_existing', 'Existing Tag');",
            [],
        )?;
        conn.execute(
            "INSERT INTO node_tags (node_id, tag_id) VALUES ('node_update', 'tag_existing');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_update_omitted', 'pending', 1);",
            [],
        )?;

        // Seed changeset item with proposed data omitting detail, summary, nodeType, tags, vaultId
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_update_omitted', 'cs_update_omitted', 'update', 'node_update', '{\"title\":\"Updated Title\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_update_omitted".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_update_omitted".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None)?;
        assert!(result);

        // Verify the node was updated with new title but original values preserved for everything else
        let (title, summary, detail, node_type, vault_id): (String, String, Option<String>, String, String) = conn.query_row(
            "SELECT title, summary, detail, node_type, vault_id FROM nodes WHERE id = 'node_update';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;

        assert_eq!(title, "Updated Title");
        assert_eq!(summary, "Original Summary");
        assert_eq!(detail, Some("Original Detail".to_string()));
        assert_eq!(node_type, "custom_type");
        assert_eq!(vault_id, "vault_open");

        // Verify existing tags were preserved
        let tag_name: String = conn.query_row(
            "SELECT t.name FROM node_tags nt JOIN tags t ON nt.tag_id = t.id WHERE nt.node_id = 'node_update';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(tag_name, "Existing Tag");

        Ok(())
    }

    #[test]
    fn test_commit_update_unredact_locked_node_fails() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vaults
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_redacted', 'Redacted Vault', 'redacted');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open Vault', 'open');",
            [],
        )?;

        // Seed encrypted node
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, encrypted_payload)
             VALUES ('node_encrypted', 'vault_redacted', 'concept', '[REDACTED]', '[Metadata Locked]', NULL, 'some-encrypted-payload');",
            [],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_unredact', 'pending', 1);",
            [],
        )?;

        // Seed changeset item: update node_encrypted to vault_open
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_unredact', 'cs_unredact', 'update', 'node_encrypted', '{\"title\":\"New Title\",\"summary\":\"New Summary\",\"detail\":\"New Detail\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_unredact".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_unredact".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Attempting to commit without session key should fail with VAULT_LOCKED
        let result = commit_changeset_transaction(&mut conn, &input, db_path, None);
        assert_eq!(result.err(), Some("VAULT_LOCKED".to_string()));

        Ok(())
    }

    #[test]
    fn test_commit_update_unredact_unlocked_node_succeeds() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        // Seed vaults
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_redacted', 'Redacted Vault', 'redacted');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open Vault', 'open');",
            [],
        )?;

        // Setup session key
        let key = [0_u8; 32];

        let secret_payload = redacted::NodeSecretPayload {
            title: "Secret Title".to_string(),
            summary: "Secret Summary".to_string(),
            detail: Some("Secret Detail".to_string()),
            source: Some("agent_extract".to_string()),
            source_type: Some("agent_extract".to_string()),
        };
        let encrypted_payload = redacted::encrypt_json(&secret_payload, &key)?;

        // Seed encrypted node
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, encrypted_payload)
             VALUES ('node_encrypted', 'vault_redacted', 'concept', '[REDACTED]', '[Metadata Locked]', NULL, ?1);",
            [encrypted_payload],
        )?;

        // Seed changeset
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_unredact', 'pending', 1);",
            [],
        )?;

        // Seed changeset item: update node_encrypted to vault_open (with edits/proposals)
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_unredact', 'cs_unredact', 'update', 'node_encrypted', '{\"title\":\"New Title\",\"summary\":\"New Summary\",\"detail\":\"New Detail\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_unredact".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_unredact".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        // Committing with session key should succeed
        let result = commit_changeset_transaction(&mut conn, &input, db_path, Some(key))?;
        assert!(result);

        // Verify the node was successfully updated and decrypted (no longer encrypted)
        let (title, summary, detail, encrypted_payload, vault_id): (String, String, Option<String>, Option<String>, String) = conn.query_row(
            "SELECT title, summary, detail, encrypted_payload, vault_id FROM nodes WHERE id = 'node_encrypted';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;

        assert_eq!(title, "New Title");
        assert_eq!(summary, "New Summary");
        assert_eq!(detail, Some("New Detail".to_string()));
        assert!(encrypted_payload.is_none() || encrypted_payload.as_deref() == Some(""));
        assert_eq!(vault_id, "vault_open");

        Ok(())
    }

    #[test]
    fn test_commit_update_persists_source_fields() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority)
             VALUES ('node_source', 'vault_open', 'concept', 'Original Title', 'Original Summary', 'Original Detail', 'manual', 'manual', '{}');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_source', 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, status)
             VALUES ('item_source', 'cs_source', 'update', 'node_source',
                     '{\"title\":\"Updated Title\",\"source\":\"chat_session\",\"sourceType\":\"chat\",\"vaultId\":\"vault_open\"}', 'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_source".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_source".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };

        let result = commit_changeset_transaction(&mut conn, &input, db_path, None)?;
        assert!(result);

        let (title, source, source_type): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT title, source, source_type FROM nodes WHERE id = 'node_source';",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;

        assert_eq!(title, "Updated Title");
        assert_eq!(source.as_deref(), Some("chat_session"));
        assert_eq!(source_type.as_deref(), Some("chat"));

        Ok(())
    }

    #[test]
    fn test_update_changeset_node_preserves_source_when_omitted() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, source, source_type, priority)
             VALUES ('node_preserve_source', 'vault_open', 'concept', 'Original Title', 'Original Summary', 'Original Detail', 'manual', 'manual', '{}');",
            [],
        )?;

        let tx = conn.transaction()?;
        let proposed = crate::memory_agent::changeset::ProposedNodeData {
            title: "Updated Title".to_string(),
            summary: "Updated Summary".to_string(),
            detail: Some("Updated Detail".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: None,
            vault_id: Some("vault_open".to_string()),
            tags: None,
            confidence: 1.0,
            action: crate::memory_agent::parser::CandidateAction::Update,
            substantial_change: None,
            source: None,
            source_type: None,
            meta: None,
        };
        update_changeset_node(&tx, "node_preserve_source", "vault_open", &proposed, None)?;
        tx.commit()?;

        let (source, source_type): (Option<String>, Option<String>) = conn.query_row(
            "SELECT source, source_type FROM nodes WHERE id = 'node_preserve_source';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        assert_eq!(source.as_deref(), Some("manual"));
        assert_eq!(source_type.as_deref(), Some("manual"));

        Ok(())
    }

    #[test]
    fn test_update_changeset_node_unredact_locked_guard() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;

        // Seed vaults
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_redacted', 'Redacted Vault', 'redacted');",
            [],
        )?;
        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open Vault', 'open');",
            [],
        )?;

        // Seed encrypted node
        conn.execute(
            "INSERT INTO nodes (id, vault_id, node_type, title, summary, detail, encrypted_payload)
             VALUES ('node_encrypted', 'vault_redacted', 'concept', '[REDACTED]', '[Metadata Locked]', NULL, 'some-encrypted-payload');",
            [],
        )?;

        let tx = conn.transaction()?;

        let proposed = crate::memory_agent::changeset::ProposedNodeData {
            title: "New Title".to_string(),
            summary: "New Summary".to_string(),
            detail: Some("New Detail".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: None,
            vault_id: Some("vault_open".to_string()),
            tags: None,
            confidence: 1.0,
            action: crate::memory_agent::parser::CandidateAction::Update,
            substantial_change: None,
            source: None,
            source_type: None,
            meta: None,
        };
        // Call update_changeset_node to move node_encrypted to vault_open without session key
        let res = update_changeset_node(&tx, "node_encrypted", "vault_open", &proposed, None);

        let err = res.err().ok_or("Expected error from guard")?;
        assert!(err.contains("Unlock redacted content with your master password before changing the node to a non-redacted tier."));

        Ok(())
    }

    #[test]
    fn test_import_accept_creates_document_spine() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_learning', 'Learning', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_import', 'pending', 2);",
            [],
        )?;
        conn.execute(
            "INSERT INTO import_jobs (id, source_name, target_vault_id, status, changeset_id, assembled_markdown,
             avg_ocr_confidence, tables_detected_unpreserved, extraction_path)
             VALUES ('job_import', 'CSE824 HW1.pdf', 'vault_learning', 'staged', 'cs_import', '# Full Doc\n\nBody',
             0.91, 2, 'hybrid');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status, sort_order)
             VALUES
             ('item_c0', 'cs_import', 'add',
              '{\"title\":\"CSE824 HW1 · Intro (1/2)\",\"summary\":\"Intro\",\"detail\":\"chunk0\",\"node_type\":\"fact\",\"vault_id\":\"vault_learning\",\"source\":\"CSE824 HW1.pdf\",\"source_type\":\"pdf_import\",\"meta\":{\"chunk_index\":0,\"token_estimate\":10,\"ocr_confidence\":0.9,\"tables_unstructured\":true},\"action\":\"add\",\"confidence\":0.9}',
              'pending', 0),
             ('item_c1', 'cs_import', 'add',
              '{\"title\":\"CSE824 HW1 · Body (2/2)\",\"summary\":\"Body\",\"detail\":\"chunk1\",\"node_type\":\"fact\",\"vault_id\":\"vault_learning\",\"source\":\"CSE824 HW1.pdf\",\"source_type\":\"pdf_import\",\"meta\":{\"chunk_index\":1,\"token_estimate\":12,\"ocr_confidence\":0.92},\"action\":\"add\",\"confidence\":0.9}',
              'pending', 1);",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_import".to_string(),
            item_actions: vec![
                ItemReviewAction {
                    item_id: "item_c0".to_string(),
                    action: "accept".to_string(),
                    edited_data: None,
                },
                ItemReviewAction {
                    item_id: "item_c1".to_string(),
                    action: "accept".to_string(),
                    edited_data: None,
                },
            ],
        };

        assert!(commit_changeset_transaction(
            &mut conn, &input, db_path, None
        )?);

        let doc_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE json_extract(meta, '$.import_role') = 'document';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(doc_count, 1);

        let (doc_id, detail, node_type): (String, Option<String>, String) = conn.query_row(
            "SELECT id, detail, node_type FROM nodes WHERE json_extract(meta, '$.import_role') = 'document' LIMIT 1;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(detail.as_deref(), Some("# Full Doc\n\nBody"));
        assert_eq!(node_type, "summary");

        let (avg_ocr, tables_flag, extraction): (f64, i64, Option<String>) = conn.query_row(
            "SELECT json_extract(meta, '$.avg_ocr_confidence'),
                    json_extract(meta, '$.tables_unstructured'),
                    json_extract(meta, '$.extraction_path')
             FROM nodes WHERE id = ?1;",
            [&doc_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert!((avg_ocr - 0.91).abs() < 0.001);
        assert_eq!(tables_flag, 1);
        assert_eq!(extraction.as_deref(), Some("hybrid"));

        let chunk_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE json_extract(meta, '$.import_role') = 'chunk';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(chunk_count, 2);

        let stamped: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes
             WHERE json_extract(meta, '$.document_id') = ?1
               AND json_extract(meta, '$.import_role') = 'chunk';",
            [&doc_id],
            |row| row.get(0),
        )?;
        assert_eq!(stamped, 2);

        let section_doors: i64 = conn.query_row(
            "SELECT COUNT(*) FROM doors WHERE source_node_id = ?1 AND label = 'section';",
            [&doc_id],
            |row| row.get(0),
        )?;
        assert_eq!(section_doors, 2);

        let next_doors: i64 = conn.query_row(
            "SELECT COUNT(*) FROM doors WHERE label = 'next';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(next_doors, 1);

        let job_status: String = conn.query_row(
            "SELECT status FROM import_jobs WHERE id = 'job_import';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(job_status, "committed");

        Ok(())
    }

    #[test]
    fn test_non_import_accept_skips_document_spine() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_open', 'Open', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_agent', 'pending', 1);",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES ('item_agent', 'cs_agent', 'add',
              '{\"title\":\"Agent Fact\",\"summary\":\"From chat\",\"detail\":\"body\",\"node_type\":\"fact\",\"vault_id\":\"vault_open\",\"sourceType\":\"agent_extract\",\"action\":\"add\",\"confidence\":0.9}',
              'pending');",
            [],
        )?;

        let input = ChangesetCommitInput {
            changeset_id: "cs_agent".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_agent".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };
        assert!(commit_changeset_transaction(
            &mut conn, &input, db_path, None
        )?);

        let docs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE json_extract(meta, '$.import_role') = 'document';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(docs, 0);
        let doors: i64 = conn.query_row("SELECT COUNT(*) FROM doors;", [], |row| row.get(0))?;
        assert_eq!(doors, 0);

        Ok(())
    }

    #[test]
    fn test_import_spine_waits_until_changeset_fully_resolved() -> Result<(), Box<dyn Error>> {
        let mut conn = setup_test_db()?;
        let db_path = Path::new("test.db");

        conn.execute(
            "INSERT INTO vaults (id, name, privacy_tier) VALUES ('vault_learning', 'Learning', 'open');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changesets (id, status, item_count) VALUES ('cs_partial', 'pending', 2);",
            [],
        )?;
        conn.execute(
            "INSERT INTO import_jobs (id, source_name, status, changeset_id, assembled_markdown)
             VALUES ('job_partial', 'doc.pdf', 'staged', 'cs_partial', '# Doc');",
            [],
        )?;
        conn.execute(
            "INSERT INTO changeset_items (id, changeset_id, item_type, proposed_data, status)
             VALUES
             ('item_p0', 'cs_partial', 'add',
              '{\"title\":\"doc · A (1/2)\",\"summary\":\"A\",\"detail\":\"a\",\"node_type\":\"fact\",\"vault_id\":\"vault_learning\",\"source\":\"doc.pdf\",\"source_type\":\"pdf_import\",\"meta\":{\"chunk_index\":0},\"action\":\"add\",\"confidence\":0.9}',
              'pending'),
             ('item_p1', 'cs_partial', 'add',
              '{\"title\":\"doc · B (2/2)\",\"summary\":\"B\",\"detail\":\"b\",\"node_type\":\"fact\",\"vault_id\":\"vault_learning\",\"source\":\"doc.pdf\",\"source_type\":\"pdf_import\",\"meta\":{\"chunk_index\":1},\"action\":\"add\",\"confidence\":0.9}',
              'pending');",
            [],
        )?;

        let first = ChangesetCommitInput {
            changeset_id: "cs_partial".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_p0".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };
        assert!(commit_changeset_transaction(
            &mut conn, &first, db_path, None
        )?);

        let docs_mid: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE json_extract(meta, '$.import_role') = 'document';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(docs_mid, 0);
        let job_mid: String = conn.query_row(
            "SELECT status FROM import_jobs WHERE id = 'job_partial';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(job_mid, "staged");

        let second = ChangesetCommitInput {
            changeset_id: "cs_partial".to_string(),
            item_actions: vec![ItemReviewAction {
                item_id: "item_p1".to_string(),
                action: "accept".to_string(),
                edited_data: None,
            }],
        };
        assert!(commit_changeset_transaction(
            &mut conn, &second, db_path, None
        )?);

        let docs_end: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE json_extract(meta, '$.import_role') = 'document';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(docs_end, 1);

        Ok(())
    }
}
