import { useEffect, useMemo, useState } from "react";
import type { Node, Vault } from "../ipc";
import { AppError } from "../services/ipcResult";
import { getNode } from "../services/nodes";
import { listVaults } from "../services/vaults";
import {
  getEffectivePrivacy,
  getPrivacyDisplayLabel,
  getVaultEffectivePrivacy,
} from "../utils/privacy";
import { DoorIcon } from "./icons";

type ActiveMemoryPanelProps = {
  selectedNodeIds: string[];
  isRedactedUnlocked: boolean;
};

type NodePriorityMeta = {
  profile: string;
  accessCount30Active: number;
};

function parsePriorityMeta(priorityJson: string): NodePriorityMeta {
  try {
    const raw = JSON.parse(priorityJson) as Record<string, unknown>;
    const profile = typeof raw.profile === "string" ? raw.profile : "standard";
    const accessCount30Active =
      typeof raw.access_count_30active === "number" ? raw.access_count_30active : 0;
    return { profile, accessCount30Active };
  } catch {
    return { profile: "standard", accessCount30Active: 0 };
  }
}

function shortLabel(node: Node): string {
  const primary = node.title.trim() || node.summary.trim() || node.detail?.trim() || node.id;
  return primary.length > 64 ? `${primary.slice(0, 63)}...` : primary;
}

function ActiveMemoryPanel({ selectedNodeIds, isRedactedUnlocked }: ActiveMemoryPanelProps) {
  const [loadedNodes, setLoadedNodes] = useState<Node[]>([]);
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [status, setStatus] = useState("");

  useEffect(() => {
    let active = true;

    void (async () => {
      if (selectedNodeIds.length === 0) {
        if (!active) {
          return;
        }
        setLoadedNodes([]);
        setStatus("");
        return;
      }

      try {
        const nodes = await Promise.all(selectedNodeIds.map((id) => getNode(id)));
        if (!active) {
          return;
        }
        setLoadedNodes(nodes.filter((node): node is Node => node !== null));
        setStatus("");
      } catch (error) {
        if (!active) {
          return;
        }
        setLoadedNodes([]);
        if (error instanceof AppError) {
          setStatus(error.message);
        } else {
          setStatus("Unable to load active memory nodes.");
        }
      }
    })();

    return () => {
      active = false;
    };
  }, [selectedNodeIds, isRedactedUnlocked]);

  useEffect(() => {
    let active = true;

    void (async () => {
      try {
        const data = await listVaults();
        if (!active) {
          return;
        }
        setVaults(data);
      } catch {
        if (!active) {
          return;
        }
        setVaults([]);
      }
    })();

    return () => {
      active = false;
    };
  }, [isRedactedUnlocked]);

  const vaultById = useMemo(() => {
    const map: Record<string, (typeof vaults)[number]> = {};
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

  const nodeCards = loadedNodes.map((node) => {
    const meta = parsePriorityMeta(node.priority);
    const effectiveTier = getNodeEffectivePrivacy(node);
    return (
      <article key={node.id} className="active-memory-item">
        <div className="active-memory-item-top">
          <strong>
            {getPrivacyDisplayLabel(shortLabel(node), effectiveTier, isRedactedUnlocked)}
          </strong>
          <span className="active-memory-door-stub">
            <DoorIcon size={14} /> 0
          </span>
        </div>
        <div className="active-memory-item-meta">
          <span className={`active-memory-rate-badge rate-${meta.profile}`}>{meta.profile}</span>
          <span className="active-memory-usage">activity {meta.accessCount30Active}</span>
        </div>
      </article>
    );
  });

  return (
    <section className="active-memory-panel">
      <header className="active-memory-header">
        <h3>Active Memory</h3>
      </header>
      {loadedNodes.length === 0 ? (
        <p className="active-memory-empty">No nodes selected for context.</p>
      ) : (
        <div className="active-memory-list">{nodeCards}</div>
      )}
      {status ? <p className="active-memory-status">{status}</p> : null}
    </section>
  );
}

export default ActiveMemoryPanel;
