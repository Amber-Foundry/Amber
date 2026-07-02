import type { EmbeddingReembedInput, EmbeddingStatus } from "../types/generated";
import { invokeTyped } from "../ipc";
import { unwrapIpcResult } from "./ipcResult.ts";

const MOCK_STATUS: EmbeddingStatus = {
  model: "avsolatorio/GIST-small-Embedding-v0",
  tier: "light",
  backend: "onnx",
  coveragePercent: 0,
  lastComputedAt: null,
  jaccardFallbackActive: true,
  reembedInProgress: false,
};

const USE_MOCK = import.meta.env.VITE_USE_EMBED_MOCK === "true";

export async function getEmbeddingStatus(): Promise<EmbeddingStatus> {
  if (USE_MOCK) return MOCK_STATUS;
  const status = await unwrapIpcResult(invokeTyped<EmbeddingStatus>("embedding_get_status"));
  return status;
}

export async function startReembed(payload: EmbeddingReembedInput): Promise<void> {
  if (USE_MOCK) return;
  await unwrapIpcResult(invokeTyped<void>("embedding_reembed_start", { payload }));
}

export async function cancelReembed(): Promise<void> {
  if (USE_MOCK) return;
  await unwrapIpcResult(invokeTyped<void>("embedding_reembed_cancel"));
}
