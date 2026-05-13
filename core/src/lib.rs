use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chat::ChatMessage;
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use tauri::Manager;

mod auth;
mod chat;
mod ipc_types;
pub mod llm;
pub mod onboarding;
mod priority;
mod privacy;
use ipc_types::{
    Backlink, Door, DoorCreateInput, Node, NodeCreateInput, NodeUpdateInput,
    OnboardingNodeCommitInput, OnboardingProposedNode, Tag, TagCreateInput, Vault,
    VaultCreateInput, VaultUpdateInput,
};

pub(crate) struct DbState {
    pub(crate) db_path: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum IpcResponse<T> {
    Ok { ok: T },
    Err { err: String },
}

pub(crate) fn into_ipc<T>(result: Result<T, String>) -> IpcResponse<T> {
    match result {
        Ok(value) => IpcResponse::Ok { ok: value },
        Err(err) => IpcResponse::Err { err },
    }
}

pub type AppError = String;
type AppState = DbState;

/// Default `max_tokens` for context assembly (`debug_assemble_context`, `llm_chat`).
/// Keep in sync with `CONTEXT_MAX_TOKENS` in `ui/constants/contextBudget.ts`.
const DEFAULT_ASSEMBLER_MAX_TOKENS: usize = 8000;

#[tauri::command]
fn greet(name: &str) -> IpcResponse<String> {
    IpcResponse::Ok {
        ok: format!("Hello, {}! You've been greeted from MindVault!", name),
    }
}

fn sqlite_db_path<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    Ok(app_data_dir.join("mindvault.db"))
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

fn generate_id(conn: &Connection, prefix: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT ?1 || '_' || lower(hex(randomblob(8)));",
        [prefix],
        |row| row.get(0),
    )
    .map_err(|err| format!("Failed generating id: {err}"))
}

fn minimal_pre_write_backup(db_path: &Path, reason: &str) -> Result<PathBuf, String> {
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
        .as_secs();
    let backup_path = backups_dir.join(format!("mindvault-pre-{reason}-{now_unix}.db"));

    let conn =
        Connection::open(db_path).map_err(|err| format!("Failed to open DB for backup: {err}"))?;

    let backup_path_str = backup_path
        .to_str()
        .ok_or("Backup path is not valid UTF-8")?
        .replace('\'', "''");

    conn.execute(&format!("VACUUM INTO '{}'", backup_path_str), [])
        .map_err(|err| {
            format!(
                "Failed creating pre-write backup {} -> {}: {err}",
                db_path.display(),
                backup_path.display()
            )
        })?;

    Ok(backup_path)
}

type OnboardingDefaultVaultSpec = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i64,
);

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

fn ensure_onboarding_vault_exists(conn: &Connection, vault_id: &str) -> Result<(), String> {
    if fetch_vault_by_id(conn, vault_id).is_ok() {
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
        return fetch_vault_by_id(conn, vault_id)
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

    fetch_vault_by_id(conn, vault_id)
        .map(|_| ())
        .map_err(|err| {
            format!("Failed verifying onboarding vault '{vault_id}' after ensure step: {err}")
        })
}

fn vault_from_row(row: &Row<'_>) -> rusqlite::Result<Vault> {
    Ok(Vault {
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
    })
}

fn node_from_row(row: &Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
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

fn fetch_vault_by_id(conn: &Connection, vault_id: &str) -> Result<Vault, String> {
    conn.query_row(
        "SELECT id, parent_vault_id, name, icon, description, privacy_tier, priority_profile, summary_node_id,
                sort_order, created_at, updated_at, deleted_at, meta
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
                   meta
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
                   meta
            FROM sub_vaults
            WHERE deleted_at IS NULL
         )
         WHERE id = ?1
         LIMIT 1;",
        [vault_id],
        vault_from_row,
    )
    .map_err(|err| format!("Failed fetching vault {vault_id}: {err}"))
}

fn fetch_node_by_id(conn: &Connection, node_id: &str) -> Result<Option<Node>, String> {
    conn.query_row(
        "SELECT id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                privacy_tier, priority, version, is_archived, created_at, updated_at, last_accessed,
                deleted_at, meta
         FROM nodes
         WHERE id = ?1;",
        [node_id],
        node_from_row,
    )
    .map(Some)
    .or_else(|err| {
        if matches!(err, rusqlite::Error::QueryReturnedNoRows) {
            Ok(None)
        } else {
            Err(format!("Failed fetching node {node_id}: {err}"))
        }
    })
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

fn fetch_nodes(conn: &Connection) -> Result<Vec<Node>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                    privacy_tier, priority, version, is_archived, created_at, updated_at, last_accessed,
                    deleted_at, meta
             FROM nodes
             WHERE deleted_at IS NULL
             ORDER BY created_at DESC;",
        )
        .map_err(|err| format!("Failed preparing node_list query: {err}"))?;

    let rows = statement
        .query_map([], node_from_row)
        .map_err(|err| format!("Failed querying nodes: {err}"))?;

    let mut nodes = Vec::new();
    for row in rows {
        nodes.push(row.map_err(|err| format!("Failed decoding node row: {err}"))?);
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
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
            [key],
            |row| row.get::<_, String>(0),
        )
        .map(Some)
        .or_else(|err| {
            if matches!(err, rusqlite::Error::QueryReturnedNoRows) {
                Ok(None)
            } else {
                Err(format!("Failed reading setting: {err}"))
            }
        })
    })())
}

#[tauri::command]
fn settings_set(key: String, value: String, state: tauri::State<'_, DbState>) -> IpcResponse<bool> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
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
fn chat_get_history(state: tauri::State<'_, AppState>) -> IpcResponse<Vec<ChatMessage>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::get_chat_history(&conn)
    })())
}

#[tauri::command]
fn chat_append_message(
    state: tauri::State<'_, AppState>,
    id: String,
    role: String,
    content: String,
) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::append_message(&conn, id, role, content)
    })())
}

#[tauri::command]
fn chat_clear_history(state: tauri::State<'_, AppState>) -> IpcResponse<()> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        chat::clear_chat_history(&conn)
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

        if let Some(parent_vault_id) = input.parent_vault_id {
            tx.execute(
                "INSERT INTO sub_vaults (
                    id, vault_id, name, icon, description, privacy_tier, priority_profile, sort_order, meta
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9);",
                params![
                    id,
                    parent_vault_id,
                    input.name,
                    input.icon,
                    input.description,
                    privacy_tier,
                    priority_profile,
                    sort_order,
                    meta
                ],
            )
            .map_err(|err| format!("Failed inserting sub-vault: {err}"))?;
        } else {
            tx.execute(
                "INSERT INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);",
                params![
                    id,
                    input.name,
                    input.icon,
                    input.description,
                    privacy_tier,
                    priority_profile,
                    sort_order,
                    meta
                ],
            )
            .map_err(|err| format!("Failed inserting vault: {err}"))?;
        }

        tx.commit()
            .map_err(|err| format!("Failed committing vault_create: {err}"))?;

        fetch_vault_by_id(&conn, &id)
    })())
}

#[tauri::command]
fn vault_list(state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Vault>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        let mut statement = conn
            .prepare(
                "SELECT id, parent_vault_id, name, icon, description, privacy_tier, priority_profile, summary_node_id,
                        sort_order, created_at, updated_at, deleted_at, meta
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
                           meta
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
                           meta
                    FROM sub_vaults
                    WHERE deleted_at IS NULL
                 )
                 ORDER BY sort_order ASC, created_at ASC;",
            )
            .map_err(|err| format!("Failed preparing vault_list query: {err}"))?;

        let rows = statement
            .query_map([], vault_from_row)
            .map_err(|err| format!("Failed querying vaults: {err}"))?;

        let mut vaults = Vec::new();
        for row in rows {
            vaults.push(row.map_err(|err| format!("Failed decoding vault row: {err}"))?);
        }
        Ok(vaults)
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
        let current = fetch_vault_by_id(&tx, &vault_id)?;
        let next_name = input.name.unwrap_or(current.name);
        let next_privacy_tier = input.privacy_tier.unwrap_or(current.privacy_tier);
        let next_priority_profile = input.priority_profile.unwrap_or(current.priority_profile);

        let affected_vaults = tx
            .execute(
                "UPDATE vaults
                 SET name = ?2,
                     privacy_tier = ?3,
                     priority_profile = ?4,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![
                    &vault_id,
                    &next_name,
                    &next_privacy_tier,
                    &next_priority_profile
                ],
            )
            .map_err(|err| format!("Failed updating vault: {err}"))?;

        if affected_vaults == 0 {
            tx.execute(
                "UPDATE sub_vaults
                 SET name = ?2,
                     privacy_tier = ?3,
                     priority_profile = ?4,
                     updated_at = datetime('now')
                 WHERE id = ?1 AND deleted_at IS NULL;",
                params![
                    &vault_id,
                    &next_name,
                    &next_privacy_tier,
                    &next_priority_profile
                ],
            )
            .map_err(|err| format!("Failed updating sub-vault: {err}"))?;
        }

        tx.commit()
            .map_err(|err| format!("Failed committing vault_update: {err}"))?;

        fetch_vault_by_id(&conn, &vault_id)
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

        tx.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                privacy_tier, priority, meta
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12);",
            params![
                id,
                input.vault_id,
                input.sub_vault_id,
                node_type,
                input.title,
                input.summary,
                input.detail,
                input.source,
                input.source_type,
                input.privacy_tier,
                priority_json,
                meta
            ],
        )
        .map_err(|err| format!("Failed inserting node: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing node_create: {err}"))?;

        fetch_node_by_id(&conn, &id)
            .and_then(|node| node.ok_or_else(|| "Node not found after insert".to_string()))
    })())
}

#[tauri::command]
fn node_get(node_id: String, state: tauri::State<'_, DbState>) -> IpcResponse<Option<Node>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        match fetch_node_by_id(&conn, &node_id)? {
            Some(node) if node.deleted_at.is_none() => Ok(Some(node)),
            _ => Ok(None),
        }
    })())
}

#[tauri::command]
fn node_list(state: tauri::State<'_, DbState>) -> IpcResponse<Vec<Node>> {
    into_ipc((|| {
        let conn = open_connection(&state.db_path)?;
        fetch_nodes(&conn)
    })())
}

#[tauri::command]
fn node_update(input: NodeUpdateInput, state: tauri::State<'_, DbState>) -> IpcResponse<Node> {
    into_ipc((|| {
        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting node_update transaction: {err}"))?;

        let current = fetch_node_by_id(&tx, &input.id)?
            .filter(|node| node.deleted_at.is_none())
            .ok_or_else(|| format!("Node not found: {}", input.id))?;

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
                 meta = ?14
             WHERE id = ?1 AND deleted_at IS NULL;",
            params![
                input.id,
                next_vault_id,
                next_sub_vault_id,
                next_node_type,
                next_title,
                next_summary,
                next_detail,
                next_source,
                next_source_type,
                next_privacy_tier,
                next_priority,
                next_version,
                next_is_archived,
                next_meta
            ],
        )
        .map_err(|err| format!("Failed updating node: {err}"))?;

        tx.commit()
            .map_err(|err| format!("Failed committing node_update: {err}"))?;

        fetch_node_by_id(&conn, &input.id).and_then(|node| {
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
            .get("today_touches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        priority_obj["today_touches"] = serde_json::json!(current_touches + 1);

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
        let conn = open_connection(&state.db_path)?;
        llm::assembler::build_context(
            &conn,
            node_ids,
            llm::assembler::AssemblerConfig {
                scope,
                max_tokens: DEFAULT_ASSEMBLER_MAX_TOKENS,
            },
        )
    })())
}

#[tauri::command]
fn llm_count_tokens(text: String) -> IpcResponse<usize> {
    into_ipc(Ok(crate::llm::assembler::count_tokens(&text)))
}

#[tauri::command]
async fn llm_list_models(provider: String, endpoint: String) -> IpcResponse<Vec<String>> {
    let parsed_provider = match provider.trim().to_lowercase().as_str() {
        "ollama" => llm::client::LlmProvider::Ollama,
        "lmstudio" => llm::client::LlmProvider::LmStudio,
        _ => {
            return IpcResponse::Err {
                err: "Unsupported provider. Use 'ollama' or 'lmstudio'.".to_string(),
            }
        }
    };
    let client = llm::client::UniversalClient::new(parsed_provider, endpoint, String::new());
    into_ipc(llm::client::LlmClient::list_models(&client).await)
}

#[tauri::command]
async fn llm_chat(
    node_ids: Vec<String>,
    scope: String,
    provider: String,
    endpoint: String,
    model: String,
    user_prompt: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let db_path = state.db_path.clone();

    let system_prompt_from_assembler = {
        let conn = open_connection(&db_path)?;
        llm::assembler::build_context(
            &conn,
            node_ids,
            llm::assembler::AssemblerConfig {
                scope,
                max_tokens: DEFAULT_ASSEMBLER_MAX_TOKENS,
            },
        )
    }?;

    let parsed_provider = match provider.trim().to_lowercase().as_str() {
        "ollama" => llm::client::LlmProvider::Ollama,
        "lmstudio" => llm::client::LlmProvider::LmStudio,
        _ => return Err("Unsupported provider. Use 'ollama' or 'lmstudio'.".to_string()),
    };

    let client = llm::client::UniversalClient::new(parsed_provider, endpoint, model);
    let messages = [llm::client::LlmMessage {
        role: "user".to_string(),
        content: user_prompt,
    }];
    llm::client::LlmClient::complete(&client, &system_prompt_from_assembler, &messages).await
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
        _ => return Err("Unsupported provider. Use 'ollama' or 'lmstudio'.".to_string()),
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
fn onboarding_commit(
    proposals: Vec<OnboardingNodeCommitInput>,
    state: tauri::State<'_, DbState>,
) -> IpcResponse<bool> {
    into_ipc((|| {
        if !proposals.is_empty() {
            let _ = minimal_pre_write_backup(&state.db_path, "onboarding-commit")?;
        }

        let mut conn = open_connection(&state.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("Failed starting onboarding_commit transaction: {err}"))?;

        for proposal in &proposals {
            let vault_id = proposal.vault_id.trim();
            if vault_id.is_empty() {
                return Err("Onboarding commit row is missing vault_id".to_string());
            }
            ensure_onboarding_vault_exists(&tx, vault_id)?;

            let title = proposal.title.trim();
            if title.is_empty() {
                return Err("Onboarding commit row has empty title".to_string());
            }
            let summary = proposal.summary.trim();
            if summary.is_empty() {
                return Err("Onboarding commit row has empty summary".to_string());
            }

            let node_id = generate_id(&tx, "node")?;
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
            let detail = proposal
                .detail
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            let priority_json = priority::DEFAULT_PRIORITY_JSON;

            tx.execute(
                "INSERT INTO nodes (
                    id, vault_id, sub_vault_id, node_type, title, summary, detail, source, source_type,
                    privacy_tier, priority, meta
                ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10);",
                params![
                    node_id,
                    vault_id,
                    node_type,
                    title,
                    summary,
                    detail,
                    Some("onboarding_wizard"),
                    source_type,
                    priority_json,
                    "{}"
                ],
            )
            .map_err(|err| format!("Failed inserting onboarding node: {err}"))?;
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
    })())
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
        let today_touches = priority_obj
            .get("today_touches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if today_touches > 0 {
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

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            db_ping,
            settings_get,
            settings_set,
            chat_get_history,
            chat_append_message,
            chat_clear_history,
            vault_create,
            vault_list,
            vault_delete,
            vault_update,
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
            onboarding_commit
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("error while running tauri application: {err}");
            std::process::exit(1);
        });
}
