use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

fn ensure_session(db: &Connection, session_id: &str) -> Result<(), crate::AppError> {
    db.execute(
        "INSERT OR IGNORE INTO sessions (id, scope_json) VALUES (?1, '[]');",
        params![session_id],
    )
    .map_err(|err| {
        eprintln!("Database error in ensure_session for {session_id}: {err}");
        "Failed ensuring chat session".to_string()
    })?;
    Ok(())
}

// MARK: Public API

pub const TEMPORARY_SESSION_ID: &str = "temporary-session";
const DEFAULT_CONVERTED_SESSION_SUMMARY: &str = "Saved Brainstorm";

fn build_session_summary(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return DEFAULT_CONVERTED_SESSION_SUMMARY.to_string();
    }

    let summary: String = trimmed.chars().take(40).collect();
    if trimmed.chars().count() > 40 {
        format!("{summary}...")
    } else {
        summary
    }
}

fn derive_converted_session_summary(rows: &[(String, String, String)]) -> String {
    rows.iter()
        .find_map(|(role, content, _)| {
            if role == "user" && !content.trim().is_empty() {
                Some(build_session_summary(content))
            } else {
                None
            }
        })
        .unwrap_or_else(|| DEFAULT_CONVERTED_SESSION_SUMMARY.to_string())
}

fn generate_session_id(db: &Connection) -> Result<String, String> {
    db.query_row("SELECT lower(hex(randomblob(16)));", [], |row| row.get(0))
        .map_err(|err| format!("Failed generating conversation id: {err}"))
}

fn select_saved_session_id(
    db: &Connection,
    target_session_id: Option<&str>,
    summary: &str,
    started_at: &str,
) -> Result<String, String> {
    if let Some(target_session_id) = target_session_id.filter(|id| *id != TEMPORARY_SESSION_ID) {
        ensure_session(db, target_session_id)
            .map_err(|err| format!("Failed ensuring target session: {err}"))?;

        let message_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1;",
                params![target_session_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("Failed checking target session message count: {err}"))?;

        if message_count == 0 {
            db.execute(
                "UPDATE sessions
                 SET summary = ?1, started_at = ?2
                 WHERE id = ?3;",
                params![summary, started_at, target_session_id],
            )
            .map_err(|err| format!("Failed updating target session metadata: {err}"))?;
            return Ok(target_session_id.to_string());
        }
    }

    let new_session_id = generate_session_id(db)?;
    db.execute(
        "INSERT INTO sessions (id, scope_json, summary, started_at)
         VALUES (?1, '[]', ?2, ?3);",
        params![&new_session_id, summary, started_at],
    )
    .map_err(|err| format!("Failed creating saved conversation: {err}"))?;

    Ok(new_session_id)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

pub fn append_message(
    db: &Connection,
    id: String,
    role: String,
    content: String,
    session_id: &str,
) -> Result<(), crate::AppError> {
    ensure_session(db, session_id)?;

    db.execute(
        "INSERT INTO session_messages (id, session_id, role, content)
         VALUES (?1, ?2, ?3, ?4);",
        params![id, session_id, role, content],
    )
    .map_err(|err| {
        eprintln!("Database error appending chat message: {err}");
        "Failed appending chat message".to_string()
    })?;

    Ok(())
}

pub fn edit_and_truncate(
    db: &Connection,
    edit_id: &str,
    new_content: &str,
    delete_ids: Vec<String>,
    session_id: &str,
) -> Result<(), crate::AppError> {
    ensure_session(db, session_id)?;

    // Wrap in a savepoint to ensure absolute atomicity across updates and batch deletes
    db.execute("SAVEPOINT edit_and_truncate_sp;", [])
        .map_err(|err| {
            eprintln!("Database error starting edit_and_truncate savepoint: {err}");
            "Failed starting chat message truncation".to_string()
        })?;

    let run_ops = || -> Result<(), crate::AppError> {
        db.execute(
            "UPDATE session_messages SET content = ?1 WHERE session_id = ?2 AND id = ?3;",
            params![new_content, session_id, edit_id],
        )
        .map_err(|err| {
            eprintln!("Database error updating chat message: {err}");
            "Failed updating chat message".to_string()
        })?;

        if !delete_ids.is_empty() {
            let placeholders = vec!["?"; delete_ids.len()].join(", ");
            let query_str = format!(
                "DELETE FROM session_messages WHERE session_id = ?1 AND id IN ({placeholders});"
            );
            let mut stmt = db.prepare(&query_str).map_err(|err| {
                eprintln!("Database error preparing delete query: {err}");
                "Failed preparing delete query".to_string()
            })?;

            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(delete_ids.len() + 1);
            params.push(&session_id as &dyn rusqlite::ToSql);
            for id in &delete_ids {
                params.push(id as &dyn rusqlite::ToSql);
            }

            stmt.execute(rusqlite::params_from_iter(params))
                .map_err(|err| {
                    eprintln!("Database error deleting subsequent chat messages: {err}");
                    "Failed deleting subsequent chat messages".to_string()
                })?;
        }
        Ok(())
    };

    match run_ops() {
        Ok(()) => {
            db.execute("RELEASE edit_and_truncate_sp;", [])
                .map_err(|err| {
                    eprintln!("Database error releasing edit_and_truncate savepoint: {err}");
                    "Failed committing chat truncation".to_string()
                })?;
            Ok(())
        }
        Err(err) => {
            if let Err(rollback_err) = db.execute("ROLLBACK TO edit_and_truncate_sp;", []) {
                eprintln!(
                    "Database error during edit_and_truncate savepoint rollback: {rollback_err}"
                );
            }
            Err(err)
        }
    }
}

pub fn get_chat_history(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<ChatMessage>, crate::AppError> {
    ensure_session(db, session_id)?;

    let mut statement = db
        .prepare(
            "SELECT id, role, content, coalesce(created_at, datetime('now'))
             FROM session_messages
             WHERE session_id = ?1
             ORDER BY created_at ASC, rowid ASC;",
        )
        .map_err(|err| {
            eprintln!("Database error preparing chat history query: {err}");
            "Failed preparing chat history query".to_string()
        })?;

    let rows = statement
        .query_map(params![session_id], |row| {
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

pub fn clear_chat_history(db: &Connection, session_id: &str) -> Result<(), crate::AppError> {
    ensure_session(db, session_id)?;

    db.execute(
        "DELETE FROM session_messages WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|err| {
        eprintln!("Database error clearing chat history: {err}");
        "Failed clearing chat history".to_string()
    })?;

    Ok(())
}

/// Purges the temporary session and all cascading messages from the database.
pub fn purge_temporary_session(db: &Connection) -> Result<(), String> {
    // ON DELETE CASCADE automatically deletes all temporary session messages
    db.execute(
        "DELETE FROM sessions WHERE id = ?1;",
        params![TEMPORARY_SESSION_ID],
    )
    .map_err(|err| format!("Failed to purge temporary session: {err}"))?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConvertedSession {
    pub session_id: String,
    pub summary: Option<String>,
}

pub fn convert_temporary_to_memory(
    conn: &mut Connection,
    target_session_id: Option<&str>,
) -> Result<ConvertedSession, String> {
    let sp = conn
        .savepoint()
        .map_err(|err| format!("Failed starting conversion savepoint: {err}"))?;

    let rows: Vec<(String, String, String)> = {
        let mut stmt = sp
            .prepare(
                "SELECT role, content, coalesce(created_at, datetime('now')) FROM session_messages
                 WHERE session_id = ?1
                 ORDER BY created_at ASC, rowid ASC;",
            )
            .map_err(|err| format!("Failed preparing temporary session query: {err}"))?;

        let res = stmt
            .query_map(params![TEMPORARY_SESSION_ID], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|err| format!("Failed querying temporary messages: {err}"))?
            .collect::<Result<Vec<(String, String, String)>, _>>()
            .map_err(|err| format!("Failed reading temporary message row: {err}"))?;
        res
    };

    if rows.is_empty() {
        return Err("No off-the-record messages to save.".to_string());
    }

    let summary = derive_converted_session_summary(&rows);
    let started_at = rows
        .first()
        .map(|(_, _, created_at)| created_at.clone())
        .ok_or_else(|| "No off-the-record messages to save.".to_string())?;

    let saved_session_id = select_saved_session_id(&sp, target_session_id, &summary, &started_at)?;

    sp.execute(
        "UPDATE session_messages
         SET session_id = ?1
         WHERE session_id = ?2;",
        params![&saved_session_id, TEMPORARY_SESSION_ID],
    )
    .map_err(|err| format!("Failed moving brainstorm messages: {err}"))?;

    sp.execute(
        "DELETE FROM sessions WHERE id = ?1;",
        params![TEMPORARY_SESSION_ID],
    )
    .map_err(|err| format!("Failed deleting temporary session shell: {err}"))?;

    sp.commit()
        .map_err(|err| format!("Failed committing savepoint: {err}"))?;

    Ok(ConvertedSession {
        session_id: saved_session_id,
        summary: Some(summary),
    })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    pub vault_id: Option<String>,
    pub started_at: String,
    pub summary: Option<String>,
}

pub fn list_sessions(db: &Connection) -> Result<Vec<ChatSession>, crate::AppError> {
    let mut statement = db
        .prepare(
            "SELECT id, vault_id, coalesce(started_at, datetime('now')), summary
             FROM sessions
             WHERE id != 'temporary-session'
             ORDER BY started_at DESC, rowid DESC;",
        )
        .map_err(|err| {
            eprintln!("Database error preparing sessions list query: {err}");
            "Failed preparing sessions list query".to_string()
        })?;

    let rows = statement
        .query_map([], |row| {
            Ok(ChatSession {
                id: row.get(0)?,
                vault_id: row.get(1)?,
                started_at: row.get(2)?,
                summary: row.get(3)?,
            })
        })
        .map_err(|err| {
            eprintln!("Database error querying sessions list: {err}");
            "Failed querying sessions list".to_string()
        })?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row.map_err(|err| format!("Failed to read session row: {err}"))?);
    }
    Ok(sessions)
}

pub fn create_session(
    db: &Connection,
    id: String,
    summary: Option<String>,
) -> Result<(), crate::AppError> {
    db.execute(
        "INSERT INTO sessions (id, scope_json, summary) VALUES (?1, '[]', ?2);",
        params![id, summary],
    )
    .map_err(|err| {
        eprintln!("Database error creating session: {err}");
        "Failed creating session".to_string()
    })?;
    Ok(())
}

pub fn delete_session(db: &Connection, id: &str) -> Result<(), crate::AppError> {
    db.execute("DELETE FROM sessions WHERE id = ?1;", params![id])
        .map_err(|err| {
            eprintln!("Database error deleting session: {err}");
            "Failed deleting session".to_string()
        })?;
    Ok(())
}

pub fn update_session_summary(
    db: &Connection,
    id: &str,
    summary: &str,
) -> Result<(), crate::AppError> {
    db.execute(
        "UPDATE sessions SET summary = ?1 WHERE id = ?2;",
        params![summary, id],
    )
    .map_err(|err| {
        eprintln!("Database error updating session summary: {err}");
        "Failed updating session summary".to_string()
    })?;
    Ok(())
}

/// Purges all empty sessions (0 messages) from the database except the most recent one.
pub fn purge_empty_sessions(db: &Connection) -> Result<(), String> {
    db.execute(
        "DELETE FROM sessions
         WHERE id != 'temporary-session'
           AND id NOT IN (SELECT DISTINCT session_id FROM session_messages)
           AND id NOT IN (
               SELECT id FROM sessions
               WHERE id NOT IN (SELECT DISTINCT session_id FROM session_messages)
                 AND id != 'temporary-session'
               ORDER BY started_at DESC, rowid DESC
               LIMIT 1
           );",
        [],
    )
    .map_err(|err| format!("Failed to purge empty sessions: {err}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Result<Connection, Box<dyn std::error::Error>> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                scope_json TEXT NOT NULL DEFAULT '[]',
                started_at TEXT NOT NULL DEFAULT (datetime('now')),
                summary TEXT
            );
            CREATE TABLE session_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now')),
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                scope TEXT NOT NULL,
                updated_at TEXT
            );
        ",
        )?;
        Ok(conn)
    }

    #[test]
    fn test_purge_immediate() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;

        // Add temporary message
        append_message(
            &conn,
            "msg_1".to_string(),
            "user".to_string(),
            "Hello brainstorm 1".to_string(),
            TEMPORARY_SESSION_ID,
        )?;

        let history = get_chat_history(&conn, TEMPORARY_SESSION_ID)?;
        assert_eq!(history.len(), 1);

        purge_temporary_session(&conn)?;

        // Verify it is completely purged
        let history_after = get_chat_history(&conn, TEMPORARY_SESSION_ID)?;
        assert_eq!(history_after.len(), 0);
        Ok(())
    }

    #[test]
    fn test_convert_temporary_to_memory_reuses_empty_target_session(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = setup_test_db()?;

        create_session(
            &conn,
            "target-session".to_string(),
            Some("New Conversation".to_string()),
        )?;
        append_message(
            &conn,
            "msg_user".to_string(),
            "user".to_string(),
            "Plan the launch checklist".to_string(),
            TEMPORARY_SESSION_ID,
        )?;
        append_message(
            &conn,
            "msg_assistant".to_string(),
            "assistant".to_string(),
            "Here is a first draft.".to_string(),
            TEMPORARY_SESSION_ID,
        )?;

        let converted = convert_temporary_to_memory(&mut conn, Some("target-session"))?;
        assert_eq!(converted.session_id, "target-session");
        assert_eq!(
            converted.summary.as_deref(),
            Some("Plan the launch checklist")
        );

        let saved_history = get_chat_history(&conn, "target-session")?;
        assert_eq!(saved_history.len(), 2);
        assert_eq!(saved_history[0].id, "msg_user");
        assert_eq!(saved_history[1].id, "msg_assistant");

        let temp_message_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1;",
            params![TEMPORARY_SESSION_ID],
            |row| row.get(0),
        )?;
        assert_eq!(temp_message_count, 0);
        Ok(())
    }

    #[test]
    fn test_purge_empty_sessions() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;

        create_session(
            &conn,
            "sess_with_msgs".to_string(),
            Some("Has Messages".to_string()),
        )?;
        append_message(
            &conn,
            "m1".to_string(),
            "user".to_string(),
            "hello".to_string(),
            "sess_with_msgs",
        )?;

        create_session(
            &conn,
            "sess_empty_old".to_string(),
            Some("Empty Old".to_string()),
        )?;
        conn.execute(
            "UPDATE sessions SET started_at = '2026-07-13 10:00:00' WHERE id = 'sess_empty_old';",
            [],
        )?;

        create_session(
            &conn,
            "sess_empty_new".to_string(),
            Some("Empty New".to_string()),
        )?;
        conn.execute(
            "UPDATE sessions SET started_at = '2026-07-13 12:00:00' WHERE id = 'sess_empty_new';",
            [],
        )?;

        purge_empty_sessions(&conn)?;

        // "sess_with_msgs" still exists
        let has_msgs_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'sess_with_msgs';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(has_msgs_exists, 1);

        // "sess_empty_old" was purged
        let empty_old_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'sess_empty_old';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(empty_old_exists, 0);

        // "sess_empty_new" still exists (most recent empty session)
        let empty_new_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 'sess_empty_new';",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(empty_new_exists, 1);

        Ok(())
    }
}
