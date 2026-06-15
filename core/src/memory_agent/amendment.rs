use rusqlite::{params, Connection};
use serde_json;

use crate::memory_agent;
use std::time::{SystemTime, UNIX_EPOCH};

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn chrono_now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let millis = now.subsec_millis();

    // Manual decomposition of Unix timestamp → UTC date/time components
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;

    // Days since epoch → Gregorian calendar
    let mut days = secs / 86400;
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let months = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &days_in_month in &months {
        if days < days_in_month {
            break;
        }
        days -= days_in_month;
        month += 1;
    }
    let day = days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, h, m, s, millis
    )
}

fn is_leap(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// Computes Jaccard similarity between two strings at the whitespace-token level.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let set_b: std::collections::HashSet<&str> = b.split_whitespace().collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
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
/// The key is prefixed with `_` so itrust-analyzer-diagnostics-view:/diagnostic%20message%20%5B4%5D?4#file:///Users/fiorittoev/Desktop/projects/MindVault/core/src/memory_agent/ammendment.rs sorts before all plain field names under
/// BTreeMap (serde_json default) and is also first when `preserve_order` /
/// IndexMap is enabled (explicit prepend).
///
/// Only called on the UPDATE (similarity-match) path — new inserts stay clean.
fn stamp_amended(
    proposed_data: serde_json::Value,
    similarity: f64,
    correction_signal: &crate::memory_agent::CorrectionSignal,
) -> Result<serde_json::Value, String> {
    let obj = proposed_data
        .as_object()
        .ok_or_else(|| "proposed_data is not a JSON object".to_string())?;

    let mut ordered = serde_json::Map::new();

    // Insert `_amended` first so it leads the object regardless of feature flags.
    ordered.insert(
        "_amended".to_string(),
        serde_json::json!({
            "at":         chrono_now_iso(),
            "similarity": similarity,
            "reason":     format!("{:?}", correction_signal),
        }),
    );

    for (k, v) in obj.iter() {
        ordered.insert(k.clone(), v.clone());
    }

    Ok(serde_json::Value::Object(ordered))
}

/// Finds the most-recent pending changeset for a (session_id, model) pair.
/// Returns `Some((changeset_id, content))` or `None`.
fn find_pending_changeset(
    conn: &Connection,
    session_id: &str,
    model: &str,
) -> Result<Option<(String, String)>, String> {
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
        Ok(Some((id, String::new())))
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
             WHERE changeset_id = ?1",
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
    let (existing_id, _content) =
        existing_changeset.ok_or_else(|| "Pending changeset unexpectedly missing".to_string())?;

    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed to start transaction: {err}"))?;

    // Load existing items once; all comparisons run against this snapshot.
    let pending_items = load_pending_items(&tx, &existing_id)?;

    for candidate in candidates {
        let resolved_vault_id = candidate
            .target_vault_key
            .as_deref()
            .and_then(crate::onboarding::vault_id_for_category_key)
            .unwrap_or("vault_root_graph")
            .to_string();

        // Build the base proposed_data for this candidate.
        let candidate_data = serde_json::json!({
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
            .map(|(item_id, existing_data)| {
                let existing_fp = candidate_fingerprint(existing_data);
                let sim = jaccard_similarity(&candidate_fp, &existing_fp);
                (item_id, sim)
            })
            .filter(|(_, sim)| *sim > 0.5)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        if let Some((matched_id, similarity)) = best_match {
            // ── UPDATE path: same candidate, corrected values ─────────────────
            //
            // Stamp `_amended` metadata so the Diff Panel can render the
            // (amended) badge without a schema migration.
            let amended_data = stamp_amended(candidate_data, similarity, correction_signal)
                .map_err(|e| format!("Failed to stamp _amended on item {}: {}", matched_id, e))?;

            let proposed_json = serde_json::to_string(&amended_data)
                .map_err(|e| format!("JSON serialization error: {}", e))?;

            tx.execute(
                "UPDATE changeset_items
                 SET proposed_data = ?1,
                     similarity    = ?2,
                     reviewed_at   = NULL,
                     sort_order    = sort_order
                 WHERE id = ?3",
                params![proposed_json, similarity, matched_id],
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
