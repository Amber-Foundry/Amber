use amber_lib::{
    check_vault_parent_assignment, fetch_descendant_vault_ids, resolve_vault_effective_privacy,
};
use rusqlite::Connection;
use std::error::Error;
use std::time::Instant;

fn setup_migrated_db() -> Result<Connection, Box<dyn Error>> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    )?;

    // Apply all migration SQL files
    let migrations_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("db")
        .join("migrations");

    let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sql"))
        .collect();

    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let sql = std::fs::read_to_string(entry.path())?;
        conn.execute_batch(&sql)?;
    }

    Ok(conn)
}

#[test]
fn test_deep_nesting_15_levels_resolution_and_performance() -> Result<(), Box<dyn Error>> {
    let conn = setup_migrated_db()?;

    // Create 15-level deep vault structure
    conn.execute(
        "INSERT INTO vaults (id, name, privacy_tier) VALUES ('v_level_0', 'Root', 'open');",
        [],
    )?;

    for depth in 1..=15 {
        let parent = format!("v_level_{}", depth - 1);
        let id = format!("v_level_{depth}");
        let tier = if depth == 8 { "locked" } else { "open" };
        let name = format!("Level {depth}");

        conn.execute(
            "INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES (?1, ?2, ?3, ?4);",
            [&id, &parent, &name, tier],
        )?;
    }

    let start = Instant::now();
    let descendants = fetch_descendant_vault_ids(&conn, "v_level_0")?;
    let duration = start.elapsed();

    assert_eq!(descendants.len(), 16); // 1 root + 15 descendants
    assert!(
        duration.as_millis() < 50,
        "Query latency should be under 50ms, was {:?}",
        duration
    );

    // Verify privacy waterfall resolution across 15 levels
    let eff_0 = resolve_vault_effective_privacy(&conn, "v_level_0")?;
    assert_eq!(eff_0, "open");

    let eff_12 = resolve_vault_effective_privacy(&conn, "v_level_12")?;
    assert_eq!(
        eff_12, "locked",
        "Level 8 locked tier must propagate to descendant level 12"
    );

    Ok(())
}

#[test]
fn test_cycle_detection_at_parent_assignment() -> Result<(), Box<dyn Error>> {
    let conn = setup_migrated_db()?;

    conn.execute(
        "INSERT INTO vaults (id, name) VALUES ('v_a', 'Vault A');",
        [],
    )?;
    conn.execute(
        "INSERT INTO vaults (id, parent_vault_id, name) VALUES ('v_b', 'v_a', 'Vault B');",
        [],
    )?;
    conn.execute(
        "INSERT INTO vaults (id, parent_vault_id, name) VALUES ('v_c', 'v_b', 'Vault C');",
        [],
    )?;

    // Attempt to make A a child of C (creating A -> B -> C -> A cycle)
    let res = check_vault_parent_assignment(&conn, Some("v_a"), "v_c");
    assert!(res.is_err(), "Cycle assignment must be rejected");
    if let Err(err_msg) = res {
        assert!(
            err_msg.contains("Cycle detected"),
            "Error should report cycle: {err_msg}"
        );
    }

    Ok(())
}

#[test]
fn test_depth_cap_at_32_levels() -> Result<(), Box<dyn Error>> {
    let conn = setup_migrated_db()?;

    conn.execute("INSERT INTO vaults (id, name) VALUES ('v_0', 'Root');", [])?;
    for i in 1..=32 {
        let parent = format!("v_{}", i - 1);
        let id = format!("v_{i}");
        let name = format!("Level {i}");
        conn.execute(
            "INSERT INTO vaults (id, parent_vault_id, name) VALUES (?1, ?2, ?3);",
            [&id, &parent, &name],
        )?;
    }

    // Assigning parent to create level 33 should exceed MAX_VAULT_NESTING_DEPTH (32)
    let res = check_vault_parent_assignment(&conn, None, "v_32");
    assert!(res.is_err(), "Nesting beyond 32 levels must be rejected");
    if let Err(err) = res {
        assert!(
            err.contains("Maximum vault nesting depth"),
            "Error message: {err}"
        );
    }

    Ok(())
}

#[test]
fn test_query_time_cycle_defense_in_depth_terminates_safely() -> Result<(), Box<dyn Error>> {
    let conn = setup_migrated_db()?;

    // Manually construct a cycle bypassing triggers (e.g. v_x -> v_y -> v_x)
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = OFF;
        INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES ('v_x', 'v_y', 'X', 'open');
        INSERT INTO vaults (id, parent_vault_id, name, privacy_tier) VALUES ('v_y', 'v_x', 'Y', 'locked');
        PRAGMA foreign_keys = ON;
        "#,
    )?;

    // Query time resolution MUST terminate via visited set / depth cap without hanging or panicking
    let start = Instant::now();
    let privacy = resolve_vault_effective_privacy(&conn, "v_x")?;
    let elapsed = start.elapsed();

    assert_eq!(privacy, "locked");
    assert!(
        elapsed.as_millis() < 50,
        "Cyclic query must terminate immediately via cycle guard"
    );

    Ok(())
}
