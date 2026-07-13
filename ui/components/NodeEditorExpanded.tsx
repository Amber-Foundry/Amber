import React, { useState, useEffect, useRef, useMemo, useTransition, useCallback } from "react";
import ReactMarkdown from "react-markdown";
import {
  remarkPluginsStable,
  rehypePluginsStable,
  createMarkdownComponents,
  preprocessWikiLinks,
  getCaretCoordinates,
  isRawLatex,
  preprocessMathDelimiters,
  ExistingNodesContext,
} from "../utils/markdownUtils";
import LatexBlock from "./LatexBlock";
import {
  getNode,
  updateNode,
  getAllNodes,
  refreshAllPriorityScores,
  touchNode,
} from "../services/nodes";
import {
  listIncomingDoors,
  listOutgoingDoors,
  createDoor,
  deleteDoor,
  repointDoor,
} from "../services/doors";
import { addNodeTag, createTag, getNodeTags, listTags, removeNodeTag } from "../services/tags";
import { isAuthSetup, setMasterPassword, verifyMasterPassword } from "../services/auth";
import { listVaults } from "../services/vaults";
import {
  getEffectivePrivacy,
  getPrivacyDisplayLabel,
  getPrivacyDisplaySummary,
  getPrivacyRank,
  getVaultDisplayPath,
  getVaultEffectivePrivacy,
} from "../utils/privacy";
import type { Backlink, Door, Node, Tag, Vault } from "../ipc";
import { saveMarkdownFile } from "../ipc";
import { AppError } from "../services/ipcResult";
import { PrivacyBadge } from "./PrivacyBadge";
import PriorityBar from "./PriorityBar";
import NodeLinkAutocomplete from "./NodeLinkAutocomplete";
import { useUIStore } from "../utils/store";
import ImportDocumentSpine from "./ImportDocumentSpine";
import { isImportDocumentNode } from "../utils/importDocument";

type NodeEditorExpandedProps = {
  nodeId: string;
  onClose: () => void;
  chartsEnabled?: boolean;
  onSelectNode: (nodeId: string) => void;
  isRedactedUnlocked: boolean;
  setIsRedactedUnlocked: (value: boolean) => void;
};

export default function NodeEditorExpanded({
  nodeId,
  onClose,
  chartsEnabled: propChartsEnabled,
  onSelectNode,
  isRedactedUnlocked,
  setIsRedactedUnlocked,
}: NodeEditorExpandedProps) {
  const storeChartsEnabled = useUIStore((state) => state.nodeEditor.chartsEnabled);
  const setNodeEditorChartsEnabled = useUIStore((state) => state.setNodeEditorChartsEnabled);
  const chartsEnabled = propChartsEnabled !== undefined ? propChartsEnabled : storeChartsEnabled;

  // Resizable split-pane state and handlers
  const [splitPercent, setSplitPercent] = useState<number>(50);
  const [isEditorCollapsed, setIsEditorCollapsed] = useState<boolean>(false);
  const [isPreviewCollapsed, setIsPreviewCollapsed] = useState<boolean>(false);
  const [lastSplitPercent, setLastSplitPercent] = useState<number>(50);
  const [isDragging, setIsDragging] = useState<boolean>(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const isDraggingRef = useRef<boolean>(false);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isDraggingRef.current = true;
    setIsDragging(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }, []);

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isDraggingRef.current || !containerRef.current) return;

      const containerRect = containerRef.current.getBoundingClientRect();
      const sidebarWidth = 290;
      const totalWidth = containerRect.width - sidebarWidth - 6; // Subtract sidebar and handle widths
      if (totalWidth <= 0) return;

      const relativeX = e.clientX - containerRect.left - sidebarWidth;
      const percent = (relativeX / totalWidth) * 100;

      if (percent < 12) {
        setIsEditorCollapsed(true);
        setIsPreviewCollapsed(false);
        setSplitPercent(0);
      } else if (percent > 88) {
        setIsPreviewCollapsed(true);
        setIsEditorCollapsed(false);
        setSplitPercent(100);
      } else {
        setIsEditorCollapsed(false);
        setIsPreviewCollapsed(false);
        setSplitPercent(percent);
        setLastSplitPercent(percent);
      }
    };

    const handleMouseUp = () => {
      if (isDraggingRef.current) {
        isDraggingRef.current = false;
        setIsDragging(false);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      }
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);

    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, []);

  const [node, setNode] = useState<Node | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [editSummary, setEditSummary] = useState("");
  const [editDetail, setEditDetail] = useState("");
  const [editPrivacy, setEditPrivacy] = useState("open");

  // Tag state
  const [nodeTags, setNodeTags] = useState<Tag[]>([]);
  const [availableTags, setAvailableTags] = useState<Tag[]>([]);
  const [tagInput, setTagInput] = useState("");
  const [isDropdownOpen, setIsDropdownOpen] = useState(false);
  const [tagRefreshKey, setTagRefreshKey] = useState(0);

  // Door connections state
  const [outgoingDoors, setOutgoingDoors] = useState<Door[]>([]);
  const [incomingDoors, setIncomingDoors] = useState<Backlink[]>([]);
  const [allNodes, setAllNodes] = useState<Node[]>([]);
  const [allNodesMap, setAllNodesMap] = useState<Record<string, Node>>({});
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [isDoorPickerOpen, setIsDoorPickerOpen] = useState(false);
  const [doorSearchQuery, setDoorSearchQuery] = useState("");
  const [doorTargetId, setDoorTargetId] = useState<string | null>(null);
  const [doorLabelInput, setDoorLabelInput] = useState("");
  const [repointDoorId, setRepointDoorId] = useState<string | null>(null);

  // Priority and Auto-save state
  const [editPriorityProfile, setEditPriorityProfile] = useState("standard");
  const [editFrozen, setEditFrozen] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "saving" | "saved">("idle");
  const [status, setStatus] = useState("");

  // Debounced live preview state
  const [debouncedPreviewDetail, setDebouncedPreviewDetail] = useState("");
  const [lastInitializedNodeId, setLastInitializedNodeId] = useState<string | null>(null);
  const [, startTransition] = useTransition();

  const saveStatusTimeoutRef = useRef<number | null>(null);
  const detailRef = useRef<HTMLTextAreaElement | null>(null);
  const saveRunIdRef = useRef<number>(0);

  const [copiedNode, setCopiedNode] = useState<boolean>(false);

  // Safe file name slugification helper
  const getTitleSlug = (title: string): string => {
    const slug = title
      .toLowerCase()
      .replace(/[^a-z0-9\s-_]+/g, "")
      .trim()
      .replace(/\s+/g, "-")
      .replace(/-+/g, "-");
    return slug || "untitled";
  };

  // Copy Markdown Action Handler
  const handleCopyMarkdown = async () => {
    try {
      await navigator.clipboard.writeText(editDetail);
      setCopiedNode(true);
      setTimeout(() => setCopiedNode(false), 2000);
    } catch (err) {
      console.error("Failed to copy markdown: ", err);
    }
  };

  // Save Markdown Action Handler (Native OS Dialog)
  const handleSaveMarkdown = async () => {
    const slug = getTitleSlug(editTitle || "untitled-node");
    const defaultName = `${slug}.md`;
    try {
      const res = await saveMarkdownFile(defaultName, editDetail);
      if ("err" in res) {
        setStatus(`Save error: ${res.err}`);
      } else if ("ok" in res && res.ok) {
        setStatus("Markdown file saved successfully!");
        setTimeout(() => setStatus(""), 3000);
      }
    } catch (err) {
      console.error("Failed to save markdown natively: ", err);
      setStatus("Failed to trigger native save dialog.");
    }
  };

  // Wikilink autocomplete state
  const [wikilinkOpen, setWikilinkOpen] = useState(false);
  const [wikilinkQuery, setWikilinkQuery] = useState("");
  const [wikilinkCursorPos, setWikilinkCursorPos] = useState(0);
  const [wikilinkDropdownPos, setWikilinkDropdownPos] = useState<{
    top: number;
    left: number;
  } | null>(null);
  const activeNodeIdRef = useRef(nodeId);

  useEffect(() => {
    activeNodeIdRef.current = nodeId;
  }, [nodeId]);

  const vaultById = useMemo(() => {
    const map: Record<string, Vault> = {};
    for (const vault of vaults) {
      map[vault.id] = vault;
    }
    return map;
  }, [vaults]);

  // Load and refresh functions
  const loadNodeData = React.useCallback(async () => {
    const requestedNodeId = nodeId;
    try {
      const [nodeRes, tagsRes, outgoingRes, incomingRes, vaultsRes, allNodesRes] =
        await Promise.all([
          getNode(requestedNodeId),
          getNodeTags(requestedNodeId),
          listOutgoingDoors(requestedNodeId),
          listIncomingDoors(requestedNodeId),
          listVaults(),
          getAllNodes(),
        ]);

      if (activeNodeIdRef.current !== requestedNodeId) {
        return;
      }

      if (!nodeRes) {
        setStatus("Node not found.");
        return;
      }

      setNode(nodeRes);
      setVaults(vaultsRes);
      setAllNodes(allNodesRes);

      const map: Record<string, Node> = {};
      for (const item of allNodesRes) {
        map[item.id] = item;
      }
      setAllNodesMap(map);

      if (tagsRes.error) setStatus(tagsRes.error.message);
      else setNodeTags(tagsRes.data ?? []);

      if (outgoingRes.error) setStatus(outgoingRes.error.message);
      else setOutgoingDoors(outgoingRes.data ?? []);

      if (incomingRes.error) setStatus(incomingRes.error.message);
      else setIncomingDoors(incomingRes.data ?? []);

      setStatus("");
    } catch (err) {
      console.error(err);
      setStatus("Failed to load node.");
    }
  }, [nodeId]);

  useEffect(() => {
    const timer = setTimeout(() => {
      void loadNodeData();
    }, 0);
    void touchNode(nodeId).catch(() => {});
    return () => clearTimeout(timer);
  }, [nodeId, tagRefreshKey, loadNodeData]);

  useEffect(() => {
    /* eslint-disable react-hooks/set-state-in-effect */
    if (node && node.id === nodeId && lastInitializedNodeId !== nodeId) {
      setLastInitializedNodeId(nodeId);
      setEditTitle(node.title ?? "");
      setEditSummary(node.summary ?? "");
      setEditDetail(node.detail ?? "");
      setDebouncedPreviewDetail(node.detail ?? "");
      setEditPrivacy(node.privacyTier ?? "open");
      try {
        const parsed = node.priority
          ? typeof node.priority === "string"
            ? JSON.parse(node.priority)
            : node.priority
          : null;
        setEditPriorityProfile(
          parsed && typeof parsed === "object" && "profile" in parsed
            ? String(parsed.profile)
            : "standard"
        );
        setEditFrozen(
          parsed && typeof parsed === "object" && "frozen" in parsed
            ? parsed.frozen === true
            : false
        );
      } catch {
        setEditPriorityProfile("standard");
        setEditFrozen(false);
      }
    }
    /* eslint-enable react-hooks/set-state-in-effect */
  }, [nodeId, node, lastInitializedNodeId]);

  useEffect(() => {
    async function loadTags() {
      const result = await listTags();
      if (!result.error) {
        setAvailableTags(result.data ?? []);
      }
    }
    void loadTags();
  }, [tagRefreshKey]);

  // Decoupled preview generation: 250ms input state typing debounce
  useEffect(() => {
    const timer = setTimeout(() => {
      startTransition(() => {
        setDebouncedPreviewDetail(editDetail);
      });
    }, 250);
    return () => clearTimeout(timer);
  }, [editDetail]);

  // Clean timeouts on unmount
  useEffect(() => {
    return () => {
      if (saveStatusTimeoutRef.current !== null) {
        window.clearTimeout(saveStatusTimeoutRef.current);
      }
    };
  }, []);

  // Sync Wikilink doors automatically after save
  const parseWikilinks = (text: string): string[] => {
    if (!text) return [];
    // Restrict input length to prevent CPU thread exhaustion / ReDoS on large text payloads
    const safeText = text.length > 50000 ? text.slice(0, 50000) : text;
    const regex = /\[\[([^|\]\n]+)\|([^\]\n]+)\]\]/g;
    const ids: string[] = [];
    let m: RegExpExecArray | null;
    while ((m = regex.exec(safeText)) !== null) {
      ids.push(m[2]);
    }
    return ids;
  };

  const syncWikilinkDoors = React.useCallback(
    async (sourceNodeId: string, detailText: string) => {
      const targetIds = parseWikilinks(detailText);
      if (targetIds.length === 0) return;

      // Fetch current outgoing doors to avoid duplicates
      const currentOutgoing = await listOutgoingDoors(sourceNodeId);
      const existingTargetIds = new Set<string>();
      if (!currentOutgoing.error && currentOutgoing.data) {
        for (const d of currentOutgoing.data) {
          if (d.targetNodeId) existingTargetIds.add(d.targetNodeId);
        }
      }

      const uniqueTargetIds = new Set<string>();
      for (const targetId of targetIds) {
        if (targetId === sourceNodeId) continue;
        if (existingTargetIds.has(targetId)) continue;
        uniqueTargetIds.add(targetId);
      }

      if (uniqueTargetIds.size === 0) return;

      const createPromises = Array.from(uniqueTargetIds).map(async (tid) => {
        const result = await createDoor({
          sourceNodeId,
          targetNodeId: tid,
        });
        return !result.error;
      });

      await Promise.all(createPromises);
      void loadNodeData();
    },
    [loadNodeData]
  );

  // Debounced auto-save engine
  useEffect(() => {
    if (!node) return;

    const currentPrivacy = node.privacyTier ?? "open";
    let currentPriorityProfile = "standard";
    let currentFrozen = false;
    try {
      const parsed = typeof node.priority === "string" ? JSON.parse(node.priority) : node.priority;
      if (parsed && typeof parsed === "object" && "profile" in parsed) {
        currentPriorityProfile = String(parsed.profile);
      }
      if (parsed && typeof parsed === "object" && "frozen" in parsed) {
        currentFrozen = parsed.frozen === true;
      }
    } catch {
      // ignore
    }

    const hasChanges =
      editTitle !== (node.title ?? "") ||
      editSummary !== (node.summary ?? "") ||
      editDetail !== (node.detail ?? "") ||
      editPrivacy !== currentPrivacy ||
      editPriorityProfile !== currentPriorityProfile ||
      editFrozen !== currentFrozen;

    if (!hasChanges) return;

    const runId = saveRunIdRef.current + 1;
    saveRunIdRef.current = runId;

    const statusTimer = window.setTimeout(() => {
      setSaveStatus("saving");
      setStatus("");
    }, 0);

    const timer = window.setTimeout(async () => {
      try {
        const freshNodeForSave = await getNode(nodeId);
        if (!freshNodeForSave) {
          throw new Error("Node was deleted before save could complete.");
        }

        const updated = await updateNode({
          id: nodeId,
          title: editTitle,
          summary: editSummary,
          detail: editDetail,
          privacyTier: editPrivacy,
          priority: JSON.stringify({
            ...(() => {
              try {
                const p =
                  typeof freshNodeForSave.priority === "string"
                    ? JSON.parse(freshNodeForSave.priority)
                    : freshNodeForSave.priority;
                return typeof p === "object" && p !== null ? p : {};
              } catch {
                return {};
              }
            })(),
            profile: editPriorityProfile,
            pinned: editPriorityProfile === "pinned",
            frozen: editFrozen,
          }),
        });

        if (runId !== saveRunIdRef.current) return;

        setNode(updated);
        if (editPriorityProfile !== currentPriorityProfile) {
          await refreshAllPriorityScores();
          const freshNode = await getNode(nodeId);
          if (freshNode && runId === saveRunIdRef.current) {
            setNode(freshNode);
          }
        }

        void syncWikilinkDoors(nodeId, editDetail);
        setSaveStatus("saved");

        if (saveStatusTimeoutRef.current !== null) {
          window.clearTimeout(saveStatusTimeoutRef.current);
        }
        saveStatusTimeoutRef.current = window.setTimeout(() => {
          if (runId === saveRunIdRef.current) {
            setSaveStatus("idle");
          }
        }, 2000);
      } catch (err) {
        if (runId !== saveRunIdRef.current) return;
        setSaveStatus("idle");
        if (err instanceof AppError) {
          setStatus(err.message);
        } else {
          setStatus("Failed to save node.");
        }
      }
    }, 1000);

    return () => {
      window.clearTimeout(statusTimer);
      window.clearTimeout(timer);
    };
  }, [
    editTitle,
    editSummary,
    editDetail,
    editPrivacy,
    editPriorityProfile,
    editFrozen,
    node,
    nodeId,
    syncWikilinkDoors,
  ]);

  // Breadcrumbs path
  const breadcrumbPath = useMemo(() => {
    if (!node) return "";
    const containerVaultId = node.subVaultId ?? node.vaultId;
    return getVaultDisplayPath(containerVaultId, vaultById, isRedactedUnlocked);
  }, [node, vaultById, isRedactedUnlocked]);

  // Privacy helpers
  const { parentTier, effectivePrivacyTier } = useMemo(() => {
    if (!node) {
      return { parentTier: "open", effectivePrivacyTier: "open" };
    }
    const subVault = node.subVaultId ? vaults.find((v) => v.id === node.subVaultId) : undefined;
    const parentVaultId = subVault?.parentVaultId ?? node.vaultId;
    const parentVault = vaults.find((v) => v.id === parentVaultId);
    const forcedParentTier = getEffectivePrivacy(
      undefined,
      subVault?.privacyTier,
      parentVault?.privacyTier
    );
    return {
      parentTier: forcedParentTier,
      effectivePrivacyTier: getEffectivePrivacy(
        editPrivacy,
        subVault?.privacyTier,
        parentVault?.privacyTier
      ),
    };
  }, [editPrivacy, node, vaults]);

  const priorityScore = useMemo(() => {
    if (!node?.priority) return null;
    try {
      const parsed = typeof node.priority === "string" ? JSON.parse(node.priority) : node.priority;
      if (typeof parsed === "number" && Number.isFinite(parsed)) return parsed;
      if (parsed && typeof parsed === "object" && "score" in parsed) {
        const value = Number((parsed as { score: unknown }).score);
        if (Number.isFinite(value)) return value;
      }
    } catch {
      const fallback = Number(node.priority);
      if (Number.isFinite(fallback)) return fallback;
    }
    return null;
  }, [node]);

  const isRedactedLocked = effectivePrivacyTier === "redacted" && !isRedactedUnlocked;
  const isContentLocked = effectivePrivacyTier === "locked" && !isRedactedUnlocked;
  const isAnyLocked = isRedactedLocked || isContentLocked;

  const [authIsSetupState, setAuthIsSetupState] = useState<boolean | null>(null);
  const [lockPasswordInput, setLockPasswordInput] = useState("");
  const [lockError, setLockError] = useState("");

  useEffect(() => {
    if (!isAnyLocked) return;
    void (async () => {
      const result = await isAuthSetup();
      if (result.error) {
        setAuthIsSetupState(false);
        setLockError(result.error.message);
      } else {
        setAuthIsSetupState(result.data ?? false);
        setLockError("");
      }
    })();
  }, [isAnyLocked]);

  async function onLockSubmit() {
    if (!lockPasswordInput) return;
    setLockError("");
    if (authIsSetupState) {
      const result = await verifyMasterPassword(lockPasswordInput);
      if (result.error) {
        setLockError(result.error.message);
        return;
      }
      if (!result.data) {
        setLockError("Incorrect password");
        return;
      }
      setIsRedactedUnlocked(true);
      setLockPasswordInput("");
      return;
    }
    const result = await setMasterPassword(lockPasswordInput);
    if (result.error) {
      setLockError(result.error.message);
      return;
    }
    if (!result.data) {
      setLockError("Failed to set master password.");
      return;
    }
    setAuthIsSetupState(true);
    setIsRedactedUnlocked(true);
    setLockPasswordInput("");
  }

  // Tags overrides
  const normalizedTagInput = tagInput.trim().toLowerCase();

  const filteredTagOptions = useMemo(() => {
    const selectedTagIds = new Set(nodeTags.map((t) => t.id));
    return availableTags.filter((t) => {
      if (selectedTagIds.has(t.id)) return false;
      if (!normalizedTagInput) return true;
      return t.name.toLowerCase().includes(normalizedTagInput);
    });
  }, [availableTags, nodeTags, normalizedTagInput]);

  const hasExactTagMatch = useMemo(() => {
    if (!normalizedTagInput) return false;
    return availableTags.some((t) => t.name.toLowerCase() === normalizedTagInput);
  }, [availableTags, normalizedTagInput]);

  async function onAddExistingTag(tag: Tag) {
    if (!node) return;
    const result = await addNodeTag(node.id, tag.id);
    if (!result.error) {
      setTagInput("");
      setIsDropdownOpen(false);
      void loadNodeData();
    }
  }

  async function onCreateAndAddTag() {
    if (!node) return;
    const name = tagInput.trim();
    if (!name) return;
    const created = await createTag({ name });
    if (created.error || !created.data) return;
    const added = await addNodeTag(node.id, created.data.id);
    if (!added.error) {
      setTagInput("");
      setIsDropdownOpen(false);
      setTagRefreshKey((v) => v + 1);
    }
  }

  async function onRemoveTag(tagId: string) {
    if (!node) return;
    const result = await removeNodeTag(node.id, tagId);
    if (!result.error) {
      setNodeTags((prev) => prev.filter((t) => t.id !== tagId));
    }
  }

  // Connections overrides
  const connectedTargetIds = useMemo(() => {
    const ids = new Set<string>();
    for (const d of outgoingDoors) {
      if (d.targetNodeId) ids.add(d.targetNodeId);
    }
    return ids;
  }, [outgoingDoors]);

  const filteredDoorTargets = useMemo(() => {
    const query = doorSearchQuery.trim().toLowerCase();
    if (!node) return [];
    return allNodes.filter((c) => {
      if (c.id === node.id) return false;
      if (connectedTargetIds.has(c.id)) return false;
      if (!query) return true;
      return c.title.toLowerCase().includes(query) || c.summary.toLowerCase().includes(query);
    });
  }, [allNodes, connectedTargetIds, doorSearchQuery, node]);

  async function onDoorDelete(doorId: string) {
    const result = await deleteDoor(doorId);
    if (!result.error) {
      void loadNodeData();
    }
  }

  async function onConfirmConnection() {
    if (!node || !doorTargetId) return;
    setStatus("");
    const result = repointDoorId
      ? await repointDoor(repointDoorId, doorTargetId)
      : await createDoor({
          sourceNodeId: node.id,
          targetNodeId: doorTargetId,
          label: doorLabelInput.trim() ? doorLabelInput.trim() : undefined,
        });

    if (!result.error) {
      void loadNodeData();
      setIsDoorPickerOpen(false);
      setDoorSearchQuery("");
      setDoorTargetId(null);
      setDoorLabelInput("");
      setRepointDoorId(null);
    } else {
      setStatus(result.error.message);
    }
  }

  // Autocomplete implementation for central editor
  const handleKeyUp = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const ta = e.currentTarget;
    const pos = ta.selectionStart;
    const text = ta.value;

    const before = text.slice(0, pos);
    const openIdx = before.lastIndexOf("[[");
    if (openIdx === -1) {
      if (wikilinkOpen) setWikilinkOpen(false);
      return;
    }

    const segment = before.slice(openIdx + 2);
    if (segment.includes("]]")) {
      if (wikilinkOpen) setWikilinkOpen(false);
      return;
    }

    setWikilinkQuery(segment);
    setWikilinkCursorPos(pos);
    setWikilinkOpen(true);

    const coords = getCaretCoordinates(ta, pos);
    setWikilinkDropdownPos(coords);
  };

  const handleSelectAutocomplete = (targetNode: Node) => {
    const ta = detailRef.current;
    if (!ta) return;

    const text = editDetail;
    const pos = wikilinkCursorPos;
    const before = text.slice(0, pos);
    const openIdx = before.lastIndexOf("[[");
    if (openIdx === -1) return;

    const replacement = `[[${targetNode.title}|${targetNode.id}]]`;
    const newText = text.slice(0, openIdx) + replacement + text.slice(pos);

    setEditDetail(newText);
    setWikilinkOpen(false);
    setWikilinkQuery("");

    // Auto-create door immediately
    if (node && targetNode.id !== node.id && !connectedTargetIds.has(targetNode.id)) {
      void createDoor({
        sourceNodeId: node.id,
        targetNodeId: targetNode.id,
      }).then((res) => {
        if (!res.error) void loadNodeData();
      });
    }

    const newCursorPos = openIdx + replacement.length;
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(newCursorPos, newCursorPos);
    });
  };

  const preprocessedMarkdown = React.useMemo(() => {
    const wLinks = preprocessWikiLinks(debouncedPreviewDetail);
    return preprocessMathDelimiters(wLinks);
  }, [debouncedPreviewDetail]);

  const markdownComponents = React.useMemo(() => {
    return createMarkdownComponents(chartsEnabled, onSelectNode, isRedactedUnlocked);
  }, [chartsEnabled, onSelectNode, isRedactedUnlocked]);

  const existingNodeIds = React.useMemo(() => {
    return new Set(Object.keys(allNodesMap));
  }, [allNodesMap]);

  function getNodeDisplayLabel(item: Node): string {
    const containerId = item.subVaultId ?? item.vaultId;
    const containerTier = vaultById[containerId]
      ? getVaultEffectivePrivacy(containerId, vaultById)
      : undefined;
    const tier = getEffectivePrivacy(item.privacyTier, null, containerTier);
    return getPrivacyDisplayLabel(item.title, tier, isRedactedUnlocked);
  }

  function getNodeDisplaySummary(item: Node, maxLength = 50): string {
    const containerId = item.subVaultId ?? item.vaultId;
    const containerTier = vaultById[containerId]
      ? getVaultEffectivePrivacy(containerId, vaultById)
      : undefined;
    const tier = getEffectivePrivacy(item.privacyTier, null, containerTier);
    const summary = item.summary.slice(0, maxLength);
    return getPrivacyDisplaySummary(summary, tier, isRedactedUnlocked);
  }

  return (
    <div className="expanded-editor-window">
      {/* Top Header Glassmorphic Bar */}
      <header className="expanded-editor-header">
        <div className="header-left">
          <button
            type="button"
            className="header-close-btn"
            onClick={onClose}
            title="Go back to workspace"
          >
            ← Back
          </button>
          <div className="header-breadcrumbs">
            {breadcrumbPath && <span className="breadcrumb-path">{breadcrumbPath} /</span>}
            <span className="breadcrumb-current">{editTitle || "Untitled Node"}</span>
          </div>
        </div>
        <div className="header-right">
          <span className={`save-status ${saveStatus}`}>
            {saveStatus === "saving"
              ? "Saving changes..."
              : saveStatus === "saved"
                ? "All saved!"
                : ""}
          </span>
          <button
            type="button"
            className="header-action-icon-btn"
            onClick={handleCopyMarkdown}
            title="Copy entire markdown source to clipboard"
          >
            {copiedNode ? "✓ Copied!" : "📋 Copy Markdown"}
          </button>
          <button
            type="button"
            className="header-action-icon-btn"
            onClick={handleSaveMarkdown}
            title="Save markdown to filesystem"
          >
            📥 Save Markdown
          </button>
          <button
            type="button"
            className={`charts-toggle-btn expanded-header-charts-btn ${chartsEnabled ? "active" : ""}`}
            onClick={() => setNodeEditorChartsEnabled(!chartsEnabled)}
            title="Toggle interactive charts and diagrams rendering in the workspace"
          >
            📊 Render Workspace Assets: {chartsEnabled ? "ON" : "OFF"}
          </button>
          <button type="button" className="header-exit-btn" onClick={onClose}>
            Close Focus
          </button>
        </div>
      </header>

      {/* Main 3-Pane Body Grid */}
      <div
        ref={containerRef}
        className="expanded-editor-layout"
        style={{
          gridTemplateColumns: isEditorCollapsed
            ? "290px 0px 0px 1fr"
            : isPreviewCollapsed
              ? "290px 1fr 0px 0px"
              : `290px calc((100% - 290px - 6px) * ${splitPercent / 100}) 6px calc((100% - 290px - 6px) * ${(100 - splitPercent) / 100})`,
        }}
      >
        {/* Left Column: Metadata & Connections */}
        <aside className="expanded-editor-sidebar">
          <div className="sidebar-group">
            <h4 className="sidebar-group-title">Metadata</h4>
            <div className="editor-meta-vertical">
              <label className="editor-privacy-vertical">
                <span>Privacy Tier</span>
                {!isRedactedLocked && (
                  <select
                    value={effectivePrivacyTier}
                    onChange={(e) => setEditPrivacy(e.target.value)}
                    disabled={isAnyLocked}
                  >
                    <option
                      value="open"
                      disabled={getPrivacyRank("open") < getPrivacyRank(parentTier)}
                    >
                      Open
                    </option>
                    <option
                      value="local_only"
                      disabled={getPrivacyRank("local_only") < getPrivacyRank(parentTier)}
                    >
                      Local-Only
                    </option>
                    <option
                      value="locked"
                      disabled={getPrivacyRank("locked") < getPrivacyRank(parentTier)}
                    >
                      Locked
                    </option>
                    <option
                      value="redacted"
                      disabled={getPrivacyRank("redacted") < getPrivacyRank(parentTier)}
                    >
                      Redacted
                    </option>
                  </select>
                )}
                <span className="effective-privacy-tag">
                  Effective: <PrivacyBadge tier={effectivePrivacyTier} />
                </span>
              </label>

              {!isRedactedLocked && (
                <label className="editor-priority-vertical">
                  <div className="priority-label-row">
                    <span>Priority Profile</span>
                    <PriorityBar score={priorityScore} />
                  </div>
                  <div className="priority-select-row">
                    <select
                      value={editPriorityProfile}
                      onChange={(e) => setEditPriorityProfile(e.target.value)}
                      disabled={isAnyLocked}
                    >
                      <option value="standard">Standard</option>
                      <option value="slow">Slow</option>
                      <option value="fast">Fast</option>
                      <option value="pinned">Pinned</option>
                    </select>
                    <button
                      type="button"
                      className={`freeze-toggle ${editFrozen ? "frozen" : ""}`}
                      onClick={() => !isAnyLocked && setEditFrozen((prev) => !prev)}
                      disabled={isAnyLocked}
                      title={
                        editFrozen
                          ? "Unfreeze — auto-optimize priority"
                          : "Freeze — protect from auto-optimize"
                      }
                    >
                      ❄️
                    </button>
                  </div>
                </label>
              )}
            </div>
          </div>

          {/* Tag Editor Section */}
          {!isRedactedLocked && (
            <div className="sidebar-group">
              <h4 className="sidebar-group-title">Tags</h4>
              <div className="tag-wrapper">
                <div className="tag-list">
                  {nodeTags.map((tag) => (
                    <span key={tag.id} className="tag-pill">
                      {tag.name}
                      {!isAnyLocked && (
                        <button
                          type="button"
                          onClick={() => onRemoveTag(tag.id)}
                          aria-label={`Remove ${tag.name}`}
                        >
                          ×
                        </button>
                      )}
                    </span>
                  ))}
                </div>
                {!isAnyLocked && (
                  <input
                    className="tag-input"
                    placeholder="Add tag..."
                    value={tagInput}
                    onChange={(e) => {
                      setTagInput(e.target.value);
                      setIsDropdownOpen(true);
                    }}
                    onFocus={() => setIsDropdownOpen(true)}
                    onBlur={() => setTimeout(() => setIsDropdownOpen(false), 150)}
                  />
                )}
                {isDropdownOpen && !isAnyLocked && (
                  <div className="tag-dropdown">
                    {filteredTagOptions.map((tag) => (
                      <button
                        key={tag.id}
                        type="button"
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => void onAddExistingTag(tag)}
                      >
                        {tag.name}
                      </button>
                    ))}
                    {normalizedTagInput && !hasExactTagMatch && (
                      <button
                        key="create-new"
                        type="button"
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => void onCreateAndAddTag()}
                      >
                        Create: "{tagInput.trim()}"
                      </button>
                    )}
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Connections / Outgoing Doors Section */}
          <div className="sidebar-group">
            <div className="connections-header">
              <h4 className="sidebar-group-title">Connections</h4>
              {!isAnyLocked && (
                <button
                  type="button"
                  className="add-connection-btn"
                  onClick={() => setIsDoorPickerOpen(!isDoorPickerOpen)}
                >
                  + Add
                </button>
              )}
            </div>

            {isDoorPickerOpen && (
              <div className="door-picker">
                <input
                  type="search"
                  value={doorSearchQuery}
                  onChange={(e) => setDoorSearchQuery(e.target.value)}
                  placeholder="Filter nodes to connect..."
                />
                <div className="door-search-results">
                  {filteredDoorTargets.map((target) => (
                    <button
                      key={target.id}
                      type="button"
                      onClick={() => setDoorTargetId(target.id)}
                    >
                      <strong>{getNodeDisplayLabel(target)}</strong>
                      <small>{getNodeDisplaySummary(target, 60)}</small>
                    </button>
                  ))}
                </div>
                {doorTargetId && (
                  <div className="door-create-row">
                    <input
                      type="text"
                      placeholder="Optional label..."
                      value={doorLabelInput}
                      onChange={(e) => setDoorLabelInput(e.target.value)}
                    />
                    <button type="button" onClick={() => void onConfirmConnection()}>
                      Connect
                    </button>
                  </div>
                )}
              </div>
            )}

            <div className="door-list">
              {node && isImportDocumentNode(node) && (
                <p className="document-sections-summary">
                  {outgoingDoors.filter((d) => d.label === "section").length} sections — navigate
                  via Document sections in the preview
                </p>
              )}
              {outgoingDoors
                .filter((door) => !(node && isImportDocumentNode(node) && door.label === "section"))
                .map((door) => {
                  const targetNode = door.targetNodeId ? allNodesMap[door.targetNodeId] : undefined;
                  const targetTitle = targetNode
                    ? getNodeDisplayLabel(targetNode)
                    : "Missing node link";
                  return (
                    <div
                      key={door.id}
                      className={`door-item ${door.status === "orphaned" ? "orphaned" : ""}`}
                    >
                      <div className="door-main">
                        {door.status !== "orphaned" && door.targetNodeId ? (
                          <button
                            type="button"
                            className="door-link-btn"
                            onClick={() => onSelectNode(door.targetNodeId!)}
                            title={`Navigate to: ${targetTitle}`}
                          >
                            <strong>{targetTitle} ↗</strong>
                          </button>
                        ) : (
                          <strong>{targetTitle}</strong>
                        )}
                        {door.status === "orphaned" && (
                          <span className="door-orphan-badge">[Orphaned]</span>
                        )}
                        {door.label && <span className="door-label">{door.label}</span>}
                      </div>
                      {!isAnyLocked && (
                        <button
                          type="button"
                          className="door-delete-btn"
                          onClick={() => void onDoorDelete(door.id)}
                          title="Delete door link"
                          aria-label={`Delete door link to ${targetTitle}`}
                        >
                          ×
                        </button>
                      )}
                    </div>
                  );
                })}
            </div>

            <div className="door-list incoming">
              {incomingDoors
                .filter((backlink) => Boolean(allNodesMap[backlink.sourceNodeId]))
                .map((backlink) => {
                  const sourceNode = allNodesMap[backlink.sourceNodeId];
                  const sourceTitle = getNodeDisplayLabel(sourceNode);
                  return (
                    <div key={backlink.id} className="door-item incoming-door">
                      <div className="door-main">
                        <button
                          type="button"
                          className="door-link-btn"
                          onClick={() => onSelectNode(backlink.sourceNodeId)}
                          title={`Navigate to: ${sourceTitle}`}
                        >
                          <strong>{sourceTitle} ↗</strong>
                        </button>
                        <span className="door-label">Incoming Door Link</span>
                      </div>
                    </div>
                  );
                })}
            </div>
          </div>
        </aside>

        {/* Center Column: Source Code Markdown Editor */}
        <section
          className={`expanded-editor-source ${isEditorCollapsed ? "collapsed" : ""}`}
          style={
            isEditorCollapsed ? { borderRight: "none", padding: 0, overflow: "hidden" } : undefined
          }
        >
          {isRedactedLocked ? (
            <div className="redacted-lock-screen">
              <span className="redacted-lock-icon">🔒</span>
              <h4 className="redacted-lock-title">Redacted Node Details</h4>
              <p className="redacted-lock-subtitle">
                {authIsSetupState === false
                  ? "Define a master password to manage security parameters."
                  : "Highly Confidential. Master password verification required."}
              </p>
              {authIsSetupState !== null && (
                <form
                  className="redacted-lock-form"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void onLockSubmit();
                  }}
                >
                  <input
                    className="redacted-lock-input"
                    type="password"
                    value={lockPasswordInput}
                    onChange={(e) => setLockPasswordInput(e.target.value)}
                    placeholder={authIsSetupState ? "Password" : "Set Master Password"}
                    autoFocus
                  />
                  <button type="submit" className="redacted-lock-button">
                    Unlock Content
                  </button>
                </form>
              )}
              {lockError && <p className="redacted-lock-error">{lockError}</p>}
            </div>
          ) : (
            <div className="source-editor-inner">
              <input
                className="source-title-input"
                value={editTitle}
                onChange={(e) => setEditTitle(e.target.value)}
                placeholder="Title..."
                disabled={isAnyLocked}
              />
              <textarea
                className="source-summary-input"
                value={editSummary}
                onChange={(e) => setEditSummary(e.target.value)}
                placeholder="Short one-sentence plain-text summary (used as LLM context summary)..."
                disabled={isAnyLocked}
                rows={2}
              />
              <div
                className="source-detail-container wikilink-wrapper"
                style={{ position: "relative", flex: 1 }}
              >
                <textarea
                  ref={detailRef}
                  className="source-detail-textarea monospace-editor"
                  value={editDetail}
                  onChange={(e) => setEditDetail(e.target.value)}
                  onKeyUp={handleKeyUp}
                  onScroll={(e) => {
                    if (wikilinkOpen) {
                      const ta = e.currentTarget;
                      const coords = getCaretCoordinates(ta, ta.selectionStart);
                      setWikilinkDropdownPos(coords);
                    }
                  }}
                  onBlur={() => setTimeout(() => setWikilinkOpen(false), 200)}
                  placeholder="Write rich markdown here... type [[ to link other nodes"
                  disabled={isAnyLocked}
                />
                {wikilinkOpen && wikilinkDropdownPos && (
                  <NodeLinkAutocomplete
                    query={wikilinkQuery}
                    position={wikilinkDropdownPos}
                    onSelect={handleSelectAutocomplete}
                    onClose={() => setWikilinkOpen(false)}
                    nodes={allNodes}
                  />
                )}
              </div>
            </div>
          )}
        </section>

        <div
          className={`split-handle ${isDragging ? "dragging" : ""}`}
          onMouseDown={handleMouseDown}
          title="Drag to resize columns"
          style={
            isEditorCollapsed || isPreviewCollapsed
              ? { visibility: "hidden", pointerEvents: "none" }
              : undefined
          }
        />

        {/* Right Column: High Fidelity Markdown Live Preview */}
        <section
          className={`expanded-editor-preview paper-preview ${isPreviewCollapsed ? "collapsed" : ""}`}
          style={isPreviewCollapsed ? { padding: 0, overflow: "hidden" } : undefined}
        >
          {isAnyLocked ? (
            <div className="preview-locked-placeholder">
              <span className="placeholder-lock-icon">🔒</span>
              <p>Preview locked. Enter password in source column to unlock details.</p>
            </div>
          ) : (
            <div className="preview-rendered-content">
              {isRawLatex(preprocessedMarkdown) ? (
                chartsEnabled ? (
                  <div style={{ marginTop: "1rem" }}>
                    <LatexBlock code={preprocessedMarkdown} />
                  </div>
                ) : (
                  <pre
                    style={{
                      margin: "1rem 0 0 0",
                      padding: "16px",
                      background: "rgba(0, 0, 0, 0.03)",
                      border: "1px solid rgba(188, 108, 37, 0.15)",
                      borderRadius: "6px",
                      overflow: "auto",
                      fontFamily: "monospace",
                      whiteSpace: "pre-wrap",
                      fontSize: "0.9rem",
                      color: "var(--text-black)",
                    }}
                  >
                    <code>{preprocessedMarkdown}</code>
                  </pre>
                )
              ) : (
                <>
                  <h1 className="preview-node-title">{editTitle || "Untitled Node"}</h1>
                  {editSummary && <p className="preview-node-summary">{editSummary}</p>}
                  {node && isImportDocumentNode(node) && (
                    <ImportDocumentSpine
                      documentNode={node}
                      allNodes={allNodes}
                      onSelectSection={onSelectNode}
                    />
                  )}
                  <hr className="preview-divider" />
                  <ExistingNodesContext.Provider value={existingNodeIds}>
                    <ReactMarkdown
                      remarkPlugins={remarkPluginsStable}
                      rehypePlugins={rehypePluginsStable}
                      components={markdownComponents}
                    >
                      {preprocessedMarkdown}
                    </ReactMarkdown>
                  </ExistingNodesContext.Provider>
                </>
              )}
            </div>
          )}
        </section>

        {isEditorCollapsed && (
          <button
            type="button"
            className="restore-pane-btn restore-left"
            onClick={() => {
              setIsEditorCollapsed(false);
              setSplitPercent(Math.max(20, Math.min(80, lastSplitPercent)));
            }}
            title="Show Editor"
          >
            <span>← Edit Source</span>
          </button>
        )}

        {isPreviewCollapsed && (
          <button
            type="button"
            className="restore-pane-btn restore-right"
            onClick={() => {
              setIsPreviewCollapsed(false);
              setSplitPercent(Math.max(20, Math.min(80, lastSplitPercent)));
            }}
            title="Show Preview"
          >
            <span>Show Preview →</span>
          </button>
        )}
      </div>

      {status && <div className="expanded-editor-status-bar">{status}</div>}
    </div>
  );
}
