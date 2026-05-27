import { useEffect, useMemo, useRef, useState, useDeferredValue } from "react";
import { createPortal } from "react-dom";
import type { Node, Vault } from "../ipc";
import { createNode, getNodes } from "../services/nodes";
import { AppError } from "../services/ipcResult";
import { createVault, listVaults } from "../services/vaults";
import {
  getEffectivePrivacy,
  getPrivacyDisplayLabel,
  getPrivacyDisplaySummary,
  getVaultDisplayLabel,
  getVaultEffectivePrivacy,
} from "../utils/privacy";
import { PrivacyBadge } from "./PrivacyBadge";

const VAULT_ICON_CHOICES = [
  "💳",
  "🪙",
  "💪",
  "📚",
  "👤",
  "💼",
  "🏠",
  "📱",
  "💻",
  "📝",
  "🧠",
  "💰",
  "🔑",
  "🎨",
  "🚀",
  "📂",
];

type NodeListProps = {
  selectedVaultId: string | null;
  selectedNodeId: string | null;
  refreshKey: number;
  onSelectNode: (nodeId: string) => void;
  onSelectVault?: (vaultId: string) => void;
  onNodeCreated: (nodeId: string) => void;
  onVaultCreated: (vaultId: string) => void;
  onBack: () => void;
  isRedactedUnlocked: boolean;
  onModalToggle?: (isOpen: boolean) => void;
};

function NodeList({
  selectedVaultId,
  selectedNodeId,
  refreshKey,
  onSelectNode,
  onSelectVault,
  onNodeCreated,
  onVaultCreated,
  onBack,
  isRedactedUnlocked,
  onModalToggle,
}: NodeListProps) {
  const [nodes, setNodes] = useState<Node[]>([]);
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const deferredSearchQuery = useDeferredValue(searchQuery);
  const resolvedQuery = searchQuery === "" ? "" : deferredSearchQuery;
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [createModalName, setCreateModalName] = useState("");
  const [createModalDescription, setCreateModalDescription] = useState("");
  const [createModalIcon, setCreateModalIcon] = useState("");
  const [createModalPrivacyTier, setCreateModalPrivacyTier] = useState("open");
  const [createModalError, setCreateModalError] = useState("");

  async function loadNodes() {
    try {
      const data = await getNodes();
      setNodes(data);
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
        return;
      }
      setError("Failed to load nodes.");
    }
  }

  async function loadVaults() {
    try {
      const data = await listVaults();
      setVaults(data);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
      } else {
        setError("Failed to load vault context.");
      }
    }
  }

  useEffect(() => {
    const timer = setTimeout(() => {
      void loadNodes();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked]);

  useEffect(() => {
    const timer = setTimeout(() => {
      void (async () => {
        await loadVaults();
      })();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      searchInputRef.current?.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [selectedVaultId]);

  useEffect(() => {
    onModalToggle?.(createModalOpen);
  }, [createModalOpen, onModalToggle]);

  const selectedVault = useMemo(() => {
    if (!selectedVaultId) {
      return null;
    }
    return vaults.find((vault) => vault.id === selectedVaultId) ?? null;
  }, [selectedVaultId, vaults]);

  const vaultById = useMemo(() => {
    const map: Record<string, Vault> = {};
    for (const vault of vaults) {
      map[vault.id] = vault;
    }
    return map;
  }, [vaults]);

  const vaultEffectivePrivacyById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const vault of vaults) {
      map[vault.id] = getVaultEffectivePrivacy(vault.id, vaultById, map);
    }
    return map;
  }, [vaultById, vaults]);

  const childVaultsByParent = useMemo(() => {
    const map = new Map<string, Vault[]>();
    for (const vault of vaults) {
      const parentId = vault.parentVaultId ?? "";
      const list = map.get(parentId) ?? [];
      list.push(vault);
      map.set(parentId, list);
    }
    for (const list of map.values()) {
      list.sort((a, b) => a.name.localeCompare(b.name));
    }
    return map;
  }, [vaults]);

  const nodesByContainer = useMemo(() => {
    const map = new Map<string, Node[]>();
    for (const node of nodes) {
      const containerId = node.subVaultId ?? node.vaultId;
      const list = map.get(containerId) ?? [];
      list.push(node);
      map.set(containerId, list);
    }
    for (const list of map.values()) {
      list.sort((a, b) => a.title.localeCompare(b.title));
    }
    return map;
  }, [nodes]);

  const selectedVaultChildren = useMemo(() => {
    if (!selectedVault) {
      return [];
    }
    return childVaultsByParent.get(selectedVault.id) ?? [];
  }, [childVaultsByParent, selectedVault]);

  const selectedVaultNodes = useMemo(() => {
    if (!selectedVault) {
      return [];
    }
    return nodesByContainer.get(selectedVault.id) ?? [];
  }, [nodesByContainer, selectedVault]);

  const backButtonLabel = selectedVault?.parentVaultId
    ? `← Back to ${getVaultDisplayLabel(selectedVault.parentVaultId, vaultById, isRedactedUnlocked)}`
    : "← Back to Vaults";

  const normalizedQuery = resolvedQuery.trim().toLowerCase();

  const filteredNodes = useMemo(() => {
    const scoped = selectedVaultNodes;
    if (!normalizedQuery) {
      return scoped;
    }
    return scoped.filter((node) => {
      const subVault = node.subVaultId ? vaultById[node.subVaultId] : undefined;
      const containerId = subVault?.id ?? node.vaultId;
      const containerTier =
        vaultEffectivePrivacyById[containerId] ?? getVaultEffectivePrivacy(containerId, vaultById);
      const effectiveTier = getEffectivePrivacy(node.privacyTier, null, containerTier);

      const isNodeRedactedLocked = effectiveTier === "redacted" && !isRedactedUnlocked;
      const title = node.title.toLowerCase();
      const titleMatch = title.includes(normalizedQuery);
      if (isNodeRedactedLocked) {
        return titleMatch;
      }

      const summary = node.summary.toLowerCase();
      return titleMatch || summary.includes(normalizedQuery);
    });
  }, [
    normalizedQuery,
    selectedVaultNodes,
    vaultById,
    vaultEffectivePrivacyById,
    isRedactedUnlocked,
  ]);

  function getNodeEffectivePrivacy(node: Node): string {
    const containerId = node.subVaultId ?? node.vaultId;
    const containerTier =
      vaultEffectivePrivacyById[containerId] ?? getVaultEffectivePrivacy(containerId, vaultById);
    return getEffectivePrivacy(node.privacyTier, null, containerTier);
  }

  async function onCreateNode() {
    if (!selectedVault) {
      return;
    }
    try {
      const input = selectedVault.parentVaultId
        ? {
            vaultId: selectedVault.parentVaultId,
            subVaultId: selectedVault.id,
            title: "Untitled Node",
            summary: "",
            nodeType: "fact",
          }
        : {
            vaultId: selectedVault.id,
            title: "Untitled Node",
            summary: "",
            nodeType: "fact",
          };
      const created = await createNode({
        ...input,
      });
      onNodeCreated(created.id);
      await loadNodes();
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
        return;
      }
      setError("Failed to create node.");
    }
  }

  async function onCreateSubVault() {
    if (!selectedVault) {
      return;
    }
    setCreateModalName("");
    setCreateModalDescription("");
    setCreateModalIcon("");
    setCreateModalPrivacyTier("open");
    setCreateModalError("");
    setCreateModalOpen(true);
  }

  async function submitCreateSubVault() {
    if (!selectedVault) {
      return;
    }
    const name = createModalName.trim();
    if (!name) {
      setCreateModalError("Enter a subvault name.");
      return;
    }

    try {
      const created = await createVault({
        name,
        description: createModalDescription.trim() || undefined,
        icon: createModalIcon.trim() || undefined,
        privacyTier: createModalPrivacyTier.trim() || undefined,
        parentVaultId: selectedVault.id,
      });
      onVaultCreated(created.id);
      onSelectVault?.(created.id);
      await Promise.all([loadNodes(), loadVaults()]);
      setCreateModalOpen(false);
      setCreateModalName("");
      setCreateModalError("");
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setCreateModalError(err.message);
        return;
      }
      setCreateModalError("Failed to create subvault.");
    }
  }

  function closeCreateSubVaultModal() {
    setCreateModalOpen(false);
    setCreateModalName("");
    setCreateModalDescription("");
    setCreateModalIcon("");
    setCreateModalPrivacyTier("open");
    setCreateModalError("");
  }

  function renderVaultSection(vault: Vault, depth = 0) {
    const childVaults = childVaultsByParent.get(vault.id) ?? [];
    const vaultNodes = nodesByContainer.get(vault.id) ?? [];
    const indentClass = depth > 0 ? " nested" : "";
    const effectiveTier =
      vaultEffectivePrivacyById[vault.id] ?? getVaultEffectivePrivacy(vault.id, vaultById);
    const isRedactedLocked = effectiveTier === "redacted" && !isRedactedUnlocked;
    const isLocked = effectiveTier === "locked" && !isRedactedUnlocked;

    return (
      <div
        key={vault.id}
        className={`vault-section${indentClass}`}
        style={{ marginLeft: depth * 14 }}
      >
        <button
          type="button"
          className={`vault-card ${selectedVaultId === vault.id ? "active" : ""}`}
          onClick={() => onSelectVault?.(vault.id)}
        >
          <span className="vault-card-title">
            <strong>{getPrivacyDisplayLabel(vault.name, effectiveTier, isRedactedUnlocked)}</strong>
            {isRedactedLocked ? (
              <small>[Metadata Locked]</small>
            ) : (
              vault.description && <small>{vault.description}</small>
            )}
          </span>
          <span className="vault-card-meta">
            <PrivacyBadge tier={effectiveTier} />
            {isLocked && <span className="privacy-lock-icon">🔒</span>}
            <span className="vault-card-chevron">›</span>
          </span>
        </button>

        {childVaults.length > 0 && (
          <div className="vault-section-group">
            {childVaults.map((child) => renderVaultSection(child, depth + 1))}
          </div>
        )}

        {vaultNodes.length > 0 && (
          <div className="node-cards nested-node-cards">
            {vaultNodes.map((node) => {
              const effectiveTier = getNodeEffectivePrivacy(node);
              const isNodeRedactedLocked = effectiveTier === "redacted" && !isRedactedUnlocked;
              const summaryText = isNodeRedactedLocked
                ? "[Metadata Locked]"
                : node.summary.slice(0, 120);

              return (
                <button
                  type="button"
                  key={node.id}
                  className={`node-card ${selectedNodeId === node.id ? "active" : ""}`}
                  onClick={() => onSelectNode(node.id)}
                >
                  <span className="node-card-title-row">
                    <strong>
                      {getPrivacyDisplayLabel(node.title, effectiveTier, isRedactedUnlocked)}
                    </strong>
                    <PrivacyBadge tier={effectiveTier} />
                  </span>
                  <p>{getPrivacyDisplaySummary(summaryText, effectiveTier, isRedactedUnlocked)}</p>
                </button>
              );
            })}
          </div>
        )}
      </div>
    );
  }

  return (
    <aside className="pane pane-middle">
      <button
        type="button"
        className="back-button"
        onClick={() => {
          if (selectedVault?.parentVaultId && onSelectVault) {
            onSelectVault(selectedVault.parentVaultId);
            return;
          }
          onBack();
        }}
      >
        {backButtonLabel}
      </button>
      <input
        ref={searchInputRef}
        type="search"
        placeholder="Search nodes..."
        className="search-input"
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
      />
      <div className="pane-header">
        <h3>Vault Contents</h3>
        <div className="pane-actions">
          <button type="button" onClick={onCreateSubVault} disabled={!selectedVault}>
            New Subvault
          </button>
          <button type="button" onClick={onCreateNode} disabled={!selectedVault}>
            New Node
          </button>
        </div>
      </div>
      {error && <p className="pane-error">{error}</p>}
      {selectedVault && (
        <div className="vault-contents">
          {selectedVaultChildren.length > 0 && (
            <div className="vault-section-group">
              {selectedVaultChildren.map((child) => renderVaultSection(child, 0))}
            </div>
          )}
          {filteredNodes.length > 0 && (
            <div className="node-cards">
              {filteredNodes.map((node) => {
                const effectiveTier = getNodeEffectivePrivacy(node);
                const isNodeRedactedLocked = effectiveTier === "redacted" && !isRedactedUnlocked;
                const summaryText = isNodeRedactedLocked
                  ? "[Metadata Locked]"
                  : node.summary.slice(0, 120);

                return (
                  <button
                    type="button"
                    key={node.id}
                    className={`node-card ${selectedNodeId === node.id ? "active" : ""}`}
                    onClick={() => onSelectNode(node.id)}
                  >
                    <span className="node-card-title-row">
                      <strong>
                        {getPrivacyDisplayLabel(node.title, effectiveTier, isRedactedUnlocked)}
                      </strong>
                      <PrivacyBadge tier={effectiveTier} />
                    </span>
                    <p>
                      {getPrivacyDisplaySummary(summaryText, effectiveTier, isRedactedUnlocked)}
                    </p>
                  </button>
                );
              })}
            </div>
          )}
          {selectedVaultChildren.length === 0 && filteredNodes.length === 0 && (
            <p className="pane-empty">
              No subvaults or nodes found in{" "}
              {getVaultDisplayLabel(selectedVault.id, vaultById, isRedactedUnlocked)}.
            </p>
          )}
        </div>
      )}
      {!selectedVault && <p className="pane-empty">Select a vault to view its contents.</p>}
      {selectedVault && filteredNodes.length === 0 && normalizedQuery && (
        <p className="pane-empty">No nodes found matching '{resolvedQuery}'.</p>
      )}
      {createModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={closeCreateSubVaultModal}>
            <div
              className="vault-settings-modal sidebar-auth-modal delete-confirm-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="modal-title">New Subvault</h3>
              <p className="modal-subtitle">
                {selectedVault
                  ? `Create a new subvault inside ${getVaultDisplayLabel(
                      selectedVault.id,
                      vaultById,
                      isRedactedUnlocked
                    )}.`
                  : "Create a new subvault."}
              </p>
              <div className="settings-fields-grid">
                <label className="settings-field">
                  <span>Subvault Name</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={createModalName}
                    onChange={(e) => setCreateModalName(e.target.value)}
                    placeholder="e.g. Research"
                    autoFocus
                  />
                </label>

                <label className="settings-field">
                  <span>Description</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={createModalDescription}
                    onChange={(e) => setCreateModalDescription(e.target.value)}
                    placeholder="e.g. Meeting notes and sources"
                  />
                </label>

                <div className="settings-field">
                  <span>Emoji / Icon</span>
                  <div className="emoji-picker-container">
                    <div className="emoji-picker-grid">
                      {VAULT_ICON_CHOICES.map((emoji) => (
                        <button
                          key={emoji}
                          type="button"
                          className={`emoji-choice-btn ${createModalIcon === emoji ? "selected" : ""}`}
                          onClick={() => setCreateModalIcon(emoji)}
                        >
                          {emoji}
                        </button>
                      ))}
                    </div>
                    <input
                      type="text"
                      value={createModalIcon}
                      onChange={(e) => setCreateModalIcon(e.target.value)}
                      placeholder="Or type a custom emoji/text"
                      maxLength={10}
                      className="settings-input custom-emoji-input"
                    />
                  </div>
                </div>

                <label className="settings-field">
                  <span>Privacy Tier</span>
                  <select
                    value={createModalPrivacyTier}
                    onChange={(e) => setCreateModalPrivacyTier(e.target.value)}
                    className="settings-select"
                  >
                    <option value="open">Open (No restriction)</option>
                    <option value="local_only">Local Only (Never cloud synced)</option>
                    <option value="locked">Locked (Requires unlock to access)</option>
                    <option value="redacted">Redacted (Hidden metadata/title)</option>
                  </select>
                </label>
              </div>
              {createModalError && <p className="redacted-lock-error">{createModalError}</p>}
              <div className="sidebar-auth-actions settings-modal-actions">
                <button
                  type="button"
                  className="redacted-lock-button sidebar-auth-cancel"
                  onClick={closeCreateSubVaultModal}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button"
                  onClick={submitCreateSubVault}
                >
                  Create
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
    </aside>
  );
}

export default NodeList;
