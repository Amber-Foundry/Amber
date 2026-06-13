import {
  memoryExtract,
  memoryExtractIfReady,
  type Changeset,
  changesetCountPending,
  changesetListPending,
  changesetListItems,
  type ChangesetItem,
  changesetCommit,
  type ChangesetCommitInput,
  changesetListResolved,
  debugSeedChangeset,
  memoryExtractForce,
} from "../ipc";
import { clearNodesCache } from "./nodes";
import { unwrapIpcResult } from "./ipcResult";

/**
 * Extracts proposed memory changesets from the current chat history.
 */
export async function extractMemory(
  provider: string,
  endpoint: string,
  model: string
): Promise<Changeset> {
  const result = await unwrapIpcResult(memoryExtract(provider, endpoint, model));
  clearNodesCache();
  return result;
}

/**
 * Checks triggers and extracts proposed memory changesets if ready (messages >= 6 and debounce passed).
 */
export async function extractMemoryIfReady(
  provider: string,
  endpoint: string,
  model: string
): Promise<Changeset | null> {
  const result = await unwrapIpcResult(memoryExtractIfReady(provider, endpoint, model));
  if (result) {
    clearNodesCache();
  }
  return result;
}

/**
 * Forces an immediate memory extraction, bypassing the debounce gate.
 * Used by the manual "Extract Now" chat toolbar button.
 */
export async function extractMemoryForce(
  provider: string,
  endpoint: string,
  model: string
): Promise<Changeset> {
  const result = await unwrapIpcResult(memoryExtractForce(provider, endpoint, model));
  clearNodesCache();
  return result;
}

/**
 * Counts total pending changeset items.
 */
export async function countPendingChangesetItems(): Promise<number> {
  return unwrapIpcResult(changesetCountPending());
}

/**
 * Lists all pending changesets.
 */
export async function listPendingChangesets(): Promise<Changeset[]> {
  return unwrapIpcResult(changesetListPending());
}

/**
 * Lists all items belonging to a specific changeset.
 */
export async function listChangesetItems(changesetId: string): Promise<ChangesetItem[]> {
  return unwrapIpcResult(changesetListItems(changesetId));
}

/**
 * Commits reviews/edits to a changeset.
 */
export async function commitChangeset(input: ChangesetCommitInput): Promise<boolean> {
  const result = await unwrapIpcResult(changesetCommit(input));
  clearNodesCache();
  return result;
}

/**
 * Lists all resolved changesets.
 */
export async function listResolvedChangesets(): Promise<Changeset[]> {
  return unwrapIpcResult(changesetListResolved());
}

/**
 * Seeds a test changeset with ADD, UPDATE, MERGE, DELETE, and ORPHAN proposals.
 */
export async function seedTestChangeset(): Promise<boolean> {
  const result = await unwrapIpcResult(debugSeedChangeset());
  if (result && typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent("mindvault-changeset-seeded"));
  }
  return result;
}

// Expose temporary debug helpers on window for manual console testing only in development builds
if (typeof window !== "undefined" && import.meta.env.DEV) {
  const w = window as unknown as Record<string, unknown>;
  w.testMemoryExtract = (provider?: string, endpoint?: string, model?: string) => {
    const p = provider || "ollama";
    const e = endpoint || "http://localhost:11434";
    const m = model || "granite4.1:3b";
    return extractMemory(p, e, m).then(console.log).catch(console.error);
  };
  w.testMemoryExtractIfReady = (provider?: string, endpoint?: string, model?: string) => {
    const p = provider || "ollama";
    const e = endpoint || "http://localhost:11434";
    const m = model || "granite4.1:3b";
    return extractMemoryIfReady(p, e, m).then(console.log).catch(console.error);
  };
  w.testCountPendingChangesetItems = () => {
    return countPendingChangesetItems().then(console.log).catch(console.error);
  };
  w.testListPendingChangesets = () => {
    return listPendingChangesets().then(console.log).catch(console.error);
  };
  w.testListChangesetItems = (changesetId: string) => {
    return listChangesetItems(changesetId).then(console.log).catch(console.error);
  };
  w.testCommitChangeset = (input: ChangesetCommitInput) => {
    return commitChangeset(input).then(console.log).catch(console.error);
  };
  w.testListResolvedChangesets = () => {
    return listResolvedChangesets().then(console.log).catch(console.error);
  };
  w.testSeedTestChangeset = () => {
    return seedTestChangeset().then(console.log).catch(console.error);
  };
}
