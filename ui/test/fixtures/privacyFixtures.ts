import type { Door, Node, Vault } from "../../types/generated";

const DEFAULT_PRIORITY =
  '{"score":0.8,"profile":"standard","pinned":false,"access_count_30active":0,"access_count_90active":0,"access_history":[],"session_touches":0,"auto_trim_threshold":0.25}';

const TIMESTAMP = "2026-06-23T00:00:00Z";

function baseVault(overrides: Partial<Vault> & Pick<Vault, "id" | "name" | "privacyTier">): Vault {
  return {
    icon: null,
    description: null,
    priorityProfile: "standard",
    summaryNodeId: null,
    sortOrder: 0,
    createdAt: TIMESTAMP,
    updatedAt: TIMESTAMP,
    deletedAt: null,
    meta: "{}",
    uiMetadata: '{"position":{"x":100,"y":100}}',
    ...overrides,
  };
}

function baseNode(
  overrides: Partial<Node> & Pick<Node, "id" | "vaultId" | "title" | "privacyTier">
): Node {
  return {
    subVaultId: null,
    nodeType: "concept",
    summary: "Summary",
    detail: "Detail",
    source: null,
    sourceType: null,
    priority: DEFAULT_PRIORITY,
    version: 1,
    isArchived: false,
    createdAt: TIMESTAMP,
    updatedAt: TIMESTAMP,
    lastAccessed: TIMESTAMP,
    deletedAt: null,
    meta: "{}",
    ...overrides,
  };
}

function baseDoor(overrides: Partial<Door> & Pick<Door, "id" | "sourceNodeId">): Door {
  return {
    targetNodeId: null,
    targetVaultId: null,
    label: "link",
    status: "active",
    orphanReason: null,
    orphanSince: null,
    createdAt: TIMESTAMP,
    updatedAt: TIMESTAMP,
    ...overrides,
  };
}

export const vaultOpen = baseVault({
  id: "vault_open",
  name: "Open Vault",
  privacyTier: "open",
});

export const vaultLocalOnly = baseVault({
  id: "vault_local",
  name: "Local Vault",
  privacyTier: "local_only",
});

export const vaultRedacted = baseVault({
  id: "vault_redacted",
  name: "Secret Vault",
  privacyTier: "redacted",
});

export const vaultLocked = baseVault({
  id: "vault_locked",
  name: "Locked Vault",
  privacyTier: "locked",
});

export const nodeOpen = baseNode({
  id: "node_open",
  vaultId: vaultOpen.id,
  title: "Open Node",
  summary: "Open summary",
  privacyTier: "open",
});

export const nodeRedacted = baseNode({
  id: "node_redacted",
  vaultId: vaultOpen.id,
  title: "Secret Node",
  summary: "Secret summary",
  privacyTier: "redacted",
});

export const nodeLocked = baseNode({
  id: "node_locked",
  vaultId: vaultOpen.id,
  title: "Locked Node",
  summary: "Locked summary",
  privacyTier: "locked",
});

export const nodeInLocalVault = baseNode({
  id: "node_local_child",
  vaultId: vaultLocalOnly.id,
  title: "Local Child Node",
  summary: "Local summary",
  privacyTier: "open",
});

export const doorOpenToRedacted = baseDoor({
  id: "door_open_redacted",
  sourceNodeId: nodeOpen.id,
  targetNodeId: nodeRedacted.id,
});

export const doorRedactedToOpen = baseDoor({
  id: "door_redacted_open",
  sourceNodeId: nodeRedacted.id,
  targetNodeId: nodeOpen.id,
});
