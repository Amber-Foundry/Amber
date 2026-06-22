DROP TABLE IF EXISTS node_embeddings;

CREATE TABLE node_embeddings (
    node_id     TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    chunk_type  TEXT NOT NULL DEFAULT 'primary'
                CHECK (chunk_type IN ('primary', 'detail', 'import')),
    model       TEXT NOT NULL,
    embedding   BLOB NOT NULL,
    computed_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (node_id, chunk_index, chunk_type)
);

CREATE INDEX idx_node_embeddings_model ON node_embeddings(model);

-- Recreate invalidation trigger (extended to title changes).
DROP TRIGGER IF EXISTS trg_invalidate_embedding_on_update;

CREATE TRIGGER trg_invalidate_embedding_on_update
AFTER UPDATE ON nodes
WHEN NEW.title IS NOT OLD.title
   OR NEW.summary IS NOT OLD.summary
   OR NEW.detail IS NOT OLD.detail
BEGIN
    DELETE FROM node_embeddings WHERE node_id = NEW.id;
END;
