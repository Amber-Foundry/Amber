-- Persist literal extraction text for View Extraction on staged import jobs.
ALTER TABLE import_jobs ADD COLUMN assembled_markdown TEXT;
