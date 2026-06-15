use rusqlite::{params, Connection};
use serde_json;

use crate::memory_agent;
use std::time::{SystemTime, UNIX_EPOCH};

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn chrono_now_iso() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let total_secs = dur.as_secs();
    let millis = dur.subsec_millis();

    let secs_of_day = total_secs % 86_400;
    let h = secs_of_day / 3_600;
    let m = (secs_of_day % 3_600) / 60;
    let s = secs_of_day % 60;

    // Convert days since epoch to (year, month, day) using a compact civil-date algorithm.
    let mut days = (total_secs / 86_400) as i64;
    // Shift epoch from 1970-01-01 to 0000-03-01 for simpler leap-year math.
    days += 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097); // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mo <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, mo, d, h, m, s, millis
    )
}

/// Extracts a lowercase `"title summary"` string from a proposed_data JSON value
/// for use as the Jaccard comparison fingerprint.
fn candidate_fingerprint(proposed_data: &serde_json::Value) -> String {
    let title = proposed_data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let summary = proposed_data
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    format!("{} {}", title, summary)
}

/// Injects `_amended` metadata into an existing proposed_data JSON value.
/// The key is prefixed with `_` so it sorts before all plain field names under
/// BTreeMap (serde_json default) and is also first when `preserve_order` /
/// IndexMap is enabled (explicit prepend).
///
/// Only called on the UPDATE (similarity-match) path — new inserts stay clean.
/// The caller always passes a valid JSON object, so this mutates in-place.
fn stamp_amended(
    proposed_data: &mut serde_json::Value,
    similarity: f64,
    correction_signal: &crate::memory_agent::CorrectionSignal,
) {
    if let Some(obj) = proposed_data.as_object_mut() {
        obj.insert(
            "_amended".to_string(),
            serde_json::json!({
                "at":         chrono_now_iso(),
                "similarity": similarity,
                "reason":     format!("{:?}", correction_signal),
            }),
        );
    }
}

/// Finds the most-recent pending changeset for a (session_id, model) pair.
/// Returns `Some(changeset_id)` or `None`.
fn find_pending_changeset(
    conn: &Connection,
    session_id: &str,
    model: &str,
) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM changesets
            WHERE session_id = ?1 AND model_used = ?2 AND status = 'pending'
            ORDER BY created_at DESC LIMIT 1",
        )
        .map_err(|e| format!("Database error: {}", e))?;

    let mut rows = stmt
        .query(params![session_id, model])
        .map_err(|e| format!("Database error: {}", e))?;

    if let Some(row) = rows.next().map_err(|e| format!("Database error: {}", e))? {
        let id: String = row.get(0).map_err(|e| format!("Database error: {}", e))?;
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

/// Loads all `changeset_items` rows for a given changeset, returning
/// `(item_id, proposed_data)` pairs. Runs inside the caller's transaction.
fn load_pending_items(
    conn: &Connection,
    changeset_id: &str,
) -> Result<Vec<(String, serde_json::Value)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, proposed_data FROM changeset_items \
             WHERE changeset_id = ?1 AND status = 'pending'",
        )
        .map_err(|e| format!("Database error: {}", e))?;

    let mut rows = stmt
        .query(params![changeset_id])
        .map_err(|e| format!("Database error: {}", e))?;

    let mut items = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Database error: {}", e))? {
        let item_id: String = row.get(0).map_err(|e| format!("Database error: {}", e))?;
        let proposed_raw: String = row.get(1).map_err(|e| format!("Database error: {}", e))?;
        let proposed_data: serde_json::Value = serde_json::from_str(&proposed_raw)
            .map_err(|e| format!("JSON parse error on item {}: {}", item_id, e))?;
        items.push((item_id, proposed_data));
    }

    Ok(items)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Either amends an existing pending changeset or creates a fresh one.
///
/// Returns `(changeset_id, amended)` where `amended` is:
/// - `false` — a brand-new changeset was created.
/// - `true`  — an existing pending changeset was updated in-place.
///
/// `correction_signal` is used only on the amend path to populate `_amended.reason`
/// in `proposed_data`; it is ignored when creating a new changeset.
pub fn amend_or_create_changeset(
    conn: &mut Connection,
    candidates: &[crate::memory_agent::CandidateNode],
    session_id: &str,
    model: &str,
    correction_signal: &crate::memory_agent::CorrectionSignal,
) -> Result<(String, bool), String> {
    // ── 1. Check for an existing pending changeset ────────────────────────────
    let existing_changeset = find_pending_changeset(conn, session_id, model)?;

    // ── 2a. No pending changeset — create a fresh one ────────────────────────
    if existing_changeset.is_none() {
        let changeset_id = {
            let tx = conn
                .transaction()
                .map_err(|err| format!("Failed to start transaction: {err}"))?;

            let pending_changeset =
                memory_agent::changeset::build_changeset(&tx, candidates, session_id)?;

            let persisted_id =
                memory_agent::persistence::persist_changeset(&tx, &pending_changeset, Some(model))?;

            tx.commit()
                .map_err(|err| format!("Failed to commit transaction: {err}"))?;

            persisted_id
        };

        return Ok((changeset_id, false));
    }

    // ── 2b. Pending changeset exists — amend in-place where possible ─────────
    let existing_id =
        existing_changeset.ok_or_else(|| "Pending changeset unexpectedly missing".to_string())?;

    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed to start transaction: {err}"))?;

    // Load existing items once; all comparisons run against this snapshot.
    let pending_items = load_pending_items(&tx, &existing_id)?;

    let mut amended_item_ids = std::collections::HashSet::new();

    for candidate in candidates {
        let resolved_vault_id = candidate
            .target_vault_key
            .as_deref()
            .and_then(crate::onboarding::vault_id_for_category_key)
            .unwrap_or("vault_root_graph")
            .to_string();

        // Build the base proposed_data for this candidate.
        let mut candidate_data = serde_json::json!({
            "title":            candidate.title,
            "summary":          candidate.summary,
            "detail":           candidate.detail,
            "nodeType":         candidate.node_type,
            "targetVaultKey":   candidate.target_vault_key,
            "vaultId":          resolved_vault_id,
            "tags":             candidate.tags,
            "confidence":       candidate.confidence,
            "action":           candidate.action,
        });

        let candidate_fp = candidate_fingerprint(&candidate_data);

        // Find the highest-similarity existing item above the 50 % threshold.
        let best_match = pending_items
            .iter()
            .filter(|(item_id, _)| !amended_item_ids.contains(item_id))
            .map(|(item_id, existing_data)| {
                let existing_fp = candidate_fingerprint(existing_data);
                let sim = memory_agent::jaccard_similarity(&candidate_fp, &existing_fp);
                (item_id, sim)
            })
            .filter(|(_, sim)| *sim > 0.5)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        if let Some((matched_id, similarity)) = best_match {
            // Track this item ID as amended so it is excluded from future matches.
            amended_item_ids.insert(matched_id.clone());

            // ── UPDATE path: same candidate, corrected values ─────────────────
            //
            // Stamp `_amended` metadata so the Diff Panel can render the
            // (amended) badge without a schema migration.
            stamp_amended(&mut candidate_data, similarity, correction_signal);

            let proposed_json = serde_json::to_string(&candidate_data)
                .map_err(|e| format!("JSON serialization error: {}", e))?;

            let item_type = match candidate.action {
                crate::memory_agent::CandidateAction::Add => "add",
                crate::memory_agent::CandidateAction::Update => "update",
                crate::memory_agent::CandidateAction::Delete => "delete",
            };

            tx.execute(
                "UPDATE changeset_items
                 SET proposed_data = ?1,
                     similarity    = ?2,
                     item_type     = ?3,
                     reviewed_at   = NULL,
                     sort_order    = sort_order
                 WHERE id = ?4",
                params![proposed_json, similarity, item_type, matched_id],
            )
            .map_err(|e| format!("Failed to update changeset_item {}: {}", matched_id, e))?;
        } else {
            // ── INSERT path: genuinely new candidate, no _amended stamp ───────
            let new_item_id = crate::generate_id(&tx, "item")
                .map_err(|e| format!("Failed generating item id: {e}"))?;
            let proposed_json = serde_json::to_string(&candidate_data)
                .map_err(|e| format!("JSON serialization error: {}", e))?;

            let item_type = match candidate.action {
                crate::memory_agent::CandidateAction::Add => "add",
                crate::memory_agent::CandidateAction::Update => "update",
                crate::memory_agent::CandidateAction::Delete => "delete",
            };

            tx.execute(
                "INSERT INTO changeset_items
                     (id, changeset_id, item_type, proposed_data, reviewed_at, sort_order)
                 VALUES (?1, ?2, ?3, ?4, NULL,
                     COALESCE(
                         (SELECT MAX(sort_order) + 1 FROM changeset_items WHERE changeset_id = ?2),
                         0
                     ))",
                params![new_item_id, existing_id, item_type, proposed_json],
            )
            .map_err(|e| format!("Failed to insert new changeset_item: {}", e))?;

            tx.execute(
                "UPDATE changesets SET item_count = item_count + 1 WHERE id = ?1",
                params![existing_id],
            )
            .map_err(|e| format!("Failed to increment item_count: {}", e))?;
        }
    }

    tx.commit()
        .map_err(|err| format!("Failed to commit transaction: {err}"))?;

    Ok((existing_id, true))
}
