use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Default session id used to anchor the always-on chat thread until
/// multi-session UI lands. The schema requires `session_id NOT NULL`
/// (see db/migrations/0001_schema_v1.sql), so we keep one canonical row.
const DEFAULT_SESSION_ID: &str = "default-session";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

fn ensure_default_session(db: &Connection) -> Result<(), crate::AppError> {
    db.execute(
        "INSERT OR IGNORE INTO sessions (id, scope_json) VALUES (?1, '[]');",
        params![DEFAULT_SESSION_ID],
    )
    .map_err(|err| {
        eprintln!("Database error in ensure_default_session: {err}");
        "Failed ensuring default chat session".to_string()
    })?;
    Ok(())
}

pub fn get_chat_history(db: &Connection) -> Result<Vec<ChatMessage>, crate::AppError> {
    ensure_default_session(db)?;

    let mut statement = db
        .prepare(
            "SELECT id, role, content, created_at
             FROM session_messages
             WHERE session_id = ?1
             ORDER BY created_at ASC, id ASC;",
        )
        .map_err(|err| {
            eprintln!("Database error preparing chat history query: {err}");
            "Failed preparing chat history query".to_string()
        })?;

    let rows = statement
        .query_map(params![DEFAULT_SESSION_ID], |row| {
            Ok(ChatMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|err| {
            eprintln!("Database error querying chat history: {err}");
            "Failed querying chat history".to_string()
        })?;

    let mut messages = Vec::new();
    for row in rows {
        messages.push(row.map_err(|err| {
            eprintln!("Database error decoding chat history row: {err}");
            "Failed decoding chat history row".to_string()
        })?);
    }
    Ok(messages)
}

pub fn append_message(
    db: &Connection,
    id: String,
    role: String,
    content: String,
) -> Result<(), crate::AppError> {
    ensure_default_session(db)?;

    db.execute(
        "INSERT INTO session_messages (id, session_id, role, content)
         VALUES (?1, ?2, ?3, ?4);",
        params![id, DEFAULT_SESSION_ID, role, content],
    )
    .map_err(|err| {
        eprintln!("Database error appending chat message: {err}");
        "Failed appending chat message".to_string()
    })?;

    Ok(())
}

pub fn clear_chat_history(db: &Connection) -> Result<(), crate::AppError> {
    ensure_default_session(db)?;

    db.execute(
        "DELETE FROM session_messages WHERE session_id = ?1;",
        params![DEFAULT_SESSION_ID],
    )
    .map_err(|err| {
        eprintln!("Database error clearing chat history: {err}");
        "Failed clearing chat history".to_string()
    })?;

    Ok(())
}

pub fn edit_and_truncate(
    db: &Connection,
    edit_id: &str,
    new_content: &str,
    delete_ids: Vec<String>,
) -> Result<(), crate::AppError> {
    ensure_default_session(db)?;

    db.execute(
        "UPDATE session_messages SET content = ?1 WHERE id = ?2;",
        params![new_content, edit_id],
    )
    .map_err(|err| {
        eprintln!("Database error updating chat message: {err}");
        "Failed updating chat message".to_string()
    })?;

    if !delete_ids.is_empty() {
        let placeholders = vec!["?"; delete_ids.len()].join(", ");
        let query_str = format!("DELETE FROM session_messages WHERE id IN ({placeholders});");
        let mut stmt = db.prepare(&query_str).map_err(|err| {
            eprintln!("Database error preparing delete query: {err}");
            "Failed preparing delete query".to_string()
        })?;
        let params: Vec<&dyn rusqlite::ToSql> = delete_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        stmt.execute(rusqlite::params_from_iter(params))
            .map_err(|err| {
                eprintln!("Database error deleting subsequent chat messages: {err}");
                "Failed deleting subsequent chat messages".to_string()
            })?;
    }

    Ok(())
}
