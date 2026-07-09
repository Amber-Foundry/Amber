use crate::ingest::job::{ImportJobProgress, IngestJobResult};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct ImportJobRow {
    pub id: String,
    pub import_type: String,
    pub source_name: Option<String>,
    pub target_vault_id: Option<String>,
    pub status: String,
    pub changeset_id: Option<String>,
    pub node_count: i32,
    pub error: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub total_pages: i32,
    pub digital_pages: i32,
    pub ocr_pages: i32,
    pub hybrid_pages: i32,
    pub avg_ocr_confidence: f32,
    pub rasterization_dpi: i32,
    pub tables_detected_unpreserved: i32,
    pub extraction_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateImportJobParams {
    pub import_type: String,
    pub source_name: String,
    pub target_vault_id: Option<String>,
    pub rasterization_dpi: i32,
}

impl Default for CreateImportJobParams {
    fn default() -> Self {
        Self {
            import_type: "pdf".to_string(),
            source_name: String::new(),
            target_vault_id: None,
            rasterization_dpi: 300,
        }
    }
}

const IMPORT_JOB_SELECT: &str = "SELECT
    id, import_type, source_name, target_vault_id, status, changeset_id, node_count,
    error, created_at, completed_at, total_pages, digital_pages, ocr_pages, hybrid_pages,
    avg_ocr_confidence, rasterization_dpi, tables_detected_unpreserved, extraction_path
    FROM import_jobs";

fn map_import_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportJobRow> {
    Ok(ImportJobRow {
        id: row.get(0)?,
        import_type: row.get(1)?,
        source_name: row.get(2)?,
        target_vault_id: row.get(3)?,
        status: row.get(4)?,
        changeset_id: row.get(5)?,
        node_count: row.get(6)?,
        error: row.get(7)?,
        created_at: row.get(8)?,
        completed_at: row.get(9)?,
        total_pages: row.get(10)?,
        digital_pages: row.get(11)?,
        ocr_pages: row.get(12)?,
        hybrid_pages: row.get(13)?,
        avg_ocr_confidence: row.get::<_, f64>(14)? as f32,
        rasterization_dpi: row.get(15)?,
        tables_detected_unpreserved: row.get(16)?,
        extraction_path: row.get(17)?,
    })
}

pub fn create_import_job(
    conn: &Connection,
    id: &str,
    params: &CreateImportJobParams,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO import_jobs (
            id, import_type, source_name, target_vault_id, status,
            total_pages, digital_pages, ocr_pages, hybrid_pages,
            avg_ocr_confidence, rasterization_dpi, tables_detected_unpreserved
        ) VALUES (?1, ?2, ?3, ?4, 'pending', 0, 0, 0, 0, 0.0, ?5, 0);",
        params![
            id,
            params.import_type,
            params.source_name,
            params.target_vault_id,
            params.rasterization_dpi,
        ],
    )
    .map_err(|err| format!("Failed to create import job: {err}"))?;
    Ok(())
}

pub fn set_import_job_status(
    conn: &Connection,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let is_terminal = status == "committed" || status == "failed";
    let error_value = if status == "failed" { error } else { None };

    if is_terminal {
        conn.execute(
            "UPDATE import_jobs
             SET status = ?1,
                 error = ?2,
                 completed_at = datetime('now')
             WHERE id = ?3;",
            params![status, error_value, id],
        )
        .map_err(|err| format!("Failed to set import job status: {err}"))?;
    } else {
        conn.execute(
            "UPDATE import_jobs
             SET status = ?1,
                 error = NULL
             WHERE id = ?2;",
            params![status, id],
        )
        .map_err(|err| format!("Failed to set import job status: {err}"))?;
    }
    Ok(())
}

pub fn update_import_job_from_progress(
    conn: &Connection,
    id: &str,
    progress: &ImportJobProgress,
) -> Result<(), String> {
    conn.execute(
        "UPDATE import_jobs
         SET status = ?1,
             total_pages = ?2,
             digital_pages = ?3,
             ocr_pages = ?4,
             hybrid_pages = ?5,
             avg_ocr_confidence = ?6
         WHERE id = ?7;",
        params![
            progress.status,
            progress.total_pages as i32,
            progress.digital_pages as i32,
            progress.ocr_pages as i32,
            progress.hybrid_pages as i32,
            progress.avg_ocr_confidence,
            id,
        ],
    )
    .map_err(|err| format!("Failed to update import job progress: {err}"))?;
    Ok(())
}

pub fn update_import_job_staged_metadata(
    conn: &Connection,
    id: &str,
    result: &IngestJobResult,
    rasterization_dpi: i32,
    extraction_path: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE import_jobs
         SET status = 'staged',
             source_name = ?1,
             total_pages = ?2,
             digital_pages = ?3,
             ocr_pages = ?4,
             hybrid_pages = ?5,
             avg_ocr_confidence = ?6,
             rasterization_dpi = ?7,
             tables_detected_unpreserved = ?8,
             extraction_path = ?9
         WHERE id = ?10;",
        params![
            result.source_name,
            result.total_pages as i32,
            result.digital_pages as i32,
            result.ocr_pages as i32,
            result.hybrid_pages as i32,
            result.avg_ocr_confidence,
            rasterization_dpi,
            result.tables_detected_unpreserved,
            extraction_path,
            id,
        ],
    )
    .map_err(|err| format!("Failed to update import job staged metadata: {err}"))?;
    Ok(())
}

pub fn get_import_job(conn: &Connection, id: &str) -> Result<Option<ImportJobRow>, String> {
    let mut stmt = conn
        .prepare(&format!("{IMPORT_JOB_SELECT} WHERE id = ?1 LIMIT 1;"))
        .map_err(|err| format!("Failed to prepare import job select: {err}"))?;

    stmt.query_row(params![id], map_import_job_row)
        .optional()
        .map_err(|err| format!("Failed to fetch import job: {err}"))
}

pub fn list_import_jobs(conn: &Connection, limit: i32) -> Result<Vec<ImportJobRow>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "{IMPORT_JOB_SELECT} ORDER BY created_at DESC LIMIT ?1;"
        ))
        .map_err(|err| format!("Failed to prepare import job list: {err}"))?;

    let rows = stmt
        .query_map(params![limit], map_import_job_row)
        .map_err(|err| format!("Failed to list import jobs: {err}"))?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(|err| format!("Failed to read import job row: {err}"))?);
    }
    Ok(results)
}

#[cfg(test)]
fn setup_test_db() -> Result<Connection, Box<dyn std::error::Error>> {
    use std::fs;
    use std::path::PathBuf;

    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("db")
            .join("migrations")
    }

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

    let dir = migrations_dir();
    if !dir.exists() {
        return Err(format!("migrations directory does not exist: {}", dir.display()).into());
    }

    let entries = fs::read_dir(&dir)?;
    let mut migrations = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("failed to get file name for path: {}", path.display()))?;

        if !file_name.ends_with(".sql") {
            continue;
        }

        let (version_text, name_rest) = file_name.split_once('_').ok_or_else(|| {
            format!("migration file must follow '<version>_<name>.sql': {file_name}")
        })?;

        let version = version_text
            .parse::<i64>()
            .map_err(|_| format!("migration version must be numeric: {file_name}"))?;

        let name = name_rest.trim_end_matches(".sql").to_string();
        migrations.push((version, name, path));
    }

    migrations.sort_by_key(|migration| migration.0);

    for (version, name, path) in migrations {
        let sql = fs::read_to_string(&path)?;
        conn.execute_batch(&sql)
            .map_err(|err| format!("migration {version}_{name} failed: {err}"))?;
    }

    conn.execute(
        "INSERT INTO vaults (id, name, icon, description, privacy_tier, priority_profile, sort_order, meta)
         VALUES ('vault_test', 'Test Vault', 'vault', 'Fixture Vault', 'open', 'standard', 0, '{}');",
        [],
    )?;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn require_job(
        conn: &Connection,
        id: &str,
    ) -> Result<ImportJobRow, Box<dyn std::error::Error>> {
        get_import_job(conn, id)?.ok_or_else(|| format!("job {id} should exist").into())
    }

    fn sample_params() -> CreateImportJobParams {
        CreateImportJobParams {
            import_type: "pdf".to_string(),
            source_name: "sample.pdf".to_string(),
            target_vault_id: Some("vault_test".to_string()),
            rasterization_dpi: 300,
        }
    }

    #[test]
    fn test_create_import_job_pending() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-001", &sample_params())?;

        let row = require_job(&conn, "job-001")?;
        assert_eq!(row.status, "pending");
        assert_eq!(row.source_name.as_deref(), Some("sample.pdf"));
        assert_eq!(row.target_vault_id.as_deref(), Some("vault_test"));
        assert_eq!(row.total_pages, 0);
        assert_eq!(row.rasterization_dpi, 300);
        assert!(row.extraction_path.is_none());
        Ok(())
    }

    #[test]
    fn test_update_import_job_from_progress() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-002", &sample_params())?;

        let progress = ImportJobProgress {
            job_id: "job-002".to_string(),
            current_page: 3,
            total_pages: 10,
            digital_pages: 7,
            ocr_pages: 2,
            hybrid_pages: 1,
            avg_ocr_confidence: 0.92,
            status: "extracting".to_string(),
        };
        update_import_job_from_progress(&conn, "job-002", &progress)?;

        let row = require_job(&conn, "job-002")?;
        assert_eq!(row.status, "extracting");
        assert_eq!(row.total_pages, 10);
        assert_eq!(row.digital_pages, 7);
        assert_eq!(row.ocr_pages, 2);
        assert_eq!(row.hybrid_pages, 1);
        assert!((row.avg_ocr_confidence - 0.92).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn test_update_import_job_staged_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-003", &sample_params())?;

        let result = IngestJobResult {
            job_id: "job-003".to_string(),
            source_name: "staged.pdf".to_string(),
            total_pages: 12,
            digital_pages: 9,
            ocr_pages: 2,
            hybrid_pages: 1,
            assembled_markdown: "# Doc".to_string(),
            chunks: vec![],
            avg_ocr_confidence: 0.985,
            tables_detected_unpreserved: 2,
            candidates: vec![],
        };
        update_import_job_staged_metadata(&conn, "job-003", &result, 300, "hybrid")?;

        let row = require_job(&conn, "job-003")?;
        assert_eq!(row.status, "staged");
        assert_eq!(row.source_name.as_deref(), Some("staged.pdf"));
        assert_eq!(row.total_pages, 12);
        assert_eq!(row.digital_pages, 9);
        assert_eq!(row.ocr_pages, 2);
        assert_eq!(row.hybrid_pages, 1);
        assert!((row.avg_ocr_confidence - 0.985).abs() < f32::EPSILON);
        assert_eq!(row.rasterization_dpi, 300);
        assert_eq!(row.tables_detected_unpreserved, 2);
        assert_eq!(row.extraction_path.as_deref(), Some("hybrid"));
        Ok(())
    }

    #[test]
    fn test_set_import_job_status_failed() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-004", &sample_params())?;

        set_import_job_status(&conn, "job-004", "failed", Some("OCR model missing"))?;

        let row = require_job(&conn, "job-004")?;
        assert_eq!(row.status, "failed");
        assert_eq!(row.error.as_deref(), Some("OCR model missing"));
        assert!(row.completed_at.is_some());
        Ok(())
    }

    #[test]
    fn test_set_import_job_status_committed() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-005", &sample_params())?;
        set_import_job_status(&conn, "job-005", "failed", Some("temporary error"))?;

        set_import_job_status(&conn, "job-005", "committed", None)?;

        let row = require_job(&conn, "job-005")?;
        assert_eq!(row.status, "committed");
        assert!(row.error.is_none());
        assert!(row.completed_at.is_some());
        Ok(())
    }

    #[test]
    fn test_list_and_get_import_jobs() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-a", &sample_params())?;
        conn.execute(
            "UPDATE import_jobs SET created_at = datetime('now', '-1 hour') WHERE id = 'job-a';",
            [],
        )?;
        create_import_job(
            &conn,
            "job-b",
            &CreateImportJobParams {
                source_name: "other.pdf".to_string(),
                ..sample_params()
            },
        )?;

        let listed = list_import_jobs(&conn, 10)?;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "job-b");
        assert_eq!(listed[1].id, "job-a");

        let limited = list_import_jobs(&conn, 1)?;
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].id, "job-b");

        let fetched = require_job(&conn, "job-a")?;
        assert_eq!(fetched.id, "job-a");
        assert!(get_import_job(&conn, "missing")?.is_none());
        Ok(())
    }

    #[test]
    fn test_full_import_job_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-life", &sample_params())?;

        let pending = require_job(&conn, "job-life")?;
        assert_eq!(pending.status, "pending");
        assert_eq!(pending.total_pages, 0);

        let progress = ImportJobProgress {
            job_id: "job-life".to_string(),
            current_page: 2,
            total_pages: 5,
            digital_pages: 3,
            ocr_pages: 2,
            hybrid_pages: 0,
            avg_ocr_confidence: 0.88,
            status: "extracting".to_string(),
        };
        update_import_job_from_progress(&conn, "job-life", &progress)?;

        let extracting = require_job(&conn, "job-life")?;
        assert_eq!(extracting.status, "extracting");
        assert_eq!(extracting.total_pages, 5);

        let result = IngestJobResult {
            job_id: "job-life".to_string(),
            source_name: "lifecycle.pdf".to_string(),
            total_pages: 5,
            digital_pages: 3,
            ocr_pages: 2,
            hybrid_pages: 0,
            assembled_markdown: String::new(),
            chunks: vec![],
            avg_ocr_confidence: 0.88,
            tables_detected_unpreserved: 0,
            candidates: vec![],
        };
        let extraction_path = crate::ingest::derive_document_extraction_path(
            result.digital_pages,
            result.ocr_pages,
            result.hybrid_pages,
        );
        update_import_job_staged_metadata(&conn, "job-life", &result, 300, extraction_path)?;

        let staged = require_job(&conn, "job-life")?;
        assert_eq!(staged.status, "staged");
        assert_eq!(staged.extraction_path.as_deref(), Some("hybrid"));

        set_import_job_status(&conn, "job-life", "committed", None)?;

        let committed = require_job(&conn, "job-life")?;
        assert_eq!(committed.status, "committed");
        assert!(committed.completed_at.is_some());
        assert!(committed.error.is_none());
        Ok(())
    }

    #[test]
    fn test_non_failed_status_clears_error() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_test_db()?;
        create_import_job(&conn, "job-006", &sample_params())?;
        set_import_job_status(&conn, "job-006", "failed", Some("boom"))?;
        set_import_job_status(&conn, "job-006", "extracting", None)?;

        let row = require_job(&conn, "job-006")?;
        assert_eq!(row.status, "extracting");
        assert!(row.error.is_none());
        Ok(())
    }
}
