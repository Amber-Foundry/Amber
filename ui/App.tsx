import { useEffect, useMemo, useState, type MouseEvent } from "react";
import ErrorBoundary from "./components/ErrorBoundary";
import NodeEditor from "./components/NodeEditor";
import NodeList from "./components/NodeList";
import VaultSidebar from "./components/VaultSidebar";
import DecayDashboard from "./components/DecayDashboard";
import LlmSettings from "./components/LlmSettings";
import ScopeIndicator from "./components/ScopeIndicator";
import ChatPanel from "./components/ChatPanel";
import ActiveMemoryPanel from "./components/ActiveMemoryPanel";
import type { ContextAssemblerScope } from "./constants/contextBudget";
import { refreshAllDecayScores } from "./services/nodes";
import "./App.css";

function App() {
  useEffect(() => {
    void refreshAllDecayScores().catch(() => {});
  }, []);
  const [selectedVaultId, setSelectedVaultId] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [leftPaneVisible, setLeftPaneVisible] = useState<boolean>(false);
  const [rightPaneVisible, setRightPaneVisible] = useState<boolean>(false);
  const [vaultRefreshKey, setVaultRefreshKey] = useState<number>(0);
  const [nodeRefreshKey, setNodeRefreshKey] = useState<number>(0);
  const [isRedactedUnlocked, setIsRedactedUnlocked] = useState<boolean>(false);
  const [showDashboard, setShowDashboard] = useState<boolean>(false);
  const [showSettings, setShowSettings] = useState<boolean>(false);
  const leftPaneExpanded = leftPaneVisible;
  const scopeNodeIds = useMemo(() => (selectedNodeId ? [selectedNodeId] : []), [selectedNodeId]);
  const [assemblerScope, setAssemblerScope] = useState<ContextAssemblerScope>("local");

  function closeAllPanes() {
    setLeftPaneVisible(false);
    setRightPaneVisible(false);
  }

  function onZenCanvasClick(event: MouseEvent<HTMLElement>) {
    if (event.target === event.currentTarget) {
      closeAllPanes();
    }
  }

  function onSelectVault(vaultId: string) {
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
      setRightPaneVisible(false);
    }
    setVaultRefreshKey((value) => value + 1);
    setNodeRefreshKey((value) => value + 1);
  }

  function onSelectNode(nodeId: string) {
    setSelectedNodeId(nodeId);
    setShowDashboard(false);
    setShowSettings(false);
    setRightPaneVisible(true);
  }

  function onNodeCreated(nodeId: string) {
    setSelectedNodeId(nodeId);
    setShowDashboard(false);
    setShowSettings(false);
    setRightPaneVisible(true);
    setNodeRefreshKey((value) => value + 1);
  }

  function onNodeDeleted(nodeId: string) {
    if (selectedNodeId === nodeId) {
      setSelectedNodeId(null);
      setRightPaneVisible(false);
    }
    setNodeRefreshKey((value) => value + 1);
  }

  function onOpenDashboard() {
    setSelectedNodeId(null);
    setShowDashboard(true);
    setShowSettings(false);
    setRightPaneVisible(true);
  }

  function onOpenSettings() {
    setSelectedNodeId(null);
    setShowDashboard(false);
    setShowSettings(true);
    setRightPaneVisible(true);
  }

  return (
    <ErrorBoundary>
      <main className="hybrid-shell">
        <section className="zen-canvas" onClick={onZenCanvasClick}>
          <ChatPanel selectedNodeIds={scopeNodeIds} scope={assemblerScope} />
        </section>

        <div
          className={`hover-zone left-zone ${leftPaneExpanded ? "expanded" : ""}`}
          onMouseEnter={() => setLeftPaneVisible(true)}
          onMouseLeave={() => setLeftPaneVisible(false)}
        >
          <div className="edge-trigger left" />
          <div className={`pane-wrap left ${leftPaneExpanded ? "show" : ""}`}>
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
              />
            ) : (
              <NodeList
                selectedVaultId={selectedVaultId}
                selectedNodeId={selectedNodeId}
                onSelectNode={onSelectNode}
                onNodeCreated={onNodeCreated}
                onBack={() => {
                  setSelectedVaultId(null);
                  setSelectedNodeId(null);
                }}
                refreshKey={nodeRefreshKey}
              />
            )}
          </div>
        </div>

        <div
          className={`hover-zone right-zone ${rightPaneVisible ? "expanded" : ""}`}
          onMouseEnter={() => setRightPaneVisible(true)}
          onMouseLeave={() => setRightPaneVisible(false)}
        >
          <div className={`pane-wrap right ${rightPaneVisible ? "show" : ""}`}>
            {showDashboard ? (
              <DecayDashboard refreshKey={nodeRefreshKey} />
            ) : showSettings ? (
              <LlmSettings />
            ) : (
              <div className="right-pane-stack">
                <ScopeIndicator
                  selectedNodeIds={scopeNodeIds}
                  scope={assemblerScope}
                  onScopeChange={setAssemblerScope}
                />
                <ActiveMemoryPanel selectedNodeIds={scopeNodeIds} />
                <NodeEditor
                  selectedNodeId={selectedNodeId}
                  onNodeDeleted={onNodeDeleted}
                  onSaveSuccess={() => setNodeRefreshKey((value) => value + 1)}
                  refreshKey={nodeRefreshKey}
                  isRedactedUnlocked={isRedactedUnlocked}
                  setIsRedactedUnlocked={setIsRedactedUnlocked}
                />
              </div>
            )}
          </div>
          <div className="edge-trigger right" />
        </div>
      </main>
    </ErrorBoundary>
  );
}

export default App;
