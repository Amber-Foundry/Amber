-- Track per-job page progress for honest Import Job Log progress bars.
ALTER TABLE import_jobs ADD COLUMN current_page INTEGER DEFAULT 0;
