import { useEffect, useMemo, useRef, useState, useDeferredValue } from "react";
import { createPortal } from "react-dom";
import type { Node, Vault } from "../ipc";
import { getAllNodes } from "../services/nodes";
import { createVault, deleteVault, listVaults, updateVault } from "../services/vaults";
import { isAuthSetup, setMasterPassword, verifyMasterPassword } from "../services/auth";
import { AppError } from "../services/ipcResult";
import {
  getEffectivePrivacy,
  getPrivacyDisplayLabel,
  getPrivacyDisplaySummary,
  getPrivacyRank,
  getVaultEffectivePrivacy as getRecursiveVaultEffectivePrivacy,
} from "../utils/privacy";

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

type VaultSidebarProps = {
  selectedVaultId: string | null;
  refreshKey: number;
  onSelectVault: (vaultId: string) => void;
  onSelectNode: (nodeId: string) => void;
  onVaultCreated: (vaultId: string) => void;
  onVaultDeleted: (vaultId: string) => void;
  onOpenDashboard: () => void;
  onOpenSettings: () => void;
  isRedactedUnlocked: boolean;
  setIsRedactedUnlocked: (value: boolean) => void;
  onModalToggle?: (isOpen: boolean) => void;
};

/**
 * Structured Tree Navigation sidebar.
 *
 * When the user types a search query:
 * - ALL vaults remain visible (never removed from DOM).
 * - Vaults / sub-vaults / nodes that do NOT match are rendered with an
 *   opacity-dimmed class (`tree-dimmed`) so they fade into the background.
 * - Vaults that contain a match are auto-expanded and highlighted with a
 *   subtle glow class (`tree-match`).
 * - Matching nodes appear inline under their parent vault, showing the
 *   breadcrumb path.
 */
function VaultSidebar({
  selectedVaultId,
  refreshKey,
  onSelectVault,
  onSelectNode,
  onVaultCreated,
  onVaultDeleted,
  onOpenDashboard,
  onOpenSettings,
  isRedactedUnlocked,
  setIsRedactedUnlocked,
  onModalToggle,
}: VaultSidebarProps) {
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [allNodes, setAllNodes] = useState<Node[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const deferredSearchQuery = useDeferredValue(searchQuery);
  const resolvedQuery = searchQuery === "" ? "" : deferredSearchQuery;
  const [expandedVaults, setExpandedVaults] = useState<Record<string, boolean>>({});
  const [editingVault, setEditingVault] = useState<Vault | null>(null);
  const [editName, setEditName] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editPrivacyTier, setEditPrivacyTier] = useState("");
  const [editIcon, setEditIcon] = useState("");
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [createModalName, setCreateModalName] = useState("");
  const [createModalDescription, setCreateModalDescription] = useState("");
  const [createModalIcon, setCreateModalIcon] = useState("");
  const [createModalPrivacyTier, setCreateModalPrivacyTier] = useState("open");
  const [createModalError, setCreateModalError] = useState("");
  const [createSubvaultModalOpen, setCreateSubvaultModalOpen] = useState(false);
  const [createSubvaultParentVaultId, setCreateSubvaultParentVaultId] = useState("");
  const [createSubvaultParentName, setCreateSubvaultParentName] = useState("");
  const [createSubvaultName, setCreateSubvaultName] = useState("");
  const [createSubvaultDescription, setCreateSubvaultDescription] = useState("");
  const [createSubvaultIcon, setCreateSubvaultIcon] = useState("");
  const [createSubvaultPrivacyTier, setCreateSubvaultPrivacyTier] = useState("open");
  const [createSubvaultError, setCreateSubvaultError] = useState("");
  const [deleteModalOpen, setDeleteModalOpen] = useState(false);
  const [deleteTargetVault, setDeleteTargetVault] = useState<Vault | null>(null);
  const [deletePasswordInput, setDeletePasswordInput] = useState("");
  const [deleteModalError, setDeleteModalError] = useState("");
  const [favoriteVaultIds, setFavoriteVaultIds] = useState<string[]>(() => {
    try {
      const saved = localStorage.getItem("favorite_vault_ids");
      return saved ? JSON.parse(saved) : [];
    } catch {
      return [];
    }
  });

  useEffect(() => {
    localStorage.setItem("favorite_vault_ids", JSON.stringify(favoriteVaultIds));
  }, [favoriteVaultIds]);

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
      map[vault.id] = getRecursiveVaultEffectivePrivacy(vault.id, vaultById, map);
    }
    return map;
  }, [vaultById, vaults]);

  useEffect(() => {
    if (vaults.length > 0) {
      try {
        const saved = localStorage.getItem("favorite_vault_ids");
        const parsed = saved ? JSON.parse(saved) : [];
        if (parsed.length === 0) {
          const defaults: string[] = [];
          for (const vault of vaults) {
            const lowerName = vault.name.toLowerCase();
            if (
              lowerName.includes("classes") ||
              lowerName.includes("credentials") ||
              lowerName.includes("vault 1") ||
              lowerName.includes("work")
            ) {
              defaults.push(vault.id);
            }
          }
          if (defaults.length > 0) {
            setTimeout(() => {
              setFavoriteVaultIds(defaults);
            }, 0);
          }
        }
      } catch (e) {
        console.error("Failed to seed default favorites", e);
      }
    }
  }, [vaults]);

  function toggleFavorite(vaultId: string) {
    setFavoriteVaultIds((prev) => {
      if (prev.includes(vaultId)) {
        return prev.filter((id) => id !== vaultId);
      } else {
        return [...prev, vaultId];
      }
    });
  }

  function getVaultEmoji(vault: Vault): string {
    const iconKey = (vault.icon || "").trim().toLowerCase();

    // First translate known icon keywords stored in DB
    if (iconKey) {
      if (iconKey === "key" || iconKey === "credentials") return "💳";
      if (iconKey === "coins" || iconKey === "finance" || iconKey === "money") return "🪙";
      if (iconKey === "heart" || iconKey === "health" || iconKey === "fitness") return "💪";
      if (iconKey === "book" || iconKey === "learning" || iconKey === "read") return "📚";
      if (iconKey === "user" || iconKey === "personal") return "👤";
      if (iconKey === "briefcase" || iconKey === "work" || iconKey === "project") return "💼";
      if (iconKey === "home" || iconKey === "vault 1") return "🏠";
      if (iconKey === "mobile" || iconKey === "phone" || iconKey === "cse") return "📱";
      if (iconKey === "classes" || iconKey === "computer" || iconKey === "laptop") return "💻";

      // If it is already a single emoji or double character, return it directly
      if (iconKey.length <= 2) {
        return vault.icon!.trim();
      }
    }

    // Fall back to name-based heuristics
    const name = vault.name.toLowerCase();
    if (name.includes("home") || name.includes("vault 1")) return "🏠";
    if (name.includes("class") || name.includes("cse")) return "💻";
    if (name.includes("credential") || name.includes("password") || name.includes("key"))
      return "💳";
    if (
      name.includes("fitness") ||
      name.includes("gym") ||
      name.includes("workout") ||
      name.includes("health")
    )
      return "💪";
    if (name.includes("project") || name.includes("work") || name.includes("mindvault"))
      return "📝";
    if (name.includes("memory") || name.includes("brain")) return "🧠";
    if (
      name.includes("book") ||
      name.includes("read") ||
      name.includes("study") ||
      name.includes("learn")
    )
      return "📚";
    if (name.includes("personal") || name.includes("private") || name.includes("user")) return "👤";
    if (
      name.includes("finance") ||
      name.includes("money") ||
      name.includes("wallet") ||
      name.includes("coins")
    )
      return "💰";

    return "📂";
  }

  const [authModalOpen, setAuthModalOpen] = useState(false);
  const [authMode, setAuthMode] = useState<"verify" | "setup">("verify");
  const [authModalTitle, setAuthModalTitle] = useState("Redacted");
  const [authModalSubtitle, setAuthModalSubtitle] = useState("");
  const [authModalSubmitLabel, setAuthModalSubmitLabel] = useState("Unlock");
  const [authUnlockOnSuccess, setAuthUnlockOnSuccess] = useState(true);
  const [authPasswordInput, setAuthPasswordInput] = useState("");
  const [authModalError, setAuthModalError] = useState("");
  const authModalResolverRef = useRef<((allowed: boolean) => void) | null>(null);

  useEffect(() => {
    const isAnyModalOpen =
      !!editingVault ||
      authModalOpen ||
      createModalOpen ||
      createSubvaultModalOpen ||
      deleteModalOpen;
    onModalToggle?.(isAnyModalOpen);
  }, [
    editingVault,
    authModalOpen,
    createModalOpen,
    createSubvaultModalOpen,
    deleteModalOpen,
    onModalToggle,
  ]);

  async function loadVaults() {
    try {
      const data = await listVaults();
      setVaults(data);
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
        return;
      }
      setError("Failed to load vaults.");
    }
  }

  useEffect(() => {
    const timer = setTimeout(() => {
      void loadVaults();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked]);

  useEffect(() => {
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const nodes = await getAllNodes(isRedactedUnlocked);
          setAllNodes(nodes);
        } catch (err) {
          if (err instanceof AppError) {
            setError(err.message);
          } else {
            setError("Failed to load nodes for search.");
          }
        }
      })();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked]);

  async function onCreateVault() {
    setCreateModalName("");
    setCreateModalDescription("");
    setCreateModalIcon("");
    setCreateModalPrivacyTier("open");
    setCreateModalError("");
    setCreateModalOpen(true);
  }

  function closeCreateVaultModal() {
    setCreateModalOpen(false);
    setCreateModalName("");
    setCreateModalDescription("");
    setCreateModalIcon("");
    setCreateModalPrivacyTier("open");
    setCreateModalError("");
  }

  async function submitCreateVaultModal() {
    const name = createModalName.trim();
    if (!name) {
      setCreateModalError("Enter a vault name.");
      return;
    }

    try {
      const created = await createVault({
        name,
        description: createModalDescription.trim() || undefined,
        icon: createModalIcon.trim() || undefined,
        privacyTier: createModalPrivacyTier.trim() || undefined,
      });
      onVaultCreated(created.id);
      await loadVaults();
      closeCreateVaultModal();
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setCreateModalError(err.message);
        return;
      }
      setCreateModalError("Failed to create vault.");
    }
  }

  async function onCreateSubVault(parentVaultId: string, parentName: string) {
    setCreateSubvaultParentVaultId(parentVaultId);
    setCreateSubvaultParentName(parentName);
    setCreateSubvaultName("");
    setCreateSubvaultDescription("");
    setCreateSubvaultIcon("");
    setCreateSubvaultPrivacyTier("open");
    setCreateSubvaultError("");
    setCreateSubvaultModalOpen(true);
  }

  function closeCreateSubvaultModal() {
    setCreateSubvaultModalOpen(false);
    setCreateSubvaultParentVaultId("");
    setCreateSubvaultParentName("");
    setCreateSubvaultName("");
    setCreateSubvaultDescription("");
    setCreateSubvaultIcon("");
    setCreateSubvaultPrivacyTier("open");
    setCreateSubvaultError("");
  }

  async function submitCreateSubvaultModal() {
    const name = createSubvaultName.trim();
    if (!name) {
      setCreateSubvaultError("Enter a subvault name.");
      return;
    }

    try {
      const created = await createVault({
        name,
        description: createSubvaultDescription.trim() || undefined,
        icon: createSubvaultIcon.trim() || undefined,
        privacyTier: createSubvaultPrivacyTier.trim() || undefined,
        parentVaultId: createSubvaultParentVaultId,
      });
      setExpandedVaults((prev) => ({ ...prev, [createSubvaultParentVaultId]: true }));
      onVaultCreated(created.id);
      await loadVaults();
      closeCreateSubvaultModal();
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setCreateSubvaultError(err.message);
        return;
      }
      setCreateSubvaultError("Failed to create sub-vault.");
    }
  }

  function openDeleteVaultModal(vault: Vault) {
    setDeleteTargetVault(vault);
    setDeletePasswordInput("");
    setDeleteModalError("");
    setDeleteModalOpen(true);
  }

  async function submitDeleteVaultModal() {
    if (!deleteTargetVault) {
      return;
    }

    const vault = deleteTargetVault;
    const effectiveTier = getVaultEffectivePrivacy(vault);
    if ((effectiveTier === "locked" || effectiveTier === "redacted") && !deletePasswordInput) {
      setDeleteModalError("Enter your master password to continue.");
      return;
    }

    if (effectiveTier === "locked" || effectiveTier === "redacted") {
      const verifyResult = await verifyMasterPassword(deletePasswordInput);
      if (verifyResult.error) {
        setDeleteModalError(verifyResult.error.message);
        return;
      }
      if (!verifyResult.data) {
        setDeleteModalError("Incorrect password");
        return;
      }
    }

    try {
      const deleted = await deleteVault(vault.id);
      if (!deleted) {
        setError("Vault could not be deleted.");
        return;
      }
      onVaultDeleted(vault.id);
      await loadVaults();
      setDeleteModalOpen(false);
      setDeleteTargetVault(null);
      setDeletePasswordInput("");
      setDeleteModalError("");
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
        return;
      }
      setError("Failed to delete vault.");
    }
  }

  function closeDeleteVaultModal() {
    setDeleteModalOpen(false);
    setDeleteTargetVault(null);
    setDeletePasswordInput("");
    setDeleteModalError("");
  }

  function closeAuthModal(allowed: boolean) {
    setAuthModalOpen(false);
    setAuthPasswordInput("");
    setAuthModalError("");
    const resolver = authModalResolverRef.current;
    authModalResolverRef.current = null;
    if (resolver) {
      resolver(allowed);
    }
  }

  function openAuthModal({
    mode,
    title,
    subtitle,
    submitLabel,
    unlockOnSuccess,
  }: {
    mode: "verify" | "setup";
    title: string;
    subtitle: string;
    submitLabel: string;
    unlockOnSuccess: boolean;
  }): Promise<boolean> {
    setAuthMode(mode);
    setAuthModalTitle(title);
    setAuthModalSubtitle(subtitle);
    setAuthModalSubmitLabel(submitLabel);
    setAuthUnlockOnSuccess(unlockOnSuccess);
    setAuthPasswordInput("");
    setAuthModalError("");
    setAuthModalOpen(true);
    return new Promise((resolve) => {
      authModalResolverRef.current = resolve;
    });
  }

  async function submitAuthModal() {
    if (!authPasswordInput) {
      return;
    }
    setAuthModalError("");
    if (authMode === "verify") {
      const verifyResult = await verifyMasterPassword(authPasswordInput);
      if (verifyResult.error) {
        setAuthModalError(verifyResult.error.message);
        return;
      }
      if (!verifyResult.data) {
        setAuthModalError("Incorrect password");
        return;
      }
      if (authUnlockOnSuccess) {
        setIsRedactedUnlocked(true);
      }
      closeAuthModal(true);
      return;
    }

    const setResult = await setMasterPassword(authPasswordInput);
    if (setResult.error) {
      setAuthModalError(setResult.error.message);
      return;
    }
    if (!setResult.data) {
      setAuthModalError("Failed to set master password.");
      return;
    }
    if (authUnlockOnSuccess) {
      setIsRedactedUnlocked(true);
    }
    closeAuthModal(true);
  }

  async function unlockRedactedFromSidebar(): Promise<boolean> {
    const setupResult = await isAuthSetup();
    if (setupResult.error) {
      setError(setupResult.error.message);
      return false;
    }

    if (setupResult.data) {
      return openAuthModal({
        mode: "verify",
        title: "Redacted",
        subtitle: "Enter your master password to unlock this tier.",
        submitLabel: "Unlock",
        unlockOnSuccess: true,
      });
    }

    return openAuthModal({
      mode: "setup",
      title: "Redacted",
      subtitle: "Set a master password to unlock this tier.",
      submitLabel: "Set Master Password",
      unlockOnSuccess: true,
    });
  }

  async function onOpenVaultSettings(vault: Vault) {
    const effectiveTier = getVaultEffectivePrivacy(vault);
    if (effectiveTier === "redacted" && !isRedactedUnlocked) {
      const unlocked = await unlockRedactedFromSidebar();
      if (!unlocked) {
        return;
      }
    }

    setEditingVault(vault);
    setEditName(vault.name);
    setEditDescription(vault.description ?? "");
    setEditPrivacyTier(vault.privacyTier);
    setEditIcon(vault.icon ?? "");
  }

  async function onSaveVaultSettings() {
    if (!editingVault) {
      return;
    }

    const currentEffectiveTier = getEffectivePrivacy(editingVault.privacyTier);
    const nextTier = editPrivacyTier.trim().toLowerCase();

    if (nextTier === "redacted" && !isRedactedUnlocked) {
      const unlocked = await unlockRedactedFromSidebar();
      if (!unlocked) {
        return;
      }
    }

    const isDowngradeFromRedacted =
      currentEffectiveTier === "redacted" && getPrivacyRank(nextTier) < getPrivacyRank("redacted");
    if (isDowngradeFromRedacted) {
      const verified = await openAuthModal({
        mode: "verify",
        title: "Confirm Privacy Downgrade",
        subtitle: "Enter your master password to downgrade from redacted.",
        submitLabel: "Confirm",
        unlockOnSuccess: false,
      });
      if (!verified) {
        return;
      }
    }

    try {
      await updateVault({
        id: editingVault.id,
        name: editName.trim(),
        privacyTier: nextTier,
        icon: editIcon.trim() || undefined,
        description: editDescription.trim() || undefined,
      });
      setEditingVault(null);
      await loadVaults();
      setError(null);
    } catch (err) {
      if (err instanceof AppError) {
        setError(err.message);
        return;
      }
      setError("Failed to update vault settings.");
    }
  }

  function getVaultEffectivePrivacy(vault: Vault): string {
    return (
      vaultEffectivePrivacyById[vault.id] ?? getRecursiveVaultEffectivePrivacy(vault.id, vaultById)
    );
  }

  async function onSelectVaultEntry(vault: Vault) {
    const effectiveTier = getVaultEffectivePrivacy(vault);
    if (effectiveTier === "redacted" && !isRedactedUnlocked) {
      const success = await unlockRedactedFromSidebar();
      if (success) {
        onSelectVault(vault.id);
      }
      return;
    }
    onSelectVault(vault.id);
  }

  async function onSelectNodeEntry(node: Node) {
    const containerId = node.subVaultId ?? node.vaultId;
    const containerTier =
      vaultEffectivePrivacyById[containerId] ??
      getRecursiveVaultEffectivePrivacy(containerId, vaultById);
    const nodeEffectiveTier = getEffectivePrivacy(node.privacyTier, null, containerTier);

    if (nodeEffectiveTier === "redacted" && !isRedactedUnlocked) {
      const success = await unlockRedactedFromSidebar();
      if (success) {
        onSelectVault(node.subVaultId ?? node.vaultId);
        onSelectNode(node.id);
      }
      return;
    }

    onSelectVault(node.subVaultId ?? node.vaultId);
    onSelectNode(node.id);
  }

  function onToggleExpand(vaultId: string) {
    setExpandedVaults((prev) => ({
      ...prev,
      [vaultId]: !prev[vaultId],
    }));
  }

  const normalizedQuery = resolvedQuery.trim().toLowerCase();
  const isSearching = normalizedQuery.length > 0;

  // ---------------------------------------------------------------------------
  // Build structured tree data with search-match metadata
  // ---------------------------------------------------------------------------
  const {
    topLevelVaults,
    childrenByParent,
    nodesByVaultId,
    matchingVaultIds,
    matchingSubVaultIds,
    matchingNodeIds,
    resultCount,
  } = useMemo(() => {
    const childMap = new Map<string, Vault[]>();
    for (const vault of vaults) {
      if (!vault.parentVaultId) {
        continue;
      }
      const existing = childMap.get(vault.parentVaultId) ?? [];
      existing.push(vault);
      childMap.set(vault.parentVaultId, existing);
    }

    for (const children of childMap.values()) {
      children.sort((a, b) => a.name.localeCompare(b.name));
    }

    const roots = vaults.filter((vault) => !vault.parentVaultId);
    roots.sort((a, b) => a.name.localeCompare(b.name));

    // Group nodes by their "tree parent" vault id
    // (sub-vault id if present, otherwise top-level vault id)
    const nodeMap = new Map<string, Node[]>();
    for (const node of allNodes) {
      const parentKey = node.subVaultId ?? node.vaultId;
      const existing = nodeMap.get(parentKey) ?? [];
      existing.push(node);
      nodeMap.set(parentKey, existing);
    }

    // Without a search query, show everything normally
    if (!normalizedQuery) {
      return {
        topLevelVaults: roots,
        childrenByParent: childMap,
        nodesByVaultId: nodeMap,
        matchingVaultIds: new Set<string>(),
        matchingSubVaultIds: new Set<string>(),
        matchingNodeIds: new Set<string>(),
        resultCount: 0,
      };
    }

    // With a search query, compute which items match
    const matchVaults = new Set<string>();
    const matchSubVaults = new Set<string>();
    const matchNodes = new Set<string>();
    let count = 0;

    // Find matching nodes
    for (const node of allNodes) {
      const titleMatch = node.title.toLowerCase().includes(normalizedQuery);
      const summaryMatch = node.summary.toLowerCase().includes(normalizedQuery);
      if (titleMatch || summaryMatch) {
        matchNodes.add(node.id);
        count++;
        // Mark parent vault/sub-vault as containing a match
        if (node.subVaultId) {
          matchSubVaults.add(node.subVaultId);
          // Also find the root vault for this sub-vault
          const subVault = vaults.find((v) => v.id === node.subVaultId);
          if (subVault?.parentVaultId) {
            matchVaults.add(subVault.parentVaultId);
          }
        }
        matchVaults.add(node.vaultId);
      }
    }

    // Find matching vaults by name
    for (const vault of vaults) {
      if (vault.name.toLowerCase().includes(normalizedQuery)) {
        if (vault.parentVaultId) {
          matchSubVaults.add(vault.id);
          matchVaults.add(vault.parentVaultId);
        } else {
          matchVaults.add(vault.id);
        }
        count++;
      }
    }

    return {
      topLevelVaults: roots,
      childrenByParent: childMap,
      nodesByVaultId: nodeMap,
      matchingVaultIds: matchVaults,
      matchingSubVaultIds: matchSubVaults,
      matchingNodeIds: matchNodes,
      resultCount: count,
    };
  }, [allNodes, normalizedQuery, vaults]);

  // ---------------------------------------------------------------------------
  // Helpers for determining match/dim state
  // ---------------------------------------------------------------------------
  function isVaultOnMatchPath(vaultId: string): boolean {
    return matchingVaultIds.has(vaultId);
  }

  function isSubVaultMatch(subVaultId: string): boolean {
    return matchingSubVaultIds.has(subVaultId);
  }

  function isNodeMatch(nodeId: string): boolean {
    return matchingNodeIds.has(nodeId);
  }

  /** Whether a vault should be expanded (user toggle OR search auto-expand). */
  function shouldExpand(vaultId: string, hasChildren: boolean): boolean {
    if (!hasChildren) return false;
    if (isSearching && isVaultOnMatchPath(vaultId)) return true;
    return expandedVaults[vaultId] ?? false;
  }

  function renderVault(vault: Vault, isFavSection: boolean) {
    const suffix = isFavSection ? "-fav" : "";
    const effectiveTier = getVaultEffectivePrivacy(vault);
    const children = childrenByParent.get(vault.id) ?? [];
    const vaultNodes = nodesByVaultId.get(vault.id) ?? [];
    const isRedactedLocked = effectiveTier === "redacted" && !isRedactedUnlocked;
    const hasExpandableContent =
      !isRedactedLocked && (children.length > 0 || vaultNodes.length > 0);
    const expanded = shouldExpand(vault.id, hasExpandableContent);

    // Dimming logic: when searching, dim everything that is NOT on a match path
    const isDimmed = isSearching && !isVaultOnMatchPath(vault.id);
    const isHighlighted = isSearching && isVaultOnMatchPath(vault.id);
    const isFav = favoriteVaultIds.includes(vault.id);
    const vaultEmoji = isRedactedLocked ? "⬛" : getVaultEmoji(vault);

    return (
      <li key={vault.id + suffix} className={isDimmed ? "tree-dimmed" : ""}>
        <div
          className={`list-item ${selectedVaultId === vault.id ? "active" : ""} ${
            isHighlighted ? "tree-match" : ""
          }`}
        >
          <button
            type="button"
            className={`tree-toggle ${!hasExpandableContent ? "empty" : ""}`}
            onClick={() => onToggleExpand(vault.id)}
            disabled={!hasExpandableContent}
            aria-label={expanded ? `Collapse ${vault.name}` : `Expand ${vault.name}`}
          >
            {!hasExpandableContent ? "" : expanded ? "▾" : "▸"}
          </button>
          <div className="vault-header">
            <button type="button" className="list-main" onClick={() => onSelectVaultEntry(vault)}>
              <span className="list-title-row">
                <span className="vault-icon-emoji">{vaultEmoji}</span>
                <span className="list-title-text">
                  {getPrivacyDisplayLabel(vault.name, effectiveTier, isRedactedUnlocked)}
                </span>
                {effectiveTier === "locked" && <span className="privacy-lock-icon">🔒</span>}
              </span>
              {isRedactedLocked ? (
                <small>[Metadata Locked]</small>
              ) : (
                vault.description && (
                  <small>
                    {getPrivacyDisplaySummary(vault.description, effectiveTier, isRedactedUnlocked)}
                  </small>
                )
              )}
            </button>
            <div className="list-actions">
              <button
                type="button"
                className={`list-favorite ${isFav ? "is-fav" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  toggleFavorite(vault.id);
                }}
                aria-label={isFav ? "Remove from Favorites" : "Add to Favorites"}
              >
                {isFav ? "★" : "☆"}
              </button>
              <button
                type="button"
                className="list-subvault"
                onClick={() => onCreateSubVault(vault.id, vault.name)}
                aria-label={`Create sub-vault under ${vault.name}`}
              >
                +
              </button>
              <button
                type="button"
                className="list-settings"
                onClick={(e) => {
                  e.stopPropagation();
                  void onOpenVaultSettings(vault);
                }}
                aria-label={`Update settings for ${vault.name}`}
              >
                ⚙️
              </button>
              <button
                type="button"
                className="list-delete"
                onClick={() => openDeleteVaultModal(vault)}
                aria-label={`Delete ${vault.name}`}
              >
                ×
              </button>
            </div>
          </div>
        </div>

        {/* ----- Expanded children: Sub-Vaults and Nodes ----- */}
        {expanded && (children.length > 0 || vaultNodes.length > 0) && (
          <ul className="tree-child-list">
            {children.map((child) => {
              const childEffectiveTier = getVaultEffectivePrivacy(child);
              const childNodes = nodesByVaultId.get(child.id) ?? [];
              const isChildRedactedLocked =
                childEffectiveTier === "redacted" && !isRedactedUnlocked;
              const childHasContent = !isChildRedactedLocked && childNodes.length > 0;
              const childIsDimmed =
                isSearching && !isSubVaultMatch(child.id) && !isVaultOnMatchPath(child.id);
              const childIsHighlighted = isSearching && isSubVaultMatch(child.id);
              const childExpanded =
                childHasContent &&
                ((isSearching && isSubVaultMatch(child.id)) || (expandedVaults[child.id] ?? false));
              const childEmoji = isChildRedactedLocked ? "⬛" : getVaultEmoji(child);

              return (
                <li key={child.id + suffix} className={childIsDimmed ? "tree-dimmed" : ""}>
                  <div
                    className={`list-item sub sub-vault-item ${
                      selectedVaultId === child.id ? "active" : ""
                    } ${childIsHighlighted ? "tree-match" : ""}`}
                  >
                    <button
                      type="button"
                      className={`tree-toggle ${!childHasContent ? "empty" : ""}`}
                      onClick={() => onToggleExpand(child.id)}
                      disabled={!childHasContent}
                      aria-label={childExpanded ? `Collapse ${child.name}` : `Expand ${child.name}`}
                    >
                      {!childHasContent ? "" : childExpanded ? "▾" : "▸"}
                    </button>
                    <button
                      type="button"
                      className="list-main"
                      onClick={() => onSelectVaultEntry(child)}
                    >
                      <span className="list-title-row">
                        <span className="vault-icon-emoji">{childEmoji}</span>
                        <span className="list-title-text">
                          {getPrivacyDisplayLabel(
                            child.name,
                            childEffectiveTier,
                            isRedactedUnlocked
                          )}
                        </span>
                        {childEffectiveTier === "locked" && (
                          <span className="privacy-lock-icon">🔒</span>
                        )}
                      </span>
                      {isChildRedactedLocked ? (
                        <small>[Metadata Locked]</small>
                      ) : (
                        child.description && (
                          <small>
                            {getPrivacyDisplaySummary(
                              child.description,
                              childEffectiveTier,
                              isRedactedUnlocked
                            )}
                          </small>
                        )
                      )}
                    </button>
                    <div className="list-actions">
                      <button
                        type="button"
                        className="list-settings"
                        onClick={(e) => {
                          e.stopPropagation();
                          void onOpenVaultSettings(child);
                        }}
                        aria-label={`Update settings for ${child.name}`}
                      >
                        ⚙️
                      </button>
                      <button
                        type="button"
                        className="list-delete"
                        onClick={() => openDeleteVaultModal(child)}
                        aria-label={`Delete ${child.name}`}
                      >
                        ×
                      </button>
                    </div>
                  </div>

                  {/* ----- Inline nodes under sub-vault ----- */}
                  {childExpanded && childNodes.length > 0 && (
                    <ul className="tree-child-list">
                      {childNodes.map((node) => {
                        const nodeDimmed = isSearching && !isNodeMatch(node.id);
                        const nodeHighlighted = isSearching && isNodeMatch(node.id);
                        const nodeEffectiveTier = getEffectivePrivacy(
                          node.privacyTier,
                          null,
                          childEffectiveTier
                        );
                        const isNodeRedactedLocked =
                          nodeEffectiveTier === "redacted" && !isRedactedUnlocked;
                        const isNodeLocked = nodeEffectiveTier === "locked";
                        const nodeIcon = isNodeRedactedLocked ? "⬛" : isNodeLocked ? "🔒" : "📄";
                        const summaryText = isNodeRedactedLocked
                          ? "[Metadata Locked]"
                          : node.summary.slice(0, 60);

                        return (
                          <li key={node.id + suffix} className={nodeDimmed ? "tree-dimmed" : ""}>
                            <button
                              type="button"
                              className={`tree-node-item ${nodeHighlighted ? "tree-match" : ""}`}
                              onClick={() => onSelectNodeEntry(node)}
                            >
                              <span className="tree-node-icon">{nodeIcon}</span>
                              <span className="tree-node-text">
                                <strong>
                                  {getPrivacyDisplayLabel(
                                    node.title,
                                    nodeEffectiveTier,
                                    isRedactedUnlocked
                                  )}
                                </strong>
                                <small>
                                  {getPrivacyDisplaySummary(
                                    summaryText,
                                    nodeEffectiveTier,
                                    isRedactedUnlocked
                                  )}
                                </small>
                              </span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </li>
              );
            })}

            {/* ----- Inline nodes directly under the vault (no sub-vault) ----- */}
            {vaultNodes.map((node) => {
              const nodeDimmed = isSearching && !isNodeMatch(node.id);
              const nodeHighlighted = isSearching && isNodeMatch(node.id);
              const nodeEffectiveTier = getEffectivePrivacy(
                node.privacyTier,
                null,
                getVaultEffectivePrivacy(vault)
              );
              const isNodeRedactedLocked = nodeEffectiveTier === "redacted" && !isRedactedUnlocked;
              const isNodeLocked = nodeEffectiveTier === "locked";
              const nodeIcon = isNodeRedactedLocked ? "⬛" : isNodeLocked ? "🔒" : "📄";
              const summaryText = isNodeRedactedLocked
                ? "[Metadata Locked]"
                : node.summary.slice(0, 60);

              return (
                <li key={node.id + suffix} className={nodeDimmed ? "tree-dimmed" : ""}>
                  <button
                    type="button"
                    className={`tree-node-item ${nodeHighlighted ? "tree-match" : ""}`}
                    onClick={() => onSelectNodeEntry(node)}
                  >
                    <span className="tree-node-icon">{nodeIcon}</span>
                    <span className="tree-node-text">
                      <strong>
                        {getPrivacyDisplayLabel(node.title, nodeEffectiveTier, isRedactedUnlocked)}
                      </strong>
                      <small>
                        {getPrivacyDisplaySummary(
                          summaryText,
                          nodeEffectiveTier,
                          isRedactedUnlocked
                        )}
                      </small>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </li>
    );
  }

  return (
    <aside className="pane pane-left">
      <div className="pane-header">
        <span className="sidebar-subtitle">Vaults</span>
        <button type="button" className="new-vault-btn" onClick={onCreateVault}>
          NEW
        </button>
      </div>

      <div className="search-container">
        <span className="search-icon">🔍</span>
        <input
          type="search"
          className="search-input-field"
          placeholder="Search vaults"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
        />
      </div>

      <button type="button" className="dashboard-trigger" onClick={onOpenDashboard}>
        <span className="active-memory-icon">🧠</span> Active Memory
      </button>

      <div className="sidebar-scrollable-content">
        {isSearching && (
          <p className="search-result-count">
            {resultCount === 0
              ? "No results"
              : `${resultCount} result${resultCount !== 1 ? "s" : ""}`}
          </p>
        )}
        {error && <p className="pane-error">{error}</p>}

        <div className="sidebar-section-container">
          <h4 className="sidebar-section-title">Favorites</h4>
          <ul className="vault-list">
            {topLevelVaults
              .filter((v) => favoriteVaultIds.includes(v.id))
              .map((vault) => renderVault(vault, true))}
            {topLevelVaults.filter((v) => favoriteVaultIds.includes(v.id)).length === 0 && (
              <li className="empty-section-tip">No favorite vaults</li>
            )}
          </ul>
        </div>

        <div className="sidebar-section-container">
          <h4 className="sidebar-section-title">Private</h4>
          <ul className="vault-list">
            {topLevelVaults
              .filter((v) => v.privacyTier && v.privacyTier !== "open")
              .map((vault) => renderVault(vault, false))}
            {topLevelVaults.filter((v) => v.privacyTier && v.privacyTier !== "open").length ===
              0 && <li className="empty-section-tip">No private vaults</li>}
          </ul>
        </div>

        <div className="sidebar-section-container">
          <h4 className="sidebar-section-title">Open</h4>
          <ul className="vault-list">
            {topLevelVaults
              .filter((v) => !v.privacyTier || v.privacyTier === "open")
              .map((vault) => renderVault(vault, false))}
            {topLevelVaults.filter((v) => !v.privacyTier || v.privacyTier === "open").length ===
              0 && <li className="empty-section-tip">No open vaults</li>}
          </ul>
        </div>
      </div>

      <div className="sidebar-footer">
        <button type="button" className="settings-trigger" onClick={onOpenSettings}>
          <span className="settings-icon">⚙️</span> Account and Settings
        </button>
      </div>

      {authModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={() => closeAuthModal(false)}>
            <div
              className="redacted-lock-screen sidebar-auth-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <span className="redacted-lock-icon" aria-hidden="true">
                🔒
              </span>
              <h4 className="redacted-lock-title">{authModalTitle}</h4>
              <p className="redacted-lock-subtitle">{authModalSubtitle}</p>
              <form
                className="redacted-lock-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  void submitAuthModal();
                }}
              >
                <input
                  className="redacted-lock-input"
                  type="password"
                  value={authPasswordInput}
                  onChange={(event) => setAuthPasswordInput(event.target.value)}
                  placeholder={
                    authMode === "setup" ? "Choose a master password" : "Master password"
                  }
                  autoFocus
                />
                <div className="sidebar-auth-actions">
                  <button
                    type="button"
                    className="redacted-lock-button sidebar-auth-cancel"
                    onClick={() => closeAuthModal(false)}
                  >
                    Cancel
                  </button>
                  <button type="submit" className="redacted-lock-button">
                    {authModalSubmitLabel}
                  </button>
                </div>
              </form>
              {authModalError && <p className="redacted-lock-error">{authModalError}</p>}
            </div>
          </div>,
          document.body
        )}

      {deleteModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={closeDeleteVaultModal}>
            <div
              className="vault-settings-modal sidebar-auth-modal delete-confirm-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="modal-title">Delete Vault</h3>
              <p className="modal-subtitle">
                {deleteTargetVault
                  ? `Delete ${deleteTargetVault.name}? This cannot be undone.`
                  : "Delete this vault? This cannot be undone."}
              </p>
              {deleteTargetVault &&
                (getVaultEffectivePrivacy(deleteTargetVault) === "locked" ||
                  getVaultEffectivePrivacy(deleteTargetVault) === "redacted") && (
                  <label className="settings-field">
                    <span>Master Password</span>
                    <input
                      type="password"
                      className="settings-input"
                      value={deletePasswordInput}
                      onChange={(e) => setDeletePasswordInput(e.target.value)}
                      placeholder="Master password"
                      autoFocus
                    />
                  </label>
                )}
              {deleteModalError && <p className="redacted-lock-error">{deleteModalError}</p>}
              <div className="sidebar-auth-actions settings-modal-actions">
                <button
                  type="button"
                  className="redacted-lock-button sidebar-auth-cancel"
                  onClick={closeDeleteVaultModal}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button"
                  onClick={submitDeleteVaultModal}
                >
                  Delete
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}

      {createModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={closeCreateVaultModal}>
            <div
              className="vault-settings-modal sidebar-auth-modal delete-confirm-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="modal-title">New Vault</h3>
              <p className="modal-subtitle">Create a new top-level vault in the sidebar.</p>
              <div className="settings-fields-grid">
                <label className="settings-field">
                  <span>Vault Name</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={createModalName}
                    onChange={(e) => setCreateModalName(e.target.value)}
                    placeholder="e.g. Finance"
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
                    placeholder="e.g. Budgets, planning, and reports"
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
                  onClick={closeCreateVaultModal}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button"
                  onClick={submitCreateVaultModal}
                >
                  Create
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}

      {createSubvaultModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={closeCreateSubvaultModal}>
            <div
              className="vault-settings-modal sidebar-auth-modal delete-confirm-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="modal-title">New Subvault</h3>
              <p className="modal-subtitle">
                {createSubvaultParentName
                  ? `Create a new subvault inside ${createSubvaultParentName}.`
                  : "Create a new subvault."}
              </p>
              <div className="settings-fields-grid">
                <label className="settings-field">
                  <span>Subvault Name</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={createSubvaultName}
                    onChange={(e) => setCreateSubvaultName(e.target.value)}
                    placeholder="e.g. Research"
                    autoFocus
                  />
                </label>

                <label className="settings-field">
                  <span>Description</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={createSubvaultDescription}
                    onChange={(e) => setCreateSubvaultDescription(e.target.value)}
                    placeholder="e.g. Notes and sources"
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
                          className={`emoji-choice-btn ${createSubvaultIcon === emoji ? "selected" : ""}`}
                          onClick={() => setCreateSubvaultIcon(emoji)}
                        >
                          {emoji}
                        </button>
                      ))}
                    </div>
                    <input
                      type="text"
                      value={createSubvaultIcon}
                      onChange={(e) => setCreateSubvaultIcon(e.target.value)}
                      placeholder="Or type a custom emoji/text"
                      maxLength={10}
                      className="settings-input custom-emoji-input"
                    />
                  </div>
                </div>

                <label className="settings-field">
                  <span>Privacy Tier</span>
                  <select
                    value={createSubvaultPrivacyTier}
                    onChange={(e) => setCreateSubvaultPrivacyTier(e.target.value)}
                    className="settings-select"
                  >
                    <option value="open">Open (No restriction)</option>
                    <option value="local_only">Local Only (Never cloud synced)</option>
                    <option value="locked">Locked (Requires unlock to access)</option>
                    <option value="redacted">Redacted (Hidden metadata/title)</option>
                  </select>
                </label>
              </div>
              {createSubvaultError && <p className="redacted-lock-error">{createSubvaultError}</p>}
              <div className="sidebar-auth-actions settings-modal-actions">
                <button
                  type="button"
                  className="redacted-lock-button sidebar-auth-cancel"
                  onClick={closeCreateSubvaultModal}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button"
                  onClick={submitCreateSubvaultModal}
                >
                  Create
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}

      {editingVault &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={() => setEditingVault(null)}>
            <div
              className="vault-settings-modal sidebar-auth-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="modal-title">Vault Settings</h3>
              <p className="modal-subtitle">Customize your vault details and security.</p>

              <div className="settings-fields-grid">
                <label className="settings-field">
                  <span>Vault Name</span>
                  <input
                    type="text"
                    value={editName}
                    onChange={(e) => setEditName(e.target.value)}
                    placeholder="e.g. Work Notes"
                    className="settings-input"
                  />
                </label>

                <label className="settings-field">
                  <span>Description</span>
                  <input
                    type="text"
                    value={editDescription}
                    onChange={(e) => setEditDescription(e.target.value)}
                    placeholder="e.g. Budget lists and project plans"
                    className="settings-input"
                  />
                </label>

                <div className="settings-field">
                  <span>Emoji / Icon</span>
                  <div className="emoji-picker-container">
                    <div className="emoji-picker-grid">
                      {[
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
                      ].map((emoji) => (
                        <button
                          key={emoji}
                          type="button"
                          className={`emoji-choice-btn ${editIcon === emoji ? "selected" : ""}`}
                          onClick={() => setEditIcon(emoji)}
                        >
                          {emoji}
                        </button>
                      ))}
                    </div>
                    <input
                      type="text"
                      value={editIcon}
                      onChange={(e) => setEditIcon(e.target.value)}
                      placeholder="Or type a custom emoji/text"
                      maxLength={10}
                      className="settings-input custom-emoji-input"
                    />
                  </div>
                </div>

                <label className="settings-field">
                  <span>Privacy Tier</span>
                  <select
                    value={editPrivacyTier}
                    onChange={(e) => setEditPrivacyTier(e.target.value)}
                    className="settings-select"
                  >
                    <option value="open">Open (No restriction)</option>
                    <option value="local_only">Local Only (Never cloud synced)</option>
                    <option value="locked">Locked (Requires unlock to access)</option>
                    <option value="redacted">Redacted (Hidden metadata/title)</option>
                  </select>
                </label>
              </div>

              <div className="sidebar-auth-actions settings-modal-actions">
                <button
                  type="button"
                  className="redacted-lock-button sidebar-auth-cancel"
                  onClick={() => setEditingVault(null)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button primary-btn"
                  onClick={onSaveVaultSettings}
                  disabled={!editName.trim()}
                >
                  Save Changes
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
    </aside>
  );
}

export default VaultSidebar;
