import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import ErrorBoundary from "./components/ErrorBoundary";
import NodeEditor from "./components/NodeEditor";
import NodeList from "./components/NodeList";
import VaultSidebar from "./components/VaultSidebar";
import PriorityDashboard from "./components/PriorityDashboard";
import LlmSettings from "./components/LlmSettings";
import ScopeIndicator from "./components/ScopeIndicator";
import ChatPanel from "./components/ChatPanel";
import SpatialWorkspace from "./components/SpatialWorkspace";
import ActiveMemoryPanel from "./components/ActiveMemoryPanel";
import OnboardingShell from "./components/OnboardingShell";
import type { ContextAssemblerScope } from "./constants/contextBudget";
import { refreshAllPriorityScores } from "./services/nodes";
import { DEV_ONBOARDING_CHANGED } from "./constants/devEvents";
import {
  getOnboardingComplete,
  setOnboardingComplete,
  getSetting,
  setSetting,
} from "./services/settings";
import NodeEditorExpanded from "./components/NodeEditorExpanded";
import DiffPanel from "./components/DiffPanel";
import styles from "./style/components/MemoryBadge.module.css";
import { countPendingChangesetItems } from "./services/memoryAgent";
import "./style/MonoStyles.css";
import ChatHistoryPanel from "./components/ChatHistoryPanel";
import { chatListSessions, chatCreateSession, getChatHistory } from "./services/chat";
import ImportHub from "./components/ImportHub";

const SIDEBAR_RAIL_WIDTH = 48;

type RightPanelView = "notes" | "dashboard" | "settings";

function App() {
  const [onboardingResolved, setOnboardingResolved] = useState<boolean>(false);
  const [needsOnboarding, setNeedsOnboarding] = useState<boolean>(false);
  const [onboardingBusy, setOnboardingBusy] = useState<boolean>(false);
  const [onboardingError, setOnboardingError] = useState<string | null>(null);
  const [pendingProposalCount, setPendingProposalCount] = useState<number>(0);
  const [isDiffPanelOpen, setIsDiffPanelOpen] = useState<boolean>(false);
  const [selectedChangesetId, setSelectedChangesetId] = useState<string | null>(null);
  const [leftPanelView, setLeftPanelView] = useState<"browse" | "history" | "import">("browse");
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const initializeSession = async () => {
      try {
        const sessions = await chatListSessions();
        if (!active) return;

        let reuseSessionId: string | null = null;
        if (sessions.length > 0) {
          // If the most recent conversation is empty (has no messages), reuse it
          const mostRecent = sessions[0];
          const history = await getChatHistory(mostRecent.id);
          if (history.length === 0) {
            reuseSessionId = mostRecent.id;
          }
        }

        if (reuseSessionId) {
          setActiveSessionId(reuseSessionId);
        } else {
          const newId = crypto.randomUUID();
          await chatCreateSession(newId, "New Conversation");
          if (!active) return;
          setActiveSessionId(newId);
          window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
        }
      } catch (err) {
        console.error("Failed to initialize chat session:", err);
      }
    };
    void initializeSession();
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    const poll = () => {
      void countPendingChangesetItems()
        .then((count) => {
          if (active) {
            setPendingProposalCount(count);
          }
        })
        .catch((error) => {
          console.error("Failed to fetch pending changeset items count:", error);
        });
    };

    if (onboardingResolved && !needsOnboarding) {
      poll();
      const intervalId = setInterval(poll, 30_000);
      return () => {
        active = false;
        clearInterval(intervalId);
      };
    }
    return () => {
      active = false;
    };
  }, [onboardingResolved, needsOnboarding]);

  useEffect(() => {
    void refreshAllPriorityScores().catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const isComplete = await getOnboardingComplete();
        if (!cancelled) {
          setNeedsOnboarding(!isComplete);
          setOnboardingResolved(true);
        }
      } catch (error) {
        if (!cancelled) {
          setOnboardingError(String(error));
          setOnboardingResolved(true);
          setNeedsOnboarding(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    function onDevOnboardingRefresh() {
      void (async () => {
        try {
          const isComplete = await getOnboardingComplete();
          setNeedsOnboarding(!isComplete);
          setOnboardingResolved(true);
          setOnboardingError(null);
        } catch (error) {
          setOnboardingError(String(error));
          setNeedsOnboarding(false);
          setOnboardingResolved(true);
        }
      })();
    }
    window.addEventListener(DEV_ONBOARDING_CHANGED, onDevOnboardingRefresh);
    return () => window.removeEventListener(DEV_ONBOARDING_CHANGED, onDevOnboardingRefresh);
  }, []);

  const [selectedVaultId, setSelectedVaultId] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [leftPanePinned, setLeftPanePinned] = useState<boolean>(false);
  const [rightPanePinned, setRightPanePinned] = useState<boolean>(false);
  const [leftPaneWidth, setLeftPaneWidth] = useState<number>(() => {
    const saved = localStorage.getItem("sidebar_left_width");
    if (saved) {
      const parsed = parseInt(saved, 10);
      if (!isNaN(parsed)) {
        return Math.max(200, Math.min(window.innerWidth * 0.4, parsed));
      }
    }
    return 280;
  });
  const leftResizeWidthRef = useRef(leftPaneWidth);
  const [rightPaneWidth, setRightPaneWidth] = useState<number>(() => {
    const saved = localStorage.getItem("sidebar_right_width");
    if (saved) {
      const parsed = parseInt(saved, 10);
      if (!isNaN(parsed)) {
        return Math.max(200, Math.min(window.innerWidth * 0.4, parsed));
      }
    }
    return 400;
  });
  const rightResizeWidthRef = useRef(rightPaneWidth);

  // Load persistent panel widths from the database on mount
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [savedLeft, savedRight] = await Promise.all([
          getSetting("sidebar_left_width"),
          getSetting("sidebar_right_width"),
        ]);
        if (!cancelled) {
          const maxWidth = window.innerWidth * 0.4;
          if (savedLeft) {
            const w = parseInt(savedLeft, 10);
            if (!isNaN(w)) {
              const clamped = Math.max(200, Math.min(maxWidth, w));
              setLeftPaneWidth(clamped);
              localStorage.setItem("sidebar_left_width", String(clamped));
            }
          }
          if (savedRight) {
            const w = parseInt(savedRight, 10);
            if (!isNaN(w)) {
              const clamped = Math.max(200, Math.min(maxWidth, w));
              setRightPaneWidth(clamped);
              localStorage.setItem("sidebar_right_width", String(clamped));
            }
          }
        }
      } catch (error) {
        console.error("Failed to load panel widths from persistent settings", error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);
  const [leftResizing, setLeftResizing] = useState<boolean>(false);
  const [rightResizing, setRightResizing] = useState<boolean>(false);
  const [vaultRefreshKey, setVaultRefreshKey] = useState<number>(0);
  const [nodeRefreshKey, setNodeRefreshKey] = useState<number>(0);
  const [isRedactedUnlocked, setIsRedactedUnlocked] = useState<boolean>(false);
  const [selectedVaultRequiresUnlock, setSelectedVaultRequiresUnlock] = useState<boolean>(false);
  const [rightPanelView, setRightPanelView] = useState<RightPanelView>("notes");
  const [sidebarModalOpen, setSidebarModalOpen] = useState<boolean>(false);
  const [editorModalOpen, setEditorModalOpen] = useState<boolean>(false);
  const [spatialModalOpen, setSpatialModalOpen] = useState<boolean>(false);
  const [chatModalOpen, setChatModalOpen] = useState<boolean>(false);
  const [viewMode, setViewMode] = useState<"chat" | "spatial" | "editor">("chat");
  const [previousViewMode, setPreviousViewMode] = useState<"chat" | "spatial">("chat");

  const handleSetViewMode = (newMode: "chat" | "spatial" | "editor") => {
    if (newMode !== "editor") {
      setPreviousViewMode(newMode);
    }
    setViewMode(newMode);
  };
  const leftPaneExpanded =
    leftPanePinned && (leftPanelView !== "browse" || !selectedVaultRequiresUnlock);
  const rightPaneExpanded = rightPanePinned;
  const scopeNodeIds = useMemo(() => (selectedNodeId ? [selectedNodeId] : []), [selectedNodeId]);
  const [assemblerScope, setAssemblerScope] = useState<ContextAssemblerScope>("local");

  // Keep widths clamped when window resizes or on initial mount
  useEffect(() => {
    const handleResize = () => {
      const maxWidth = window.innerWidth * 0.4;
      setLeftPaneWidth((w) => Math.max(200, Math.min(maxWidth, w)));
      setRightPaneWidth((w) => Math.max(200, Math.min(maxWidth, w)));
    };
    handleResize();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  useEffect(() => {
    if (!leftResizing) {
      return;
    }

    let finished = false;

    const persistLeftWidth = () => {
      const finalWidth = leftResizeWidthRef.current;
      localStorage.setItem("sidebar_left_width", String(finalWidth));
      void setSetting("sidebar_left_width", String(finalWidth)).catch(() => {});
    };

    const finishResize = () => {
      if (finished) {
        return;
      }
      finished = true;
      setLeftResizing(false);
      persistLeftWidth();
    };

    const handleMouseMove = (moveEvent: globalThis.MouseEvent) => {
      const maxWidth = window.innerWidth * 0.4;
      const width = Math.max(200, Math.min(maxWidth, moveEvent.clientX));
      leftResizeWidthRef.current = width;
      setLeftPaneWidth(width);
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        finishResize();
      }
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", finishResize);
    window.addEventListener("blur", finishResize);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", finishResize);
      window.removeEventListener("blur", finishResize);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      if (!finished) {
        persistLeftWidth();
      }
    };
  }, [leftResizing]);

  useEffect(() => {
    if (!rightResizing) {
      return;
    }

    let finished = false;

    const persistRightWidth = () => {
      const finalWidth = rightResizeWidthRef.current;
      localStorage.setItem("sidebar_right_width", String(finalWidth));
      void setSetting("sidebar_right_width", String(finalWidth)).catch(() => {});
    };

    const finishResize = () => {
      if (finished) {
        return;
      }
      finished = true;
      setRightResizing(false);
      persistRightWidth();
    };

    const handleMouseMove = (moveEvent: globalThis.MouseEvent) => {
      const maxWidth = window.innerWidth * 0.4;
      const width = Math.max(200, Math.min(maxWidth, window.innerWidth - moveEvent.clientX));
      rightResizeWidthRef.current = width;
      setRightPaneWidth(width);
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        finishResize();
      }
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", finishResize);
    window.addEventListener("blur", finishResize);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", finishResize);
      window.removeEventListener("blur", finishResize);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      if (!finished) {
        persistRightWidth();
      }
    };
  }, [rightResizing]);

  const handleLeftResizeMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    leftResizeWidthRef.current = leftPaneWidth;
    setLeftResizing(true);
  };

  const handleRightResizeMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    rightResizeWidthRef.current = rightPaneWidth;
    setRightResizing(true);
  };

  function onLeftRailToggle(view: "browse" | "history" | "import") {
    if (view === "browse" && selectedVaultRequiresUnlock) {
      return;
    }
    if (leftPanePinned && leftPanelView === view) {
      setLeftPanePinned(false);
      return;
    }
    setLeftPanelView(view);
    setLeftPanePinned(true);
  }

  function onRightRailToggle(view: RightPanelView) {
    if (rightPanePinned && rightPanelView === view) {
      setRightPanePinned(false);
      return;
    }
    setRightPanelView(view);
    setRightPanePinned(true);
  }

  function closeAllPanes() {
    // The left pane is meant to be persistently pinned in spatial view,
    // so clicking the canvas layout does not clear leftPanePinned.
    setRightPanePinned(false);
  }

  function onZenCanvasClick(event: MouseEvent<HTMLElement>) {
    if (event.target === event.currentTarget) {
      closeAllPanes();
    }
  }

  function onSelectVault(vaultId: string | null) {
    setSelectedVaultId(vaultId);
    setSelectedNodeId(null);
    setRightPanelView("notes");
    setLeftPanelView("browse");
    setLeftPanePinned(Boolean(vaultId));
    setNodeRefreshKey((value) => value + 1);
  }

  function onFocusVault(vaultId: string | null) {
    setSelectedVaultId(vaultId);
    setSelectedNodeId(null);
    setRightPanelView("notes");
    setLeftPanelView("browse");
    setNodeRefreshKey((value) => value + 1);
  }

  function onVaultCreated(vaultId: string) {
    onSelectVault(vaultId);
    setVaultRefreshKey((value) => value + 1);
  }

  function onVaultDeleted(vaultId: string) {
    if (selectedVaultId === vaultId) {
      setSelectedVaultId(null);
      setSelectedNodeId(null);
      setRightPanePinned(false);
    }
    setVaultRefreshKey((value) => value + 1);
    setNodeRefreshKey((value) => value + 1);
  }

  function onVaultUpdated(_vaultId: string) {
    setVaultRefreshKey((value) => value + 1);
  }

  function onSelectNode(nodeId: string) {
    setSelectedNodeId(nodeId);
    setRightPanelView("notes");
    setRightPanePinned(true);
  }

  function onNodeCreated(nodeId: string) {
    setSelectedNodeId(nodeId);
    setRightPanelView("notes");
    setRightPanePinned(true);
    setNodeRefreshKey((value) => value + 1);
  }

  function onNodeDeleted(nodeId: string) {
    if (selectedNodeId === nodeId) {
      setSelectedNodeId(null);
      setRightPanePinned(false);
    }
    setNodeRefreshKey((value) => value + 1);
  }

  function onNodeUpdated(_nodeId: string) {
    setNodeRefreshKey((value) => value + 1);
  }

  function onOpenDashboard() {
    setSelectedNodeId(null);
    setRightPanelView("dashboard");
    setRightPanePinned(true);
  }

  function onOpenSettings() {
    setSelectedNodeId(null);
    setRightPanelView("settings");
    setRightPanePinned(true);
  }

  async function completeOnboardingShell() {
    // onboardingCommit already sets onboarding_complete = true atomically in the DB.
    // Treat the commit as the sole source of truth and only update local React state here.
    setNeedsOnboarding(false);
  }

  async function skipOnboardingShell() {
    setOnboardingBusy(true);
    setOnboardingError(null);
    try {
      await setOnboardingComplete(true);
      setNeedsOnboarding(false);
    } catch (error) {
      setOnboardingError(String(error));
    } finally {
      setOnboardingBusy(false);
    }
  }

  const zenCanvasStyle = {
    left:
      viewMode === "editor"
        ? "0px"
        : `${SIDEBAR_RAIL_WIDTH + (leftPaneExpanded || sidebarModalOpen ? leftPaneWidth : 0)}px`,
    right:
      viewMode === "editor"
        ? "0px"
        : `${SIDEBAR_RAIL_WIDTH + (rightPaneExpanded ? rightPaneWidth : 0)}px`,
  };

  return (
    <ErrorBoundary>
      <main
        className={`hybrid-shell ${leftPanePinned ? "left-pinned" : ""} ${rightPanePinned ? "right-pinned" : ""} ${sidebarModalOpen || editorModalOpen || spatialModalOpen || chatModalOpen ? "modal-open" : ""} ${leftResizing || rightResizing ? "is-resizing" : ""}`}
      >
        {!onboardingResolved ? (
          <section className="onboarding-shell">
            <div className="onboarding-card onboarding-loading-card">
              <h1>Loading MindVault...</h1>
              <p>Checking onboarding status.</p>
            </div>
          </section>
        ) : null}
        {onboardingResolved && needsOnboarding ? (
          <OnboardingShell
            onComplete={completeOnboardingShell}
            onSkip={skipOnboardingShell}
            busy={onboardingBusy}
            errorMessage={onboardingError}
          />
        ) : null}
        {onboardingResolved && needsOnboarding ? null : (
          <>
            <div className={styles.memoryBadgeContainer}>
              <span className={styles.memoryBadgeTitle}>Memory Agent</span>
              <button
                type="button"
                className={styles.memoryBadgePendingBadge}
                title="Memory Proposals"
                onClick={() => setIsDiffPanelOpen(true)}
              >
                <span>Proposals</span>
                {pendingProposalCount > 0 ? (
                  <span
                    style={{
                      marginLeft: "6px",
                      background: "#bc6c25",
                      color: "#fff",
                      borderRadius: "10px",
                      padding: "1px 6px",
                      fontSize: "0.65rem",
                      fontWeight: "bold",
                    }}
                  >
                    {pendingProposalCount}
                  </span>
                ) : (
                  <span
                    style={{
                      marginLeft: "6px",
                      opacity: 0.6,
                      fontSize: "0.65rem",
                    }}
                  >
                    0
                  </span>
                )}
              </button>
            </div>
            <div className="app-workspace">
              <section className="zen-canvas" onClick={onZenCanvasClick} style={zenCanvasStyle}>
                {/* Floating segment view toggle */}
                {viewMode !== "editor" && (
                  <div className="canvas-view-toggle-pill" onClick={(e) => e.stopPropagation()}>
                    <button
                      className={`canvas-view-toggle-btn ${viewMode === "chat" ? "active" : ""}`}
                      onClick={() => handleSetViewMode("chat")}
                    >
                      💬 Recall / Chat
                    </button>
                    <button
                      className={`canvas-view-toggle-btn ${viewMode === "spatial" ? "active" : ""}`}
                      onClick={() => handleSetViewMode("spatial")}
                    >
                      🕸️ Spatial Workspace
                    </button>
                  </div>
                )}
                {/* Decoupled chat panel, persists regardless of viewmode */}
                <div
                  style={{
                    position: "absolute",
                    inset: 0,
                    display: viewMode === "chat" ? "flex" : "none",
                    flexDirection: "column",
                    alignItems: "center",
                    pointerEvents: viewMode === "chat" ? "auto" : "none",
                    overflow: "hidden",
                  }}
                >
                  {activeSessionId && (
                    <ChatPanel
                      selectedNodeIds={scopeNodeIds}
                      scope={assemblerScope}
                      selectedVaultId={selectedVaultId}
                      onSelectVault={onSelectVault}
                      onOpenSettings={onOpenSettings}
                      isRedactedUnlocked={isRedactedUnlocked}
                      onModalToggle={setChatModalOpen}
                      onSelectNode={onSelectNode}
                      nodeRefreshKey={nodeRefreshKey}
                      visible={viewMode === "chat"}
                      onRefreshPendingCount={() => {
                        void countPendingChangesetItems()
                          .then(setPendingProposalCount)
                          .catch(console.error);
                      }}
                      activeSessionId={activeSessionId}
                      onActivateSession={setActiveSessionId}
                    />
                  )}
                </div>

                {viewMode === "editor" ? (
                  selectedNodeId && (
                    <NodeEditorExpanded
                      nodeId={selectedNodeId}
                      onClose={() => handleSetViewMode(previousViewMode)}
                      onSelectNode={onSelectNode}
                      isRedactedUnlocked={isRedactedUnlocked}
                      setIsRedactedUnlocked={setIsRedactedUnlocked}
                    />
                  )
                ) : viewMode === "spatial" ? (
                  <SpatialWorkspace
                    selectedVaultId={selectedVaultId}
                    selectedNodeId={selectedNodeId}
                    onSelectVault={onSelectVault}
                    onFocusVault={onFocusVault}
                    onSelectNode={onSelectNode}
                    refreshKey={vaultRefreshKey + nodeRefreshKey}
                    onVaultCreated={onVaultCreated}
                    onVaultDeleted={onVaultDeleted}
                    onVaultUpdated={onVaultUpdated}
                    onNodeCreated={onNodeCreated}
                    onNodeDeleted={onNodeDeleted}
                    onNodeUpdated={onNodeUpdated}
                    isRedactedUnlocked={isRedactedUnlocked}
                    setIsRedactedUnlocked={setIsRedactedUnlocked}
                    onSelectedVaultRequiresUnlockChange={setSelectedVaultRequiresUnlock}
                    onModalToggle={setSpatialModalOpen}
                    isLeftPanePinned={leftPanePinned}
                    onLeftPanePinChange={setLeftPanePinned}
                  />
                ) : null}
              </section>

              {viewMode !== "editor" && (
                <div
                  className={`pane-wrap left ${leftPaneExpanded || sidebarModalOpen ? "show" : ""}`}
                  style={{ width: `${leftPaneWidth}px` }}
                >
                  {leftPanelView === "history" ? (
                    <ChatHistoryPanel
                      activeSessionId={activeSessionId}
                      setActiveSessionId={setActiveSessionId}
                    />
                  ) : leftPanelView === "import" ? (
                    <ImportHub
                      onOpenImportChangeset={(changesetId) => {
                        setSelectedChangesetId(changesetId);
                        setIsDiffPanelOpen(true);
                      }}
                    />
                  ) : !selectedVaultId ? (
                    <VaultSidebar
                      selectedVaultId={selectedVaultId}
                      onSelectVault={onSelectVault}
                      onSelectNode={onSelectNode}
                      onVaultCreated={onVaultCreated}
                      onVaultDeleted={onVaultDeleted}
                      onOpenDashboard={onOpenDashboard}
                      onOpenSettings={onOpenSettings}
                      refreshKey={vaultRefreshKey}
                      isRedactedUnlocked={isRedactedUnlocked}
                      setIsRedactedUnlocked={setIsRedactedUnlocked}
                      onModalToggle={setSidebarModalOpen}
                    />
                  ) : (
                    <NodeList
                      selectedVaultId={selectedVaultId}
                      selectedNodeId={selectedNodeId}
                      onSelectNode={onSelectNode}
                      onSelectVault={onSelectVault}
                      onNodeCreated={onNodeCreated}
                      onVaultCreated={onVaultCreated}
                      onBack={() => {
                        onSelectVault(null);
                        setLeftPanePinned(true);
                      }}
                      refreshKey={nodeRefreshKey}
                      isRedactedUnlocked={isRedactedUnlocked}
                      onModalToggle={setSidebarModalOpen}
                    />
                  )}
                  {/* Left Resize Handle */}
                  <div
                    className={`resize-handle left-handle ${leftResizing ? "active" : ""}`}
                    onMouseDown={handleLeftResizeMouseDown}
                  />
                </div>
              )}

              {viewMode !== "editor" && (
                <div
                  className={`pane-wrap right ${rightPaneExpanded ? "show" : ""}`}
                  style={{ width: `${rightPaneWidth}px` }}
                >
                  {rightPanelView === "dashboard" ? (
                    <PriorityDashboard
                      refreshKey={nodeRefreshKey}
                      isRedactedUnlocked={isRedactedUnlocked}
                    />
                  ) : rightPanelView === "settings" ? (
                    <LlmSettings />
                  ) : (
                    <div className="right-pane-stack">
                      <ScopeIndicator
                        selectedNodeIds={scopeNodeIds}
                        scope={assemblerScope}
                        onScopeChange={setAssemblerScope}
                      />
                      <ActiveMemoryPanel
                        selectedNodeIds={scopeNodeIds}
                        isRedactedUnlocked={isRedactedUnlocked}
                      />
                      <NodeEditor
                        selectedNodeId={selectedNodeId}
                        onNodeDeleted={onNodeDeleted}
                        onSaveSuccess={() => setNodeRefreshKey((value) => value + 1)}
                        refreshKey={nodeRefreshKey}
                        isRedactedUnlocked={isRedactedUnlocked}
                        setIsRedactedUnlocked={setIsRedactedUnlocked}
                        onModalToggle={setEditorModalOpen}
                        onSelectNode={onSelectNode}
                        onExpand={() => handleSetViewMode("editor")}
                      />
                    </div>
                  )}
                  {/* Right Resize Handle */}
                  <div
                    className={`resize-handle right-handle ${rightResizing ? "active" : ""}`}
                    onMouseDown={handleRightResizeMouseDown}
                  />
                </div>
              )}

              {/* Left Sidebar Icon Rail */}
              {/* Left Sidebar Icon Rail */}
              {viewMode !== "editor" && (
                <nav className="icon-rail icon-rail-left" aria-label="Left panel views">
                  <button
                    type="button"
                    className={`icon-rail-btn ${leftPaneExpanded && leftPanelView === "browse" ? "active" : ""}`}
                    onClick={() => onLeftRailToggle("browse")}
                    disabled={selectedVaultRequiresUnlock}
                    title={
                      selectedVaultRequiresUnlock
                        ? "Unlock redacted vault first"
                        : leftPaneExpanded && leftPanelView === "browse"
                          ? "Collapse browser panel"
                          : "Open browser panel"
                    }
                    aria-label="Toggle vault and node browser panel"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <rect x="3" y="3" width="18" height="18" rx="2" />
                      <path d="M9 3v18" />
                    </svg>
                  </button>
                  <button
                    type="button"
                    className={`icon-rail-btn ${leftPaneExpanded && leftPanelView === "history" ? "active" : ""}`}
                    onClick={() => onLeftRailToggle("history")}
                    title="Chat History"
                    aria-label="Toggle chat history panel"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <circle cx="12" cy="12" r="9" />
                      <path d="M12 7v5l3 3" />
                    </svg>
                  </button>
                  <button
                    type="button"
                    className={`icon-rail-btn ${leftPaneExpanded && leftPanelView === "import" ? "active" : ""}`}
                    onClick={() => onLeftRailToggle("import")}
                    title="Import"
                    aria-label="Toggle import panel"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <path d="M12 3v12" />
                      <path d="M7 10l5 5 5-5" />
                      <path d="M4 19h16" />
                    </svg>
                  </button>
                </nav>
              )}

              {/* Right Sidebar Icon Rail */}
              {viewMode !== "editor" && (
                <nav className="icon-rail icon-rail-right" aria-label="Right panel views">
                  <button
                    type="button"
                    className={`icon-rail-btn ${rightPanePinned && rightPanelView === "notes" ? "active" : ""}`}
                    onClick={() => onRightRailToggle("notes")}
                    title="Notes & Memory"
                    aria-label="Toggle notes and memory panel"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                      <path d="M14 2v6h6" />
                      <path d="M9 13h6" />
                      <path d="M9 17h6" />
                    </svg>
                  </button>
                  <button
                    type="button"
                    className={`icon-rail-btn ${rightPanePinned && rightPanelView === "dashboard" ? "active" : ""}`}
                    onClick={() => onRightRailToggle("dashboard")}
                    title="Priority Dashboard"
                    aria-label="Toggle priority dashboard panel"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <rect x="3" y="12" width="4" height="9" />
                      <rect x="10" y="6" width="4" height="15" />
                      <rect x="17" y="9" width="4" height="12" />
                    </svg>
                  </button>
                  <button
                    type="button"
                    className={`icon-rail-btn ${rightPanePinned && rightPanelView === "settings" ? "active" : ""}`}
                    onClick={() => onRightRailToggle("settings")}
                    title="Settings"
                    aria-label="Toggle settings panel"
                  >
                    <svg
                      width="18"
                      height="18"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <circle cx="12" cy="12" r="3" />
                      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                  </button>
                </nav>
              )}
            </div>
            {isDiffPanelOpen && (
              <DiffPanel
                onClose={() => setIsDiffPanelOpen(false)}
                activeChangesetId={selectedChangesetId}
                onSelectChangeset={setSelectedChangesetId}
                onRefreshPendingCount={() => {
                  void countPendingChangesetItems()
                    .then(setPendingProposalCount)
                    .catch(console.error);
                }}
              />
            )}
          </>
        )}
      </main>
    </ErrorBoundary>
  );
}

export default App;
