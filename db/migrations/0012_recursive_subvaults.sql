-- Migration: 0012_recursive_subvaults.sql
-- Unify vaults and sub_vaults into a single self-referencing vaults table with parent_vault_id.

-- 1. Add parent_vault_id column to vaults table
ALTER TABLE vaults ADD COLUMN parent_vault_id TEXT REFERENCES vaults(id) ON DELETE CASCADE;

-- 2. Create index on parent_vault_id for fast recursive tree queries
CREATE INDEX IF NOT EXISTS idx_vaults_parent ON vaults(parent_vault_id);

-- 3. Migrate all existing sub_vaults rows into vaults table
INSERT OR IGNORE INTO vaults (
    id,
    parent_vault_id,
    name,
    icon,
    description,
    privacy_tier,
    priority_profile,
    summary_node_id,
    sort_order,
    created_at,
    updated_at,
    deleted_at,
    meta,
    ui_metadata,
    encrypted_payload
)
SELECT 
    id,
    vault_id AS parent_vault_id,
    name,
    icon,
    description,
    CASE WHEN privacy_tier IS NULL OR privacy_tier = '' THEN 'open' ELSE privacy_tier END AS privacy_tier,
    CASE WHEN priority_profile IS NULL OR priority_profile = '' THEN 'standard' ELSE priority_profile END AS priority_profile,
    summary_node_id,
    COALESCE(sort_order, 0) AS sort_order,
    COALESCE(created_at, datetime('now')) AS created_at,
    COALESCE(updated_at, datetime('now')) AS updated_at,
    deleted_at,
    COALESCE(meta, '{}') AS meta,
    COALESCE(ui_metadata, '{}') AS ui_metadata,
    encrypted_payload
FROM sub_vaults;

-- 4. Update nodes table so vault_id references the immediate container vault (if node was in a sub_vault)
UPDATE nodes
SET vault_id = sub_vault_id
WHERE sub_vault_id IS NOT NULL 
  AND sub_vault_id != ''
  AND EXISTS (SELECT 1 FROM vaults WHERE id = nodes.sub_vault_id);

-- 5. Sync triggers from sub_vaults to vaults for backward compatibility
CREATE TRIGGER IF NOT EXISTS trg_sync_sub_vaults_to_vaults_insert
AFTER INSERT ON sub_vaults
BEGIN
    INSERT OR REPLACE INTO vaults (
        id, parent_vault_id, name, icon, description, privacy_tier, priority_profile,
        summary_node_id, sort_order, created_at, updated_at, deleted_at, meta, ui_metadata, encrypted_payload
    ) VALUES (
        NEW.id, NEW.vault_id, NEW.name, NEW.icon, NEW.description,
        CASE WHEN NEW.privacy_tier IS NULL OR NEW.privacy_tier = '' THEN 'open' ELSE NEW.privacy_tier END,
        CASE WHEN NEW.priority_profile IS NULL OR NEW.priority_profile = '' THEN 'standard' ELSE NEW.priority_profile END,
        NEW.summary_node_id, COALESCE(NEW.sort_order, 0),
        CASE WHEN NEW.created_at IS NULL THEN datetime('now') ELSE NEW.created_at END,
        CASE WHEN NEW.updated_at IS NULL THEN datetime('now') ELSE NEW.updated_at END,
        NEW.deleted_at, COALESCE(NEW.meta, '{}'), COALESCE(NEW.ui_metadata, '{}'), NEW.encrypted_payload
    );
END;

CREATE TRIGGER IF NOT EXISTS trg_sync_sub_vaults_to_vaults_update
AFTER UPDATE ON sub_vaults
BEGIN
    UPDATE vaults SET
        parent_vault_id = NEW.vault_id,
        name = NEW.name,
        icon = NEW.icon,
        description = NEW.description,
        privacy_tier = CASE WHEN NEW.privacy_tier IS NULL OR NEW.privacy_tier = '' THEN 'open' ELSE NEW.privacy_tier END,
        priority_profile = CASE WHEN NEW.priority_profile IS NULL OR NEW.priority_profile = '' THEN 'standard' ELSE NEW.priority_profile END,
        summary_node_id = NEW.summary_node_id,
        sort_order = COALESCE(NEW.sort_order, 0),
        updated_at = CASE WHEN NEW.updated_at IS NULL THEN datetime('now') ELSE NEW.updated_at END,
        deleted_at = NEW.deleted_at,
        meta = COALESCE(NEW.meta, '{}'),
        ui_metadata = COALESCE(NEW.ui_metadata, '{}'),
        encrypted_payload = NEW.encrypted_payload
    WHERE id = OLD.id;
END;

-- 6. Update invalidation trigger on vaults to recursively invalidate node embeddings across descendant trees
DROP TRIGGER IF EXISTS trg_invalidate_embedding_on_vault_update;
CREATE TRIGGER trg_invalidate_embedding_on_vault_update
AFTER UPDATE OF privacy_tier ON vaults
WHEN NEW.privacy_tier IS NOT OLD.privacy_tier
BEGIN
    DELETE FROM node_embeddings
    WHERE node_id IN (
        WITH RECURSIVE vault_tree(id) AS (
            SELECT NEW.id
            UNION ALL
            SELECT v.id FROM vaults v JOIN vault_tree vt ON v.parent_vault_id = vt.id
        )
        SELECT id FROM nodes WHERE vault_id IN (SELECT id FROM vault_tree)
    );
END;
