import { useEffect, useMemo, useState, type MouseEvent } from "react";
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
import "./style/MonoStyles.css";

function App() {
  const [onboardingResolved, setOnboardingResolved] = useState<boolean>(false);
  const [needsOnboarding, setNeedsOnboarding] = useState<boolean>(false);
  const [onboardingBusy, setOnboardingBusy] = useState<boolean>(false);
  const [onboardingError, setOnboardingError] = useState<string | null>(null);

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
  const [showDashboard, setShowDashboard] = useState<boolean>(false);
  const [showSettings, setShowSettings] = useState<boolean>(false);
  const [sidebarModalOpen, setSidebarModalOpen] = useState<boolean>(false);
  const [editorModalOpen, setEditorModalOpen] = useState<boolean>(false);
  const [spatialModalOpen, setSpatialModalOpen] = useState<boolean>(false);
  const [viewMode, setViewMode] = useState<"chat" | "spatial">("chat");
  const leftPaneExpanded = leftPanePinned && !selectedVaultRequiresUnlock;
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

  const handleLeftResizeMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setLeftResizing(true);
    let finalWidth = leftPaneWidth;
    const handleMouseMove = (moveEvent: globalThis.MouseEvent) => {
      const maxWidth = window.innerWidth * 0.4;
      finalWidth = Math.max(200, Math.min(maxWidth, moveEvent.clientX));
      setLeftPaneWidth(finalWidth);
    };
    const handleMouseUp = () => {
      setLeftResizing(false);
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      localStorage.setItem("sidebar_left_width", String(finalWidth));
      void setSetting("sidebar_left_width", String(finalWidth)).catch(() => {});
    };
    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
  };

  const handleRightResizeMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setRightResizing(true);
    let finalWidth = rightPaneWidth;
    const handleMouseMove = (moveEvent: globalThis.MouseEvent) => {
      const maxWidth = window.innerWidth * 0.4;
      finalWidth = Math.max(200, Math.min(maxWidth, window.innerWidth - moveEvent.clientX));
      setRightPaneWidth(finalWidth);
    };
    const handleMouseUp = () => {
      setRightResizing(false);
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      localStorage.setItem("sidebar_right_width", String(finalWidth));
      void setSetting("sidebar_right_width", String(finalWidth)).catch(() => {});
    };
    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
  };

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
    setShowDashboard(false);
    setShowSettings(false);
    setLeftPanePinned(Boolean(vaultId));
    setNodeRefreshKey((value) => value + 1);
  }

  function onFocusVault(vaultId: string | null) {
    setSelectedVaultId(vaultId);
    setSelectedNodeId(null);
    setShowDashboard(false);
    setShowSettings(false);
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
    setShowDashboard(false);
    setShowSettings(false);
    setRightPanePinned(true);
  }

  function onNodeCreated(nodeId: string) {
    setSelectedNodeId(nodeId);
    setShowDashboard(false);
    setShowSettings(false);
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
    setShowDashboard(true);
    setShowSettings(false);
    setRightPanePinned(true);
  }

  function onOpenSettings() {
    setSelectedNodeId(null);
    setShowDashboard(false);
    setShowSettings(true);
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

  const leftToggleStyle = {
    left: leftPaneExpanded || sidebarModalOpen ? `${leftPaneWidth + 16}px` : "16px",
    zIndex: 1005,
  };

  const rightToggleStyle = {
    right: rightPaneExpanded ? `${rightPaneWidth + 16}px` : "16px",
    zIndex: 1005,
  };

  const zenCanvasStyle = {
    left: leftPaneExpanded || sidebarModalOpen ? `${leftPaneWidth}px` : "0px",
    right: rightPaneExpanded ? `${rightPaneWidth}px` : "0px",
  };

  return (
    <ErrorBoundary>
      <main
        className={`hybrid-shell ${leftPanePinned ? "left-pinned" : ""} ${rightPanePinned ? "right-pinned" : ""} ${sidebarModalOpen || editorModalOpen || spatialModalOpen ? "modal-open" : ""} ${leftResizing || rightResizing ? "is-resizing" : ""}`}
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
            <section className="zen-canvas" onClick={onZenCanvasClick} style={zenCanvasStyle}>
              {/* Floating segment view toggle */}
              <div className="canvas-view-toggle-pill" onClick={(e) => e.stopPropagation()}>
                <button
                  className={`canvas-view-toggle-btn ${viewMode === "chat" ? "active" : ""}`}
                  onClick={() => setViewMode("chat")}
                >
                  💬 Recall / Chat
                </button>
                <button
                  className={`canvas-view-toggle-btn ${viewMode === "spatial" ? "active" : ""}`}
                  onClick={() => setViewMode("spatial")}
                >
                  🕸️ Spatial Workspace
                </button>
              </div>

              {viewMode === "spatial" ? (
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
              ) : (
                <ChatPanel
                  selectedNodeIds={scopeNodeIds}
                  scope={assemblerScope}
                  selectedVaultId={selectedVaultId}
                  onSelectVault={onSelectVault}
                  onOpenSettings={onOpenSettings}
                  isRedactedUnlocked={isRedactedUnlocked}
                />
              )}
            </section>

            <div
              className={`pane-wrap left ${leftPaneExpanded || sidebarModalOpen ? "show" : ""}`}
              style={{ width: `${leftPaneWidth}px` }}
            >
              {!selectedVaultId ? (
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
                  onBack={() => onSelectVault(null)}
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

            <div
              className={`pane-wrap right ${rightPaneExpanded ? "show" : ""}`}
              style={{ width: `${rightPaneWidth}px` }}
            >
              {showDashboard ? (
                <PriorityDashboard
                  refreshKey={nodeRefreshKey}
                  isRedactedUnlocked={isRedactedUnlocked}
                />
              ) : showSettings ? (
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
                  />
                </div>
              )}
              {/* Right Resize Handle */}
              <div
                className={`resize-handle right-handle ${rightResizing ? "active" : ""}`}
                onMouseDown={handleRightResizeMouseDown}
              />
            </div>

            {/* Left Sidebar Toggle Button */}
            <button
              className={`sidebar-toggle-btn left ${leftPaneExpanded || sidebarModalOpen ? "open" : ""}`}
              onClick={() => {
                if (selectedVaultRequiresUnlock) {
                  return;
                }
                setLeftPanePinned(!leftPanePinned);
              }}
              style={leftToggleStyle}
              title={
                selectedVaultRequiresUnlock
                  ? "Unlock redacted vault first"
                  : leftPanePinned
                    ? "Collapse Left Panel"
                    : "Pin Left Panel"
              }
              aria-label="Toggle left panel"
              disabled={selectedVaultRequiresUnlock}
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
                {leftPaneExpanded ? (
                  <polygon points="16,9 12,12 16,15" fill="currentColor" />
                ) : (
                  <polygon points="12,9 16,12 12,15" fill="currentColor" />
                )}
              </svg>
            </button>

            {/* Right Sidebar Toggle Button */}
            <button
              className={`sidebar-toggle-btn right ${rightPaneExpanded ? "open" : ""}`}
              onClick={() => setRightPanePinned(!rightPanePinned)}
              style={rightToggleStyle}
              title={rightPanePinned ? "Collapse Right Panel" : "Pin Right Panel"}
              aria-label="Toggle right panel"
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
                <path d="M15 3v18" />
                {rightPaneExpanded ? (
                  <polygon points="8,9 12,12 8,15" fill="currentColor" />
                ) : (
                  <polygon points="12,9 8,12 12,15" fill="currentColor" />
                )}
              </svg>
            </button>
          </>
        )}
      </main>
    </ErrorBoundary>
  );
}

export default App;
