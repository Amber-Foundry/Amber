-- Recreate invalidation triggers to include privacy_tier and vault columns, preventing stale embeddings.
DROP TRIGGER IF EXISTS trg_invalidate_embedding_on_update;

CREATE TRIGGER trg_invalidate_embedding_on_update
AFTER UPDATE ON nodes
WHEN NEW.title IS NOT OLD.title
   OR NEW.summary IS NOT OLD.summary
   OR NEW.detail IS NOT OLD.detail
   OR NEW.privacy_tier IS NOT OLD.privacy_tier
   OR NEW.vault_id IS NOT OLD.vault_id
   OR NEW.sub_vault_id IS NOT OLD.sub_vault_id
   OR NEW.deleted_at IS NOT OLD.deleted_at
BEGIN
    DELETE FROM node_embeddings WHERE node_id = NEW.id;
END;

-- Invalidate embeddings of nodes in a root vault when the root vault's privacy tier changes.
DROP TRIGGER IF EXISTS trg_invalidate_embedding_on_vault_update;

CREATE TRIGGER trg_invalidate_embedding_on_vault_update
AFTER UPDATE OF privacy_tier ON vaults
WHEN NEW.privacy_tier IS NOT OLD.privacy_tier
BEGIN
    DELETE FROM node_embeddings
    WHERE node_id IN (
        SELECT id FROM nodes WHERE vault_id = NEW.id
    );
END;

-- Invalidate embeddings of nodes in a sub-vault when the sub-vault's privacy tier changes.
DROP TRIGGER IF EXISTS trg_invalidate_embedding_on_sub_vault_update;

CREATE TRIGGER trg_invalidate_embedding_on_sub_vault_update
AFTER UPDATE OF privacy_tier ON sub_vaults
WHEN NEW.privacy_tier IS NOT OLD.privacy_tier
BEGIN
    DELETE FROM node_embeddings
    WHERE node_id IN (
        SELECT id FROM nodes WHERE sub_vault_id = NEW.id
    );
END;
