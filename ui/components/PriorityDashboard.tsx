import { useEffect, useMemo, useRef, useState } from "react";
import {
  getAllNodes,
  getNode,
  optimizeAllPriorityProfiles,
  refreshAllPriorityScores,
  updateNode,
} from "../services/nodes";
import type { Node } from "../ipc";
import { AppError } from "../services/ipcResult";
import { listVaults } from "../services/vaults";
import {
  getEffectivePrivacy,
  getPrivacyDisplayLabel,
  getVaultEffectivePrivacy,
} from "../utils/privacy";
import {
  parsePriorityJson,
  getPriorityScore,
  getPriorityProfile,
  isFrozen,
  getAccessCount,
} from "../utils/priority";
import PriorityBar from "./PriorityBar";
import { BrainIcon, SparklesIcon, SnowflakeIcon } from "./icons";

type PriorityDashboardProps = {
  refreshKey: number;
  isRedactedUnlocked: boolean;
};

function PriorityDashboard({ refreshKey, isRedactedUnlocked }: PriorityDashboardProps) {
  const [nodes, setNodes] = useState<Node[]>([]);
  const [vaults, setVaults] = useState<Awaited<ReturnType<typeof listVaults>>>([]);
  const [status, setStatus] = useState("");
  const [profileOverrides, setProfileOverrides] = useState<Record<string, string>>({});
  const [frozenOverrides, setFrozenOverrides] = useState<Record<string, boolean>>({});
  const [isOptimizing, setIsOptimizing] = useState(false);
  const saveTimersRef = useRef<Record<string, number>>({});

  async function fetchNodes() {
    try {
      await refreshAllPriorityScores();
      const all = await getAllNodes();
      setNodes(all);
      const allVaults = await listVaults();
      setVaults(allVaults);
      const profiles: Record<string, string> = {};
      const frozen: Record<string, boolean> = {};
      for (const node of all) {
        profiles[node.id] = getPriorityProfile(node);
        frozen[node.id] = isFrozen(node);
      }
      setProfileOverrides(profiles);
      setFrozenOverrides(frozen);
      setStatus("");
    } catch (err) {
      if (err instanceof AppError) {
        setStatus(err.message);
      } else {
        setStatus("Failed to load nodes.");
      }
    }
  }

  const vaultById = useMemo(() => {
    const map: Record<string, Awaited<ReturnType<typeof listVaults>>[number]> = {};
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

  function getNodeEffectivePrivacy(node: Node): string {
    const containerId = node.subVaultId ?? node.vaultId;
    const containerTier =
      vaultEffectivePrivacyById[containerId] ?? getVaultEffectivePrivacy(containerId, vaultById);
    return getEffectivePrivacy(node.privacyTier, null, containerTier);
  }

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void fetchNodes();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [refreshKey, isRedactedUnlocked]);

  useEffect(
    () => () => {
      for (const id of Object.keys(saveTimersRef.current)) {
        window.clearTimeout(saveTimersRef.current[id]);
      }
    },
    []
  );

  const sorted = useMemo(() => {
    return [...nodes].sort((a, b) => getPriorityScore(b) - getPriorityScore(a));
  }, [nodes]);

  function onChangeProfile(node: Node, nextProfile: string) {
    setProfileOverrides((prev) => ({ ...prev, [node.id]: nextProfile }));

    if (saveTimersRef.current[node.id]) {
      window.clearTimeout(saveTimersRef.current[node.id]);
    }

    saveTimersRef.current[node.id] = window.setTimeout(() => {
      void (async () => {
        try {
          const freshNode = await getNode(node.id);
          if (!freshNode) return;
          const priorityObj = parsePriorityJson(freshNode.priority);
          priorityObj.profile = nextProfile;
          priorityObj.pinned = nextProfile === "pinned";
          await updateNode({
            id: node.id,
            priority: JSON.stringify(priorityObj),
          });
          await refreshAllPriorityScores();
          const freshNodes = await getAllNodes();
          setNodes(freshNodes);
        } catch (err) {
          if (err instanceof AppError) {
            setStatus(err.message);
          } else {
            setStatus("Failed to update priority profile.");
          }
        }
      })();
    }, 600);
  }

  function onToggleFreeze(node: Node) {
    const currentFrozen = frozenOverrides[node.id] ?? isFrozen(node);
    const nextFrozen = !currentFrozen;
    setFrozenOverrides((prev) => ({ ...prev, [node.id]: nextFrozen }));

    void (async () => {
      try {
        const freshNode = await getNode(node.id);
        if (!freshNode) return;
        const priorityObj = parsePriorityJson(freshNode.priority);
        priorityObj.frozen = nextFrozen;
        await updateNode({
          id: node.id,
          priority: JSON.stringify(priorityObj),
        });
        const freshNodes = await getAllNodes();
        setNodes(freshNodes);
      } catch (err) {
        if (err instanceof AppError) {
          setStatus(err.message);
        } else {
          setStatus("Failed to toggle freeze.");
        }
      }
    })();
  }

  async function onAutoOptimize() {
    setIsOptimizing(true);
    try {
      await optimizeAllPriorityProfiles();
      await refreshAllPriorityScores();
      const freshNodes = await getAllNodes();
      setNodes(freshNodes);
      const profiles: Record<string, string> = {};
      const frozen: Record<string, boolean> = {};
      for (const node of freshNodes) {
        profiles[node.id] = getPriorityProfile(node);
        frozen[node.id] = isFrozen(node);
      }
      setProfileOverrides(profiles);
      setFrozenOverrides(frozen);
      setStatus("");
    } catch (err) {
      if (err instanceof AppError) {
        setStatus(err.message);
      } else {
        setStatus("Failed to auto-optimize.");
      }
    }
    setIsOptimizing(false);
  }

  return (
    <aside className="pane pane-right">
      <div className="pane-header">
        <h3>
          <BrainIcon size={16} /> Active Memory
        </h3>
        <button
          type="button"
          className="optimize-button"
          disabled={isOptimizing}
          onClick={() => void onAutoOptimize()}
        >
          {isOptimizing ? (
            "Optimizing…"
          ) : (
            <>
              <SparklesIcon size={14} /> Auto-Optimize
            </>
          )}
        </button>
      </div>
      {status && <p className="pane-error">{status}</p>}
      {sorted.length === 0 && <p className="pane-empty">No nodes yet.</p>}
      <div className="dashboard-list">
        {sorted.length > 0 && (
          <div className="dashboard-header">
            <span>Node</span>
            <span>Priority</span>
            <span>Activity</span>
            <span>Speed</span>
            <span></span>
          </div>
        )}
        {sorted.map((node) => {
          const score = getPriorityScore(node);
          const count30 = getAccessCount(node, "access_count_30active") || 0;
          const count90 = getAccessCount(node, "access_count_90active") || 0;
          const currentProfile = profileOverrides[node.id] ?? getPriorityProfile(node);
          const frozen = frozenOverrides[node.id] ?? isFrozen(node);
          const effectiveTier = getNodeEffectivePrivacy(node);
          return (
            <div key={node.id} className={`dashboard-row ${frozen ? "dashboard-frozen" : ""}`}>
              <div className="dashboard-title">
                {getPrivacyDisplayLabel(node.title, effectiveTier, isRedactedUnlocked)}
              </div>
              <PriorityBar score={score} />
              <span
                className="dashboard-activity"
                title="Touches: last 30 sessions · last 90 sessions"
              >
                {count30} · {count90}
              </span>
              <select
                className="dashboard-rate"
                value={currentProfile}
                onChange={(e) => onChangeProfile(node, e.target.value)}
              >
                <option value="standard">Standard</option>
                <option value="slow">Slow</option>
                <option value="fast">Fast</option>
                <option value="pinned">Pinned</option>
              </select>
              <button
                type="button"
                className={`freeze-toggle ${frozen ? "frozen" : ""}`}
                onClick={() => onToggleFreeze(node)}
                title={
                  frozen ? "Unfreeze — allow auto-optimize" : "Freeze — protect from auto-optimize"
                }
              >
                <SnowflakeIcon size={14} />
              </button>
            </div>
          );
        })}
      </div>
    </aside>
  );
}

export default PriorityDashboard;
