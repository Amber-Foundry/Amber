-- Alter import_jobs table to store granular metrics for ingestion jobs.
ALTER TABLE import_jobs ADD COLUMN total_pages INTEGER DEFAULT 0;
ALTER TABLE import_jobs ADD COLUMN digital_pages INTEGER DEFAULT 0;
ALTER TABLE import_jobs ADD COLUMN ocr_pages INTEGER DEFAULT 0;
ALTER TABLE import_jobs ADD COLUMN hybrid_pages INTEGER DEFAULT 0;
ALTER TABLE import_jobs ADD COLUMN avg_ocr_confidence REAL DEFAULT 0.0;
ALTER TABLE import_jobs ADD COLUMN rasterization_dpi INTEGER DEFAULT 300;
ALTER TABLE import_jobs ADD COLUMN tables_detected_unpreserved INTEGER DEFAULT 0;
ALTER TABLE import_jobs ADD COLUMN extraction_path TEXT CHECK (extraction_path IS NULL OR extraction_path IN ('digital', 'ocr-bundled', 'hybrid'));
