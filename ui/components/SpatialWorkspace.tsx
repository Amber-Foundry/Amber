import React, { useState, useEffect, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import {
  listVaults,
  createVault,
  deleteVault,
  updateVault,
  updateVaultPosition,
  updateVaultColorTheme,
} from "../services/vaults";
import { isAuthSetup, setMasterPassword, verifyMasterPassword } from "../services/auth";
import { listAllDoors } from "../services/doors";
import { getNodes, createNode, deleteNode, updateNode } from "../services/nodes";
import { sanitizeSvgText } from "../utils/svgSanitizer";
import {
  getEffectivePrivacy,
  getPrivacyDisplayLabel,
  getVaultEffectivePrivacy,
  shouldOmitSpatialConnector,
} from "../utils/privacy";
import type { Vault, Node, Door } from "../types/generated";
import "../style/components/SpatialWorkspace.css";

interface SpatialWorkspaceProps {
  selectedVaultId: string | null;
  selectedNodeId: string | null;
  onSelectVault: (id: string | null) => void;
  onFocusVault?: (id: string | null) => void;
  onSelectNode: (id: string) => void;
  refreshKey: number;
  onVaultCreated?: (id: string) => void;
  onVaultDeleted?: (id: string) => void;
  onVaultUpdated?: (id: string) => void;
  onNodeCreated?: (id: string) => void;
  onNodeDeleted?: (id: string) => void;
  onNodeUpdated?: (id: string) => void;
  isRedactedUnlocked?: boolean;
  setIsRedactedUnlocked?: (value: boolean) => void;
  onSelectedVaultRequiresUnlockChange?: (requiresUnlock: boolean) => void;
  onModalToggle?: (isOpen: boolean) => void;
  isLeftPanePinned?: boolean;
  onLeftPanePinChange?: (pinned: boolean) => void;
}

const CURATED_PALETTE = [
  { id: "ember", name: "Ember Orange", hsl: "hsl(24, 95%, 50%)" },
  { id: "gold", name: "Gold Velvet", hsl: "hsl(42, 85%, 42%)" },
  { id: "emerald", name: "Emerald Forest", hsl: "hsl(142, 70%, 35%)" },
  { id: "teal", name: "Teal Abyss", hsl: "hsl(174, 85%, 30%)" },
  { id: "ocean", name: "Deep Ocean", hsl: "hsl(215, 80%, 46%)" },
  { id: "royal", name: "Amethyst Royal", hsl: "hsl(265, 65%, 52%)" },
  { id: "storm", name: "Slate Storm", hsl: "hsl(220, 20%, 42%)" },
  { id: "rose", name: "Rose Crypt", hsl: "hsl(354, 70%, 45%)" },
];

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

function getVaultTheme(name: string) {
  const lowercaseName = name.toLowerCase();
  if (
    lowercaseName.includes("credential") ||
    lowercaseName.includes("secret") ||
    lowercaseName.includes("password") ||
    lowercaseName.includes("key")
  ) {
    return {
      class: "theme-credentials",
      emoji: "🔐",
      accent: "#b56a37",
    };
  }
  if (
    lowercaseName.includes("work") ||
    lowercaseName.includes("briefcase") ||
    lowercaseName.includes("job") ||
    lowercaseName.includes("project")
  ) {
    return {
      class: "theme-work",
      emoji: "💼",
      accent: "#3182ce",
    };
  }
  if (
    lowercaseName.includes("education") ||
    lowercaseName.includes("learning") ||
    lowercaseName.includes("school") ||
    lowercaseName.includes("book") ||
    lowercaseName.includes("study")
  ) {
    return {
      class: "theme-education",
      emoji: "🎓",
      accent: "#805ad5",
    };
  }
  if (
    lowercaseName.includes("coding") ||
    lowercaseName.includes("code") ||
    lowercaseName.includes("tech") ||
    lowercaseName.includes("dev") ||
    lowercaseName.includes("program")
  ) {
    return {
      class: "theme-coding",
      emoji: "💻",
      accent: "#38a169",
    };
  }
  if (
    lowercaseName.includes("personal") ||
    lowercaseName.includes("user") ||
    lowercaseName.includes("profile") ||
    lowercaseName.includes("me")
  ) {
    return {
      class: "theme-personal",
      emoji: "👤",
      accent: "#b56a37",
    };
  }
  if (
    lowercaseName.includes("finance") ||
    lowercaseName.includes("coin") ||
    lowercaseName.includes("money") ||
    lowercaseName.includes("wealth")
  ) {
    return {
      class: "theme-finance",
      emoji: "🪙",
      accent: "#b56a37",
    };
  }
  return {
    class: "theme-default",
    emoji: "📂",
    accent: "#b56a37",
  };
}

function getVaultEmoji(icon: string | null | undefined, name: string): string {
  const iconKey = (icon || "").trim().toLowerCase();

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

    if (iconKey.length <= 2) {
      return icon!.trim();
    }
  }

  return getVaultTheme(name).emoji;
}

export default function SpatialWorkspace({
  selectedVaultId,
  selectedNodeId,
  onSelectVault,
  onFocusVault,
  onSelectNode,
  refreshKey,
  onVaultCreated,
  onVaultDeleted,
  onVaultUpdated,
  onNodeCreated,
  onNodeDeleted,
  onNodeUpdated,
  isRedactedUnlocked = false,
  setIsRedactedUnlocked,
  onSelectedVaultRequiresUnlockChange,
  onModalToggle,
  isLeftPanePinned = false,
  onLeftPanePinChange,
}: SpatialWorkspaceProps) {
  // --- Data States ---
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [doors, setDoors] = useState<Door[]>([]);
  const [loading, setLoading] = useState(true);
  const [localRefresh, setLocalRefresh] = useState(0);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // Auto-clear error message after 5 seconds
  useEffect(() => {
    if (errorMsg) {
      const timer = setTimeout(() => {
        setErrorMsg(null);
      }, 5000);
      return () => clearTimeout(timer);
    }
  }, [errorMsg]);

  // --- Canvas Navigation States ---
  const [zoom, setZoom] = useState(0.85);
  const [pan, setPan] = useState({ x: 100, y: 80 });
  const [isDragging, setIsDragging] = useState(false);
  const [dragStart, setDragStart] = useState({ x: 0, y: 0 });

  // --- Card Dragging States ---
  const [activeDragCardId, setActiveDragCardId] = useState<string | null>(null);
  const [cardDragOffset, setCardDragOffset] = useState({ x: 0, y: 0 });
  const [cardPositions, setCardPositions] = useState<Record<string, { x: number; y: number }>>({});

  // --- Inline Editing States ---
  const [editingItemId, setEditingItemId] = useState<string | null>(null);
  const [editingItemType, setEditingItemType] = useState<"vault" | "subvault" | "node" | null>(
    null
  );
  const [editValue, setEditValue] = useState("");

  // --- Inline Adding States ---
  const [addingToVaultId, setAddingToVaultId] = useState<string | null>(null);
  const [addingToSubvaultId, setAddingToSubvaultId] = useState<string | null>(null);
  const [addingType, setAddingType] = useState<"node" | "subvault" | "global-vault" | null>(null);
  const [addInputValue, setAddInputValue] = useState("");
  const [isAddingBusy, setIsAddingBusy] = useState(false);
  const [vaultCreateModalOpen, setVaultCreateModalOpen] = useState(false);
  const [vaultCreateMode, setVaultCreateMode] = useState<"global-vault" | "subvault" | null>(null);
  const [vaultCreateParentId, setVaultCreateParentId] = useState<string | null>(null);
  const [vaultCreateParentName, setVaultCreateParentName] = useState("");
  const [vaultCreateName, setVaultCreateName] = useState("");
  const [vaultCreateDescription, setVaultCreateDescription] = useState("");
  const [vaultCreateIcon, setVaultCreateIcon] = useState("");
  const [vaultCreatePrivacyTier, setVaultCreatePrivacyTier] = useState("open");
  const [vaultCreateError, setVaultCreateError] = useState("");
  const [redactedAuthModalOpen, setRedactedAuthModalOpen] = useState(false);
  const [redactedAuthMode, setRedactedAuthMode] = useState<"verify" | "setup">("verify");
  const [redactedAuthTitle, setRedactedAuthTitle] = useState("Redacted");
  const [redactedAuthSubtitle, setRedactedAuthSubtitle] = useState("");
  const [redactedAuthSubmitLabel, setRedactedAuthSubmitLabel] = useState("Unlock");
  const [redactedAuthPasswordInput, setRedactedAuthPasswordInput] = useState("");
  const [redactedAuthError, setRedactedAuthError] = useState("");
  const redactedAuthResolverRef = useRef<((allowed: boolean) => void) | null>(null);

  // --- Deletion Confirmation States ---
  const [deleteArmedId, setDeleteArmedId] = useState<string | null>(null);
  const [deleteArmedTimer, setDeleteArmedTimer] = useState<ReturnType<typeof setTimeout> | null>(
    null
  );

  // --- Search State ---
  const [searchQuery, setSearchQuery] = useState("");

  const worldRef = useRef<HTMLDivElement>(null);

  // Load backend data
  const loadData = async () => {
    try {
      const [allVaults, allNodes, allDoors] = await Promise.all([
        listVaults(),
        getNodes(),
        listAllDoors(),
      ]);

      setVaults(allVaults);
      setNodes(allNodes);

      // Filter doors based on valid endpoints
      if (allDoors.data) {
        setDoors(allDoors.data);
      }

      // Initialize/sync card coordinates incrementally from SQLite uiMetadata column
      setCardPositions((prev) => {
        const next = { ...prev };
        allVaults.forEach((v, index) => {
          // Subvaults don't float independently; they are nested in their parent card
          if (v.parentVaultId) return;

          // Only initialize/overwrite if it doesn't already exist in local state
          if (!next[v.id]) {
            let parsed: { position?: { x: number; y: number } } = {};
            try {
              if (v.uiMetadata) {
                parsed = JSON.parse(v.uiMetadata);
              }
            } catch (e) {
              console.error("Error parsing uiMetadata for vault:", v.id, e);
            }

            next[v.id] = parsed.position || {
              x: (index % 3) * 350 + 80,
              y: Math.floor(index / 3) * 550 + 100,
            };
          }
        });
        return next;
      });
    } catch (e) {
      console.error("Failed to load workspace data:", e);
      setErrorMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void loadData();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [refreshKey, localRefresh, isRedactedUnlocked]);

  useEffect(() => {
    onModalToggle?.(vaultCreateModalOpen || redactedAuthModalOpen);
  }, [vaultCreateModalOpen, redactedAuthModalOpen, onModalToggle]);

  // Clean up delete arming timer
  useEffect(() => {
    return () => {
      if (deleteArmedTimer) clearTimeout(deleteArmedTimer);
    };
  }, [deleteArmedTimer]);

  // --- Drag & Zoom Canvas Handlers ---
  const handleMouseDown = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    if (
      target.closest(".spatial-vault-card") ||
      target.closest(".spatial-header-bar") ||
      target.closest(".spatial-bottom-bar") ||
      target.closest(".spatial-bottom-bar-pill")
    ) {
      return;
    }
    if (selectedVaultId && !isLeftPanePinned) {
      onSelectVault(null);
    }
    setIsDragging(true);
    setDragStart({ x: e.clientX - pan.x, y: e.clientY - pan.y });
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (activeDragCardId) {
      // Dragging a card
      const newX = Math.round(e.clientX / zoom - cardDragOffset.x);
      const newY = Math.round(e.clientY / zoom - cardDragOffset.y);

      setCardPositions((prev) => ({
        ...prev,
        [activeDragCardId]: { x: newX, y: newY },
      }));

      // Call debounced updater
      void updateVaultPosition(activeDragCardId, newX, newY);
      return;
    }

    if (!isDragging) return;
    setPan({
      x: e.clientX - dragStart.x,
      y: e.clientY - dragStart.y,
    });
  };

  const handleMouseUp = () => {
    setIsDragging(false);
    setActiveDragCardId(null);
  };

  const handleWheel = (e: React.WheelEvent) => {
    const zoomFactor = 1.08;
    const newZoom =
      e.deltaY < 0 ? Math.min(zoom * zoomFactor, 2.5) : Math.max(zoom / zoomFactor, 0.25);
    setZoom(newZoom);
  };

  const resetView = () => {
    const container = worldRef.current?.parentElement;
    const cards = worldRef.current?.querySelectorAll<HTMLElement>(".spatial-vault-card");

    if (!container || !cards || cards.length === 0) {
      setZoom(0.85);
      setPan({ x: 100, y: 80 });
      return;
    }

    const margin = 80;
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;

    cards.forEach((card) => {
      const left = card.offsetLeft;
      const top = card.offsetTop;
      const right = left + card.offsetWidth;
      const bottom = top + card.offsetHeight;

      minX = Math.min(minX, left);
      minY = Math.min(minY, top);
      maxX = Math.max(maxX, right);
      maxY = Math.max(maxY, bottom);
    });

    if (
      !Number.isFinite(minX) ||
      !Number.isFinite(minY) ||
      !Number.isFinite(maxX) ||
      !Number.isFinite(maxY)
    ) {
      setZoom(0.85);
      setPan({ x: 100, y: 80 });
      return;
    }

    const contentWidth = Math.max(1, maxX - minX);
    const contentHeight = Math.max(1, maxY - minY);
    const availableWidth = Math.max(1, container.clientWidth - margin * 2);
    const availableHeight = Math.max(1, container.clientHeight - margin * 2);
    const fitZoom = Math.max(
      0.25,
      Math.min(1, availableWidth / contentWidth, availableHeight / contentHeight)
    );

    const contentCenterX = minX + contentWidth / 2;
    const contentCenterY = minY + contentHeight / 2;
    const targetPanX = Math.round(container.clientWidth / 2 - contentCenterX * fitZoom);
    const targetPanY = Math.round(container.clientHeight / 2 - contentCenterY * fitZoom);

    setZoom(fitZoom);
    setPan({ x: targetPanX, y: targetPanY });
  };

  // --- Card Dragging MouseDown Handler ---
  const handleCardMouseDown = (
    e: React.MouseEvent,
    vaultId: string,
    currentPos: { x: number; y: number }
  ) => {
    e.stopPropagation();
    const target = e.target as HTMLElement;
    if (
      target.closest("button") ||
      target.closest("input") ||
      target.closest(".spatial-node-row") ||
      target.closest(".spatial-subvault-container")
    ) {
      return;
    }
    setActiveDragCardId(vaultId);
    setCardDragOffset({
      x: e.clientX / zoom - currentPos.x,
      y: e.clientY / zoom - currentPos.y,
    });
  };

  // --- Direct Data Management CRUD Handlers ---

  // Inline rename saving
  const handleRenameSave = async (id: string, type: "vault" | "subvault" | "node") => {
    if (!editValue.trim()) {
      setEditingItemId(null);
      return;
    }
    try {
      if (type === "vault" || type === "subvault") {
        await updateVault({ id, name: editValue.trim() });
        if (onVaultUpdated) onVaultUpdated(id);
      } else {
        await updateNode({ id, title: editValue.trim() });
        if (onNodeUpdated) onNodeUpdated(id);
      }
      setLocalRefresh((prev) => prev + 1);
    } catch (e) {
      console.error("Failed to rename item:", e);
      setErrorMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setEditingItemId(null);
    }
  };

  const handleRenameKeyDown = (
    e: React.KeyboardEvent,
    id: string,
    type: "vault" | "subvault" | "node"
  ) => {
    if (e.key === "Enter") {
      handleRenameSave(id, type);
    } else if (e.key === "Escape") {
      setEditingItemId(null);
    }
  };

  // Inline adding confirmer
  const handleAddSubmit = async () => {
    if (!addInputValue.trim()) {
      setAddingType(null);
      return;
    }
    setIsAddingBusy(true);
    try {
      if (addingType === "global-vault") {
        const v = await createVault({ name: addInputValue.trim() });
        if (onVaultCreated) onVaultCreated(v.id);
      } else if (addingType === "subvault" && addingToVaultId) {
        await createVault({ name: addInputValue.trim(), parentVaultId: addingToVaultId });
        if (onVaultUpdated) onVaultUpdated(addingToVaultId);
      } else if (addingType === "node" && addingToVaultId) {
        // Omit hardcoded privacyTier: "open" so it inherits parent vault/subvault privacy tier securely
        const n = await createNode({
          vaultId: addingToVaultId,
          subVaultId: addingToSubvaultId || undefined,
          title: addInputValue.trim(),
          summary: "",
          nodeType: "concept",
        });
        if (onNodeCreated) onNodeCreated(n.id);
      }
      setLocalRefresh((prev) => prev + 1);
      setAddInputValue("");
      setAddingType(null);
    } catch (e) {
      console.error("Failed to create inline item:", e);
      setErrorMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setIsAddingBusy(false);
    }
  };

  const openVaultCreateModal = (
    mode: "global-vault" | "subvault",
    parentId: string | null = null,
    parentName = ""
  ) => {
    setVaultCreateMode(mode);
    setVaultCreateParentId(parentId);
    setVaultCreateParentName(parentName);
    setVaultCreateName("");
    setVaultCreateDescription("");
    setVaultCreateIcon("");
    setVaultCreatePrivacyTier("open");
    setVaultCreateError("");
    setVaultCreateModalOpen(true);
  };

  const closeVaultCreateModal = () => {
    setVaultCreateModalOpen(false);
    setVaultCreateMode(null);
    setVaultCreateParentId(null);
    setVaultCreateParentName("");
    setVaultCreateName("");
    setVaultCreateDescription("");
    setVaultCreateIcon("");
    setVaultCreatePrivacyTier("open");
    setVaultCreateError("");
  };

  const closeRedactedAuthModal = (allowed = false) => {
    setRedactedAuthModalOpen(false);
    setRedactedAuthMode("verify");
    setRedactedAuthTitle("Redacted");
    setRedactedAuthSubtitle("");
    setRedactedAuthSubmitLabel("Unlock");
    setRedactedAuthPasswordInput("");
    setRedactedAuthError("");
    const resolver = redactedAuthResolverRef.current;
    redactedAuthResolverRef.current = null;
    resolver?.(allowed);
  };

  const openRedactedAuthModal = async ({
    mode,
    title,
    subtitle,
    submitLabel,
  }: {
    mode: "verify" | "setup";
    title: string;
    subtitle: string;
    submitLabel: string;
  }): Promise<boolean> => {
    setRedactedAuthMode(mode);
    setRedactedAuthTitle(title);
    setRedactedAuthSubtitle(subtitle);
    setRedactedAuthSubmitLabel(submitLabel);
    setRedactedAuthPasswordInput("");
    setRedactedAuthError("");
    setRedactedAuthModalOpen(true);
    return new Promise((resolve) => {
      redactedAuthResolverRef.current = resolve;
    });
  };

  async function submitRedactedAuthModal() {
    if (!redactedAuthPasswordInput) {
      return;
    }
    setRedactedAuthError("");

    if (redactedAuthMode === "verify") {
      const verifyResult = await verifyMasterPassword(redactedAuthPasswordInput);
      if (verifyResult.error) {
        setRedactedAuthError(verifyResult.error.message);
        return;
      }
      if (!verifyResult.data) {
        setRedactedAuthError("Incorrect password");
        return;
      }
      setIsRedactedUnlocked?.(true);
      onSelectedVaultRequiresUnlockChange?.(false);
      closeRedactedAuthModal(true);
      return;
    }

    const setResult = await setMasterPassword(redactedAuthPasswordInput);
    if (setResult.error) {
      setRedactedAuthError(setResult.error.message);
      return;
    }
    if (!setResult.data) {
      setRedactedAuthError("Failed to set master password.");
      return;
    }
    setIsRedactedUnlocked?.(true);
    onSelectedVaultRequiresUnlockChange?.(false);
    closeRedactedAuthModal(true);
  }

  async function unlockRedactedFromSpatial(): Promise<boolean> {
    const setupResult = await isAuthSetup();
    if (setupResult.error) {
      setErrorMsg(setupResult.error.message);
      return false;
    }

    if (setupResult.data) {
      return openRedactedAuthModal({
        mode: "verify",
        title: "Redacted",
        subtitle: "Enter your master password to unlock this vault.",
        submitLabel: "Unlock",
      });
    }

    return openRedactedAuthModal({
      mode: "setup",
      title: "Redacted",
      subtitle: "Set a master password to unlock this vault.",
      submitLabel: "Set Master Password",
    });
  }

  async function handleOpenVaultRequest(vault: Vault) {
    const vaultEffectiveTier =
      vaultEffectivePrivacyById[vault.id] ?? getEffectivePrivacy(vault.privacyTier);
    const isRedactedVault = vaultEffectiveTier === "redacted";
    const requiresUnlock = isRedactedVault && !isRedactedUnlocked;

    if (requiresUnlock) {
      onSelectedVaultRequiresUnlockChange?.(true);
      onLeftPanePinChange?.(false);
      const unlocked = await unlockRedactedFromSpatial();
      if (!unlocked) {
        return;
      }
      onSelectVault(vault.id);
      return;
    }

    if (selectedVaultId === vault.id && isLeftPanePinned) {
      onSelectVault(null);
      return;
    }

    onSelectVault(vault.id);
  }

  const submitVaultCreateModal = async () => {
    const name = vaultCreateName.trim();
    if (!name) {
      setVaultCreateError("Enter a vault name.");
      return;
    }

    setIsAddingBusy(true);
    try {
      if (vaultCreateMode === "global-vault") {
        const created = await createVault({
          name,
          description: vaultCreateDescription.trim() || undefined,
          icon: vaultCreateIcon.trim() || undefined,
          privacyTier: vaultCreatePrivacyTier.trim() || undefined,
        });
        if (onVaultCreated) onVaultCreated(created.id);
      } else if (vaultCreateMode === "subvault" && vaultCreateParentId) {
        const created = await createVault({
          name,
          description: vaultCreateDescription.trim() || undefined,
          icon: vaultCreateIcon.trim() || undefined,
          privacyTier: vaultCreatePrivacyTier.trim() || undefined,
          parentVaultId: vaultCreateParentId,
        });
        if (onVaultUpdated) onVaultUpdated(vaultCreateParentId);
        if (onVaultCreated) onVaultCreated(created.id);
      }
      setLocalRefresh((prev) => prev + 1);
      closeVaultCreateModal();
    } catch (e) {
      setVaultCreateError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsAddingBusy(false);
    }
  };

  const handleAddKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      handleAddSubmit();
    } else if (e.key === "Escape") {
      setAddingType(null);
    }
  };

  // Safe two-click deletion confirmation
  const handleArmDelete = (e: React.MouseEvent, itemId: string) => {
    e.stopPropagation();
    if (deleteArmedId === itemId) {
      // Confirmed deletion!
      executeDelete(itemId);
      setDeleteArmedId(null);
      if (deleteArmedTimer) clearTimeout(deleteArmedTimer);
    } else {
      // First click: arm
      setDeleteArmedId(itemId);
      if (deleteArmedTimer) clearTimeout(deleteArmedTimer);
      const timer = setTimeout(() => {
        setDeleteArmedId(null);
      }, 3500); // Reset arm state after 3.5 seconds
      setDeleteArmedTimer(timer);
    }
  };

  const executeDelete = async (itemId: string) => {
    try {
      // Check if it's a node
      const isNode = nodes.some((n) => n.id === itemId);
      if (isNode) {
        const deleted = await deleteNode(itemId);
        if (deleted && onNodeDeleted) onNodeDeleted(itemId);
      } else {
        const deleted = await deleteVault(itemId);
        if (deleted && onVaultDeleted) onVaultDeleted(itemId);
      }
      setLocalRefresh((prev) => prev + 1);
    } catch (e) {
      console.error("Failed to delete item:", e);
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  };

  // --- Search glowing & dimming evaluations ---
  const query = searchQuery.toLowerCase().trim();

  // Evaluates matching metrics for nodes
  const isNodeMatch = (n: Node) => {
    if (!query) return false;
    const sv = n.subVaultId ? vaults.find((v) => v.id === n.subVaultId) : null;
    const v = vaults.find((vault) => vault.id === n.vaultId);
    const effectiveTier = getEffectivePrivacy(n.privacyTier, sv?.privacyTier, v?.privacyTier);
    const isRedacted = effectiveTier === "redacted" && !isRedactedUnlocked;
    const isLocked = effectiveTier === "locked" && !isRedactedUnlocked;
    const title = n.title.toLowerCase();
    if (isRedacted || isLocked) {
      return title.includes(query);
    }
    const summary = n.summary.toLowerCase();
    return title.includes(query) || summary.includes(query);
  };

  // Evaluates matching metrics for subvaults
  const isSubvaultMatch = (sv: Vault) => {
    if (!query) return false;
    if (sv.name.toLowerCase().includes(query)) return true;
    const svNodes = nodes.filter((n) => n.subVaultId === sv.id);
    return svNodes.some(isNodeMatch);
  };

  // Evaluates matching metrics for vaults
  const isVaultMatch = (v: Vault) => {
    if (!query) return false;
    if (v.name.toLowerCase().includes(query)) return true;

    // Check direct nodes
    const directNodes = nodes.filter((n) => n.vaultId === v.id && !n.subVaultId);
    if (directNodes.some(isNodeMatch)) return true;

    // Check child subvaults
    const childSubvaults = vaults.filter((sv) => sv.parentVaultId === v.id);
    return childSubvaults.some(isSubvaultMatch);
  };

  const renderNodeBadges = (isPinned: boolean, isCtx: boolean, hasDoor: boolean) => {
    const badgeItems: Array<{ key: string; className: string; label: string; title: string }> = [];

    if (isPinned) {
      badgeItems.push({
        key: "pinned",
        className: "badge-pin",
        label: "Pinned",
        title: "Pinned for quick access",
      });
    }
    if (isCtx && !isPinned) {
      badgeItems.push({
        key: "context",
        className: "badge-ctx",
        label: "Context",
        title: "Part of the active context",
      });
    }
    if (hasDoor) {
      badgeItems.push({
        key: "linked",
        className: "badge-door",
        label: "Linked",
        title: "Linked to another node",
      });
    }

    return (
      <>
        {badgeItems.map((badge) => (
          <span
            key={badge.key}
            className={`spatial-node-badge spatial-node-badge-icon-only ${badge.className}`}
            title={badge.title}
            aria-label={badge.label}
          >
            <span className="spatial-node-badge-icon" aria-hidden="true">
              {badge.key === "pinned" ? "•" : badge.key === "context" ? "◌" : "↗"}
            </span>
          </span>
        ))}
      </>
    );
  };

  const renderNodeBadgeIcons = (isPinned: boolean, isCtx: boolean, hasDoor: boolean) => {
    const badgeItems: Array<{ key: string; className: string; label: string; title: string }> = [];

    if (isPinned) {
      badgeItems.push({
        key: "pinned",
        className: "badge-pin",
        label: "Pinned",
        title: "Pinned for quick access",
      });
    }
    if (isCtx && !isPinned) {
      badgeItems.push({
        key: "context",
        className: "badge-ctx",
        label: "Context",
        title: "Part of the active context",
      });
    }
    if (hasDoor) {
      badgeItems.push({
        key: "linked",
        className: "badge-door",
        label: "Linked",
        title: "Linked to another node",
      });
    }

    return (
      <span className="spatial-node-badge-group">
        {badgeItems.map((badge) => (
          <span
            key={badge.key}
            className={`spatial-node-badge spatial-node-badge-icon-only ${badge.className}`}
            title={badge.title}
            aria-label={badge.label}
          >
            <span className="spatial-node-badge-icon" aria-hidden="true">
              {badge.key === "pinned" ? "•" : badge.key === "context" ? "◌" : "↗"}
            </span>
          </span>
        ))}
      </span>
    );
  };
  void renderNodeBadges;
  void renderNodeBadgeIcons;

  // Filter top level vaults to render
  const topVaults = useMemo(() => {
    return vaults.filter((v) => !v.parentVaultId);
  }, [vaults]);

  const vaultById: Record<string, Vault> = {};
  for (const vault of vaults) {
    vaultById[vault.id] = vault;
  }

  const vaultEffectivePrivacyById: Record<string, string> = {};
  for (const vault of vaults) {
    vaultEffectivePrivacyById[vault.id] = getVaultEffectivePrivacy(
      vault.id,
      vaultById,
      vaultEffectivePrivacyById
    );
  }

  const selectedVaultRequiresUnlock = Boolean(
    selectedVaultId &&
    (() => {
      const selectedVault = vaultById[selectedVaultId];
      if (!selectedVault) {
        return false;
      }
      const selectedVaultTier =
        vaultEffectivePrivacyById[selectedVault.id] ??
        getEffectivePrivacy(selectedVault.privacyTier);
      return selectedVaultTier === "redacted" && !isRedactedUnlocked;
    })()
  );

  const focusedVaultLabel = selectedVaultId
    ? (() => {
        const vault = vaultById[selectedVaultId];
        if (!vault) return "";
        const effectiveTier =
          vaultEffectivePrivacyById[selectedVaultId] ?? getEffectivePrivacy(vault.privacyTier);
        return getPrivacyDisplayLabel(vault.name, effectiveTier, isRedactedUnlocked);
      })()
    : "";

  useEffect(() => {
    onSelectedVaultRequiresUnlockChange?.(selectedVaultRequiresUnlock);
  }, [onSelectedVaultRequiresUnlockChange, selectedVaultRequiresUnlock]);

  // Compute subvault list for each top-level vault
  const getSubvaultsFor = (vaultId: string) => {
    return vaults.filter((v) => v.parentVaultId === vaultId);
  };

  // Compute nodes list for a specific vault and optional subvault
  const getNodesFor = (vaultId: string, subvaultId: string | null) => {
    return nodes.filter(
      (n) => n.vaultId === vaultId && (subvaultId ? n.subVaultId === subvaultId : !n.subVaultId)
    );
  };

  // --- GPU-Accelerated World Coordinate SVG Connectors ---
  // Calculates center coordinate positions of elements to draw connection curves mathematically
  const connectionPaths = (() => {
    if (loading || topVaults.length === 0) return [];

    const paths: Array<{
      id: string;
      d: string;
      label: string;
      isLocked: boolean;
      active: boolean;
      middlePoint: { x: number; y: number };
      markerId: string;
      badgeWidth: number;
      badgeX: number;
    }> = [];

    // Helper to calculate card relative coordinate offset mathematically
    const getNodeCenterPos = (
      vaultId: string,
      subvaultId: string | null,
      nodeId: string | null
    ) => {
      const cardPos = cardPositions[vaultId];
      if (!cardPos) return null;

      // Card layout constants matching SpatialWorkspace.css padding and margins
      const cardWidth = 280;
      const paddingTop = 14;
      const headerHeight = 40;
      const rowHeight = 36;
      const containerGap = 10;
      const subvaultHeaderHeight = 28;

      const vaultNodes = nodes.filter((n) => n.vaultId === vaultId && !n.subVaultId);
      const subVaultsList = vaults.filter((sv) => sv.parentVaultId === vaultId);

      if (!nodeId) {
        // Target is the Vault header itself
        return {
          left: { x: cardPos.x, y: cardPos.y + paddingTop + headerHeight / 2 },
          right: { x: cardPos.x + cardWidth, y: cardPos.y + paddingTop + headerHeight / 2 },
        };
      }

      // Check if it's a direct vault node
      const directIndex = vaultNodes.findIndex((n) => n.id === nodeId);
      if (directIndex !== -1) {
        const offset = paddingTop + headerHeight + directIndex * rowHeight + rowHeight / 2;
        return {
          left: { x: cardPos.x, y: cardPos.y + offset },
          right: { x: cardPos.x + cardWidth, y: cardPos.y + offset },
        };
      }

      // Scan subvault offsets
      let currentOffset = paddingTop + headerHeight + vaultNodes.length * rowHeight;
      for (const sv of subVaultsList) {
        currentOffset += containerGap; // Margin inside subvault card
        const svNodes = nodes.filter((n) => n.subVaultId === sv.id);

        if (sv.id === subvaultId) {
          const svNodeIndex = svNodes.findIndex((n) => n.id === nodeId);
          if (svNodeIndex !== -1) {
            const offset =
              currentOffset + subvaultHeaderHeight + 6 + svNodeIndex * rowHeight + rowHeight / 2;
            return {
              left: { x: cardPos.x + 8, y: cardPos.y + offset },
              right: { x: cardPos.x + cardWidth - 8, y: cardPos.y + offset },
            };
          }
        }

        // Add previous subvault's total height to current offset
        currentOffset += 8 + subvaultHeaderHeight + 6 + svNodes.length * rowHeight + 34 + 8;
      }

      // Fallback
      return {
        left: { x: cardPos.x, y: cardPos.y + paddingTop + headerHeight / 2 },
        right: { x: cardPos.x + cardWidth, y: cardPos.y + paddingTop + headerHeight / 2 },
      };
    };

    doors.forEach((door) => {
      // Source node
      const srcNode = nodes.find((n) => n.id === door.sourceNodeId);
      if (!srcNode) return;

      const srcContainerId = srcNode.subVaultId ?? srcNode.vaultId;
      const srcContainerTier =
        vaultEffectivePrivacyById[srcContainerId] ?? vaultById[srcContainerId]?.privacyTier;
      const srcEffectiveTier = getEffectivePrivacy(srcNode.privacyTier, null, srcContainerTier);
      if (shouldOmitSpatialConnector(srcEffectiveTier, isRedactedUnlocked)) return;

      const srcPosObj = getNodeCenterPos(srcNode.vaultId, srcNode.subVaultId, srcNode.id);
      if (!srcPosObj) return;

      // Target node or target vault
      let tgtX = 0;
      let tgtY = 0;
      let tgtEffectiveTier = "open";

      // Determine horizontal relationship to choose optimal anchors (left/right)
      const srcCardPos = cardPositions[srcNode.vaultId];
      let tgtCardPos = null;
      if (door.targetNodeId) {
        const tgtNode = nodes.find((n) => n.id === door.targetNodeId);
        if (tgtNode) {
          tgtCardPos = cardPositions[tgtNode.vaultId];
          const tgtContainerId = tgtNode.subVaultId ?? tgtNode.vaultId;
          const tgtContainerTier =
            vaultEffectivePrivacyById[tgtContainerId] ?? vaultById[tgtContainerId]?.privacyTier;
          tgtEffectiveTier = getEffectivePrivacy(tgtNode.privacyTier, null, tgtContainerTier);
        }
      } else if (door.targetVaultId) {
        const tgtVault = vaults.find((v) => v.id === door.targetVaultId);
        if (tgtVault) {
          tgtCardPos = cardPositions[tgtVault.id];
          tgtEffectiveTier =
            vaultEffectivePrivacyById[tgtVault.id] ?? getEffectivePrivacy(tgtVault.privacyTier);
        }
      }

      if (shouldOmitSpatialConnector(tgtEffectiveTier, isRedactedUnlocked)) return;

      // Determine horizontal direction of routing
      let routeDirection: "left-to-right" | "right-to-left" = "left-to-right";
      if (srcCardPos && tgtCardPos) {
        if (srcCardPos.x + 140 < tgtCardPos.x + 140) {
          routeDirection = "left-to-right";
        } else {
          routeDirection = "right-to-left";
        }
      }

      // Select anchors based on direction
      let srcX = 0;
      let srcY = 0;
      if (routeDirection === "left-to-right") {
        srcX = srcPosObj.right.x;
        srcY = srcPosObj.right.y;

        if (door.targetNodeId) {
          const tgtNode = nodes.find((n) => n.id === door.targetNodeId);
          if (tgtNode) {
            const tgtPosObj = getNodeCenterPos(tgtNode.vaultId, tgtNode.subVaultId, tgtNode.id);
            if (tgtPosObj) {
              tgtX = tgtPosObj.left.x;
              tgtY = tgtPosObj.left.y;
            }
          }
        } else if (door.targetVaultId) {
          const tgtPosObj = getNodeCenterPos(door.targetVaultId, null, null);
          if (tgtPosObj) {
            tgtX = tgtPosObj.left.x;
            tgtY = tgtPosObj.left.y;
          }
        }
      } else {
        srcX = srcPosObj.left.x;
        srcY = srcPosObj.left.y;

        if (door.targetNodeId) {
          const tgtNode = nodes.find((n) => n.id === door.targetNodeId);
          if (tgtNode) {
            const tgtPosObj = getNodeCenterPos(tgtNode.vaultId, tgtNode.subVaultId, tgtNode.id);
            if (tgtPosObj) {
              tgtX = tgtPosObj.right.x;
              tgtY = tgtPosObj.right.y;
            }
          }
        } else if (door.targetVaultId) {
          const tgtPosObj = getNodeCenterPos(door.targetVaultId, null, null);
          if (tgtPosObj) {
            tgtX = tgtPosObj.right.x;
            tgtY = tgtPosObj.right.y;
          }
        }
      }

      // If we couldn't resolve targets, skip
      if (tgtX === 0 && tgtY === 0) return;

      // SVG pathway drawing cubic bezier control anchors
      const controlDistance = Math.max(80, Math.abs(tgtX - srcX) / 2);
      const ctrl1X =
        routeDirection === "left-to-right" ? srcX + controlDistance : srcX - controlDistance;
      const ctrl2X =
        routeDirection === "left-to-right" ? tgtX - controlDistance : tgtX + controlDistance;
      const d = `M ${srcX} ${srcY} C ${ctrl1X} ${srcY}, ${ctrl2X} ${tgtY}, ${tgtX} ${tgtY}`;

      // Middle curve calculation for dynamic label tags
      const midX = (srcX + tgtX) / 2;
      const midY = (srcY + tgtY) / 2;

      // Evaluate visual active indicator
      const active = selectedNodeId === srcNode.id || selectedNodeId === door.targetNodeId;
      const isLocked =
        (srcEffectiveTier === "locked" || tgtEffectiveTier === "locked") && !isRedactedUnlocked;
      const normalizedLabel = (door.label || "").trim();
      const visibleLabel =
        normalizedLabel && normalizedLabel.toLowerCase() !== "wikilink" ? normalizedLabel : "";

      // Calculate dynamic label and badge metrics
      const labelText = isLocked ? "🔒 Locked" : sanitizeSvgText(visibleLabel);
      const badgeWidth = Math.max(80, labelText.length * 6.5 + 16);
      const badgeX = -badgeWidth / 2;

      // Calculate matching arrowhead marker ID
      const markerId = active ? "arrow-active" : isLocked ? "arrow-locked" : "arrow-default";

      paths.push({
        id: door.id,
        d,
        label: visibleLabel,
        isLocked,
        active,
        middlePoint: { x: midX, y: midY },
        markerId,
        badgeWidth,
        badgeX,
      });
    });

    return paths;
  })();

  if (loading) {
    return (
      <div
        className="spatial-canvas-container"
        style={{ display: "flex", alignItems: "center", justifyContent: "center" }}
      >
        <p style={{ color: "#b56a37", fontWeight: "bold" }}>Loading Spatial Workspace...</p>
      </div>
    );
  }

  return (
    <div
      className="spatial-canvas-container"
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onWheel={handleWheel}
    >
      {/* Top Glassmorphic bar controls */}
      {focusedVaultLabel && (
        <div className="spatial-header-bar">
          <span className="spatial-header-title">{focusedVaultLabel}</span>
          {selectedVaultId && onLeftPanePinChange && (
            <button
              type="button"
              className={`spatial-header-lock-btn ${isLeftPanePinned ? "active" : ""}`}
              onClick={(e) => {
                e.stopPropagation();
                if (selectedVaultRequiresUnlock) {
                  return;
                }
                onLeftPanePinChange(!isLeftPanePinned);
              }}
              title={
                selectedVaultRequiresUnlock
                  ? "Unlock redacted vault first"
                  : isLeftPanePinned
                    ? "Unpin active vault panel"
                    : "Pin active vault panel"
              }
              aria-pressed={isLeftPanePinned}
              disabled={selectedVaultRequiresUnlock}
            >
              {isLeftPanePinned ? "📌" : "📍"}
            </button>
          )}
        </div>
      )}

      {/* Primary Transform World Container */}
      <div
        ref={worldRef}
        className="spatial-canvas-world"
        style={{
          transform: `translate3d(${pan.x}px, ${pan.y}px, 0) scale3d(${zoom}, ${zoom}, 1)`,
        }}
      >
        {/* SVG Pathways Connector Overlay */}
        <svg className="spatial-svg-overlay">
          <defs>
            <marker
              id="arrow-default"
              viewBox="0 0 10 10"
              refX="8"
              refY="5"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" fill="rgba(188, 108, 37, 0.6)" />
            </marker>
            <marker
              id="arrow-active"
              viewBox="0 0 10 10"
              refX="8"
              refY="5"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" fill="#b56a37" />
            </marker>
            <marker
              id="arrow-locked"
              viewBox="0 0 10 10"
              refX="8"
              refY="5"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" fill="#a3a09a" />
            </marker>
          </defs>
          {connectionPaths.map((path) => (
            <g key={path.id}>
              {/* Backing pathway path with dynamic arrowhead */}
              <path
                className={`spatial-connection-line ${path.active ? "connection-active" : ""} ${path.isLocked ? "connection-locked" : ""}`}
                d={path.d}
                markerEnd={`url(#${path.markerId})`}
              />
              {/* Sanitized Dynamic labels overlays */}
              {path.label && (
                <g
                  className="spatial-connection-badge"
                  transform={`translate(${path.middlePoint.x}, ${path.middlePoint.y})`}
                >
                  <rect
                    className="spatial-badge-bg"
                    x={path.badgeX}
                    y={-10}
                    width={path.badgeWidth}
                    height={20}
                  />
                  <text
                    className={`spatial-badge-text ${path.isLocked ? "spatial-badge-text-locked" : ""}`}
                    y={1}
                  >
                    {path.isLocked ? "🔒 Locked" : sanitizeSvgText(path.label)}
                  </text>
                </g>
              )}
              {/* Locked/Redacted badge indicator overlay */}
              {!path.label && path.isLocked && (
                <g
                  className="spatial-connection-badge"
                  transform={`translate(${path.middlePoint.x}, ${path.middlePoint.y})`}
                >
                  <circle cx={0} cy={0} r={10} fill="white" stroke="#a3a09a" strokeWidth={1} />
                  <text y={2.5} style={{ fontSize: "9px", textAnchor: "middle" }}>
                    🔒
                  </text>
                </g>
              )}
            </g>
          ))}
        </svg>

        {/* Absolute positioned Vault Cards grid list */}
        {topVaults.map((vault) => {
          const pos = cardPositions[vault.id] || { x: 100, y: 100 };
          const subvaultsList = getSubvaultsFor(vault.id);
          const directNodesList = getNodesFor(vault.id, null);
          const vaultEffectiveTier =
            vaultEffectivePrivacyById[vault.id] ?? getEffectivePrivacy(vault.privacyTier);

          // Evaluate search matches
          const hasQuery = query.length > 0;
          const isVMatch = isVaultMatch(vault);
          const shouldDim = hasQuery && !isVMatch;
          const isVaultRedactedTier = vaultEffectiveTier === "redacted";
          const isVaultLockedTier = vaultEffectiveTier === "locked";
          const isVaultRedactedLocked = isVaultRedactedTier && !isRedactedUnlocked;
          const isVaultContentLocked = isVaultLockedTier && !isRedactedUnlocked;
          const isVaultEditable =
            vaultEffectiveTier === "open" ||
            vaultEffectiveTier === "local_only" ||
            isRedactedUnlocked;
          const canEditVaultColor = vaultEffectiveTier !== "redacted";

          // Calculate total count of items in this vault card
          const totalItemCount =
            directNodesList.length +
            subvaultsList.reduce((acc, sv) => acc + getNodesFor(vault.id, sv.id).length, 0);

          // Get domain curated theme
          const theme = getVaultTheme(vault.name);

          let colorTheme = "";
          try {
            if (vault.uiMetadata) {
              const parsed = JSON.parse(vault.uiMetadata);
              colorTheme = parsed.colorTheme || "";
            }
          } catch (e) {
            console.error("Error parsing uiMetadata for theme:", vault.id, e);
          }

          // Card css class mappings
          let cardClasses = `spatial-vault-card`;
          if (colorTheme) {
            cardClasses += ` theme-color-${colorTheme}`;
          } else {
            cardClasses += ` ${theme.class}`;
          }
          if (selectedVaultId === vault.id) cardClasses += " active-selection";
          if (shouldDim) cardClasses += " spatial-dimmed";
          if (hasQuery && isVMatch) cardClasses += " spatial-glow";
          if (isVaultRedactedTier) cardClasses += " spatial-redacted-card";
          if (isVaultLockedTier) cardClasses += " spatial-locked-card";
          if (isVaultRedactedLocked) cardClasses += " spatial-redacted-card-locked";

          return (
            <div
              key={vault.id}
              className={cardClasses}
              data-vault-id={vault.id}
              style={{
                left: `${pos.x}px`,
                top: `${pos.y}px`,
                zIndex: selectedVaultId === vault.id ? 5 : 2,
              }}
              onMouseDown={(e) => handleCardMouseDown(e, vault.id, pos)}
            >
              {/* Vault card header */}
              <div className="spatial-card-header">
                <div className="spatial-card-title-area" onClick={() => onFocusVault?.(vault.id)}>
                  <span className="spatial-card-emoji">
                    {isVaultRedactedLocked ? "⬛" : getVaultEmoji(vault.icon, vault.name)}
                  </span>
                  {editingItemId === vault.id ? (
                    <input
                      className="spatial-card-name-input"
                      autoFocus
                      value={editValue}
                      onChange={(e) => setEditValue(e.target.value)}
                      onBlur={() => handleRenameSave(vault.id, "vault")}
                      onKeyDown={(e) => handleRenameKeyDown(e, vault.id, "vault")}
                    />
                  ) : (
                    <span
                      className="spatial-card-name"
                      onDoubleClick={() => {
                        if (!isVaultEditable) return;
                        setEditingItemId(vault.id);
                        setEditingItemType("vault");
                        setEditValue(vault.name);
                      }}
                    >
                      {getPrivacyDisplayLabel(vault.name, vaultEffectiveTier, isRedactedUnlocked)}
                    </span>
                  )}
                </div>

                <div className="spatial-card-header-right">
                  {isVaultRedactedLocked && (
                    <span className="spatial-card-lock-badge">redacted</span>
                  )}
                  {isVaultContentLocked && <span className="spatial-card-lock-badge">locked</span>}
                  <span className="spatial-card-count-badge">{totalItemCount}</span>
                </div>

                <div className="spatial-card-actions">
                  {isVaultEditable && (
                    <button
                      className="spatial-card-action-btn"
                      onClick={() => {
                        setEditingItemId(vault.id);
                        setEditingItemType("vault");
                        setEditValue(vault.name);
                      }}
                    >
                      ✏️
                    </button>
                  )}

                  <button
                    className={`spatial-card-action-btn ${deleteArmedId === vault.id ? "delete-armed" : ""}`}
                    onClick={(e) => handleArmDelete(e, vault.id)}
                    title="Click twice to delete vault card"
                  >
                    {deleteArmedId === vault.id ? "Confirm?" : "🗑️"}
                  </button>
                </div>
              </div>

              {editingItemId === vault.id && editingItemType === "vault" && canEditVaultColor && (
                <div
                  className="spatial-card-color-picker"
                  onMouseDown={(e) => {
                    e.stopPropagation();
                    e.preventDefault();
                  }}
                >
                  {CURATED_PALETTE.filter((color) => color.id !== "rose").map((color) => (
                    <button
                      key={color.id}
                      className={`spatial-color-dot ${colorTheme === color.id ? "active" : ""}`}
                      style={{ backgroundColor: color.hsl }}
                      type="button"
                      onMouseDown={(e) => {
                        e.stopPropagation();
                        e.preventDefault();
                      }}
                      onClick={async (e) => {
                        e.stopPropagation();
                        const saved = await updateVaultColorTheme(vault.id, color.id);
                        if (!saved) {
                          setErrorMsg("Failed to update vault color.");
                          return;
                        }
                        setLocalRefresh((prev) => prev + 1);
                      }}
                      title={color.name}
                    />
                  ))}
                </div>
              )}

              {/* Direct Node Rows list */}
              <ul
                className={`spatial-node-list ${isVaultRedactedLocked ? "spatial-redacted-content-locked" : ""}`}
              >
                {directNodesList.map((node) => {
                  const nodeMatch = isNodeMatch(node);
                  const nodeEffectiveTier = getEffectivePrivacy(
                    node.privacyTier,
                    null,
                    vaultEffectiveTier
                  );
                  const isRedacted = nodeEffectiveTier === "redacted" && !isRedactedUnlocked;
                  const isNodeLocked = nodeEffectiveTier === "locked" && !isRedactedUnlocked;
                  const isNodeEditable =
                    nodeEffectiveTier === "open" ||
                    nodeEffectiveTier === "local_only" ||
                    isRedactedUnlocked;
                  const isNSelected = selectedNodeId === node.id;

                  let nodeClasses = "spatial-node-row";
                  if (isNSelected) nodeClasses += " selected-node";
                  if (hasQuery && nodeMatch) nodeClasses += " spatial-glow-node";
                  if (isRedacted) nodeClasses += " spatial-redacted-node-row";
                  if (isNodeLocked) nodeClasses += " spatial-locked-node-row";

                  return (
                    <li
                      key={node.id}
                      className={nodeClasses}
                      onClick={() => !isRedacted && onSelectNode(node.id)}
                    >
                      <div className="spatial-node-title-area">
                        <span className="spatial-node-dot" />
                        {editingItemId === node.id ? (
                          <input
                            className="spatial-node-title-input"
                            autoFocus
                            value={editValue}
                            onChange={(e) => setEditValue(e.target.value)}
                            onBlur={() => handleRenameSave(node.id, "node")}
                            onKeyDown={(e) => handleRenameKeyDown(e, node.id, "node")}
                          />
                        ) : (
                          <span
                            className={`spatial-node-title ${isRedacted ? "spatial-redacted-label" : ""}`}
                            onDoubleClick={() => {
                              if (!isNodeEditable) return;
                              setEditingItemId(node.id);
                              setEditingItemType("node");
                              setEditValue(node.title);
                            }}
                          >
                            {getPrivacyDisplayLabel(
                              node.title,
                              nodeEffectiveTier,
                              isRedactedUnlocked
                            )}
                          </span>
                        )}
                      </div>

                      <div className="spatial-node-right-area">
                        {isNodeLocked && (
                          <span style={{ fontSize: "0.75rem", marginRight: "4px" }}>🔒</span>
                        )}
                      </div>

                      <div className="spatial-card-actions">
                        {isNodeEditable && (
                          <button
                            className="spatial-card-action-btn"
                            onClick={(e) => {
                              e.stopPropagation();
                              setEditingItemId(node.id);
                              setEditingItemType("node");
                              setEditValue(node.title);
                            }}
                          >
                            ✏️
                          </button>
                        )}
                        {isNodeEditable && (
                          <button
                            className={`spatial-card-action-btn ${deleteArmedId === node.id ? "delete-armed" : ""}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleArmDelete(e, node.id);
                            }}
                          >
                            {deleteArmedId === node.id ? "Ok?" : "🗑️"}
                          </button>
                        )}
                      </div>
                    </li>
                  );
                })}
              </ul>

              {/* Subvault Nesting Groups inside Card */}
              {subvaultsList.map((subvault) => {
                const subNodes = getNodesFor(vault.id, subvault.id);
                const subvaultEffectiveTier =
                  vaultEffectivePrivacyById[subvault.id] ??
                  getEffectivePrivacy(subvault.privacyTier, null, vaultEffectiveTier);
                const subvaultRedactedLocked =
                  subvaultEffectiveTier === "redacted" && !isRedactedUnlocked;
                const subvaultContentLocked =
                  subvaultEffectiveTier === "locked" && !isRedactedUnlocked;
                const subvaultEditable =
                  subvaultEffectiveTier === "open" ||
                  subvaultEffectiveTier === "local_only" ||
                  isRedactedUnlocked;

                return (
                  <div
                    key={subvault.id}
                    className={`spatial-subvault-container ${
                      isVaultRedactedLocked ? "spatial-redacted-content-locked" : ""
                    }`}
                  >
                    <div className="spatial-subvault-header">
                      <div
                        className="spatial-subvault-title-area"
                        onClick={() => onFocusVault?.(subvault.id)}
                      >
                        {editingItemId === subvault.id ? (
                          <input
                            className="spatial-subvault-title-input"
                            autoFocus
                            value={editValue}
                            onChange={(e) => setEditValue(e.target.value)}
                            onBlur={() => handleRenameSave(subvault.id, "subvault")}
                            onKeyDown={(e) => handleRenameKeyDown(e, subvault.id, "subvault")}
                          />
                        ) : (
                          <span>
                            {subvaultRedactedLocked
                              ? "⬛ [REDACTED]"
                              : `${getVaultEmoji(subvault.icon, subvault.name)} ${getPrivacyDisplayLabel(
                                  subvault.name,
                                  subvaultEffectiveTier,
                                  isRedactedUnlocked
                                )}`}
                          </span>
                        )}
                      </div>

                      <div className="spatial-card-actions">
                        {subvaultRedactedLocked && <span>⬛</span>}
                        {subvaultContentLocked && <span>🔒</span>}
                        {subvaultEditable && (
                          <button
                            className="spatial-card-action-btn"
                            onClick={(e) => {
                              e.stopPropagation();
                              setEditingItemId(subvault.id);
                              setEditingItemType("subvault");
                              setEditValue(subvault.name);
                            }}
                          >
                            ✏️
                          </button>
                        )}
                        {subvaultEditable && (
                          <button
                            className={`spatial-card-action-btn ${deleteArmedId === subvault.id ? "delete-armed" : ""}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleArmDelete(e, subvault.id);
                            }}
                          >
                            {deleteArmedId === subvault.id ? "Ok?" : "🗑️"}
                          </button>
                        )}
                      </div>
                    </div>
                    {/* Nodes under Subvault */}
                    <ul
                      className={`spatial-node-list ${
                        subvaultRedactedLocked ? "spatial-redacted-content-locked" : ""
                      }`}
                    >
                      {subNodes.map((node) => {
                        const nodeMatch = isNodeMatch(node);
                        const nodeEffectiveTier = getEffectivePrivacy(
                          node.privacyTier,
                          null,
                          subvaultEffectiveTier
                        );
                        const isRedacted = nodeEffectiveTier === "redacted" && !isRedactedUnlocked;
                        const isNodeLocked = nodeEffectiveTier === "locked" && !isRedactedUnlocked;
                        const isNodeEditable =
                          nodeEffectiveTier === "open" ||
                          nodeEffectiveTier === "local_only" ||
                          isRedactedUnlocked;
                        const isNSelected = selectedNodeId === node.id;

                        let nodeClasses = "spatial-node-row";
                        if (isNSelected) nodeClasses += " selected-node";
                        if (hasQuery && nodeMatch) nodeClasses += " spatial-glow-node";
                        if (isRedacted) nodeClasses += " spatial-redacted-node-row";
                        if (isNodeLocked) nodeClasses += " spatial-locked-node-row";

                        return (
                          <li
                            key={node.id}
                            className={nodeClasses}
                            onClick={() => !isRedacted && onSelectNode(node.id)}
                          >
                            <div className="spatial-node-title-area">
                              <span className="spatial-node-dot" />
                              {editingItemId === node.id ? (
                                <input
                                  className="spatial-node-title-input"
                                  autoFocus
                                  value={editValue}
                                  onChange={(e) => setEditValue(e.target.value)}
                                  onBlur={() => handleRenameSave(node.id, "node")}
                                  onKeyDown={(e) => handleRenameKeyDown(e, node.id, "node")}
                                />
                              ) : (
                                <span
                                  className={`spatial-node-title ${isRedacted ? "spatial-redacted-label" : ""}`}
                                  onDoubleClick={() => {
                                    if (!isNodeEditable) return;
                                    setEditingItemId(node.id);
                                    setEditingItemType("node");
                                    setEditValue(node.title);
                                  }}
                                >
                                  {getPrivacyDisplayLabel(
                                    node.title,
                                    nodeEffectiveTier,
                                    isRedactedUnlocked
                                  )}
                                </span>
                              )}
                            </div>

                            <div className="spatial-node-right-area">
                              {isNodeLocked && (
                                <span style={{ fontSize: "0.75rem", marginRight: "4px" }}>🔒</span>
                              )}
                            </div>

                            <div className="spatial-card-actions">
                              {isNodeEditable && (
                                <button
                                  className="spatial-card-action-btn"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    setEditingItemId(node.id);
                                    setEditingItemType("node");
                                    setEditValue(node.title);
                                  }}
                                >
                                  ✏️
                                </button>
                              )}
                              {isNodeEditable && (
                                <button
                                  className={`spatial-card-action-btn ${deleteArmedId === node.id ? "delete-armed" : ""}`}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    handleArmDelete(e, node.id);
                                  }}
                                >
                                  {deleteArmedId === node.id ? "Ok?" : "🗑️"}
                                </button>
                              )}
                            </div>
                          </li>
                        );
                      })}
                    </ul>

                    {/* Inline Node creation inside Subvault */}
                    {subvaultEditable && (
                      <div style={{ marginTop: "4px" }}>
                        {addingType === "node" && addingToSubvaultId === subvault.id ? (
                          <div className="spatial-add-input-wrapper">
                            <input
                              className="spatial-add-inline-input"
                              autoFocus
                              placeholder="Node title..."
                              value={addInputValue}
                              onChange={(e) => setAddInputValue(e.target.value)}
                              onKeyDown={handleAddKeyDown}
                              disabled={isAddingBusy}
                            />
                            <button
                              className="spatial-add-confirm-btn"
                              onClick={handleAddSubmit}
                              disabled={isAddingBusy}
                            >
                              {isAddingBusy ? "..." : "Add"}
                            </button>
                            <button
                              className="spatial-add-cancel-btn"
                              onClick={() => setAddingType(null)}
                              disabled={isAddingBusy}
                            >
                              x
                            </button>
                          </div>
                        ) : (
                          <button
                            className="spatial-add-inline-btn"
                            onClick={() => {
                              setAddingType("node");
                              setAddingToVaultId(vault.id);
                              setAddingToSubvaultId(subvault.id);
                              setAddInputValue("");
                            }}
                          >
                            + node
                          </button>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}

              {/* Bottom Actions Row containing '+ node' / '+ subvault' / 'open vault' */}
              <div className="spatial-card-bottom-actions">
                {isVaultEditable && (
                  <div style={{ display: "flex", gap: "8px", flex: 1 }}>
                    {addingType === "node" &&
                    addingToVaultId === vault.id &&
                    !addingToSubvaultId ? (
                      <div className="spatial-add-input-wrapper">
                        <input
                          className="spatial-add-inline-input"
                          autoFocus
                          placeholder="Node title..."
                          value={addInputValue}
                          onChange={(e) => setAddInputValue(e.target.value)}
                          onKeyDown={handleAddKeyDown}
                          disabled={isAddingBusy}
                        />
                        <button
                          className="spatial-add-confirm-btn"
                          onClick={handleAddSubmit}
                          disabled={isAddingBusy}
                        >
                          {isAddingBusy ? "..." : "Add"}
                        </button>
                        <button
                          className="spatial-add-cancel-btn"
                          onClick={() => setAddingType(null)}
                          disabled={isAddingBusy}
                        >
                          x
                        </button>
                      </div>
                    ) : addingType === "subvault" && addingToVaultId === vault.id ? (
                      <div className="spatial-add-input-wrapper">
                        <input
                          className="spatial-add-inline-input"
                          autoFocus
                          placeholder="Subvault name..."
                          value={addInputValue}
                          onChange={(e) => setAddInputValue(e.target.value)}
                          onKeyDown={handleAddKeyDown}
                          disabled={isAddingBusy}
                        />
                        <button
                          className="spatial-add-confirm-btn"
                          onClick={handleAddSubmit}
                          disabled={isAddingBusy}
                        >
                          {isAddingBusy ? "..." : "Add"}
                        </button>
                        <button
                          className="spatial-add-cancel-btn"
                          onClick={() => setAddingType(null)}
                          disabled={isAddingBusy}
                        >
                          x
                        </button>
                      </div>
                    ) : (
                      <>
                        <button
                          className="spatial-add-inline-btn"
                          style={{ flex: 1 }}
                          onClick={() => {
                            setAddingType("node");
                            setAddingToVaultId(vault.id);
                            setAddingToSubvaultId(null);
                            setAddInputValue("");
                          }}
                        >
                          + node
                        </button>
                        <button
                          className="spatial-add-inline-btn"
                          style={{ flex: 1 }}
                          onClick={() => {
                            openVaultCreateModal("subvault", vault.id, vault.name);
                          }}
                        >
                          + subvault
                        </button>
                      </>
                    )}
                  </div>
                )}

                <button
                  className="spatial-card-open-vault"
                  onClick={() => {
                    void handleOpenVaultRequest(vault);
                  }}
                >
                  {isVaultRedactedLocked
                    ? "unlock vault →"
                    : selectedVaultId === vault.id && isLeftPanePinned
                      ? "close vault ←"
                      : "open vault →"}
                </button>
              </div>
            </div>
          );
        })}
      </div>

      {vaultCreateModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={closeVaultCreateModal}>
            <div
              className="vault-settings-modal sidebar-auth-modal spatial-create-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="modal-title">
                {vaultCreateMode === "subvault" ? "New Subvault" : "New Vault"}
              </h3>
              <p className="modal-subtitle">
                {vaultCreateMode === "subvault" && vaultCreateParentName
                  ? `Create a new subvault inside ${vaultCreateParentName}.`
                  : "Create a new top-level vault in Spatial Workspace."}
              </p>
              <div className="settings-fields-grid">
                <label className="settings-field">
                  <span>{vaultCreateMode === "subvault" ? "Subvault Name" : "Vault Name"}</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={vaultCreateName}
                    onChange={(e) => setVaultCreateName(e.target.value)}
                    placeholder={vaultCreateMode === "subvault" ? "e.g. Research" : "e.g. Finance"}
                    autoFocus
                  />
                </label>

                <label className="settings-field">
                  <span>Description</span>
                  <input
                    type="text"
                    className="settings-input"
                    value={vaultCreateDescription}
                    onChange={(e) => setVaultCreateDescription(e.target.value)}
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
                          className={`emoji-choice-btn ${vaultCreateIcon === emoji ? "selected" : ""}`}
                          onClick={() => setVaultCreateIcon(emoji)}
                        >
                          {emoji}
                        </button>
                      ))}
                    </div>
                    <input
                      type="text"
                      value={vaultCreateIcon}
                      onChange={(e) => setVaultCreateIcon(e.target.value)}
                      placeholder="Or type a custom emoji/text"
                      maxLength={10}
                      className="settings-input custom-emoji-input"
                    />
                  </div>
                </div>

                <label className="settings-field">
                  <span>Privacy Tier</span>
                  <select
                    value={vaultCreatePrivacyTier}
                    onChange={(e) => setVaultCreatePrivacyTier(e.target.value)}
                    className="settings-select"
                  >
                    <option value="open">Open (No restriction)</option>
                    <option value="local_only">Local Only (Never cloud synced)</option>
                    <option value="locked">Locked (Requires unlock to access)</option>
                    <option value="redacted">Redacted (Hidden metadata/title)</option>
                  </select>
                </label>
              </div>
              {vaultCreateError && <p className="redacted-lock-error">{vaultCreateError}</p>}
              <div className="sidebar-auth-actions settings-modal-actions">
                <button
                  type="button"
                  className="redacted-lock-button sidebar-auth-cancel"
                  onClick={closeVaultCreateModal}
                  disabled={isAddingBusy}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button"
                  onClick={submitVaultCreateModal}
                  disabled={isAddingBusy}
                >
                  {isAddingBusy ? "Creating..." : "Create"}
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}

      {redactedAuthModalOpen &&
        createPortal(
          <div className="sidebar-auth-overlay" onClick={() => closeRedactedAuthModal(false)}>
            <div
              className="redacted-lock-screen sidebar-auth-modal spatial-redacted-auth-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="redacted-lock-icon">🔓</div>
              <h3 className="redacted-lock-title">{redactedAuthTitle}</h3>
              <p className="redacted-lock-copy">{redactedAuthSubtitle}</p>
              <input
                type="password"
                className="redacted-lock-input"
                placeholder={
                  redactedAuthMode === "setup" ? "Choose a master password" : "Master password"
                }
                value={redactedAuthPasswordInput}
                onChange={(e) => setRedactedAuthPasswordInput(e.target.value)}
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    void submitRedactedAuthModal();
                  } else if (e.key === "Escape") {
                    closeRedactedAuthModal(false);
                  }
                }}
              />
              {redactedAuthError && <p className="redacted-lock-error">{redactedAuthError}</p>}
              <div className="redacted-lock-actions">
                <button
                  type="button"
                  className="redacted-lock-button sidebar-auth-cancel"
                  onClick={() => closeRedactedAuthModal(false)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="redacted-lock-button"
                  onClick={() => void submitRedactedAuthModal()}
                >
                  {redactedAuthSubmitLabel}
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}

      {/* Exquisite premium bottom pill control bar matching Image 2 mockup */}
      <div className="spatial-bottom-bar-pill">
        <span className="spatial-pill-search-icon">🔍</span>
        <input
          className="spatial-pill-search-input"
          placeholder="Search nodes..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
        />
        <button
          className="spatial-pill-create-btn"
          onClick={() => {
            openVaultCreateModal("global-vault");
          }}
          title="Create a new top level vault card"
        >
          + Vault
        </button>
        <div className="spatial-pill-divider" />
        <button className="spatial-pill-reset-btn" onClick={resetView}>
          Reset
        </button>
        <div className="spatial-pill-divider" />
        <div className="spatial-pill-zoom">{Math.round(zoom * 100)}%</div>
      </div>

      {/* Glassmorphic Sliding Error Toast */}
      {errorMsg && (
        <div className="spatial-error-toast">
          <span className="spatial-error-toast-icon">⚠️</span>
          <span className="spatial-error-toast-text">{errorMsg}</span>
          <button className="spatial-error-toast-close" onClick={() => setErrorMsg(null)}>
            ✕
          </button>
        </div>
      )}
    </div>
  );
}
