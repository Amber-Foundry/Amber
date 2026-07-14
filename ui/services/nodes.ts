import {
  chatWithLlm,
  priorityOptimizeAll,
  priorityRefreshAll,
  debugAssembleContext,
  listLlmModels,
  nodeCreate,
  nodeDelete,
  nodeGet,
  nodeList,
  nodeTouch,
  nodeUpdate,
  type Node,
  type NodeCreateInput,
  type NodeUpdateInput,
} from "../ipc";
import { unwrapIpcResult } from "./ipcResult";

let cachedNodes: Node[] | null = null;
let cachedUnlockState: boolean | null = null;
let pendingNodesPromise: Promise<Node[]> | null = null;

export function clearNodesCache(): void {
  cachedNodes = null;
  cachedUnlockState = null;
  pendingNodesPromise = null;
}

export async function createNode(input: NodeCreateInput): Promise<Node> {
  clearNodesCache();
  return unwrapIpcResult(nodeCreate(input));
}

export async function getNode(nodeId: string): Promise<Node | null> {
  return unwrapIpcResult(nodeGet(nodeId));
}

export async function getNodes(isRedactedUnlocked?: boolean): Promise<Node[]> {
  const requestedState = isRedactedUnlocked !== undefined ? isRedactedUnlocked : null;
  if (cachedUnlockState !== requestedState) {
    cachedNodes = null;
    pendingNodesPromise = null;
    cachedUnlockState = requestedState;
  }

  if (isRedactedUnlocked === undefined) {
    if (pendingNodesPromise) {
      return pendingNodesPromise;
    }
    clearNodesCache();
    const promise = unwrapIpcResult(nodeList()).finally(() => {
      if (pendingNodesPromise === promise) {
        pendingNodesPromise = null;
      }
    });
    pendingNodesPromise = promise;
    return promise;
  }

  if (cachedNodes) {
    return cachedNodes;
  }
  if (pendingNodesPromise) {
    return pendingNodesPromise;
  }

  const promise = unwrapIpcResult(nodeList()).then(
    (nodes) => {
      cachedNodes = nodes;
      pendingNodesPromise = null;
      return nodes;
    },
    (error) => {
      pendingNodesPromise = null;
      throw error;
    }
  );
  pendingNodesPromise = promise;
  return promise;
}

export async function getAllNodes(isRedactedUnlocked?: boolean): Promise<Node[]> {
  return getNodes(isRedactedUnlocked);
}

export async function updateNode(input: NodeUpdateInput): Promise<Node> {
  clearNodesCache();
  return unwrapIpcResult(nodeUpdate(input));
}

export async function deleteNode(nodeId: string): Promise<boolean> {
  clearNodesCache();
  return unwrapIpcResult(nodeDelete(nodeId));
}

export async function touchNode(nodeId: string): Promise<boolean> {
  const result = await unwrapIpcResult(nodeTouch(nodeId));
  clearNodesCache();
  return result;
}

export async function refreshAllPriorityScores(): Promise<number> {
  clearNodesCache();
  return unwrapIpcResult(priorityRefreshAll());
}

export async function optimizeAllPriorityProfiles(): Promise<number> {
  clearNodesCache();
  return unwrapIpcResult(priorityOptimizeAll());
}

export async function debugBuildContext(nodeIds: string[], scope: string): Promise<string> {
  return unwrapIpcResult(debugAssembleContext(nodeIds, scope));
}

export async function getLlmModels(provider: string, endpoint: string): Promise<string[]> {
  return unwrapIpcResult(listLlmModels(provider, endpoint));
}

export async function chatWithScope(
  nodeIds: string[],
  scope: string,
  provider: string,
  endpoint: string,
  model: string,
  userPrompt: string,
  chartsEnabled: boolean,
  isRedactedUnlocked: boolean,
  sessionId: string,
  attachedDocument?: string | null,
  maxAssemblerTokens?: number | null
): Promise<string> {
  const result = await unwrapIpcResult(
    chatWithLlm(
      nodeIds,
      scope,
      provider,
      endpoint,
      model,
      userPrompt,
      chartsEnabled,
      isRedactedUnlocked,
      sessionId,
      attachedDocument,
      maxAssemblerTokens
    )
  );
  clearNodesCache();
  return result;
}

export async function searchNodes(query: string): Promise<Node[]> {
  const nodes = await getAllNodes();
  if (!query) return nodes;
  const lowerQuery = query.toLowerCase();
  return nodes.filter(
    (node) =>
      node.title.toLowerCase().includes(lowerQuery) ||
      (node.summary && node.summary.toLowerCase().includes(lowerQuery))
  );
}
