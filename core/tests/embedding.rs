use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_db_path(label: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(std::env::temp_dir().join(format!("amber_{label}_{nanos}.sqlite")))
}

#[test]
fn embedding_get_status_returns_seeded_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("embedding_status")?;
    mindvault_lib::test_helper_init_embedding_db(db_path.clone())?;

    let status = mindvault_lib::test_helper_embedding_get_status(db_path.clone())?;

    assert_eq!(status.model, "avsolatorio/GIST-small-Embedding-v0");
    assert_eq!(status.tier, "light");
    assert_eq!(status.backend, "onnx");
    assert_eq!(status.coverage_percent, 0.0);
    assert!(!status.reembed_in_progress);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}

#[test]
fn embedding_reembed_cancel_sets_active_cancel_token() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = unique_db_path("embedding_cancel")?;

    let cancelled = mindvault_lib::test_helper_embedding_reembed_cancel(db_path.clone())?;

    assert!(cancelled);

    let _remove_result = fs::remove_file(db_path);
    Ok(())
}
