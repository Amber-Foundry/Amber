/// TEMP functions to be replaced with IPC calls once merged
type EmbeddingStatus = {
  activeModel: string;
  tier: string;
  backend: string;
  coveragePercent: number;
  lastComputedAt: string | null;
  jaccardFallbackActive: boolean;
  reembedInProgress: boolean;
};
export async function invokeTyped<T>(_command: string, _payload?: unknown): Promise<T> {
  console.warn(`invokeTyped mock called: ${_command}`);

  return undefined as T;
}
///END TEMP functions to be replaced with IPC calls once mergedß
const MOCK_STATUS: EmbeddingStatus = {
  activeModel: "avsolatorio/GIST-small-Embedding-v0",
  tier: "light",
  backend: "onnx",
  coveragePercent: 0,
  lastComputedAt: null,
  jaccardFallbackActive: true,
  reembedInProgress: false,
};

const USE_MOCK = import.meta.env.VITE_USE_EMBED_MOCK !== "false"; // default: mock ON; Commit 16 sets VITE_USE_EMBED_MOCK=false in .env.development

export async function getEmbeddingStatus(): Promise<EmbeddingStatus> {
  if (USE_MOCK) return MOCK_STATUS;
  return invokeTyped<EmbeddingStatus>("embedding_get_status");
}

export async function startReembed(): Promise<void> {
  if (USE_MOCK) return;
  return invokeTyped<void>("embedding_reembed_start");
}

export async function cancelReembed(): Promise<void> {
  if (USE_MOCK) return;
  return invokeTyped<void>("embedding_reembed_cancel");
}
