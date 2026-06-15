use crate::memory_agent::correction;
use rusqlite::{params, Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

fn get_setting_int(conn: &Connection, key: &str) -> Result<i64, String> {
    let val_str: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("Failed reading setting {key}: {err}"))?;

    match val_str {
        Some(s) => s
            .parse::<i64>()
            .map_err(|err| format!("Failed parsing setting {key} as i64: {err}")),
        None => Ok(0),
    }
}

fn set_setting_int(conn: &Connection, key: &str, value: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value, scope, updated_at)
         VALUES (?1, ?2, 'global', datetime('now'))
         ON CONFLICT(key) DO UPDATE
         SET value = excluded.value,
             scope = excluded.scope,
             updated_at = datetime('now');",
        params![key, value.to_string()],
    )
    .map_err(|err| format!("Failed writing setting {key}: {err}"))?;
    Ok(())
}

/// Resets the last extraction message count to 0 if the chat history has been cleared or reset.
pub fn align_last_extract_count(
    conn: &Connection,
    current_message_count: i64,
) -> Result<(), String> {
    let last_extract_message_count =
        get_setting_int(conn, "memory_agent_last_extract_message_count")?;

    if current_message_count < last_extract_message_count {
        set_setting_int(conn, "memory_agent_last_extract_message_count", 0)?;
    }
    Ok(())
}

/// Evaluates whether a session is ready for background memory extraction.
/// Returns true if the message count since the last extraction is >= 6
/// AND the time since the last extraction is >= 2 minutes (120 seconds).
pub fn should_extract(conn: &Connection, session_id: &str) -> Result<bool, String> {
    // 1. Count current messages in the session
    let current_message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1;",
            [session_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed querying session message count: {err}"))?;

    // 2. Retrieve last extracted count
    let last_extract_message_count =
        get_setting_int(conn, "memory_agent_last_extract_message_count")?;

    // 3. Compute message count difference
    let diff = current_message_count - last_extract_message_count;
    if diff < 6 {
        return Ok(false);
    }

    // 4. Retrieve last extraction timestamp
    let last_extract_timestamp = get_setting_int(conn, "memory_agent_last_extract_timestamp")?;

    // 5. Fetch current epoch time in seconds
    let current_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("System clock before UNIX_EPOCH: {err}"))?
        .as_secs() as i64;

    // 6. Check time debounce (120 seconds = 2 minutes)
    let time_diff = current_timestamp - last_extract_timestamp;
    if time_diff < 120 {
        return Ok(false);
    }

    Ok(true)
}

/// Marks extraction complete by updating settings with the current message count and timestamp.
pub fn mark_extraction_complete(
    conn: &Connection,
    current_message_count: i64,
) -> Result<(), String> {
    let current_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("System clock before UNIX_EPOCH: {err}"))?
        .as_secs() as i64;

    set_setting_int(
        conn,
        "memory_agent_last_extract_message_count",
        current_message_count,
    )?;
    set_setting_int(
        conn,
        "memory_agent_last_extract_timestamp",
        current_timestamp,
    )?;

    Ok(())
}

/// Evaluates whether a correction signal should bypass the standard debounce gate.
/// Returns true if a correction was detected AND there are at least 3 messages
/// in the session (minimum viable context for extraction).
pub fn should_extract_correction(
    conn: &Connection,
    session_id: &str,
    message: &str,
) -> Result<bool, String> {
    // 1. Query latest user message prior to this one in session
    let previous_message: Option<String> = conn
        .query_row(
            "SELECT content FROM session_messages WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 1 OFFSET 1;",
            [session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("Failed querying latest message: {err}"))?;

    // 2. Query all pending changeset_items with status 'pending' for this session and extract their proposed_data column values
    let pending_data: Vec<String> = conn
        .prepare(
            "SELECT ci.proposed_data \
             FROM changeset_items ci \
             JOIN changesets c ON ci.changeset_id = c.id \
             WHERE ci.status = 'pending' AND c.session_id = ?1;",
        )
        .map_err(|err| format!("Failed preparing pending changeset query: {err}"))?
        .query_map([session_id], |row| row.get(0))
        .map_err(|err| format!("Failed querying pending changeset items: {err}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|err| format!("Failed reading pending changeset row: {err}"))?;

    let signal: bool =
        correction::has_correction_signal(message, previous_message.as_deref(), &pending_data);

    // 3. Check for signal detection and message count threshold (3)
    let message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1;",
            [session_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("Failed querying session message count: {err}"))?;

    if signal && message_count >= 3 {
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = match Connection::open_in_memory() {
            Ok(c) => c,
            Err(e) => panic!("Failed to open in-memory DB: {e}"),
        };
        let ddl = "
            CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                scope TEXT NOT NULL,
                updated_at TEXT
            );
            CREATE TABLE session_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now'))
            );
        ";
        if let Err(e) = conn.execute_batch(ddl) {
            panic!("Failed to create DDL: {e}");
        }
        conn
    }

    static MSG_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn add_test_messages(conn: &Connection, count: usize) {
        use std::sync::atomic::Ordering;
        for _ in 0..count {
            let idx = MSG_COUNTER.fetch_add(1, Ordering::SeqCst);
            let id = format!("msg_{idx}");
            if let Err(e) = conn.execute(
                "INSERT INTO session_messages (id, session_id, role, content) VALUES (?1, 'default-session', 'user', 'hello');",
                params![id],
            ) {
                panic!("Failed to insert test message: {e}");
            }
        }
    }

    #[test]
    fn test_should_extract_conditions() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db();
        let session_id = "default-session";

        // 1. Empty chat history: should be false
        assert!(!should_extract(&conn, session_id)?);

        // 2. Add 5 messages (less than 6 threshold): should be false
        add_test_messages(&conn, 5);
        assert!(!should_extract(&conn, session_id)?);

        // 3. Add 1 more message (total 6): should be true since timestamp defaults to 0
        add_test_messages(&conn, 1);
        assert!(should_extract(&conn, session_id)?);

        // 4. Mark extraction complete at 6 messages
        mark_extraction_complete(&conn, 6)?;

        // 5. Try triggering immediately: should be false (time debounce triggered)
        assert!(!should_extract(&conn, session_id)?);

        // 6. Force timestamp backward by 150 seconds to bypass debounce
        let current_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        set_setting_int(
            &conn,
            "memory_agent_last_extract_timestamp",
            current_timestamp - 150,
        )?;

        // 7. Still false because current count (6) is not >= 6 messages since last extraction (6)
        assert!(!should_extract(&conn, session_id)?);

        // 8. Add 6 more messages (total 12)
        add_test_messages(&conn, 6);

        // 9. Now should be true (both count diff >= 6 and debounce window passed)
        assert!(should_extract(&conn, session_id)?);

        Ok(())
    }

    #[test]
    fn test_message_deletion_does_not_trigger_extraction() -> Result<(), Box<dyn std::error::Error>>
    {
        let conn = setup_test_db();
        let session_id = "default-session";

        // 1. Add 10 messages and mark extraction complete
        add_test_messages(&conn, 10);
        mark_extraction_complete(&conn, 10)?;

        // 2. Force timestamp backward to bypass debounce
        let current_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        set_setting_int(
            &conn,
            "memory_agent_last_extract_timestamp",
            current_timestamp - 300,
        )?;

        // 3. Simulate user deleting messages: remove 5 messages so only 5 remain
        conn.execute(
            "DELETE FROM session_messages WHERE id IN (
                SELECT id FROM session_messages WHERE session_id = ?1 LIMIT 5
            );",
            params![session_id],
        )?;

        // 4. Verify count dropped below last_extract_message_count
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1;",
            [session_id],
            |row| row.get(0),
        )?;
        assert!(
            remaining < 10,
            "Expected fewer than 10 messages after deletion"
        );

        // 5. Run alignment (should reset setting to 0 since 5 < 10)
        align_last_extract_count(&conn, remaining)?;
        assert_eq!(
            get_setting_int(&conn, "memory_agent_last_extract_message_count")?,
            0
        );

        // 6. Should NOT trigger extraction even though debounce has passed,
        //    because no new messages were added (remaining 5 messages < 6 required).
        assert!(
            !should_extract(&conn, session_id)?,
            "Deleting messages should not trigger extraction"
        );

        Ok(())
    }
}
