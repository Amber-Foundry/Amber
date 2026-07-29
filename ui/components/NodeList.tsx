import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useDeferredValue,
  useCallback,
  Fragment,
} from "react";
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
import { isImportChunkNode } from "../utils/importDocument";
import { PrivacyBadge } from "./PrivacyBadge";
import { VaultIcon } from "./VaultIcon";
import { VAULT_ICON_KEYS } from "./VaultIconUtils";
import ImportProvenanceBadges from "./ImportProvenanceBadges";

const VAULT_ICON_CHOICES = VAULT_ICON_KEYS;

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

  const loadNodes = useCallback(async () => {
    try {
      const data = await getNodes(isRedactedUnlocked);
      setNodes(data);
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
        return;
      }
      setError("Failed to load nodes.");
    }
  }, [isRedactedUnlocked]);

  const loadVaults = useCallback(async () => {
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
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => {
      void loadNodes();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked, loadNodes]);

  useEffect(() => {
    const timer = setTimeout(() => {
      void (async () => {
        await loadVaults();
      })();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked, loadVaults]);

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
      if (isImportChunkNode(node)) {
        continue;
      }
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

  const ancestorVaults = useMemo(() => {
    if (!selectedVault) {
      return [];
    }
    const list: Vault[] = [];
    const visited = new Set<string>();
    let currId: string | null | undefined = selectedVault.id;
    while (currId) {
      if (visited.has(currId)) break;
      visited.add(currId);
      const v: Vault | undefined = vaultById[currId];
      if (v) {
        list.push(v);
        currId = v.parentVaultId;
      } else {
        break;
      }
    }
    return list.reverse();
  }, [selectedVault, vaultById]);

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

  const breadcrumbsRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (breadcrumbsRef.current) {
      breadcrumbsRef.current.scrollLeft = breadcrumbsRef.current.scrollWidth;
    }
  }, [selectedVaultId]);

  return (
    <aside className="pane pane-middle">
      {ancestorVaults.length <= 1 && (
        <button type="button" className="back-button" onClick={onBack}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M19 12H5" />
            <path d="m12 19-7-7 7-7" />
          </svg>
          Back to Vaults
        </button>
      )}
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
          {ancestorVaults.length > 1 && (
            <nav ref={breadcrumbsRef} className="breadcrumbs-nav" aria-label="Breadcrumbs">
              <button type="button" className="breadcrumb-item" onClick={onBack}>
                Vaults
              </button>
              {ancestorVaults.map((v, index) => {
                const isLast = index === ancestorVaults.length - 1;
                const effectiveTier =
                  vaultEffectivePrivacyById[v.id] ?? getVaultEffectivePrivacy(v.id, vaultById);
                const label = getPrivacyDisplayLabel(v.name, effectiveTier, isRedactedUnlocked);
                return (
                  <Fragment key={v.id}>
                    <span className="breadcrumb-separator">
                      <svg
                        width="12"
                        height="12"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2.5"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        aria-hidden="true"
                      >
                        <path d="m9 18 6-6-6-6" />
                      </svg>
                    </span>
                    {isLast ? (
                      <span className="breadcrumb-item current">{label}</span>
                    ) : (
                      <button
                        type="button"
                        className="breadcrumb-item"
                        onClick={() => onSelectVault?.(v.id)}
                      >
                        {label}
                      </button>
                    )}
                  </Fragment>
                );
              })}
            </nav>
          )}

          {selectedVaultChildren.length > 0 && (
            <div className="vault-section-group">
              {selectedVaultChildren.map((child) => {
                const effectiveTier =
                  vaultEffectivePrivacyById[child.id] ??
                  getVaultEffectivePrivacy(child.id, vaultById);
                const isRedactedLocked = effectiveTier === "redacted" && !isRedactedUnlocked;
                const childSubvaultCount = (childVaultsByParent.get(child.id) ?? []).length;
                const childNodeCount = (nodesByContainer.get(child.id) ?? []).length;

                return (
                  <button
                    type="button"
                    key={child.id}
                    className="vault-card subvault-direct-card"
                    onClick={() => onSelectVault?.(child.id)}
                  >
                    <span className="vault-card-title">
                      <span className="folder-icon">
                        <svg
                          width="16"
                          height="16"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        >
                          <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
                        </svg>
                      </span>
                      <strong>
                        {getPrivacyDisplayLabel(child.name, effectiveTier, isRedactedUnlocked)}
                      </strong>
                      {isRedactedLocked ? (
                        <small>[Metadata Locked]</small>
                      ) : (
                        child.description && <small>{child.description}</small>
                      )}
                    </span>
                    <span className="vault-card-meta">
                      <span className="count-badge">
                        {childSubvaultCount > 0 &&
                          `${childSubvaultCount} subvault${childSubvaultCount > 1 ? "s" : ""}`}
                        {childSubvaultCount > 0 && childNodeCount > 0 && " • "}
                        {childNodeCount > 0 &&
                          `${childNodeCount} note${childNodeCount > 1 ? "s" : ""}`}
                        {childSubvaultCount === 0 && childNodeCount === 0 && "Empty"}
                      </span>
                      <PrivacyBadge tier={effectiveTier} />
                      <span className="vault-card-chevron">
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        >
                          <path d="m9 18 6-6-6-6" />
                        </svg>
                      </span>
                    </span>
                  </button>
                );
              })}
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
                    {!isNodeRedactedLocked && (
                      <ImportProvenanceBadges node={node} allNodes={nodes} />
                    )}
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
                  <span>Icon</span>
                  <div className="emoji-picker-container">
                    <div className="emoji-picker-grid">
                      {VAULT_ICON_CHOICES.map((key) => (
                        <button
                          key={key}
                          type="button"
                          className={`emoji-choice-btn ${createModalIcon === key ? "selected" : ""}`}
                          onClick={() => setCreateModalIcon(key)}
                          aria-label={`Select ${key} icon`}
                        >
                          <VaultIcon icon={key} name="" size={18} />
                        </button>
                      ))}
                    </div>
                    <input
                      type="text"
                      value={createModalIcon}
                      onChange={(e) => setCreateModalIcon(e.target.value)}
                      placeholder="Or type a custom icon key"
                      maxLength={24}
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
