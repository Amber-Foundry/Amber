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
            "SELECT id, role, content, created_at
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

/// Purges the temporary session messages based on the configured retention policy.
pub fn purge_temporary_session_with_retention(
    db: &Connection,
    retention: &str,
) -> Result<(), String> {
    if retention == "7_days" {
        // Delete messages older than 7 days
        db.execute(
            "DELETE FROM session_messages 
             WHERE session_id = ?1 
               AND created_at <= datetime('now', '-7 days');",
            params![TEMPORARY_SESSION_ID],
        )
        .map_err(|err| format!("Failed to purge old temporary session messages: {err}"))?;
    } else {
        // Default to immediate deletion of the entire temporary session
        purge_temporary_session(db)?;
    }
    Ok(())
}

pub fn convert_temporary_to_memory(conn: &Connection) -> Result<(), String> {
    conn.execute("SAVEPOINT convert_temporary_sp;", [])
        .map_err(|err| format!("Failed starting conversion savepoint: {err}"))?;

    let run_convert = || -> Result<(), String> {
        let mut stmt = conn
            .prepare(
                "SELECT role, content, created_at FROM session_messages
                 WHERE session_id = 'temporary-session'
                 ORDER BY created_at ASC, rowid ASC;",
            )
            .map_err(|err| format!("Failed preparing temporary session query: {err}"))?;

        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|err| format!("Failed querying temporary messages: {err}"))?
            .collect::<Result<_, _>>()
            .map_err(|err| format!("Failed reading temporary message row: {err}"))?;

        for (role, content, created_at) in rows {
            conn.execute(
                "INSERT INTO session_messages (id, session_id, role, content, created_at)
                 VALUES (lower(hex(randomblob(8))), 'default-session', ?1, ?2, ?3);",
                params![role, content, created_at],
            )
            .map_err(|err| format!("Failed inserting converted message: {err}"))?;
        }

        purge_temporary_session(conn)?;
        Ok(())
    };

    match run_convert() {
        Ok(()) => {
            conn.execute("RELEASE convert_temporary_sp;", [])
                .map_err(|err| format!("Failed releasing savepoint: {err}"))?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK TO convert_temporary_sp;", []);
            Err(err)
        }
    }
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
                scope_json TEXT NOT NULL DEFAULT '[]'
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

        // Call retention check with "immediate"
        purge_temporary_session_with_retention(&conn, "immediate")?;

        // Verify it is completely purged
        let history_after = get_chat_history(&conn, TEMPORARY_SESSION_ID)?;
        assert_eq!(history_after.len(), 0);
        Ok(())
    }

    #[test]
    fn test_purge_7_days() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;

        // Insert a new temporary session manually
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, scope_json) VALUES ('temporary-session', '[]');",
            [],
        )?;

        // Add old message (older than 7 days)
        conn.execute(
            "INSERT INTO session_messages (id, session_id, role, content, created_at)
             VALUES ('msg_old', 'temporary-session', 'user', 'Old brainstorm', datetime('now', '-8 days'));",
            [],
        )?;

        // Add new message (today)
        conn.execute(
            "INSERT INTO session_messages (id, session_id, role, content, created_at)
             VALUES ('msg_new', 'temporary-session', 'user', 'New brainstorm', datetime('now'));",
            [],
        )?;

        let history = get_chat_history(&conn, TEMPORARY_SESSION_ID)?;
        assert_eq!(history.len(), 2);

        // Call retention check with "7_days"
        purge_temporary_session_with_retention(&conn, "7_days")?;

        // Verify the old one was deleted and the new one was kept
        let history_after = get_chat_history(&conn, TEMPORARY_SESSION_ID)?;
        assert_eq!(history_after.len(), 1);
        assert_eq!(history_after[0].id, "msg_new");
        Ok(())
    }
}
