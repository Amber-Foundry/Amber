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

export function clearNodesCache(): void {
  cachedNodes = null;
  cachedUnlockState = null;
}

export async function createNode(input: NodeCreateInput): Promise<Node> {
  clearNodesCache();
  return unwrapIpcResult(nodeCreate(input));
}

export async function getNode(nodeId: string): Promise<Node | null> {
  return unwrapIpcResult(nodeGet(nodeId));
}

export async function getNodes(isRedactedUnlocked?: boolean): Promise<Node[]> {
  if (isRedactedUnlocked !== undefined && cachedUnlockState !== isRedactedUnlocked) {
    cachedNodes = null;
    cachedUnlockState = isRedactedUnlocked;
  }
  if (cachedNodes) {
    return cachedNodes;
  }
  const nodes = await unwrapIpcResult(nodeList());
  cachedNodes = nodes;
  return nodes;
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
  isRedactedUnlocked: boolean
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
      isRedactedUnlocked
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
