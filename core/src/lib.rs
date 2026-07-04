use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chat::ChatMessage;
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use tauri::{Emitter, Manager};

mod auth;
mod chat;
pub mod embed;
pub mod ingest;
pub mod ipc_types;
pub mod llm;
pub mod memory_agent;
pub mod models;
pub mod ocr;
pub mod onboarding;
mod priority;
mod privacy;
mod redacted;
use ipc_types::{
    Backlink, Changeset, ChangesetItem, Door, DoorCreateInput, Node, NodeCreateInput,
    NodeUpdateInput, OnboardingNodeCommitInput, OnboardingProposedNode, Tag, TagCreateInput, Vault,
    VaultCreateInput, VaultUpdateInput,
};
pub use ipc_types::{EmbeddingReembedInput, EmbeddingStatus};

// MARK: Internal Types and Constants

/// Default `max_tokens` for context assembly (`debug_assemble_context`, `llm_chat`).
/// Keep in sync with `CONTEXT_MAX_TOKENS` in `ui/constants/contextBudget.ts`.
const DEFAULT_ASSEMBLER_MAX_TOKENS: usize = 8000;

static MEMORY_AGENT_LIMITER: std::sync::OnceLock<
    governor::RateLimiter<
        governor::state::direct::NotKeyed,
        governor::state::InMemoryState,
        governor::clock::DefaultClock,
    >,
> = std::sync::OnceLock::new();

static EMBEDDING_LIMITER: std::sync::OnceLock<
    governor::RateLimiter<
        governor::state::direct::NotKeyed,
        governor::state::InMemoryState,
        governor::clock::DefaultClock,
    >,
> = std::sync::OnceLock::new();

type AppState = DbState;

type OnboardingDefaultVaultSpec = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i64,
);

struct RawVaultRecord {
    id: String,
    parent_vault_id: Option<String>,
    name: String,
    icon: Option<String>,
    description: Option<String>,
    privacy_tier: String,
    priority_profile: String,
    summary_node_id: Option<String>,
    sort_order: i64,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    meta: String,
    ui_metadata: String,
    encrypted_payload: Option<String>,
}

struct RawNodeRecord {
    id: String,
    vault_id: String,
    sub_vault_id: Option<String>,
    node_type: String,
    title: String,
    summary: String,
    detail: Option<String>,
    source: Option<String>,
    source_type: Option<String>,
    privacy_tier: Option<String>,
    priority: String,
    version: i64,
    is_archived: bool,
    created_at: String,
    updated_at: String,
    last_accessed: String,
    deleted_at: Option<String>,
    meta: String,
    encrypted_payload: Option<String>,
}

struct ValidatedProposal<'a> {
    vault_id: &'a str,
    title: &'a str,
    summary: &'a str,
    detail: Option<String>,
    node_type: &'a str,
    source_type: &'a str,
    tags: Option<&'a Vec<String>>,
}

// MARK: Internal Helpers

fn resolve_vault_privacy_in_memory(
    vault_id: &str,
    map: &std::collections::HashMap<String, (Option<String>, String)>,
) -> String {
    let mut current_id = Some(vault_id.to_string());
    let mut strictest = "open".to_string();
    let mut depth = 0;
    while let Some(id) = current_id {
        depth += 1;
        if depth > 100 {
            break;
        }
        if let Some((parent_id, tier)) = map.get(&id) {
            strictest =
                privacy::get_effective_privacy(Some(tier.as_str()), None, Some(strictest.as_str()))
                    .to_string();
            current_id = parent_id.clone();
        } else {
            break;
        }
    }
    strictest
}

fn fetch_private_referenced_nodes(
    conn: &Connection,
    unique_node_ids: &std::collections::HashSet<String>,
    vault_map: &std::collections::HashMap<String, (Option<String>, String)>,
) -> Result<std::collections::HashSet<String>, String> {
    let mut private_nodes = std::collections::HashSet::new();

    if unique_node_ids.is_empty() {
        return Ok(private_nodes);
    }

    let unique_node_ids_vec: Vec<String> = unique_node_ids.iter().cloned().collect();

    for chunk in unique_node_ids_vec.chunks(900) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let query_str = format!(
            "SELECT n.id, n.vault_id, n.sub_vault_id, COALESCE(o.privacy_tier, n.privacy_tier)
             FROM nodes n
             LEFT JOIN privacy_overrides o ON n.id = o.node_id
             WHERE n.id IN ({placeholders}) AND n.deleted_at IS NULL;"
        );

        let mut node_stmt = conn
            .prepare(&query_str)
            .map_err(|err| format!("Failed preparing batched nodes privacy query: {err}"))?;

        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let mut node_rows = node_stmt
            .query(rusqlite::params_from_iter(params))
            .map_err(|err| format!("Failed querying batched nodes privacy: {err}"))?;

        while let Some(row) = node_rows
            .next()
            .map_err(|err| format!("Failed reading batched node privacy row: {err}"))?
        {
            let id: String = row
                .get(0)
                .map_err(|err| format!("Failed decoding id: {err}"))?;
            let vault_id: String = row
                .get(1)
                .map_err(|err| format!("Failed decoding vault_id: {err}"))?;
            let sub_vault_id: Option<String> = row
                .get(2)
                .map_err(|err| format!("Failed decoding sub_vault_id: {err}"))?;
            let node_privacy_tier: Option<String> = row
                .get(3)
                .map_err(|err| format!("Failed decoding node_privacy_tier: {err}"))?;

            let container_tier = if let Some(ref sv_id) = sub_vault_id {
                resolve_vault_privacy_in_memory(sv_id, vault_map)
            } else {
                resolve_vault_privacy_in_memory(&vault_id, vault_map)
            };

            let effective = privacy::get_effective_privacy(
                node_privacy_tier.as_deref(),
                None,
                Some(container_tier.as_str()),
            );

            if effective == "redacted" || effective == "locked" {
                private_nodes.insert(id);
            }
        }
    }

    Ok(private_nodes)
}

fn fetch_latest_user_message(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT content FROM session_messages
         WHERE session_id = ?1 AND role = 'user'
         ORDER BY created_at DESC, rowid DESC
         LIMIT 1;",
        [session_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|err| format!("Failed querying latest user message: {err}"))
}

fn sqlite_db_path<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    Ok(app_data_dir.join("mindvault.db"))
}

fn raw_vault_from_row(row: &Row<'_>) -> rusqlite::Result<RawVaultRecord> {
    Ok(RawVaultRecord {
        id: row.get(0)?,
        parent_vault_id: row.get(1)?,
        name: row.get(2)?,
        icon: row.get(3)?,
        description: row.get(4)?,
        privacy_tier: row.get(5)?,
        priority_profile: row.get(6)?,
        summary_node_id: row.get(7)?,
        sort_order: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        deleted_at: row.get(11)?,
        meta: row.get(12)?,
        ui_metadata: row.get(13)?,
        encrypted_payload: row.get(14)?,
    })
}

fn resolve_vault_record(
    raw: RawVaultRecord,
    session_key: Option<redacted::SessionKey>,
) -> Result<Vault, String> {
    let mut vault = Vault {
        id: raw.id,
        parent_vault_id: raw.parent_vault_id,
        name: raw.name,
        icon: raw.icon,
        description: raw.description,
        privacy_tier: raw.privacy_tier,
        priority_profile: raw.priority_profile,
        summary_node_id: raw.summary_node_id,
        sort_order: raw.sort_order,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
        deleted_at: raw.deleted_at,
        meta: raw.meta,
        ui_metadata: raw.ui_metadata,
    };

    if raw.encrypted_payload.is_some() {
        match (raw.encrypted_payload.as_deref(), session_key) {
            (Some(payload), Some(key)) => {
                let decrypted: redacted::VaultSecretPayload =
                    redacted::decrypt_json(payload, &key)?;
                vault.name = decrypted.name;
                vault.icon = decrypted.icon;
                vault.description = decrypted.description;
            }
            _ => redacted::apply_locked_vault_placeholder(&mut vault),
        }
    }

    Ok(vault)
}

fn raw_node_from_row(row: &Row<'_>) -> rusqlite::Result<RawNodeRecord> {
    Ok(RawNodeRecord {
        id: row.get(0)?,
        vault_id: row.get(1)?,
        sub_vault_id: row.get(2)?,
        node_type: row.get(3)?,
        title: row.get(4)?,
        summary: row.get(5)?,
        detail: row.get(6)?,
        source: row.get(7)?,
        source_type: row.get(8)?,
        privacy_tier: row.get(9)?,
        priority: row.get(10)?,
        version: row.get(11)?,
        is_archived: row.get::<_, i64>(12)? != 0,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        last_accessed: row.get(15)?,
        deleted_at: row.get(16)?,
        meta: row.get(17)?,
        encrypted_payload: row.get(18)?,
    })
}

fn tag_from_row(row: &Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn door_from_row(row: &Row<'_>) -> rusqlite::Result<Door> {
    Ok(Door {
        id: row.get(0)?,
        source_node_id: row.get(1)?,
        target_node_id: row.get(2)?,
        target_vault_id: row.get(3)?,
        label: row.get(4)?,
        status: row.get(5)?,
        orphan_reason: row.get(6)?,
        orphan_since: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn backlink_from_row(row: &Row<'_>) -> rusqlite::Result<Backlink> {
    Ok(Backlink {
        id: row.get(0)?,
        target_node_id: row.get(1)?,
        source_node_id: row.get(2)?,
        door_id: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn onboarding_default_vault_spec(vault_id: &str) -> Option<OnboardingDefaultVaultSpec> {
    match vault_id {
        "vault_root_graph" => Some((
            "Root Graph",
            "root",
            "Always-loaded cross-vault context graph.",
            "open",
            "standard",
            "{}",
            0_i64,
        )),
        "vault_credentials" => Some((
            "Credentials",
            "key",
            "Local-only secrets and API keys.",
            "locked",
            "pinned",
            "{}",
            1_i64,
        )),
        "vault_personal" => Some((
            "Personal",
            "user",
            "Identity, preferences, interests, and personal context.",
            "open",
            "standard",
            "{}",
            2_i64,
        )),
        "vault_work" => Some((
            "Work",
            "briefcase",
            "Professional goals, projects, and operating context.",
            "open",
            "standard",
            "{}",
            3_i64,
        )),
        "vault_learning" => Some((
            "Learning",
            "book",
            "Skills, study notes, and ongoing learning tracks.",
            "open",
            "standard",
            "{}",
            4_i64,
        )),
        "vault_health" => Some((
            "Health",
            "heart",
            "Well-being routines, health notes, and constraints.",
            "local_only",
            "standard",
            "{}",
            5_i64,
        )),
        "vault_finance" => Some((
            "Finance",
            "coins",
            "Budgets, financial plans, and money-related context.",
            "local_only",
            "standard",
            "{}",
            6_i64,
        )),
        _ => None,
    }
}

fn resolve_node_record(
    raw: RawNodeRecord,
    session_key: Option<redacted::SessionKey>,
) -> Result<Node, String> {
    let mut node = Node {
        id: raw.id,
        vault_id: raw.vault_id,
        sub_vault_id: raw.sub_vault_id,
        node_type: raw.node_type,
        title: raw.title,
        summary: raw.summary,
        detail: raw.detail,
        source: raw.source,
        source_type: raw.source_type,
        privacy_tier: raw.privacy_tier,
        priority: raw.priority,
        version: raw.version,
        is_archived: raw.is_archived,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
        last_accessed: raw.last_accessed,
        deleted_at: raw.deleted_at,
        meta: raw.meta,
    };

    if raw.encrypted_payload.is_some() {
        match (raw.encrypted_payload.as_deref(), session_key) {
            (Some(payload), Some(key)) => {
                let decrypted: redacted::NodeSecretPayload = redacted::decrypt_json(payload, &key)?;
                node.title = decrypted.title;
                node.summary = decrypted.summary;
                node.detail = decrypted.detail;
                node.source = decrypted.source;
                node.source_type = decrypted.source_type;
            }
            _ => redacted::apply_locked_node_placeholder(&mut node),
        }
    }

    Ok(node)
}

fn fetch_vault_by_id(
    conn: &Connection,
    vault_id: &str,
    session_key: Option<redacted::SessionKey>,
) -> Result<Vault, String> {
    let raw = conn
        .query_row(
            "SELECT id, parent_vault_id, name, icon, description, privacy_tier, priority_profile, summary_node_id,
                    sort_order, created_at, updated_at, deleted_at, meta, ui_metadata, encrypted_payload
             FROM (
                SELECT id,
                       NULL AS parent_vault_id,
                       name,
                       icon,
                       description,
                       privacy_tier,
                       priority_profile,
                       summary_node_id,
                       sort_order,
                       created_at,
                       updated_at,
                       deleted_at,
                       meta,
                       ui_metadata,
                       encrypted_payload
                FROM vaults
                WHERE deleted_at IS NULL
                UNION ALL
                SELECT id,
                       vault_id AS parent_vault_id,
                       name,
                       icon,
                       description,
                       COALESCE(privacy_tier, 'open') AS privacy_tier,
                       COALESCE(priority_profile, 'standard') AS priority_profile,
                       summary_node_id,
                       sort_order,
                       created_at,
                       updated_at,
                       deleted_at,
                       meta,
                       ui_metadata,
                       encrypted_payload
                FROM sub_vaults
                WHERE deleted_at IS NULL
             )
             WHERE id = ?1
             LIMIT 1;",
            [vault_id],
            raw_vault_from_row,
        )
        .map_err(|err| format!("Failed fetching vault {vault_id}: {err}"))?;
    resolve_vault_record(raw, session_key)
}

fn fetch_node_by_id(
    conn: &Connection,
    node_id: &str,
    session_key: Option<redacted::SessionKey>,
) -> Result<Option<Node>, String> {
    match conn.query_row(
        "SELECT id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                privacy_tier, priority, version, is_archived, created_at, updated_at, last_accessed,
                deleted_at, meta, encrypted_payload
         FROM nodes
         WHERE id = ?1;",
        [node_id],
        raw_node_from_row,
    ) {
        Ok(raw) => resolve_node_record(raw, session_key).map(Some),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(format!("Failed fetching node {node_id}: {err}")),
    }
}

fn fetch_tag_by_id(conn: &Connection, tag_id: &str) -> Result<Tag, String> {
    conn.query_row(
        "SELECT id, name, color, created_at
         FROM tags
         WHERE id = ?1;",
        [tag_id],
        tag_from_row,
    )
    .map_err(|err| format!("Failed fetching tag {tag_id}: {err}"))
}

fn fetch_door_by_id(conn: &Connection, door_id: &str) -> Result<Door, String> {
    conn.query_row(
        "SELECT id, source_node_id, target_node_id, target_vault_id, label, status, orphan_reason,
                orphan_since, created_at, updated_at
         FROM doors
         WHERE id = ?1;",
        [door_id],
        door_from_row,
    )
    .map_err(|err| format!("Failed fetching door {door_id}: {err}"))
}

fn fetch_nodes(
    conn: &Connection,
    session_key: Option<redacted::SessionKey>,
) -> Result<Vec<Node>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                    privacy_tier, priority, version, is_archived, created_at, updated_at, last_accessed,
                    deleted_at, meta, encrypted_payload
             FROM nodes
             WHERE deleted_at IS NULL
             ORDER BY created_at DESC;",
        )
        .map_err(|err| format!("Failed preparing node_list query: {err}"))?;

    let rows = statement
        .query_map([], raw_node_from_row)
        .map_err(|err| format!("Failed querying nodes: {err}"))?;

    let mut nodes = Vec::new();
    for row in rows {
        let raw = row.map_err(|err| format!("Failed decoding node row: {err}"))?;
        nodes.push(resolve_node_record(raw, session_key)?);
    }
    Ok(nodes)
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("db")
        .join("migrations")
}

fn load_migration_files() -> Result<Vec<(i64, String, PathBuf)>, String> {
    let dir = migrations_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&dir).map_err(|err| {
        format!(
            "Failed to read migrations directory {}: {err}",
            dir.display()
        )
    })?;

    let mut migrations = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read migration entry: {err}"))?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if !file_name.ends_with(".sql") {
            continue;
        }

        let Some((version_text, rest)) = file_name.split_once('_') else {
            return Err(format!(
                "Migration file must follow '<version>_<name>.sql': {file_name}"
            ));
        };

        let version = version_text
            .parse::<i64>()
            .map_err(|_| format!("Migration version must be numeric: {file_name}"))?;

        let name = rest.trim_end_matches(".sql").to_string();
        if name.is_empty() {
            return Err(format!("Migration name is missing in file: {file_name}"));
        }

        migrations.push((version, name, path));
    }

    migrations.sort_by_key(|migration| migration.0);

    for i in 1..migrations.len() {
        if migrations[i - 1].0 == migrations[i].0 {
            return Err(format!(
                "Duplicate migration version detected: {}",
                migrations[i].0
            ));
        }
    }

    Ok(migrations)
}

fn run_migrations(conn: &mut Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    )
    .map_err(|err| format!("Failed creating schema_migrations table: {err}"))?;

    for (version, name, path) in load_migration_files()? {
        let already_applied: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM schema_migrations WHERE version = ?1;",
                [version],
                |row| row.get(0),
            )
            .map_err(|err| format!("Failed checking migration {version}: {err}"))?;

        if already_applied > 0 {
            continue;
        }

        let sql = fs::read_to_string(&path)
            .map_err(|err| format!("Failed reading {}: {err}", path.display()))?;

        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting migration transaction: {err}"))?;

        tx.execute_batch(&sql)
            .map_err(|err| format!("Migration {} failed: {err}", path.display()))?;

        tx.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2);",
            params![version, name],
        )
        .map_err(|err| format!("Failed recording migration {}: {err}", path.display()))?;

        tx.commit()
            .map_err(|err| format!("Failed committing migration transaction: {err}"))?;
    }

    Ok(())
}

fn run_seed_data(conn: &mut Connection) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed starting seed transaction: {err}"))?;

    tx.execute_batch(
        "INSERT OR IGNORE INTO settings (key, value, scope) VALUES
            ('default_model', '\"local\"', 'global'),
            ('local_model_endpoint', '\"http://localhost:11434\"', 'global'),
            ('priority_check_interval_h', '24', 'global'),
            ('auto_trim_threshold', '0.25', 'global'),
            ('snapshot_on_session_end', 'true', 'global'),
            ('onboarding_complete', 'false', 'global');",
    )
    .map_err(|err| format!("Failed inserting default settings: {err}"))?;

    embed::seed_embedding_defaults(&tx)
        .map_err(|err| format!("Failed inserting default embedding settings: {err}"))?;

    tx.execute(
        "INSERT OR IGNORE INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);",
        params![
            "vault_root_graph",
            "Root Graph",
            "root",
            "Always-loaded cross-vault context graph.",
            "open",
            "standard",
            0_i64,
            "{}"
        ],
    )
    .map_err(|err| format!("Failed inserting Root Graph vault: {err}"))?;

    tx.execute(
        "INSERT OR IGNORE INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);",
        params![
            "vault_credentials",
            "Credentials",
            "key",
            "Local-only secrets and API keys.",
            "locked",
            "pinned",
            1_i64,
            "{}"
        ],
    )
    .map_err(|err| format!("Failed inserting Credentials vault: {err}"))?;

    tx.commit()
        .map_err(|err| format!("Failed committing seed transaction: {err}"))?;

    Ok(())
}

fn ensure_encrypted_vault_can_be_unredacted(
    current_is_encrypted: bool,
    session_key: Option<redacted::SessionKey>,
    should_encrypt: bool,
) -> Result<(), String> {
    if current_is_encrypted && !should_encrypt && session_key.is_none() {
        return Err(
            "Unlock redacted content with your master password before changing the vault to a non-redacted tier."
                .to_string(),
        );
    }
    Ok(())
}

fn ensure_encrypted_node_can_be_unredacted(
    current_is_encrypted: bool,
    session_key: Option<redacted::SessionKey>,
    should_encrypt: bool,
) -> Result<(), String> {
    if current_is_encrypted && !should_encrypt && session_key.is_none() {
        return Err(
            "Unlock redacted content with your master password before changing the node to a non-redacted tier."
                .to_string(),
        );
    }
    Ok(())
}

fn map_onboarding_proposed(node: crate::onboarding::ProposedNode) -> OnboardingProposedNode {
    let resolved_vault_id = node
        .target_vault_key
        .as_deref()
        .and_then(crate::onboarding::vault_id_for_category_key)
        .or_else(|| {
            node.category
                .as_deref()
                .and_then(crate::onboarding::vault_id_for_category_key)
        })
        .map(|s| s.to_string());

    OnboardingProposedNode {
        title: node.title,
        summary: node.summary,
        detail: node.detail,
        category: node.category,
        target_vault_key: node.target_vault_key,
        tags: node.tags,
        node_type: node.node_type,
        resolved_vault_id,
    }
}

fn insert_onboarding_node(
    tx: &rusqlite::Transaction,
    proposal: &ValidatedProposal,
) -> Result<(), String> {
    let vault = fetch_vault_by_id(tx, proposal.vault_id, None)?;
    let (resolved_vault_id, resolved_sub_vault_id) = match vault.parent_vault_id {
        Some(parent_id) => (parent_id, Some(vault.id)),
        None => (vault.id, None),
    };

    let priority_json = priority::DEFAULT_PRIORITY_JSON;

    let node_id = generate_id(tx, "node")?;

    tx.execute(
        "INSERT INTO nodes (
            id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
            privacy_tier, priority, meta
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11);",
        params![
            node_id,
            resolved_vault_id,
            resolved_sub_vault_id,
            proposal.node_type,
            proposal.title,
            proposal.summary,
            proposal.detail,
            Some("onboarding_wizard"),
            proposal.source_type,
            priority_json,
            "{}"
        ],
    )
    .map_err(|err| format!("Failed inserting onboarding node: {err}"))?;

    if let Some(tags) = proposal.tags {
        for tag_name in tags {
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
                    let new_id = generate_id(tx, "tag")?;
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

fn run_priority_refresh(db_path: &std::path::Path) -> Result<usize, String> {
    let conn = open_connection(db_path)?;
    let mut statement = conn
        .prepare("SELECT id, vault_id, priority FROM nodes WHERE deleted_at IS NULL;")
        .map_err(|err| format!("Failed preparing priority refresh query: {err}"))?;

    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|err| format!("Failed querying nodes for priority: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed reading priority rows: {err}"))?;

    // Pass 1: Determine which vaults had activity today.
    let mut active_vaults = std::collections::HashSet::new();
    for (_, vault_id, priority_json_str) in &rows {
        let priority_obj: serde_json::Value =
            serde_json::from_str(priority_json_str).unwrap_or_else(|_| serde_json::json!({}));
        let session_touches = priority_obj
            .get("session_touches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if session_touches > 0 {
            active_vaults.insert(vault_id.clone());
        }
    }

    // Pass 2: Roll over and recalculate scores with vault-relative context.
    let mut updated_count: usize = 0;

    for (id, vault_id, priority_json_str) in &rows {
        let priority_obj: serde_json::Value =
            serde_json::from_str(priority_json_str).unwrap_or_else(|_| serde_json::json!({}));

        let vault_is_active = active_vaults.contains(vault_id);
        let mut priority_obj = priority::calculate_rollover(priority_obj, vault_is_active);

        let profile = priority_obj
            .get("profile")
            .and_then(|v| v.as_str())
            .unwrap_or("standard");

        let access_30d = priority_obj
            .get("access_count_30active")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let link_count = priority_obj
            .get("link_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let new_score = priority::calculate_score(access_30d, link_count, profile);
        priority_obj["score"] = serde_json::json!(new_score);

        let updated_json = serde_json::to_string(&priority_obj)
            .map_err(|err| format!("Failed serializing priority for node {id}: {err}"))?;

        conn.execute(
            "UPDATE nodes SET priority = ?2, updated_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL;",
            params![id, updated_json],
        )
        .map_err(|err| format!("Failed updating priority for node {id}: {err}"))?;

        updated_count += 1;
    }

    Ok(updated_count)
}

// MARK: Public API

pub(crate) struct DbState {
    pub(crate) db_path: PathBuf,
    pub(crate) redacted_session_key: Mutex<Option<redacted::SessionKey>>,
    pub(crate) embed_job: Arc<Mutex<Option<embed::EmbedJobHandle>>>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum IpcResponse<T> {
    Ok { ok: T },
    Err { err: String },
}

pub type AppError = String;

pub fn check_rate_limit(key: &str) -> Result<(), String> {
    if key == "memory_agent" {
        let limiter = MEMORY_AGENT_LIMITER.get_or_init(|| {
            let quota = match governor::Quota::with_period(std::time::Duration::from_secs(10)) {
                Some(q) => q,
                None => {
                    // Fall back to a standard rate limit of 1 per second in the impossible event
                    // that with_period fails, avoiding expect/unwrap entirely for Clippy.
                    let fallback_nonzero = std::num::NonZeroU32::MIN;
                    governor::Quota::per_second(fallback_nonzero)
                }
            };
            governor::RateLimiter::direct(quota)
        });

        if limiter.check().is_err() {
            return Err(
                "Rate limit exceeded for memory extraction. Please wait before running it again."
                    .to_string(),
            );
        }
    }
    if key == "embedding" {
        let limiter = EMBEDDING_LIMITER.get_or_init(|| {
            let quota = match governor::Quota::with_period(std::time::Duration::from_secs(5)) {
                Some(q) => q,
                None => {
                    let fallback_nonzero = std::num::NonZeroU32::MIN;
                    governor::Quota::per_second(fallback_nonzero)
                }
            };
            governor::RateLimiter::direct(quota)
        });

        if limiter.check().is_err() {
            return Err(
                "Rate limit exceeded for embedding operations. Please wait before starting another embedding job."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn with_embed_job_lock<T, F>(
    embed_job: &Arc<Mutex<Option<embed::EmbedJobHandle>>>,
    f: F,
) -> Result<T, String>
where
    F: FnOnce(&mut Option<embed::EmbedJobHandle>) -> T,
{
    let mut guard = embed_job
        .lock()
        .map_err(|_| "Embedding job state lock is poisoned; restart the app.".to_string())?;
    Ok(f(&mut guard))
}

pub fn build_embedding_status(
    conn: &Connection,
    reembed_in_progress: bool,
) -> Result<EmbeddingStatus, String> {
    let settings = embed::get_embedding_settings(conn)?;
    let (embedded, total) = embed::storage::count_coverage(conn, &settings.model)?;
    let coverage_percent = if total == 0 {
        0.0
    } else {
        (embedded as f64 / total as f64) * 100.0
    };
    let jaccard_fallback_active = if settings.backend.eq_ignore_ascii_case("onnx") {
        if let Ok(paths) = embed::model_artifact_paths(&settings.model) {
            !paths.onnx.exists() || !paths.tokenizer.exists()
        } else {
            true
        }
    } else {
        false
    };

    Ok(EmbeddingStatus {
        model: settings.model,
        tier: settings.tier,
        backend: settings.backend,
        coverage_percent,
        last_computed_at: settings.last_computed_at,
        jaccard_fallback_active,
        reembed_in_progress,
    })
}

fn build_embed_engine_from_config(
    conn: &Connection,
    settings: &embed::EmbeddingSettings,
) -> Result<Box<dyn embed::EmbedEngine>, embed::EmbedError> {
    match settings.backend.to_ascii_lowercase().as_str() {
        "onnx" => {
            let config = embed::chunking_config_for_settings(settings).map_err(|err| {
                embed::EmbedError::InferenceFailed(format!("embedding ONNX config failed: {err}"))
            })?;
            Ok(Box::new(embed::BundledEmbedEngine::new(
                settings.model.clone(),
                config.dims,
            )?))
        }
        "ollama" => {
            let registry = embed::load_registry().map_err(|err| {
                embed::EmbedError::InferenceFailed(format!("embedding registry load failed: {err}"))
            })?;
            if settings.model != registry.ollama_default.model_id {
                return Err(embed::EmbedError::InferenceFailed(format!(
                    "embedding.model '{}' does not match Ollama default model '{}'",
                    settings.model, registry.ollama_default.model_id
                )));
            }
            let endpoint = embed::get_local_model_endpoint(conn).map_err(|err| {
                embed::EmbedError::InferenceFailed(format!(
                    "local model endpoint read failed: {err}"
                ))
            })?;
            Ok(Box::new(embed::OllamaEmbedEngine::new(
                endpoint,
                settings.model.clone(),
                registry.ollama_default.dims,
            )))
        }
        other => Err(embed::EmbedError::InferenceFailed(format!(
            "unsupported embedding backend: {other}"
        ))),
    }
}

fn build_embed_engine_from_settings(
    conn: &Connection,
) -> Result<Box<dyn embed::EmbedEngine>, embed::EmbedError> {
    let settings = embed::get_embedding_settings(conn).map_err(|err| {
        embed::EmbedError::InferenceFailed(format!("embedding settings read failed: {err}"))
    })?;
    build_embed_engine_from_config(conn, &settings)
}

struct SingleNodeEmbedTask {
    db_path: PathBuf,
    node_id: String,
    is_unlocked: bool,
}

static SINGLE_NODE_EMBED_TX: std::sync::OnceLock<std::sync::mpsc::Sender<SingleNodeEmbedTask>> =
    std::sync::OnceLock::new();

fn spawn_single_node_embedding(db_path: PathBuf, node_id: String, is_unlocked: bool) {
    let tx = SINGLE_NODE_EMBED_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<SingleNodeEmbedTask>();

        std::thread::spawn(move || {
            let mut cached_conn: Option<(PathBuf, Connection)> = None;
            let mut cached_engine: Option<(embed::EmbeddingSettings, Box<dyn embed::EmbedEngine>)> =
                None;

            for task in rx {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), String> {
                        let conn_is_valid = match &cached_conn {
                            Some((path, _)) => path == &task.db_path,
                            None => false,
                        };
                        if !conn_is_valid {
                            let new_conn = open_connection(&task.db_path)?;
                            cached_conn = Some((task.db_path.clone(), new_conn));
                        }
                        let conn = &mut cached_conn
                            .as_mut()
                            .ok_or_else(|| "Connection caching logic error".to_string())?
                            .1;

                        let current_settings =
                            embed::get_embedding_settings(conn).map_err(|err| err.to_string())?;

                        let need_rebuild = match &cached_engine {
                            Some((settings, _)) => {
                                settings.model != current_settings.model
                                    || settings.backend != current_settings.backend
                                    || settings.tier != current_settings.tier
                            }
                            None => true,
                        };

                        if need_rebuild {
                            let engine = build_embed_engine_from_config(conn, &current_settings)
                                .map_err(|err| err.to_string())?;
                            cached_engine = Some((current_settings, engine));
                        }

                        let (_, engine) = cached_engine
                            .as_ref()
                            .ok_or_else(|| "Engine caching logic error".to_string())?;

                        let cancel = AtomicBool::new(false);
                        let embedded = embed::embed_node(
                            conn,
                            &task.node_id,
                            engine.as_ref(),
                            &cancel,
                            task.is_unlocked,
                        )
                        .map_err(|err| err.to_string())?;

                        if embedded {
                            let now_iso = chrono::Utc::now().to_rfc3339();
                            let _ = embed::set_embedding_last_computed_at(conn, &now_iso);
                        }

                        Ok(())
                    },
                ));

                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        eprintln!(
                            "[embed] single-node embedding failed for {}: {err}",
                            task.node_id
                        );
                        cached_conn = None;
                        cached_engine = None;
                    }
                    Err(_) => {
                        eprintln!(
                            "[embed] single-node embedding panicked for {}",
                            task.node_id
                        );
                        cached_conn = None;
                        cached_engine = None;
                    }
                }
            }
        });

        tx
    });

    if let Err(err) = tx.send(SingleNodeEmbedTask {
        db_path,
        node_id,
        is_unlocked,
    }) {
        eprintln!("[embed] failed to send task to single-node embedding queue: {err}");
    }
}

fn clear_matching_embed_job(
    embed_job: &Arc<Mutex<Option<embed::EmbedJobHandle>>>,
    cancel: &Arc<AtomicBool>,
) {
    match embed_job.lock() {
        Ok(mut guard) => {
            if let Some(active_handle) = guard.as_ref() {
                if Arc::ptr_eq(&active_handle.cancel, cancel) {
                    *guard = None;
                }
            }
        }
        Err(_) => {
            eprintln!("[re-embed] embed_job lock poisoned during worker cleanup");
        }
    }
}

pub fn is_node_private(conn: &Connection, node_id: &str) -> Result<bool, String> {
    let node_info = conn.query_row(
        "SELECT n.vault_id, n.sub_vault_id, COALESCE(o.privacy_tier, n.privacy_tier)
             FROM nodes n
             LEFT JOIN privacy_overrides o ON n.id = o.node_id
             WHERE n.id = ?1 AND n.deleted_at IS NULL
             LIMIT 1;",
        [node_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );

    let (vault_id, sub_vault_id, privacy_tier) = match node_info {
        Ok(info) => info,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false), // Node doesn't exist or is deleted
        Err(err) => return Err(format!("Database error in is_node_private: {err}")),
    };

    let effective = resolve_node_effective_privacy(
        conn,
        &vault_id,
        sub_vault_id.as_deref(),
        privacy_tier.as_deref(),
    )?;

    Ok(effective == "redacted" || effective == "locked")
}

pub fn log_memory_agent_error(conn: &Connection, raw_response: &str) -> Result<(), String> {
    let existing_str: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'memory_agent_errors' LIMIT 1;",
            [],
            |row| row.get(0),
        )
        .ok();

    let mut errors: Vec<String> = match existing_str {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => Vec::new(),
    };

    errors.push(raw_response.to_string());

    if errors.len() > 5 {
        let skip_count = errors.len() - 5;
        errors = errors.into_iter().skip(skip_count).collect();
    }

    let new_str = serde_json::to_string(&errors)
        .map_err(|err| format!("Failed to serialize memory agent errors: {err}"))?;

    conn.execute(
        "INSERT INTO settings (key, value, scope, updated_at)
         VALUES ('memory_agent_errors', ?1, 'global', datetime('now'))
         ON CONFLICT(key) DO UPDATE
         SET value = excluded.value,
             updated_at = datetime('now');",
        [new_str],
    )
    .map_err(|err| format!("Failed to write memory_agent_errors setting: {err}"))?;

    Ok(())
}

pub fn minimal_pre_write_backup(
    conn: &Connection,
    db_path: &Path,
    reason: &str,
) -> Result<PathBuf, String> {
    let parent = db_path
        .parent()
        .ok_or_else(|| format!("Database path has no parent: {}", db_path.display()))?;
    let backups_dir = parent.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|err| {
        format!(
            "Failed creating backups directory {}: {err}",
            backups_dir.display()
        )
    })?;
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("System clock before UNIX_EPOCH: {err}"))?
        .as_millis();
    let backup_path = backups_dir.join(format!("mindvault-pre-{reason}-{now_unix}.db"));

    let mut backup_conn = Connection::open(&backup_path)
        .map_err(|err| format!("Failed to open DB for backup: {err}"))?;

    let backup = rusqlite::backup::Backup::new(conn, &mut backup_conn)
        .map_err(|err| format!("Failed to initialize backup: {err}"))?;

    backup.step(-1).map_err(|err| {
        format!(
            "Failed creating pre-write backup {} -> {}: {err}",
            db_path.display(),
            backup_path.display()
        )
    })?;

    // Apply retention policy: keep only the most recent 10 pre-write backups globally
    let _ = enforce_backup_retention(&backups_dir, 10);

    Ok(backup_path)
}

pub fn enforce_backup_retention(backups_dir: &Path, max_backups: usize) -> Result<(), String> {
    let mut files = vec![];

    let entries =
        fs::read_dir(backups_dir).map_err(|e| format!("Failed to read backups directory: {e}"))?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("mindvault-pre-") && name_str.ends_with(".db") {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    files.push((modified, entry.path(), metadata.len()));
                }
            }
        }
    }

    // Sort descending by modified time (newest first)
    files.sort_by_key(|b| std::cmp::Reverse(b.0));

    let max_size = 50 * 1024 * 1024; // 50 MB
    let mut current_size = 0u64;
    let mut current_count = 0usize;
    let mut limit_exceeded = false;

    for (index, (_modified, path, size)) in files.iter().enumerate() {
        if index == 0 {
            // Always keep at least the newest backup file
            current_size += size;
            current_count += 1;
            continue;
        }

        if limit_exceeded || current_size + size > max_size || current_count + 1 > max_backups {
            limit_exceeded = true;
            let _ = fs::remove_file(path);
        } else {
            current_size += size;
            current_count += 1;
        }
    }

    Ok(())
}

pub fn ensure_onboarding_vault_exists(conn: &Connection, vault_id: &str) -> Result<(), String> {
    if fetch_vault_by_id(conn, vault_id, None).is_ok() {
        return Ok(());
    }
    let Some((name, icon, description, privacy_tier, priority_profile, meta, sort_order)) =
        onboarding_default_vault_spec(vault_id)
    else {
        return Err(format!(
            "Invalid onboarding commit vault_id '{vault_id}': no matching vault and not a known onboarding default"
        ));
    };

    let revived = conn
        .execute(
            "UPDATE vaults
             SET deleted_at = NULL,
                 updated_at = datetime('now')
             WHERE id = ?1;",
            [vault_id],
        )
        .map_err(|err| format!("Failed reviving onboarding vault '{vault_id}': {err}"))?;
    if revived > 0 {
        return fetch_vault_by_id(conn, vault_id, None)
            .map(|_| ())
            .map_err(|err| {
                format!("Failed fetching revived onboarding vault '{vault_id}': {err}")
            });
    }

    conn.execute(
        "INSERT OR IGNORE INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);",
        params![
            vault_id,
            name,
            icon,
            description,
            privacy_tier,
            priority_profile,
            sort_order,
            meta
        ],
    )
    .map_err(|err| format!("Failed creating missing onboarding vault '{vault_id}': {err}"))?;

    fetch_vault_by_id(conn, vault_id, None)
        .map(|_| ())
        .map_err(|err| {
            format!("Failed verifying onboarding vault '{vault_id}' after ensure step: {err}")
        })
}

pub fn execute_onboarding_commit(
    proposals: &[OnboardingNodeCommitInput],
    db_path: &Path,
) -> Result<bool, String> {
    (|| {
        let mut conn = open_connection(db_path)?;
        let mut validated_proposals = Vec::with_capacity(proposals.len());

        // 1. Validate all proposals first
        for proposal in proposals {
            let vault_id = proposal.vault_id.trim();
            if vault_id.is_empty() {
                return Err("Onboarding commit row is missing vault_id".to_string());
            }

            let title = proposal.title.trim();
            if title.is_empty() {
                return Err("Onboarding commit row has empty title".to_string());
            }

            let summary = proposal.summary.trim();
            if summary.is_empty() {
                return Err("Onboarding commit row has empty summary".to_string());
            }

            let node_type = proposal
                .node_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("concept");

            let valid_node_types = [
                "concept",
                "fact",
                "project",
                "preference",
                "event",
                "instruction",
                "identity",
                "summary",
            ];
            if !valid_node_types.contains(&node_type) {
                return Err(format!(
                    "Invalid node_type '{}'. Must be one of {:?}",
                    node_type, valid_node_types
                ));
            }

            let source_type = proposal
                .source_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("onboarding");

            let valid_source_types = [
                "manual",
                "pdf_import",
                "transcript_import",
                "ai_transfer",
                "agent_extract",
                "onboarding",
            ];
            if !valid_source_types.contains(&source_type) {
                return Err(format!(
                    "Invalid source_type '{}'. Must be one of {:?}",
                    source_type, valid_source_types
                ));
            }
            ensure_onboarding_vault_exists(&conn, vault_id)?;

            let detail = proposal
                .detail
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(String::from);

            validated_proposals.push(ValidatedProposal {
                vault_id,
                title,
                summary,
                detail,
                node_type,
                source_type,
                tags: proposal.tags.as_ref(),
            });
        }

        // 2. Take pre-write backup (expensive, only run if payload is completely valid)
        if !proposals.is_empty() {
            let _ = minimal_pre_write_backup(&conn, db_path, "onboarding-commit")?;
        }

        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting onboarding_commit transaction: {err}"))?;

        // 3. Process and write
        for proposal in validated_proposals {
            insert_onboarding_node(&tx, &proposal)?;
        }

        tx.execute(
            "INSERT INTO settings (key, value, scope, updated_at)
             VALUES ('onboarding_complete', 'true', 'global', datetime('now'))
             ON CONFLICT(key) DO UPDATE
             SET value = excluded.value,
                 scope = excluded.scope,
                 updated_at = datetime('now');",
            [],
        )
        .map_err(|err| format!("Failed setting onboarding_complete=true: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing onboarding_commit: {err}"))?;

        Ok(true)
    })()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::<tauri::Wry>::default()
        .setup(|app| {
            let db_path = sqlite_db_path(app)?;
            let mut conn = open_connection(&db_path)
                .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
            run_migrations(&mut conn)
                .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
            run_seed_data(&mut conn).map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
            app.manage(DbState {
                db_path: db_path.clone(),
                redacted_session_key: Mutex::new(None),
                embed_job: Arc::new(Mutex::new(None)),
            });

            let bg_path = db_path;
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(24 * 60 * 60));
                match run_priority_refresh(&bg_path) {
                    Ok(count) => {
                        if count > 0 {
                            eprintln!("[priority] refreshed {count} node(s)");
                        }
                    }
                    Err(err) => {
                        eprintln!("[priority] background refresh failed: {err}");
                    }
                }
            });
            chat::purge_temporary_session(&conn)?;
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            db_ping,
            settings_get,
            settings_set,
            embedding_get_status,
            embedding_reembed_start,
            embedding_reembed_cancel,
            chat_get_history,
            chat_append_message,
            chat_clear_history,
            chat_edit_and_truncate,
            chat_set_off_the_record,
            chat_is_off_the_record,
            chat_convert_temporary_to_memory,
            chat_list_sessions,
            chat_create_session,
            chat_delete_session,
            chat_update_session_summary,
            vault_create,
            vault_list,
            vault_delete,
            vault_update,
            vault_update_position,
            vault_update_color_theme,
            vault_get,
            node_create,
            node_get,
            node_list,
            node_update,
            node_delete,
            node_touch,
            tag_list,
            tag_create,
            node_tags_get,
            node_tag_add,
            node_tag_remove,
            door_create,
            door_list_outgoing,
            door_list_incoming,
            door_list_all,
            door_delete,
            door_repoint,
            auth::auth_secret_is_setup,
            auth::auth_secret_set,
            auth::auth_secret_verify,
            priority_refresh_all,
            priority_optimize_all,
            debug_assemble_context,
            llm_count_tokens,
            llm_list_models,
            llm_chat,
            onboarding_extract_proposals,
            onboarding_commit,
            save_markdown_file,
            memory_extract,
            memory_extract_if_ready,
            memory_extract_force,
            changeset_count_pending,
            changeset_list_pending,
            changeset_list_items,
            changeset_commit,
            changeset_list_resolved,
            debug_seed_changeset
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("error while running tauri application: {err}");
            std::process::exit(1);
        });
}

pub(crate) fn into_ipc<T>(result: Result<T, String>) -> IpcResponse<T> {
    match result {
        Ok(value) => IpcResponse::Ok { ok: value },
        Err(err) => IpcResponse::Err { err },
    }
}

pub(crate) fn open_connection(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path)
        .map_err(|err| format!("Failed opening database {}: {err}", db_path.display()))?;
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA trusted_schema = OFF;
        PRAGMA defensive = ON;
        ",
    )
    .map_err(|err| format!("Failed setting SQLite pragmas: {err}"))?;
    Ok(conn)
}

pub(crate) fn generate_id(conn: &Connection, prefix: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT ?1 || '_' || lower(hex(randomblob(8)));",
        [prefix],
        |row| row.get(0),
    )
    .map_err(|err| format!("Failed generating id: {err}"))
}

pub(crate) fn resolve_vault_effective_privacy(
    conn: &Connection,
    vault_id: &str,
) -> Result<String, String> {
    let mut current_id = Some(vault_id.to_string());
    let mut strictest = "open".to_string();

    while let Some(id) = current_id {
        let record = conn
            .query_row(
                "SELECT parent_vault_id, privacy_tier
                 FROM (
                    SELECT id, NULL AS parent_vault_id, privacy_tier
                    FROM vaults
                    WHERE deleted_at IS NULL
                    UNION ALL
                    SELECT id, vault_id AS parent_vault_id, COALESCE(privacy_tier, 'open') AS privacy_tier
                    FROM sub_vaults
                    WHERE deleted_at IS NULL
                 )
                 WHERE id = ?1
                 LIMIT 1;",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                    ))
                },
            )
            .map_err(|err| format!("Failed resolving vault privacy for {vault_id}: {err}"))?;

        strictest =
            privacy::get_effective_privacy(Some(record.1.as_str()), None, Some(strictest.as_str()))
                .to_string();
        current_id = record.0;
    }

    Ok(strictest)
}

pub(crate) fn resolve_node_effective_privacy(
    conn: &Connection,
    vault_id: &str,
    sub_vault_id: Option<&str>,
    node_privacy_tier: Option<&str>,
) -> Result<String, String> {
    let container_tier = if let Some(sub_vault_id) = sub_vault_id {
        resolve_vault_effective_privacy(conn, sub_vault_id)?
    } else {
        resolve_vault_effective_privacy(conn, vault_id)?
    };

    Ok(
        privacy::get_effective_privacy(node_privacy_tier, None, Some(container_tier.as_str()))
            .to_string(),
    )
}

pub async fn execute_memory_extraction_pipeline(
    provider: String,
    endpoint: String,
    model: String,
    db_path: PathBuf,
    correction_signal: Option<memory_agent::CorrectionSignal>,
) -> Result<Changeset, String> {
    // 1. Load and filter chat history synchronously within scoped block to drop connection before await
    let chat_history = {
        let conn = open_connection(&db_path)?;

        let mut stmt = conn
            .prepare(
                "SELECT id, role, content, node_refs, created_at
                 FROM session_messages
                 WHERE session_id = 'default-session'
                 ORDER BY created_at ASC, rowid ASC;",
            )
            .map_err(|err| format!("Failed preparing session_messages query: {err}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|err| format!("Failed querying session_messages: {err}"))?;

        let mut raw_messages = Vec::new();
        for r in rows {
            raw_messages.push(r.map_err(|err| format!("Failed reading message row: {err}"))?);
        }

        // Collect all unique node IDs referenced in the messages
        let mut unique_node_ids = std::collections::HashSet::new();
        for (_, _, _, node_refs_json, _) in &raw_messages {
            let node_ids: Vec<String> = serde_json::from_str(node_refs_json).unwrap_or_default();
            for id in node_ids {
                if !id.trim().is_empty() {
                    unique_node_ids.insert(id);
                }
            }
        }

        // Load all vaults/sub-vaults into a local cache map in a single query
        let mut vault_map = std::collections::HashMap::new();
        let mut vault_stmt = conn
            .prepare(
                "SELECT id, NULL AS parent_vault_id, privacy_tier FROM vaults WHERE deleted_at IS NULL
                 UNION ALL
                 SELECT id, vault_id AS parent_vault_id, COALESCE(privacy_tier, 'open') AS privacy_tier
                 FROM sub_vaults WHERE deleted_at IS NULL;",
            )
            .map_err(|err| format!("Failed preparing vaults union query: {err}"))?;

        let vault_rows = vault_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|err| format!("Failed querying vaults union: {err}"))?;

        for r in vault_rows {
            let (id, parent_id, tier) =
                r.map_err(|err| format!("Failed reading vault row: {err}"))?;
            vault_map.insert(id, (parent_id, tier));
        }

        // Resolves a vault effective privacy tier using the in-memory cache
        // Query the privacy parameters of all unique referenced nodes in batched chunks.
        let private_nodes = fetch_private_referenced_nodes(&conn, &unique_node_ids, &vault_map)?;

        // Filter messages in memory using the fast private-nodes HashSet lookup
        let mut filtered_history = Vec::new();
        for (id, role, content, node_refs_json, created_at) in raw_messages {
            let node_ids: Vec<String> = serde_json::from_str(&node_refs_json).unwrap_or_default();
            let mut has_private_ref = false;
            for node_id in &node_ids {
                if private_nodes.contains(node_id) {
                    has_private_ref = true;
                    break;
                }
            }
            if !has_private_ref {
                filtered_history.push(chat::ChatMessage {
                    id,
                    role,
                    content,
                    created_at,
                });
            }
        }

        if filtered_history.len() < 3 {
            return Err(
                "Insufficient chat history (need at least 3 messages) to extract memory."
                    .to_string(),
            );
        }
        filtered_history
    };

    // 2. Format conversation
    let mut conversation_text = String::new();
    for msg in &chat_history {
        conversation_text.push_str(&format!("{}: {}\n", msg.role, msg.content));
    }
    let user_content = format!("<conversation>\n{}\n</conversation>", conversation_text);

    // 3. Build LLM message
    let messages = [llm::client::LlmMessage {
        role: "user".to_string(),
        content: user_content,
    }];

    // 4. Resolve provider
    let parsed_provider = match provider.trim().to_lowercase().as_str() {
        "ollama" => llm::client::LlmProvider::Ollama,
        "lmstudio" => llm::client::LlmProvider::LmStudio,
        "anthropic" => llm::client::LlmProvider::Anthropic,
        "openai" => llm::client::LlmProvider::OpenAi,
        "google" => llm::client::LlmProvider::Google,
        "xai" => llm::client::LlmProvider::XAi,
        _ => return Err("Unsupported provider. Use 'ollama', 'lmstudio', 'anthropic', 'openai', 'google', or 'xai'.".to_string()),
    };

    // 5. Call UniversalClient::complete
    let client = llm::client::UniversalClient::new(
        parsed_provider,
        endpoint.trim().to_string(),
        model.trim().to_string(),
    );

    let raw = llm::client::LlmClient::complete(
        &client,
        memory_agent::prompt::MEMORY_EXTRACTION_SYSTEM_PROMPT,
        &messages,
    )
    .await?;

    // 6. Parse response
    let candidates = match memory_agent::parser::parse_candidates_from_llm_output(&raw) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Failed to parse candidates JSON, logging raw response and recovering gracefully: {err}");
            // Reuse a single connection for logging, persisting, and querying
            let mut conn = open_connection(&db_path)?;
            if let Err(log_err) = log_memory_agent_error(&conn, &raw) {
                eprintln!("Failed to log memory agent error: {log_err}");
            }
            // Persist an empty changeset gracefully
            let tx = conn
                .transaction()
                .map_err(|err| format!("Failed to start transaction: {err}"))?;

            let pending_changeset = memory_agent::PendingChangeset {
                session_id: "default-session".to_string(),
                model_used: Some(model.clone()),
                items: Vec::new(),
            };

            let changeset_id = memory_agent::persistence::persist_changeset(
                &tx,
                &pending_changeset,
                Some(&model),
            )?;

            tx.commit()
                .map_err(|err| format!("Failed to commit transaction: {err}"))?;

            // Retrieve the newly persisted empty changeset
            let cs = memory_agent::persistence::get_changeset_by_id(&conn, &changeset_id)?;

            return Ok(cs);
        }
    };

    // 7. Reuse a single connection for changeset build, persist, and retrieval
    let mut conn = open_connection(&db_path)?;
    let embed_engine = build_embed_engine_from_settings(&conn).ok();

    let changeset_id: String = if let Some(ref signal) = correction_signal {
        let (id, _amended) = memory_agent::amendment::amend_or_create_changeset(
            &mut conn,
            &candidates,
            "default-session",
            &model,
            signal,
            embed_engine.as_deref(),
        )?;
        id
    } else {
        let pending_changeset = memory_agent::changeset::build_changeset(
            &conn,
            &candidates,
            "default-session",
            embed_engine.as_deref(),
        )?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed to start transaction: {err}"))?;
        let persisted_id =
            memory_agent::persistence::persist_changeset(&tx, &pending_changeset, Some(&model))?;
        tx.commit()
            .map_err(|err| format!("Failed to commit transaction: {err}"))?;
        persisted_id
    };

    // 8. Retrieve the newly persisted Changeset (reusing same connection)
    let cs = memory_agent::persistence::get_changeset_by_id(&conn, &changeset_id)?;

    Ok(cs)
}

pub async fn test_helper_memory_extract_force(
    provider: String,
    endpoint: String,
    model: String,
    db_path: std::path::PathBuf,
) -> Result<ipc_types::Changeset, String> {
    use tauri::Manager;
    let app = tauri::test::mock_app();
    app.manage(AppState {
        db_path,
        redacted_session_key: std::sync::Mutex::new(None),
        embed_job: Arc::new(std::sync::Mutex::new(None)),
    });
    let state = app.state::<AppState>();
    memory_extract_force(provider, endpoint, model, state).await
}

pub fn test_helper_init_embedding_db(db_path: std::path::PathBuf) -> Result<(), String> {
    let mut conn = open_connection(&db_path)?;
    run_migrations(&mut conn)?;
    run_seed_data(&mut conn)
}

pub fn test_helper_run_migrations(db_path: std::path::PathBuf) -> Result<(), String> {
    let mut conn = open_connection(&db_path)?;
    run_migrations(&mut conn)
}

pub fn test_helper_embedding_get_status(
    db_path: std::path::PathBuf,
) -> Result<EmbeddingStatus, String> {
    use tauri::Manager;
    let app = tauri::test::mock_app();
    app.manage(AppState {
        db_path,
        redacted_session_key: std::sync::Mutex::new(None),
        embed_job: Arc::new(std::sync::Mutex::new(None)),
    });
    let state = app.state::<AppState>();
    match embedding_get_status(state) {
        IpcResponse::Ok { ok } => Ok(ok),
        IpcResponse::Err { err } => Err(err),
    }
}

pub fn test_helper_embedding_reembed_cancel(db_path: std::path::PathBuf) -> Result<bool, String> {
    use tauri::Manager;
    let cancel = Arc::new(AtomicBool::new(false));
    let app = tauri::test::mock_app();
    app.manage(AppState {
        db_path,
        redacted_session_key: std::sync::Mutex::new(None),
        embed_job: Arc::new(std::sync::Mutex::new(Some(embed::EmbedJobHandle {
            cancel: Arc::clone(&cancel),
        }))),
    });
    let state = app.state::<AppState>();
    match embedding_reembed_cancel(state) {
        IpcResponse::Ok { ok: () } => Ok(cancel.load(Ordering::Relaxed)),
        IpcResponse::Err { err } => Err(err),
    }
}

pub fn test_helper_settings_set(
    db_path: std::path::PathBuf,
    key: String,
    value: String,
) -> Result<bool, String> {
    use tauri::Manager;
    let app = tauri::test::mock_app();
    app.manage(AppState {
        db_path,
        redacted_session_key: std::sync::Mutex::new(None),
        embed_job: Arc::new(std::sync::Mutex::new(None)),
    });
    let state = app.state::<AppState>();
    match settings_set(key, value, state) {
        IpcResponse::Ok { ok } => Ok(ok),
        IpcResponse::Err { err } => Err(err),
    }
}

// MARK: Tauri Commands

#[tauri::command]
fn greet(name: &str) -> IpcResponse<String> {
    IpcResponse::Ok {
        ok: format!("Hello, {}! You've been greeted from MindVault!", name),
    }
}

#[tauri::command]
fn debug_seed_changeset(state: tauri::State<'_, DbState>) -> IpcResponse<bool> {
    #[cfg(not(debug_assertions))]
    {
        let _ = state;
        IpcResponse::Err {
            err: "Debug seed command is disabled in production builds.".to_string(),
        }
    }

    #[cfg(debug_assertions)]
    {
        let mut conn = match open_connection(&state.db_path) {
            Ok(c) => c,
            Err(e) => return IpcResponse::Err { err: e },
        };

        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(e) => return IpcResponse::Err { err: e.to_string() },
        };

        let run_seed = || -> Result<(), rusqlite::Error> {
            // Seed nodes
            tx.execute(
                "INSERT OR REPLACE INTO nodes (id, vault_id, title, summary, detail, node_type, version) VALUES \
                ('node_update_test', 'vault_personal', 'Favorite Programming Language', 'User likes Rust programming', \
                'Rust is a systems programming language focusing on safety and speed.', 'concept', 1);",
                [],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO nodes (id, vault_id, title, summary, detail, node_type, version) VALUES \
                ('node_merge_test', 'vault_personal', 'MindVault Architectural Design', 'MindVault uses SQLite and Rust.', \
                'The storage model uses rusqlite in the core backend.', 'concept', 1);",
                [],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO nodes (id, vault_id, title, summary, detail, node_type, version) VALUES \
                ('node_delete_test', 'vault_personal', 'Temporary Scrap Node', 'This is a temporary scrap memory node.', \
                'This node will be soft-deleted in tests.', 'concept', 1);",
                [],
            )?;

            // Seed door
            tx.execute(
                "INSERT OR REPLACE INTO doors (id, source_node_id, target_node_id, status) VALUES \
                ('door_orphan_test', 'node_update_test', 'node_delete_test', 'orphaned');",
                [],
            )?;

            // Seed changeset
            tx.execute(
                "INSERT OR REPLACE INTO changesets (id, session_id, status, item_count, accepted_count, dismissed_count, model_used) VALUES \
                ('cs_debug_all', 'test-session', 'pending', 5, 0, 0, 'dev-seed-model');",
                [],
            )?;

            // Seed changeset items
            tx.execute(
                "INSERT OR REPLACE INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, door_id, status) VALUES \
                ('item_add', 'cs_debug_all', 'add', NULL, \
                '{\"title\":\"Indefinite Integral of x^3 * e^(x^2)\",\"summary\":\"Math exploration\",\"detail\":\"The user solved the integral using integration by parts.\",\"tags\":[\"math\",\"calculus\"],\"vaultId\":\"vault_personal\"}', \
                '{}', NULL, NULL, NULL, 'pending');",
                [],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, door_id, status) VALUES \
                ('item_update', 'cs_debug_all', 'update', 'node_update_test', \
                '{\"title\":\"My Favorite Programming Language\",\"summary\":\"User highly prefers Rust coding\",\"detail\":\"Rust is a systems programming language focusing on memory safety, speed, and concurrency.\",\"tags\":[\"rust\",\"systems\",\"coding\"],\"vaultId\":\"vault_personal\"}', \
                '{\"title\":\"Favorite Programming Language\",\"summary\":\"User likes Rust programming\",\"detail\":\"Rust is a systems programming language focusing on safety and speed.\",\"tags\":[\"rust\",\"coding\"],\"vaultId\":\"vault_personal\"}', \
                0.85, NULL, NULL, 'pending');",
                [],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, door_id, status) VALUES \
                ('item_merge', 'cs_debug_all', 'merge', 'node_merge_test', \
                '{\"detail\":\"Memory Agent triggers debounced runs and compiles changesets into transactional items.\",\"tags\":[\"tauri\",\"architecture\"]}', \
                '{\"title\":\"MindVault Architectural Design\",\"summary\":\"MindVault uses SQLite and Rust.\",\"detail\":\"The storage model uses rusqlite in the core backend.\",\"tags\":[\"sqlite\",\"architecture\"],\"vaultId\":\"vault_personal\"}', \
                0.6, NULL, NULL, 'pending');",
                [],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, door_id, status) VALUES \
                ('item_delete', 'cs_debug_all', 'delete', 'node_delete_test', \
                '{\"summary\":\"The temporary scrap memory node is no longer needed since the session has ended.\"}', \
                '{\"title\":\"Temporary Scrap Node\",\"summary\":\"This is a temporary scrap memory node.\",\"tags\":[\"scrap\"],\"vaultId\":\"vault_personal\"}', \
                1.0, NULL, NULL, 'pending');",
                [],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO changeset_items (id, changeset_id, item_type, target_node_id, proposed_data, existing_data, similarity, merge_with_id, door_id, status) VALUES \
                ('item_orphan', 'cs_debug_all', 'repoint_door', 'node_update_test', \
                '{\"doorId\":\"door_orphan_test\",\"targetNodeId\":\"node_update_test\"}', \
                '{}', NULL, NULL, 'door_orphan_test', 'pending');",
                [],
            )?;

            Ok(())
        };

        match run_seed() {
            Ok(_) => {
                if let Err(e) = tx.commit() {
                    IpcResponse::Err { err: e.to_string() }
                } else {
                    IpcResponse::Ok { ok: true }
                }
            }
            Err(e) => IpcResponse::Err { err: e.to_string() },
        }
    }
}

#[tauri::command]
fn changeset_count_pending(state: tauri::State<'_, AppState>) -> Result<i64, String> {
    let conn = open_connection(&state.db_path)?;
    memory_agent::persistence::count_pending_items(&conn)
}

#[tauri::command]
fn changeset_list_pending(state: tauri::State<'_, AppState>) -> Result<Vec<Changeset>, String> {
    let conn = open_connection(&state.db_path)?;
    memory_agent::persistence::list_pending_changesets(&conn)
}

#[tauri::command]
fn changeset_list_items(
    changeset_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ChangesetItem>, String> {
    let conn = open_connection(&state.db_path)?;
    memory_agent::persistence::list_changeset_items(&conn, &changeset_id)
}

#[tauri::command]
fn changeset_list_resolved(state: tauri::State<'_, AppState>) -> Result<Vec<Changeset>, String> {
    let conn = open_connection(&state.db_path)?;
    memory_agent::persistence::list_resolved_changesets(&conn)
}

#[tauri::command]
fn db_ping(state: tauri::State<'_, DbState>) -> IpcResponse<String> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let version: String = conn
            .query_row("SELECT sqlite_version();", [], |row| row.get(0))
            .map_err(|err| format!("SQLite ping failed: {err}"))?;
        Ok(format!("SQLite connected (version {version})"))
    })())
}

#[tauri::command]
fn settings_get(key: String, state: tauri::State<'_, DbState>) -> IpcResponse<Option<String>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let value = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
                [key],
                |row| row.get::<_, String>(0),
            )
            .ok();

        if let Some(val) = value {
            // Attempt decryption if it looks like an encrypted payload and we have a session key
            if val.starts_with(r#"{"v":1,"alg":"aes-256-gcm""#) {
                if let Some(session_key) = redacted::get_session_key(&state) {
                    if let Ok(decrypted) = redacted::decrypt_json::<String>(&val, &session_key) {
                        return Ok(Some(decrypted));
                    }
                }
            }
            Ok(Some(val))
        } else {
            Ok(None)
        }
    })())
}

fn clean_setting_value(value: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<String>(value) {
        parsed
    } else {
        value.to_string()
    }
}

#[tauri::command]
fn settings_set(
    key: String,
    mut value: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let cleaned_value = clean_setting_value(&value);

        if key == "local_model_endpoint" {
            let parsed_url = reqwest::Url::parse(&cleaned_value)
                .map_err(|err| format!("Invalid local_model_endpoint URL: {err}"))?;
            if parsed_url.scheme() != "http" && parsed_url.scheme() != "https" {
                return Err("local_model_endpoint must be an HTTP or HTTPS URL".to_string());
            }
        }

        if key == "embedding.backend" {
            let val_lower = cleaned_value.trim().to_ascii_lowercase();
            if val_lower != "onnx" && val_lower != "ollama" {
                return Err(format!("Unsupported embedding backend: {cleaned_value}"));
            }

            let registry = embed::load_registry()
                .map_err(|err| format!("Failed to load embedding registry: {err}"))?;

            // If switching to ollama, ensure model matches ollama_default
            if val_lower == "ollama" {
                let current_model = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
                        [embed::EMBEDDING_MODEL_KEY],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .map(|v| clean_setting_value(&v));

                let needs_model_update = match current_model {
                    Some(m) => m != registry.ollama_default.model_id,
                    None => true,
                };
                if needs_model_update {
                    let encoded_model = serde_json::to_string(&registry.ollama_default.model_id)
                        .map_err(|err| format!("Failed to serialize model: {err}"))?;
                    conn.execute(
                        "INSERT INTO settings (key, value, scope, updated_at)
                         VALUES (?1, ?2, 'global', datetime('now'))
                         ON CONFLICT(key) DO UPDATE
                         SET value = excluded.value,
                             updated_at = datetime('now');",
                        params![embed::EMBEDDING_MODEL_KEY, encoded_model],
                    )
                    .map_err(|err| format!("Failed updating model setting: {err}"))?;
                }
            } else if val_lower == "onnx" {
                // If switching to onnx, ensure model is valid for the current tier
                let current_tier = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
                        [embed::EMBEDDING_TIER_KEY],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .map(|v| clean_setting_value(&v))
                    .unwrap_or_else(|| "light".to_string());

                let expected_model = match current_tier.as_str() {
                    "light" => &registry.tiers.light.model_id,
                    "standard" => &registry.tiers.standard.model_id,
                    "quality" => &registry.tiers.quality.model_id,
                    _ => &registry.tiers.light.model_id,
                };

                let encoded_model = serde_json::to_string(expected_model)
                    .map_err(|err| format!("Failed to serialize model: {err}"))?;
                conn.execute(
                    "INSERT INTO settings (key, value, scope, updated_at)
                     VALUES (?1, ?2, 'global', datetime('now'))
                     ON CONFLICT(key) DO UPDATE
                     SET value = excluded.value,
                         updated_at = datetime('now');",
                    params![embed::EMBEDDING_MODEL_KEY, encoded_model],
                )
                .map_err(|err| format!("Failed updating model setting: {err}"))?;
            }
        }

        if key == "embedding.model" {
            let backend = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
                    [embed::EMBEDDING_BACKEND_KEY],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .map(|v| clean_setting_value(&v))
                .unwrap_or_else(|| "onnx".to_string());

            let registry = embed::load_registry()
                .map_err(|err| format!("Failed to load embedding registry: {err}"))?;

            if backend == "ollama" {
                if cleaned_value != registry.ollama_default.model_id {
                    return Err(format!(
                        "embedding model '{}' is not valid for Ollama; expected '{}'",
                        cleaned_value, registry.ollama_default.model_id
                    ));
                }
            } else if backend == "onnx" {
                let valid_models = [
                    &registry.tiers.light.model_id,
                    &registry.tiers.standard.model_id,
                    &registry.tiers.quality.model_id,
                ];
                if !valid_models.contains(&&cleaned_value) {
                    return Err(format!(
                        "embedding model '{}' is not valid for ONNX",
                        cleaned_value
                    ));
                }
            }
        }

        if key == "embedding.tier" {
            let val_lower = cleaned_value.trim().to_ascii_lowercase();
            if val_lower != "light" && val_lower != "standard" && val_lower != "quality" {
                return Err(format!("Unsupported embedding tier: {cleaned_value}"));
            }

            // If backend is onnx, auto-update the model to match the new tier
            let backend = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
                    [embed::EMBEDDING_BACKEND_KEY],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .map(|v| clean_setting_value(&v))
                .unwrap_or_else(|| "onnx".to_string());

            if backend == "onnx" {
                let registry = embed::load_registry()
                    .map_err(|err| format!("Failed to load embedding registry: {err}"))?;

                let expected_model = match val_lower.as_str() {
                    "light" => &registry.tiers.light.model_id,
                    "standard" => &registry.tiers.standard.model_id,
                    "quality" => &registry.tiers.quality.model_id,
                    _ => &registry.tiers.light.model_id,
                };

                let encoded_model = serde_json::to_string(expected_model)
                    .map_err(|err| format!("Failed to serialize model: {err}"))?;
                conn.execute(
                    "INSERT INTO settings (key, value, scope, updated_at)
                     VALUES (?1, ?2, 'global', datetime('now'))
                     ON CONFLICT(key) DO UPDATE
                     SET value = excluded.value,
                         updated_at = datetime('now');",
                    params![embed::EMBEDDING_MODEL_KEY, encoded_model],
                )
                .map_err(|err| format!("Failed updating model setting for tier change: {err}"))?;
            }
        }

        // If this is a sensitive key, try to encrypt it
        if key.starts_with("mindvault.llm.") && key.ends_with(".apikey") {
            if let Some(session_key) = redacted::get_session_key(&state) {
                if let Ok(encrypted) = redacted::encrypt_json(&value, &session_key) {
                    value = encrypted;
                }
            }
        }

        conn.execute(
            "INSERT INTO settings (key, value, scope, updated_at)
             VALUES (?1, ?2, 'global', datetime('now'))
             ON CONFLICT(key) DO UPDATE
             SET value = excluded.value,
                 scope = excluded.scope,
                 updated_at = datetime('now');",
            params![key, value],
        )
        .map_err(|err| format!("Failed writing setting: {err}"))?;
        Ok(true)
    })())
}

#[tauri::command]
fn embedding_get_status(state: tauri::State<'_, AppState>) -> IpcResponse<EmbeddingStatus> {
    into_ipc((|| {
        let reembed_in_progress = with_embed_job_lock(&state.embed_job, |slot| slot.is_some())?;
        let conn = open_connection(&state.db_path)?;
        build_embedding_status(&conn, reembed_in_progress)
    })())
}

#[tauri::command]
fn embedding_reembed_start(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    payload: EmbeddingReembedInput,
) -> IpcResponse<()> {
    into_ipc((|| {
        check_rate_limit("embedding")?;
        embed::validate_reembed_input(&payload)?;

        let embed_job = Arc::clone(&state.embed_job);
        let cancel = Arc::new(AtomicBool::new(false));

        with_embed_job_lock(&embed_job, |slot| {
            if slot.is_some() {
                return Err(
                    "An embedding job is already active. Please cancel it or wait for it to finish."
                        .to_string(),
                );
            }
            *slot = Some(embed::EmbedJobHandle {
                cancel: Arc::clone(&cancel),
            });
            Ok(())
        })??;

        let result = (|| -> Result<(String, Connection), String> {
            let mut conn = open_connection(&state.db_path)?;

            let temp_settings = embed::EmbeddingSettings {
                model: payload.model.trim().to_string(),
                tier: payload.tier.trim().to_ascii_lowercase(),
                backend: payload.backend.trim().to_ascii_lowercase(),
                last_computed_at: None,
            };
            build_embed_engine_from_config(&conn, &temp_settings)
                .map_err(|err| format!("Failed to initialize embedding engine: {err}"))?;

            let tx = conn
                .transaction()
                .map_err(|err| format!("Failed starting embedding settings transaction: {err}"))?;
            let old_model = embed::get_embedding_settings(&tx)?.model;

            embed::set_embedding_model(&tx, payload.model.trim())?;
            embed::set_embedding_tier(&tx, &payload.tier.trim().to_ascii_lowercase())?;
            embed::set_embedding_backend(&tx, &payload.backend.trim().to_ascii_lowercase())?;

            tx.commit()
                .map_err(|err| format!("Failed committing embedding settings: {err}"))?;
            Ok((old_model, conn))
        })();

        let (old_model_id, conn) = match result {
            Ok(val) => val,
            Err(err) => {
                let _ = with_embed_job_lock(&embed_job, |slot| {
                    *slot = None;
                });
                return Err(err);
            }
        };

        if let Ok(status) = build_embedding_status(&conn, true) {
            let _emit_result = app_handle.emit("embedding-status-changed", status);
        }

        let is_unlocked = redacted::get_session_key(&state).is_some();
        let db_path = state.db_path.clone();
        let app_handle_clone = app_handle.clone();
        let embed_job_worker = Arc::clone(&embed_job);
        let worker_cancel = Arc::clone(&cancel);
        let worker_old_model = old_model_id;

        tauri::async_runtime::spawn_blocking(move || {
            let mut conn = match open_connection(&db_path) {
                Ok(conn) => conn,
                Err(err) => {
                    eprintln!("[re-embed] Failed to open background connection: {err}");
                    clear_matching_embed_job(&embed_job_worker, &worker_cancel);
                    return;
                }
            };

            let engine = match build_embed_engine_from_settings(&conn) {
                Ok(engine) => engine,
                Err(err) => {
                    eprintln!("[re-embed] Failed to build engine in worker: {err}");
                    clear_matching_embed_job(&embed_job_worker, &worker_cancel);
                    if let Ok(status) = build_embedding_status(&conn, false) {
                        let _emit_result =
                            app_handle_clone.emit("embedding-status-changed", status);
                    }
                    return;
                }
            };

            let old_model_arg = Some(worker_old_model.as_str());
            let result = embed::embed_all_nodes(
                &mut conn,
                engine.as_ref(),
                &worker_cancel,
                old_model_arg,
                is_unlocked,
            );
            match result {
                embed::EmbedJobResult::Completed { nodes_embedded } => {
                    eprintln!("[re-embed] completed: embedded {nodes_embedded} node(s)");
                }
                embed::EmbedJobResult::Cancelled => {
                    eprintln!("[re-embed] cancelled");
                }
                embed::EmbedJobResult::Failed(err) => {
                    eprintln!("[re-embed] failed: {err}");
                }
            }

            clear_matching_embed_job(&embed_job_worker, &worker_cancel);

            if let Ok(status) = build_embedding_status(&conn, false) {
                let _emit_result = app_handle_clone.emit("embedding-status-changed", status);
            }
        });

        Ok(())
    })())
}

#[tauri::command]
fn embedding_reembed_cancel(state: tauri::State<'_, AppState>) -> IpcResponse<()> {
    into_ipc(with_embed_job_lock(&state.embed_job, |slot| {
        if let Some(handle) = slot.as_ref() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
    }))
}

#[tauri::command]
fn chat_get_history(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> IpcResponse<Vec<ChatMessage>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::get_chat_history(&conn, &session_id)
    })())
}

#[tauri::command]
fn chat_append_message(
    state: tauri::State<'_, AppState>,
    id: String,
    role: String,
    content: String,
    session_id: String,
) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::append_message(&conn, id, role, content, &session_id)
    })())
}

#[tauri::command]
fn chat_clear_history(state: tauri::State<'_, AppState>, session_id: String) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::clear_chat_history(&conn, &session_id)
    })())
}

#[tauri::command]
fn chat_edit_and_truncate(
    state: tauri::State<'_, AppState>,
    edit_id: String,
    new_content: String,
    delete_ids: Vec<String>,
    session_id: String,
) -> IpcResponse<()> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting chat_edit_and_truncate transaction: {err}"))?;
        chat::edit_and_truncate(&tx, &edit_id, &new_content, delete_ids, &session_id)?;
        tx.commit().map_err(|err| {
            format!("Failed committing chat_edit_and_truncate transaction: {err}")
        })?;
        Ok(())
    })())
}

#[tauri::command]
fn chat_list_sessions(state: tauri::State<'_, AppState>) -> IpcResponse<Vec<chat::ChatSession>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::list_sessions(&conn)
    })())
}

#[tauri::command]
fn chat_create_session(
    state: tauri::State<'_, AppState>,
    id: String,
    summary: Option<String>,
) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::create_session(&conn, id, summary)
    })())
}

#[tauri::command]
fn chat_delete_session(state: tauri::State<'_, AppState>, id: String) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::delete_session(&conn, &id)
    })())
}

#[tauri::command]
fn chat_update_session_summary(
    state: tauri::State<'_, AppState>,
    id: String,
    summary: String,
) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::update_session_summary(&conn, &id, &summary)
    })())
}

#[tauri::command]
fn vault_create(input: VaultCreateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Vault> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting vault_create transaction: {err}"))?;

        let id = generate_id(&tx, "vault")?;
        let privacy_tier = input.privacy_tier.unwrap_or_else(|| "open".to_string());
        let priority_profile = input
            .priority_profile
            .unwrap_or_else(|| "standard".to_string());
        let sort_order = input.sort_order.unwrap_or(0);
        let meta = input.meta.unwrap_or_else(|| "{}".to_string());
        let session_key = redacted::get_session_key(&state);

        let parent_tier = if let Some(parent_vault_id) = input.parent_vault_id.as_deref() {
            Some(resolve_vault_effective_privacy(&tx, parent_vault_id)?)
        } else {
            None
        };
        let effective_privacy = privacy::get_effective_privacy(
            Some(privacy_tier.as_str()),
            None,
            parent_tier.as_deref(),
        );
        let is_redacted = effective_privacy == "redacted";
        let encrypted_payload = if is_redacted {
            let key = session_key.ok_or_else(|| {
                "Unlock redacted content with your master password before creating a redacted vault."
                    .to_string()
            })?;
            Some(redacted::encrypt_json(
                &redacted::VaultSecretPayload {
                    name: input.name.clone(),
                    icon: input.icon.clone(),
                    description: input.description.clone(),
                },
                &key,
            )?)
        } else {
            None
        };
        let stored_name = if is_redacted {
            "[REDACTED]".to_string()
        } else {
            input.name.clone()
        };
        let stored_description = if is_redacted {
            Some("[Metadata Locked]".to_string())
        } else {
            input.description.clone()
        };
        let stored_icon = if is_redacted {
            None
        } else {
            input.icon.clone()
        };

        if let Some(parent_vault_id) = input.parent_vault_id {
            tx.execute(
                "INSERT INTO sub_vaults (
                    id, vault_id, name, icon, description, privacy_tier, priority_profile, sort_order, meta, encrypted_payload
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10);",
                params![
                    id,
                    parent_vault_id,
                    stored_name,
                    stored_icon,
                    stored_description,
                    privacy_tier,
                    priority_profile,
                    sort_order,
                    meta,
                    encrypted_payload
                ],
            )
            .map_err(|err| format!("Failed inserting sub-vault: {err}"))?;
        } else {
            tx.execute(
                "INSERT INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta, encrypted_payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9);",
                params![
                    id,
                    stored_name,
                    stored_icon,
                    stored_description,
                    privacy_tier,
                    priority_profile,
                    sort_order,
                    meta,
                    encrypted_payload
                ],
            )
            .map_err(|err| format!("Failed inserting vault: {err}"))?;
        }

        tx.commit()
            .map_err(|err| format!("Failed committing vault_create: {err}"))?;

        fetch_vault_by_id(&conn, &id, session_key)
    })())
}

#[tauri::command]
fn vault_list(state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Vault>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let session_key = redacted::get_session_key(&state);
        let mut statement = conn
            .prepare(
                "SELECT id, parent_vault_id, name, icon, description, privacy_tier, priority_profile, summary_node_id,
                        sort_order, created_at, updated_at, deleted_at, meta, ui_metadata, encrypted_payload
                 FROM (
                    SELECT id,
                           NULL AS parent_vault_id,
                           name,
                           icon,
                           description,
                           privacy_tier,
                           priority_profile,
                           summary_node_id,
                           sort_order,
                           created_at,
                           updated_at,
                           deleted_at,
                           meta,
                           ui_metadata,
                           encrypted_payload
                    FROM vaults
                    WHERE deleted_at IS NULL
                    UNION ALL
                    SELECT id,
                           vault_id AS parent_vault_id,
                           name,
                           icon,
                           description,
                           COALESCE(privacy_tier, 'open') AS privacy_tier,
                           COALESCE(priority_profile, 'standard') AS priority_profile,
                           summary_node_id,
                           sort_order,
                           created_at,
                           updated_at,
                           deleted_at,
                           meta,
                           ui_metadata,
                           encrypted_payload
                    FROM sub_vaults
                    WHERE deleted_at IS NULL
                 )
                 ORDER BY sort_order ASC, created_at ASC;",
            )
            .map_err(|err| format!("Failed preparing vault_list query: {err}"))?;

        let rows = statement
            .query_map([], raw_vault_from_row)
            .map_err(|err| format!("Failed querying vaults: {err}"))?;

        let mut vaults = Vec::new();
        for row in rows {
            let raw = row.map_err(|err| format!("Failed decoding vault row: {err}"))?;
            vaults.push(resolve_vault_record(raw, session_key)?);
        }
        Ok(vaults)
    })())
}

#[tauri::command]
fn vault_update_position(
    vault_id: String,
    x: f64,
    y: f64,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting vault_update_position transaction: {err}"))?;

        let current_meta: String = tx
            .query_row(
                "SELECT COALESCE(ui_metadata, '{}') FROM (
                    SELECT ui_metadata FROM vaults WHERE id = ?1 AND deleted_at IS NULL
                    UNION ALL
                    SELECT ui_metadata FROM sub_vaults WHERE id = ?1 AND deleted_at IS NULL
                 ) LIMIT 1;",
                [&vault_id],
                |row| row.get(0),
            )
            .map_err(|err| {
                format!("Failed fetching current ui_metadata for vault {vault_id}: {err}")
            })?;

        let mut meta_val: serde_json::Value =
            serde_json::from_str(&current_meta).unwrap_or_else(|_| serde_json::json!({}));
        meta_val["position"] = serde_json::json!({ "x": x, "y": y });
        let updated_meta = serde_json::to_string(&meta_val)
            .map_err(|err| format!("Failed serializing updated ui_metadata: {err}"))?;

        let affected_vaults = tx
            .execute(
                "UPDATE vaults
                 SET ui_metadata = ?2,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![&vault_id, &updated_meta],
            )
            .map_err(|err| format!("Failed updating vaults position: {err}"))?;

        let affected_sub = if affected_vaults == 0 {
            tx.execute(
                "UPDATE sub_vaults
                 SET ui_metadata = ?2,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![&vault_id, &updated_meta],
            )
            .map_err(|err| format!("Failed updating sub_vaults position: {err}"))?
        } else {
            0
        };

        tx.commit()
            .map_err(|err| format!("Failed committing vault_update_position transaction: {err}"))?;

        Ok(affected_vaults + affected_sub > 0)
    })())
}

#[tauri::command]
fn vault_update_color_theme(
    vault_id: String,
    color_theme: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn.transaction().map_err(|err| {
            format!("Failed starting vault_update_color_theme transaction: {err}")
        })?;

        let current_meta: String = tx
            .query_row(
                "SELECT ui_metadata FROM (
                    SELECT ui_metadata FROM vaults WHERE id = ?1 AND deleted_at IS NULL
                    UNION ALL
                    SELECT ui_metadata FROM sub_vaults WHERE id = ?1 AND deleted_at IS NULL
                 ) LIMIT 1;",
                [&vault_id],
                |row| row.get(0),
            )
            .map_err(|err| {
                format!("Failed fetching current ui_metadata for vault {vault_id}: {err}")
            })?;

        let mut meta_val: serde_json::Value =
            serde_json::from_str(&current_meta).unwrap_or_else(|_| serde_json::json!({}));
        meta_val["colorTheme"] = serde_json::json!(color_theme);
        let updated_meta = serde_json::to_string(&meta_val)
            .map_err(|err| format!("Failed serializing updated ui_metadata: {err}"))?;

        let affected_vaults = tx
            .execute(
                "UPDATE vaults
                 SET ui_metadata = ?2,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![&vault_id, &updated_meta],
            )
            .map_err(|err| format!("Failed updating vaults color theme: {err}"))?;

        let affected_sub = if affected_vaults == 0 {
            tx.execute(
                "UPDATE sub_vaults
                 SET ui_metadata = ?2,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![&vault_id, &updated_meta],
            )
            .map_err(|err| format!("Failed updating sub_vaults color theme: {err}"))?
        } else {
            0
        };

        tx.commit().map_err(|err| {
            format!("Failed committing vault_update_color_theme transaction: {err}")
        })?;

        Ok(affected_vaults + affected_sub > 0)
    })())
}

#[tauri::command]
fn vault_get(vault_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<Option<Vault>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        match fetch_vault_by_id(&conn, &vault_id, redacted::get_session_key(&state)) {
            Ok(v) => Ok(Some(v)),
            Err(e) => {
                if e.contains("QueryReturnedNoRows") || e.contains("no rows") {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    })())
}

#[tauri::command]
fn door_list_all(state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Door>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare(
                "SELECT d.id, d.source_node_id, d.target_node_id, d.target_vault_id, d.label, d.status,
                        d.orphan_reason, d.orphan_since, d.created_at, d.updated_at,
                        tn.privacy_tier AS target_node_privacy,
                        tv.privacy_tier AS target_vault_privacy,
                        tsv.privacy_tier AS target_sub_vault_privacy
                 FROM doors d
                 LEFT JOIN nodes tn ON d.target_node_id = tn.id AND tn.deleted_at IS NULL
                 LEFT JOIN vaults tv ON tn.vault_id = tv.id AND tv.deleted_at IS NULL
                 LEFT JOIN sub_vaults tsv ON tn.sub_vault_id = tsv.id AND tsv.deleted_at IS NULL
                 WHERE d.orphan_since IS NULL;"
            )
            .map_err(|err| format!("Failed preparing door_list_all query: {err}"))?;

        let rows = statement
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let source_node_id: String = row.get(1)?;
                let mut target_node_id: Option<String> = row.get(2)?;
                let target_vault_id: Option<String> = row.get(3)?;
                let mut label: Option<String> = row.get(4)?;
                let status: String = row.get(5)?;
                let orphan_reason: Option<String> = row.get(6)?;
                let orphan_since: Option<String> = row.get(7)?;
                let created_at: String = row.get(8)?;
                let updated_at: String = row.get(9)?;

                let target_node_privacy: Option<String> = row.get(10)?;
                let target_vault_privacy: Option<String> = row.get(11)?;
                let target_sub_vault_privacy: Option<String> = row.get(12)?;

                if target_node_id.is_some() {
                    let effective_privacy = privacy::get_effective_privacy(
                        target_node_privacy.as_deref(),
                        target_sub_vault_privacy.as_deref(),
                        target_vault_privacy.as_deref(),
                    );
                    if effective_privacy == "redacted" {
                        target_node_id = Some("redacted-node-stub".to_string());
                        label = Some("[REDACTED]".to_string());
                    }
                }

                Ok(Door {
                    id,
                    source_node_id,
                    target_node_id,
                    target_vault_id,
                    label,
                    status,
                    orphan_reason,
                    orphan_since,
                    created_at,
                    updated_at,
                })
            })
            .map_err(|err| format!("Failed querying all doors: {err}"))?;

        let mut doors = Vec::new();
        for row in rows {
            doors.push(row.map_err(|err| format!("Failed decoding door row: {err}"))?);
        }
        Ok(doors)
    })())
}

#[tauri::command]
fn vault_delete(vault_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<bool> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting vault_delete transaction: {err}"))?;
        let affected_vaults = tx
            .execute(
                "UPDATE vaults
                 SET deleted_at = datetime('now'),
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                [&vault_id],
            )
            .map_err(|err| format!("Failed deleting vault: {err}"))?;
        let affected_sub_vaults = tx
            .execute(
                "UPDATE sub_vaults
                 SET deleted_at = datetime('now'),
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                [&vault_id],
            )
            .map_err(|err| format!("Failed deleting sub-vault: {err}"))?;
        tx.commit()
            .map_err(|err| format!("Failed committing vault_delete: {err}"))?;
        Ok(affected_vaults + affected_sub_vaults > 0)
    })())
}

#[tauri::command]
fn vault_update(input: VaultUpdateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Vault> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting vault_update transaction: {err}"))?;

        let vault_id = input.id;
        let session_key = redacted::get_session_key(&state);
        let current = fetch_vault_by_id(&tx, &vault_id, session_key)?;
        let current_privacy_tier = current.privacy_tier.clone();
        let next_name = input.name.unwrap_or(current.name);
        let next_privacy_tier = input.privacy_tier.unwrap_or(current_privacy_tier.clone());
        let next_priority_profile = input.priority_profile.unwrap_or(current.priority_profile);
        let next_icon = input.icon.or(current.icon);
        let next_description = input.description.or(current.description);
        let parent_tier = current
            .parent_vault_id
            .as_deref()
            .map(|parent_id| resolve_vault_effective_privacy(&tx, parent_id))
            .transpose()?;
        let next_effective_privacy = privacy::get_effective_privacy(
            Some(next_privacy_tier.as_str()),
            None,
            parent_tier.as_deref(),
        );
        let current_effective_privacy = privacy::get_effective_privacy(
            Some(current_privacy_tier.as_str()),
            None,
            parent_tier.as_deref(),
        );
        let should_encrypt = next_effective_privacy == "redacted";
        let current_is_encrypted = tx
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM vaults
                    WHERE id = ?1 AND deleted_at IS NULL AND encrypted_payload IS NOT NULL
                    UNION ALL
                    SELECT 1
                    FROM sub_vaults
                    WHERE id = ?1 AND deleted_at IS NULL AND encrypted_payload IS NOT NULL
                );",
                [&vault_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|err| format!("Failed checking redacted state for vault {vault_id}: {err}"))?
            > 0;
        ensure_encrypted_vault_can_be_unredacted(
            current_is_encrypted,
            session_key,
            should_encrypt,
        )?;
        let encrypted_payload = if should_encrypt {
            let key = session_key.ok_or_else(|| {
                "Unlock redacted content with your master password before saving.".to_string()
            })?;
            Some(redacted::encrypt_json(
                &redacted::VaultSecretPayload {
                    name: next_name.clone(),
                    icon: next_icon.clone(),
                    description: next_description.clone(),
                },
                &key,
            )?)
        } else {
            None
        };
        let stored_name = if should_encrypt {
            "[REDACTED]".to_string()
        } else {
            next_name.clone()
        };
        let stored_description = if should_encrypt {
            Some("[Metadata Locked]".to_string())
        } else {
            next_description.clone()
        };
        let stored_icon = if should_encrypt {
            None
        } else {
            next_icon.clone()
        };
        let next_encrypted_payload = if should_encrypt {
            encrypted_payload.clone()
        } else {
            None
        };

        let affected_vaults = tx
            .execute(
                "UPDATE vaults
                 SET name = ?2,
                     privacy_tier = ?3,
                     priority_profile = ?4,
                     icon = ?5,
                     description = ?6,
                     encrypted_payload = ?7,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![
                    &vault_id,
                    &stored_name,
                    &next_privacy_tier,
                    &next_priority_profile,
                    &stored_icon,
                    &stored_description,
                    &next_encrypted_payload
                ],
            )
            .map_err(|err| format!("Failed updating vault: {err}"))?;

        if affected_vaults == 0 {
            tx.execute(
                "UPDATE sub_vaults
                 SET name = ?2,
                     privacy_tier = ?3,
                     priority_profile = ?4,
                     icon = ?5,
                     description = ?6,
                     encrypted_payload = ?7,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![
                    &vault_id,
                    &stored_name,
                    &next_privacy_tier,
                    &next_priority_profile,
                    &stored_icon,
                    &stored_description,
                    &next_encrypted_payload
                ],
            )
            .map_err(|err| format!("Failed updating sub-vault: {err}"))?;
        }

        let affected_node_ids = if next_effective_privacy != current_effective_privacy {
            let mut stmt = if affected_vaults > 0 {
                tx.prepare("SELECT id FROM nodes WHERE vault_id = ?1 AND deleted_at IS NULL;")
            } else {
                tx.prepare("SELECT id FROM nodes WHERE sub_vault_id = ?1 AND deleted_at IS NULL;")
            }
            .map_err(|err| format!("Failed preparing node query: {err}"))?;

            let rows = stmt
                .query_map([&vault_id], |row| row.get::<_, String>(0))
                .map_err(|err| format!("Failed querying node IDs: {err}"))?;

            let mut ids = Vec::new();
            for r in rows {
                ids.push(r.map_err(|err| format!("Failed to read node ID: {err}"))?);
            }
            ids
        } else {
            Vec::new()
        };

        tx.commit()
            .map_err(|err| format!("Failed committing vault_update: {err}"))?;

        if !affected_node_ids.is_empty() {
            let is_unlocked = session_key.is_some();
            for node_id in affected_node_ids {
                spawn_single_node_embedding(state.db_path.clone(), node_id, is_unlocked);
            }
        }

        fetch_vault_by_id(&conn, &vault_id, session_key)
    })())
}

#[tauri::command]
fn tag_list(state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Tag>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare(
                "SELECT id, name, color, created_at
                 FROM tags
                 ORDER BY name ASC;",
            )
            .map_err(|err| format!("Failed preparing tag_list query: {err}"))?;

        let rows = statement
            .query_map([], tag_from_row)
            .map_err(|err| format!("Failed querying tags: {err}"))?;

        let mut tags = Vec::new();
        for row in rows {
            tags.push(row.map_err(|err| format!("Failed decoding tag row: {err}"))?);
        }
        Ok(tags)
    })())
}

#[tauri::command]
fn tag_create(input: TagCreateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Tag> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting tag_create transaction: {err}"))?;

        let id = generate_id(&tx, "tag")?;
        tx.execute(
            "INSERT INTO tags (id, name, color) VALUES (?1, ?2, ?3);",
            params![id, input.name, input.color],
        )
        .map_err(|err| format!("Failed inserting tag: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing tag_create: {err}"))?;

        fetch_tag_by_id(&conn, &id)
    })())
}

#[tauri::command]
fn node_tags_get(node_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Tag>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare(
                "SELECT t.id, t.name, t.color, t.created_at
                 FROM tags t
                 JOIN node_tags nt ON t.id = nt.tag_id
                 WHERE nt.node_id = ?1
                 ORDER BY t.name ASC;",
            )
            .map_err(|err| format!("Failed preparing node_tags_get query: {err}"))?;

        let rows = statement
            .query_map([node_id], tag_from_row)
            .map_err(|err| format!("Failed querying node tags: {err}"))?;

        let mut tags = Vec::new();
        for row in rows {
            tags.push(row.map_err(|err| format!("Failed decoding node tag row: {err}"))?);
        }
        Ok(tags)
    })())
}

#[tauri::command]
fn node_tag_add(
    node_id: String,
    tag_id: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let affected = conn
            .execute(
                "INSERT OR IGNORE INTO node_tags (node_id, tag_id) VALUES (?1, ?2);",
                params![node_id, tag_id],
            )
            .map_err(|err| format!("Failed adding node tag: {err}"))?;
        Ok(affected > 0)
    })())
}

#[tauri::command]
fn node_tag_remove(
    node_id: String,
    tag_id: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let affected = conn
            .execute(
                "DELETE FROM node_tags WHERE node_id = ?1 AND tag_id = ?2;",
                params![node_id, tag_id],
            )
            .map_err(|err| format!("Failed removing node tag: {err}"))?;
        Ok(affected > 0)
    })())
}

#[tauri::command]
fn door_create(input: DoorCreateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Door> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting door_create transaction: {err}"))?;

        let id = generate_id(&tx, "door")?;
        tx.execute(
            "INSERT INTO doors (id, source_node_id, target_node_id, target_vault_id, label)
             VALUES (?1, ?2, ?3, ?4, ?5);",
            params![
                id,
                input.source_node_id,
                input.target_node_id,
                input.target_vault_id,
                input.label
            ],
        )
        .map_err(|err| format!("Failed inserting door: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing door_create: {err}"))?;

        fetch_door_by_id(&conn, &id)
    })())
}

#[tauri::command]
fn door_list_outgoing(node_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Door>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare(
                "SELECT id, source_node_id, target_node_id, target_vault_id, label, status, orphan_reason,
                        orphan_since, created_at, updated_at
                 FROM doors
                 WHERE source_node_id = ?1
                 ORDER BY created_at DESC;",
            )
            .map_err(|err| format!("Failed preparing door_list_outgoing query: {err}"))?;

        let rows = statement
            .query_map([node_id], door_from_row)
            .map_err(|err| format!("Failed querying outgoing doors: {err}"))?;

        let mut doors = Vec::new();
        for row in rows {
            doors.push(row.map_err(|err| format!("Failed decoding outgoing door row: {err}"))?);
        }
        Ok(doors)
    })())
}

#[tauri::command]
fn door_list_incoming(
    node_id: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<Vec<Backlink>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare(
                "SELECT id, target_node_id, source_node_id, door_id, created_at
                 FROM backlinks
                 WHERE target_node_id = ?1
                 ORDER BY created_at DESC;",
            )
            .map_err(|err| format!("Failed preparing door_list_incoming query: {err}"))?;

        let rows = statement
            .query_map([node_id], backlink_from_row)
            .map_err(|err| format!("Failed querying incoming doors: {err}"))?;

        let mut backlinks = Vec::new();
        for row in rows {
            backlinks
                .push(row.map_err(|err| format!("Failed decoding incoming backlink row: {err}"))?);
        }
        Ok(backlinks)
    })())
}

#[tauri::command]
fn door_delete(door_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let affected = conn
            .execute("DELETE FROM doors WHERE id = ?1;", [door_id])
            .map_err(|err| format!("Failed deleting door: {err}"))?;
        Ok(affected > 0)
    })())
}

#[tauri::command]
fn door_repoint(
    door_id: String,
    target_node_id: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let affected = conn
            .execute(
                "UPDATE doors
                 SET target_node_id = ?2,
                     status = 'active',
                     orphan_reason = NULL,
                     orphan_since = NULL,
                     updated_at = datetime('now')
                 WHERE id = ?1;",
                params![door_id, target_node_id],
            )
            .map_err(|err| format!("Failed repointing door: {err}"))?;
        Ok(affected > 0)
    })())
}

#[tauri::command]
fn node_create(input: NodeCreateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Node> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting node_create transaction: {err}"))?;

        let id = generate_id(&tx, "node")?;
        let node_type = input.node_type.unwrap_or_else(|| "concept".to_string());
        let priority_json = input
            .priority
            .unwrap_or_else(|| priority::DEFAULT_PRIORITY_JSON.to_string());
        let meta = input.meta.unwrap_or_else(|| "{}".to_string());
        let session_key = redacted::get_session_key(&state);
        let effective_privacy = resolve_node_effective_privacy(
            &tx,
            &input.vault_id,
            input.sub_vault_id.as_deref(),
            input.privacy_tier.as_deref(),
        )?;
        let is_redacted = effective_privacy == "redacted";
        let encrypted_payload = if is_redacted {
            let key = session_key.ok_or_else(|| {
                "Unlock redacted content with your master password before creating a redacted node."
                    .to_string()
            })?;
            Some(redacted::encrypt_json(
                &redacted::NodeSecretPayload {
                    title: input.title.clone(),
                    summary: input.summary.clone(),
                    detail: input.detail.clone(),
                    source: input.source.clone(),
                    source_type: input.source_type.clone(),
                },
                &key,
            )?)
        } else {
            None
        };
        let stored_title = if is_redacted {
            "[REDACTED]".to_string()
        } else {
            input.title.clone()
        };
        let stored_summary = if is_redacted {
            "[Metadata Locked]".to_string()
        } else {
            input.summary.clone()
        };

        tx.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                privacy_tier, priority, meta, encrypted_payload
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13);",
            params![
                id,
                input.vault_id,
                input.sub_vault_id,
                node_type,
                stored_title,
                stored_summary,
                if is_redacted {
                    None::<String>
                } else {
                    input.detail
                },
                if is_redacted {
                    None::<String>
                } else {
                    input.source
                },
                if is_redacted {
                    None::<String>
                } else {
                    input.source_type
                },
                input.privacy_tier,
                priority_json,
                meta,
                encrypted_payload
            ],
        )
        .map_err(|err| format!("Failed inserting node: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing node_create: {err}"))?;

        let is_unlocked = session_key.is_some();
        spawn_single_node_embedding(state.db_path.clone(), id.clone(), is_unlocked);

        fetch_node_by_id(&conn, &id, session_key)
            .and_then(|node| node.ok_or_else(|| "Node not found after insert".to_string()))
    })())
}

#[tauri::command]
fn node_get(node_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<Option<Node>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        match fetch_node_by_id(&conn, &node_id, redacted::get_session_key(&state))? {
            Some(node) if node.deleted_at.is_none() => Ok(Some(node)),
            _ => Ok(None),
        }
    })())
}

#[tauri::command]
fn node_list(state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Node>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        fetch_nodes(&conn, redacted::get_session_key(&state))
    })())
}

#[tauri::command]
fn node_update(input: NodeUpdateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Node> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting node_update transaction: {err}"))?;

        let session_key = redacted::get_session_key(&state);
        let current = fetch_node_by_id(&tx, &input.id, session_key)?
            .filter(|node| node.deleted_at.is_none())
            .ok_or_else(|| format!("Node not found: {}", input.id))?;

        let current_title = current.title.clone();
        let current_summary = current.summary.clone();
        let current_detail = current.detail.clone();
        let current_effective_privacy = resolve_node_effective_privacy(
            &tx,
            &current.vault_id,
            current.sub_vault_id.as_deref(),
            current.privacy_tier.as_deref(),
        )?;

        let next_vault_id = input.vault_id.unwrap_or(current.vault_id);
        let next_sub_vault_id = input.sub_vault_id.or(current.sub_vault_id);
        let next_node_type = input.node_type.unwrap_or(current.node_type);
        let next_title = input.title.unwrap_or(current.title);
        let next_summary = input.summary.unwrap_or(current.summary);
        let next_detail = input.detail.or(current.detail);
        let next_source = input.source.or(current.source);
        let next_source_type = input.source_type.or(current.source_type);
        let next_privacy_tier = input.privacy_tier.or(current.privacy_tier);
        let next_priority = input.priority.unwrap_or(current.priority);
        let next_is_archived = if input.is_archived.unwrap_or(current.is_archived) {
            1_i64
        } else {
            0_i64
        };
        let next_meta = input.meta.unwrap_or(current.meta);
        let next_version = current.version + 1;
        let effective_privacy = resolve_node_effective_privacy(
            &tx,
            &next_vault_id,
            next_sub_vault_id.as_deref(),
            next_privacy_tier.as_deref(),
        )?;
        let should_encrypt = effective_privacy == "redacted";
        let current_is_encrypted =
            tx.query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM nodes
                    WHERE id = ?1 AND deleted_at IS NULL AND encrypted_payload IS NOT NULL
                );",
                [&input.id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|err| {
                format!(
                    "Failed checking redacted state for node {}: {err}",
                    input.id
                )
            })? > 0;
        ensure_encrypted_node_can_be_unredacted(current_is_encrypted, session_key, should_encrypt)?;
        let needs_reembed = embed::stored_text_columns_changed(
            &current_title,
            &current_summary,
            current_detail.as_deref(),
            current_is_encrypted,
            should_encrypt,
            &current_effective_privacy,
            &effective_privacy,
            &next_title,
            &next_summary,
            next_detail.as_deref(),
        );
        let encrypted_payload = if should_encrypt {
            let key = session_key.ok_or_else(|| {
                "Unlock redacted content with your master password before saving.".to_string()
            })?;
            Some(redacted::encrypt_json(
                &redacted::NodeSecretPayload {
                    title: next_title.clone(),
                    summary: next_summary.clone(),
                    detail: next_detail.clone(),
                    source: next_source.clone(),
                    source_type: next_source_type.clone(),
                },
                &key,
            )?)
        } else {
            None
        };
        let stored_title = if should_encrypt {
            "[REDACTED]".to_string()
        } else {
            next_title.clone()
        };
        let stored_summary = if should_encrypt {
            "[Metadata Locked]".to_string()
        } else {
            next_summary.clone()
        };

        tx.execute(
            "UPDATE nodes
             SET vault_id = ?2,
                 sub_vault_id = ?3,
                 node_type = ?4,
                 title = ?5,
                 summary = ?6,
                 detail = ?7,
                 source = ?8,
                 source_type = ?9,
                 privacy_tier = ?10,
                 priority = ?11,
                 version = ?12,
                 is_archived = ?13,
                 updated_at = datetime('now'),
                 meta = ?14,
                 encrypted_payload = ?15
             WHERE id = ?1 AND deleted_at IS NULL;",
            params![
                input.id,
                next_vault_id,
                next_sub_vault_id,
                next_node_type,
                stored_title,
                stored_summary,
                if should_encrypt {
                    None::<String>
                } else {
                    next_detail
                },
                if should_encrypt {
                    None::<String>
                } else {
                    next_source
                },
                if should_encrypt {
                    None::<String>
                } else {
                    next_source_type
                },
                next_privacy_tier,
                next_priority,
                next_version,
                next_is_archived,
                next_meta,
                encrypted_payload
            ],
        )
        .map_err(|err| format!("Failed updating node: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing node_update: {err}"))?;

        if needs_reembed {
            let is_unlocked = session_key.is_some();
            spawn_single_node_embedding(state.db_path.clone(), input.id.clone(), is_unlocked);
        }

        fetch_node_by_id(&conn, &input.id, session_key).and_then(|node| {
            node.filter(|n| n.deleted_at.is_none())
                .ok_or_else(|| format!("Node not found after update: {}", input.id))
        })
    })())
}

#[tauri::command]
fn node_touch(node_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;

        let priority_json_str: String = conn
            .query_row(
                "SELECT priority FROM nodes WHERE id = ?1 AND deleted_at IS NULL;",
                [&node_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("Failed reading priority for node {node_id}: {err}"))?;

        let mut priority_obj: serde_json::Value =
            serde_json::from_str(&priority_json_str).unwrap_or_else(|_| serde_json::json!({}));

        let current_touches = priority_obj
            .get("session_touches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        priority_obj["session_touches"] = serde_json::json!(current_touches + 1);

        let updated_json = serde_json::to_string(&priority_obj)
            .map_err(|err| format!("Failed serializing priority for node {node_id}: {err}"))?;

        let affected = conn
            .execute(
                "UPDATE nodes SET last_accessed = datetime('now'), priority = ?2 WHERE id = ?1 AND deleted_at IS NULL;",
                params![&node_id, &updated_json],
            )
            .map_err(|err| format!("Failed touching node: {err}"))?;
        Ok(affected > 0)
    })())
}

#[tauri::command]
fn node_delete(node_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let affected = conn
            .execute(
                "UPDATE nodes
                 SET deleted_at = datetime('now'),
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                [node_id],
            )
            .map_err(|err| format!("Failed deleting node: {err}"))?;
        Ok(affected > 0)
    })())
}

#[tauri::command]
fn debug_assemble_context(
    node_ids: Vec<String>,
    scope: String,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<String> {
    into_ipc((|| {
        // Because a user is running a direct test context request,
        // we'll assume they just want to see the un-stubbed result for whatever they selected
        // to simplify the scope of debug.
        let conn = open_connection(&state.db_path)?;
        llm::assembler::build_context(
            &conn,
            node_ids,
            llm::assembler::AssemblerConfig {
                scope,
                max_tokens: DEFAULT_ASSEMBLER_MAX_TOKENS,
                is_unlocked: true, // debug command overrides privacy checks locally
            },
        )
    })())
}

#[tauri::command]
fn llm_count_tokens(text: String) -> IpcResponse<usize> {
    into_ipc(Ok(crate::llm::assembler::count_tokens(&text)))
}

#[tauri::command]
fn onboarding_commit(
    proposals: Vec<OnboardingNodeCommitInput>,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc(execute_onboarding_commit(&proposals, &state.db_path))
}

#[tauri::command]
fn priority_refresh_all(state: tauri::State<'_, DbState>) -> IpcResponse<usize> {
    into_ipc(run_priority_refresh(&state.db_path))
}

#[tauri::command]
fn priority_optimize_all(state: tauri::State<'_, DbState>) -> IpcResponse<usize> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare("SELECT id, priority FROM nodes WHERE deleted_at IS NULL;")
            .map_err(|err| format!("Failed preparing optimize query: {err}"))?;

        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|err| format!("Failed querying nodes for optimize: {err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("Failed reading optimize rows: {err}"))?;

        let mut updated_count: usize = 0;

        for (id, priority_json_str) in &rows {
            let mut priority_obj: serde_json::Value =
                serde_json::from_str(priority_json_str).unwrap_or_else(|_| serde_json::json!({}));

            let current_profile = priority_obj
                .get("profile")
                .and_then(|v| v.as_str())
                .unwrap_or("standard");

            if current_profile == "pinned" {
                continue;
            }

            let frozen = priority_obj
                .get("frozen")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if frozen {
                continue;
            }

            let count_30d = priority_obj
                .get("access_count_30active")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let new_profile = if count_30d >= 7 {
                "slow"
            } else if count_30d <= 2 {
                "fast"
            } else {
                "standard"
            };

            if new_profile == current_profile {
                continue;
            }

            priority_obj["profile"] = serde_json::json!(new_profile);
            priority_obj["pinned"] = serde_json::json!(false);

            let updated_json = serde_json::to_string(&priority_obj)
                .map_err(|err| format!("Failed serializing optimize for node {id}: {err}"))?;

            conn.execute(
                "UPDATE nodes SET priority = ?2, updated_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL;",
                params![id, updated_json],
            )
            .map_err(|err| format!("Failed optimizing node {id}: {err}"))?;

            updated_count += 1;
        }

        Ok(updated_count)
    })())
}

#[tauri::command]
fn chat_set_off_the_record(enabled: bool, state: tauri::State<'_, AppState>) -> IpcResponse<bool> {
    let conn = match open_connection(&state.db_path) {
        Ok(c) => c,
        Err(err) => return IpcResponse::Err { err },
    };

    if !enabled {
        if let Err(err) = chat::purge_temporary_session(&conn) {
            eprintln!("Failed to purge temporary session when disabling OTR: {err}");
        }
    }

    // Store transient setting in the DB
    let res = conn.execute(
        "INSERT INTO settings (key, value, scope) VALUES ('session_off_the_record', ?1, 'global')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now');",
        [enabled.to_string()],
    );
    match res {
        Ok(_) => IpcResponse::Ok { ok: enabled },
        Err(err) => IpcResponse::Err {
            err: err.to_string(),
        },
    }
}

#[tauri::command]
fn chat_is_off_the_record(state: tauri::State<'_, AppState>) -> IpcResponse<bool> {
    let conn = match open_connection(&state.db_path) {
        Ok(c) => c,
        Err(err) => return IpcResponse::Err { err },
    };
    let is_otr: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'session_off_the_record' LIMIT 1;",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "false".to_string());

    IpcResponse::Ok {
        ok: is_otr == "true",
    }
}

#[tauri::command]
async fn memory_extract_force(
    provider: String,
    endpoint: String,
    model: String,
    state: tauri::State<'_, AppState>,
) -> Result<Changeset, String> {
    check_rate_limit("memory_agent")?;
    let db_path = state.db_path.clone();
    let conn = open_connection(&db_path)?;

    // Verify minimum message threshold (at least 3 messages)
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id = 'default-session';",
            [],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed querying message count: {err}"))?;
    if count < 3 {
        return Err("Need at least 3 messages to extract memory.".to_string());
    }

    // Since memory_extract_force is a manual bypass/trigger, check if there's a correction
    // signal in the latest user message to see if we should run it as a correction.
    let latest_user_msg = fetch_latest_user_message(&conn, "default-session")?;
    let correction_signal = if let Some(msg_content) = latest_user_msg {
        memory_agent::trigger::should_extract_correction(&conn, "default-session", &msg_content)?
    } else {
        None
    };

    drop(conn);

    // Execute pipeline with the detected correction signal
    let result = execute_memory_extraction_pipeline(
        provider,
        endpoint,
        model,
        db_path.clone(),
        correction_signal,
    )
    .await;

    // Mark extraction complete
    let conn = open_connection(&db_path)?;
    memory_agent::trigger::mark_extraction_complete(&conn, count)?;

    result
}

#[tauri::command]
async fn llm_list_models(provider: String, endpoint: String) -> IpcResponse<Vec<String>> {
    let parsed_provider = match provider.trim().to_lowercase().as_str() {
        "ollama" => llm::client::LlmProvider::Ollama,
        "lmstudio" => llm::client::LlmProvider::LmStudio,
        "anthropic" => llm::client::LlmProvider::Anthropic,
        "openai" => llm::client::LlmProvider::OpenAi,
        "google" => llm::client::LlmProvider::Google,
        "xai" => llm::client::LlmProvider::XAi,
        _ => {
            return IpcResponse::Err {
                err: "Unsupported provider. Use 'ollama', 'lmstudio', 'anthropic', 'openai', 'google', or 'xai'.".to_string(),
            }
        }
    };
    let client = llm::client::UniversalClient::new(parsed_provider, endpoint, String::new());
    into_ipc(llm::client::LlmClient::list_models(&client).await)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn llm_chat(
    node_ids: Vec<String>,
    scope: String,
    provider: String,
    endpoint: String,
    model: String,
    user_prompt: String,
    charts_enabled: bool,
    is_redacted_unlocked: bool,
    state: tauri::State<'_, AppState>,
    session_id: &str,
) -> Result<String, String> {
    let db_path = state.db_path.clone();
    let persona_instruction = "You are MindVault's personalized, context-aware memory assistant.";

    let mut system_prompt = if session_id == "temporary-session" {
        "[Off the Record Mode: Context assembly has been bypassed. No personal memories or notes are accessible in this session.]".to_string()
    } else {
        let conn = open_connection(&db_path)?;
        llm::assembler::build_context(
            &conn,
            node_ids,
            llm::assembler::AssemblerConfig {
                scope,
                max_tokens: DEFAULT_ASSEMBLER_MAX_TOKENS,
                is_unlocked: is_redacted_unlocked,
            },
        )?
    };

    let chart_instruction = "\n\n\
    [VISUALIZATION SYSTEM CONTRACT]\n\
    You have access to an interactive charting system. When a user EXPLICITLY asks you to graph, plot, chart, or visualize something, output the specification as JSON inside a ```chart code fence. Never use text-based ASCII art representations.\n\
    \n\
    CRITICAL: ONLY output a ```chart fence when the user's intent is to SEE a visual graph or chart. If the user asks you to SOLVE, EXPLAIN, COMPUTE, EVALUATE, PROVE, DERIVE, or CALCULATE something, respond with a normal text/markdown explanation showing your work and the answer. You MAY optionally append a ```chart fence AFTER your explanation if a graph would help illustrate the result, but the textual answer must come first and must be complete on its own.\n\
    \n\
    PROMPT DIRECTIVES:\n\
    1. GRAPH/PLOT REQUESTS: When the user says 'graph', 'plot', 'chart', 'visualize', 'show me a graph of', or similar — output the ```chart fence directly as the primary response. Keep any surrounding text minimal.\n\
    2. SOLVE/EXPLAIN REQUESTS: When the user says 'solve', 'evaluate', 'compute', 'integrate', 'differentiate', 'prove', 'explain', 'help me with', 'what is', 'calculate', 'find' — write a full textual solution in markdown. Use LaTeX math notation ($$...$$ or $...$) for equations. You may add a ```chart fence at the end to illustrate, but it is optional.\n\
    3. STRING COLUMNS ARE NOT NUMERIC VALUES: When plotting datasets, columns with text/string values MUST be used as labels or categorical axes (e.g. in the 'x' array, 'labels' array, or 'theta' array). NEVER put string arrays into numeric arrays (like 'y', 'r', or 'values').\n\
    4. PIE CHART STRUCTURE: Plotly pie charts MUST use 'labels' and 'values' inside the trace. NEVER use 'x' and 'y' for pie charts. Example: { \"type\": \"pie\", \"labels\": [\"A\", \"B\"], \"values\": [40, 60] }.\n\
    5. RADAR CHART STRUCTURE: Plotly radar charts MUST use \"scatterpolar\" with 'r' and 'theta'. Close the polygon by repeating the first element at the end of both arrays.\n\
    6. DO NOT ATTEMPT UNSUPPORTED DIAGRAMS: If the user requests Venn diagrams, flowcharts, mind maps, Gantt charts, network graphs, or 3D surface charts, explain the limitation in plain text. Never approximate with scatter/bubble overlays.\n\
    \n\
    CHART SCHEMA TYPES:\n\
    \n\
    1. For mathematical equations or functions (e.g., y = 2x + 1), output type \"function\":\n\
    ```chart\n\
    {\n\
      \"type\": \"function\",\n\
      \"title\": \"y = 2x + 1\",\n\
      \"expressions\": [\n\
        { \"expression\": \"2*x + 1\", \"color\": \"#b56a37\", \"label\": \"y = 2x + 1\" }\n\
      ],\n\
      \"domainX\": [-5, 5],\n\
      \"domainY\": [-5, 5]\n\
    }\n\
    ```\n\
    \n\
    2. For statistical data (bar, line, pie, scatterpolar), output a Plotly.js JSON with \"data\" and optional \"layout\":\n\
    ```chart\n\
    {\n\
      \"type\": \"plotly\",\n\
      \"data\": [\n\
        { \"x\": [\"Apples\", \"Bananas\", \"Cherries\"], \"y\": [12, 18, 5], \"type\": \"bar\", \"marker\": { \"color\": \"#b56a37\" } }\n\
      ],\n\
      \"layout\": {\n\
        \"title\": \"Fruit Counts\"\n\
      }\n\
    }\n\
    ```\n\
    Always output fully valid JSON (double quotes for keys and string values). Do not embed comments inside the JSON.";

    if charts_enabled {
        if system_prompt.is_empty() {
            system_prompt = format!("{persona_instruction} {chart_instruction}");
        } else {
            system_prompt = format!("{persona_instruction}{chart_instruction}");
        }
    } else if system_prompt.is_empty() {
        system_prompt = persona_instruction.to_string();
    } else {
        system_prompt = format!("{persona_instruction}\n\n{system_prompt}");
    }

    let parsed_provider = match provider.trim().to_lowercase().as_str() {
        "ollama" => llm::client::LlmProvider::Ollama,
        "lmstudio" => llm::client::LlmProvider::LmStudio,
        "anthropic" => llm::client::LlmProvider::Anthropic,
        "openai" => llm::client::LlmProvider::OpenAi,
        "google" => llm::client::LlmProvider::Google,
        "xai" => llm::client::LlmProvider::XAi,
        _ => return Err("Unsupported provider. Use 'ollama', 'lmstudio', 'anthropic', 'openai', 'google', or 'xai'.".to_string()),
    };

    let client = llm::client::UniversalClient::new(parsed_provider, endpoint, model);
    let messages = [llm::client::LlmMessage {
        role: "user".to_string(),
        content: user_prompt,
    }];
    llm::client::LlmClient::complete(&client, &system_prompt, &messages).await
}

#[tauri::command]
async fn onboarding_extract_proposals(
    answers_json: String,
    provider: String,
    endpoint: String,
    model: String,
) -> Result<Vec<OnboardingProposedNode>, String> {
    onboarding::validate_answers_json(&answers_json)?;
    let model_trimmed = model.trim();
    if model_trimmed.is_empty() {
        return Err("Model name is required for onboarding extraction.".to_string());
    }

    let parsed_provider = match provider.trim().to_lowercase().as_str() {
        "ollama" => llm::client::LlmProvider::Ollama,
        "lmstudio" => llm::client::LlmProvider::LmStudio,
        "anthropic" => llm::client::LlmProvider::Anthropic,
        "openai" => llm::client::LlmProvider::OpenAi,
        "google" => llm::client::LlmProvider::Google,
        "xai" => llm::client::LlmProvider::XAi,
        _ => return Err("Unsupported provider. Use 'ollama', 'lmstudio', 'anthropic', 'openai', 'google', or 'xai'.".to_string()),
    };

    let client = llm::client::UniversalClient::new(
        parsed_provider,
        endpoint.trim().to_string(),
        model_trimmed.to_string(),
    );

    let user_content = onboarding::build_onboarding_extraction_user_message(&answers_json);
    let messages = [llm::client::LlmMessage {
        role: "user".to_string(),
        content: user_content,
    }];

    let raw = llm::client::LlmClient::complete(
        &client,
        onboarding::ONBOARDING_EXTRACTION_SYSTEM_PROMPT,
        &messages,
    )
    .await?;

    let proposals = onboarding::parse_proposals_from_llm_output(&raw)?;
    Ok(proposals.into_iter().map(map_onboarding_proposed).collect())
}

#[tauri::command]
async fn save_markdown_file(default_name: String, content: String) -> IpcResponse<bool> {
    if content.len() > 10_000_000 {
        return IpcResponse::Err {
            err: "Content exceeds maximum export size".into(),
        };
    }

    let path = rfd::AsyncFileDialog::new()
        .set_file_name(&default_name)
        .add_filter("Markdown", &["md"])
        .save_file()
        .await;

    match path {
        Some(handle) => {
            let path_buf = handle.path().to_path_buf();
            let write_res =
                tauri::async_runtime::spawn_blocking(move || std::fs::write(path_buf, content))
                    .await;

            match write_res {
                Ok(Ok(_)) => IpcResponse::Ok { ok: true },
                Ok(Err(e)) => IpcResponse::Err { err: e.to_string() },
                Err(join_err) => IpcResponse::Err {
                    err: format!("Spawn blocking failed: {join_err}"),
                },
            }
        }
        None => IpcResponse::Ok { ok: false },
    }
}

#[tauri::command]
async fn memory_extract(
    provider: String,
    endpoint: String,
    model: String,
    state: tauri::State<'_, AppState>,
) -> Result<Changeset, String> {
    // 1. Enforce governor rate-limiting first
    check_rate_limit("memory_agent")?;

    // 2. Execute shared pipeline
    let db_path = state.db_path.clone();
    execute_memory_extraction_pipeline(provider, endpoint, model, db_path, None).await
}

#[tauri::command]
async fn memory_extract_if_ready(
    provider: String,
    endpoint: String,
    model: String,
    state: tauri::State<'_, AppState>,
    session_id: &str,
) -> Result<Option<Changeset>, String> {
    if session_id == "temporary-session" {
        return Ok(None);
    }

    // 1. Enforce governor rate-limiting first
    check_rate_limit("memory_agent")?;

    // 2. Open connection synchronously to run trigger checks
    let db_path = state.db_path.clone();
    let conn = open_connection(&db_path)?;

    let current_message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1;",
            [session_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed querying session message count: {err}"))?;

    // 4. Align last extract count if chat history was cleared/reset
    memory_agent::trigger::align_last_extract_count(&conn, current_message_count)?;

    // 5. Check trigger
    let mut ready = memory_agent::trigger::should_extract(&conn, session_id)?;
    let mut correction_signal = None;

    // Always scan the latest user message for a correction signal to ensure
    // corrections are correctly routed to the amendment engine instead of creating duplicates,
    // even if the standard extraction cooldown has already elapsed.
    let latest_user_msg = fetch_latest_user_message(&conn, session_id)?;
    if let Some(msg_content) = latest_user_msg {
        let signal =
            memory_agent::trigger::should_extract_correction(&conn, session_id, &msg_content)?;
        if signal.is_some() {
            ready = true;
            correction_signal = signal;
        }
    }

    if !ready {
        return Ok(None);
    }

    // Drop connection explicitly before starting any await calls
    drop(conn);

    // 5. Execute shared pipeline (capture result without early-returning on error,
    //    so we always mark the extraction as attempted and respect cooldown windows)
    let pipeline_result = execute_memory_extraction_pipeline(
        provider,
        endpoint,
        model,
        db_path.clone(),
        correction_signal,
    )
    .await;

    // 6. Mark extraction complete/attempted *before* propagating any error,
    //    so that should_extract respects the 6-message and 2-minute cooldown
    let conn = open_connection(&db_path)?;
    memory_agent::trigger::mark_extraction_complete(&conn, current_message_count)?;

    // 7. Now propagate the pipeline result
    Ok(Some(pipeline_result?))
}

#[tauri::command]
async fn changeset_commit(
    input: ipc_types::ChangesetCommitInput,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let db_path = state.db_path.clone();
    let session_key = redacted::get_session_key(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = open_connection(&db_path)?;
        memory_agent::commit_changeset_transaction(&mut conn, &input, &db_path, session_key)
    })
    .await
    .map_err(|err| format!("Failed to spawn blocking changeset commit task: {err}"))?
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatConversionResult {
    session_id: String,
    summary: Option<String>,
    changeset: Option<Changeset>,
    extraction_error: Option<String>,
}

#[tauri::command]
async fn chat_convert_temporary_to_memory(
    provider: String,
    endpoint: String,
    model: String,
    target_session_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ChatConversionResult, String> {
    let db_path = state.db_path.clone();
    let mut conn = open_connection(&db_path)?;

    let converted_session =
        chat::convert_temporary_to_memory(&mut conn, target_session_id.as_deref())?;

    conn.execute(
        "UPDATE settings SET value = 'false', updated_at = datetime('now')
         WHERE key = 'session_off_the_record';",
        [],
    )
    .map_err(|err| format!("Failed disabling OTR setting: {err}"))?;

    drop(conn);

    match execute_memory_extraction_pipeline(provider, endpoint, model, db_path, None).await {
        Ok(changeset) => Ok(ChatConversionResult {
            session_id: converted_session.session_id,
            summary: converted_session.summary,
            changeset: Some(changeset),
            extraction_error: None,
        }),
        Err(err) => Ok(ChatConversionResult {
            session_id: converted_session.session_id,
            summary: converted_session.summary,
            changeset: None,
            extraction_error: Some(err),
        }),
    }
}

/// MARK: Tests
#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use rusqlite::Connection;

    use super::{
        ensure_encrypted_node_can_be_unredacted, ensure_encrypted_vault_can_be_unredacted,
    };

    #[test]
    fn encrypted_vault_cannot_be_unredacted_without_session_key() {
        let result = ensure_encrypted_vault_can_be_unredacted(true, None, false);
        assert!(result.is_err());
    }

    #[test]
    fn encrypted_vault_can_stay_redacted_without_session_key() {
        let result = ensure_encrypted_vault_can_be_unredacted(true, None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn unencrypted_vault_is_not_blocked_without_session_key() {
        let result = ensure_encrypted_vault_can_be_unredacted(false, None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn unlocked_vault_can_be_unredacted() {
        let session_key = Some([7_u8; 32]);
        let result = ensure_encrypted_vault_can_be_unredacted(true, session_key, false);
        assert!(result.is_ok());
    }

    #[test]
    fn encrypted_node_cannot_be_unredacted_without_session_key() {
        let result = ensure_encrypted_node_can_be_unredacted(true, None, false);
        assert!(result.is_err());
    }

    #[test]
    fn encrypted_node_can_stay_redacted_without_session_key() {
        let result = ensure_encrypted_node_can_be_unredacted(true, None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn unencrypted_node_is_not_blocked_without_session_key() {
        let result = ensure_encrypted_node_can_be_unredacted(false, None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn unlocked_node_can_be_unredacted() {
        let session_key = Some([9_u8; 32]);
        let result = ensure_encrypted_node_can_be_unredacted(true, session_key, false);
        assert!(result.is_ok());
    }

    #[test]
    fn fetch_private_referenced_nodes_chunks_large_in_lists() {
        let mut conn = Connection::open_in_memory()
            .unwrap_or_else(|err| panic!("expected in-memory sqlite connection: {err}"));
        conn.execute_batch(
            "CREATE TABLE nodes (
                id TEXT PRIMARY KEY,
                vault_id TEXT NOT NULL,
                sub_vault_id TEXT,
                privacy_tier TEXT,
                deleted_at TEXT
             );
             CREATE TABLE privacy_overrides (
                node_id TEXT PRIMARY KEY,
                privacy_tier TEXT
             );",
        )
        .unwrap_or_else(|err| panic!("expected test schema to initialize: {err}"));

        let tx = conn
            .transaction()
            .unwrap_or_else(|err| panic!("expected test transaction: {err}"));
        let mut unique_node_ids = HashSet::new();

        for index in 0..1001 {
            let node_id = format!("node_{index}");
            unique_node_ids.insert(node_id.clone());
            tx.execute(
                "INSERT INTO nodes (id, vault_id, sub_vault_id, privacy_tier, deleted_at)
                 VALUES (?1, 'vault_root', NULL, 'locked', NULL);",
                [node_id],
            )
            .unwrap_or_else(|err| panic!("expected node insert to succeed: {err}"));
        }

        tx.commit()
            .unwrap_or_else(|err| panic!("expected test commit: {err}"));

        let mut vault_map: HashMap<String, (Option<String>, String)> = HashMap::new();
        vault_map.insert("vault_root".to_string(), (None, "open".to_string()));

        let private_nodes =
            super::fetch_private_referenced_nodes(&conn, &unique_node_ids, &vault_map)
                .unwrap_or_else(|err| {
                    panic!("expected batched privacy lookup to succeed for >999 ids: {err}")
                });

        assert_eq!(private_nodes.len(), 1001);
        assert!(private_nodes.contains("node_0"));
        assert!(private_nodes.contains("node_1000"));
    }

    #[test]
    fn embedding_rate_limit_rejects_immediate_second_call() {
        let first = super::check_rate_limit("embedding");
        assert!(first.is_ok());

        let second = super::check_rate_limit("embedding");
        assert!(second.is_err());
    }
}
