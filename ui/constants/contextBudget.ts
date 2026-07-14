/**
 * Default context token budget for `debug_assemble_context` / `llm_chat`.
 * Must match `DEFAULT_ASSEMBLER_MAX_TOKENS` in `core/src/lib.rs`.
 */
export const CONTEXT_MAX_TOKENS = 8000 as const;

/** Privacy tier passed to the assembler (`cloud` vs `local` filtering). */
export type ContextAssemblerScope = "local" | "cloud";

/**
 * Fallback context window registry for cloud models where window size cannot
 * be queried at runtime.
 */
export const CLOUD_MODEL_CONTEXT_REGISTRY: Record<string, number> = {
  // OpenAI
  "gpt-4o": 128000,
  "gpt-4o-mini": 128000,
  "gpt-4-turbo": 128000,
  "gpt-4": 8192,
  "gpt-3.5-turbo": 16385,

  // Anthropic
  "claude-3-5-sonnet": 200000,
  "claude-3-5-haiku": 200000,
  "claude-3-opus": 200000,
  "claude-3-sonnet": 200000,
  "claude-3-haiku": 200000,

  // Google
  "gemini-1.5-pro": 1000000,
  "gemini-1.5-flash": 1000000,
  "gemini-1.0-pro": 32768,

  // xAI
  "grok-2": 128000,
  "grok-beta": 128000,

  // Default fallback for any other unrecognized cloud model
  default: 8000,
};

/**
 * Resolve context window limit for cloud models (supporting exact registry lookup
 * and smart fuzzy fallback mappings based on typed model names).
 */
export function getCloudModelContextLimit(model: string, provider: string): number {
  const modelClean = model.trim();
  const modelLower = modelClean.toLowerCase();

  // Try exact match or base name match first
  const limit =
    CLOUD_MODEL_CONTEXT_REGISTRY[modelClean] ||
    CLOUD_MODEL_CONTEXT_REGISTRY[modelClean.split("/").pop() || ""];

  if (limit) {
    return limit;
  }

  // Fuzzy matches based on provider / model name content
  if (provider === "google" || modelLower.includes("gemini")) {
    return 1000000; // Standard modern Gemini context window (1M)
  }
  if (provider === "anthropic" || modelLower.includes("claude")) {
    return 200000; // Standard Claude context window (200k)
  }
  if (
    provider === "openai" ||
    modelLower.includes("gpt-4") ||
    modelLower.includes("gpt-3") ||
    modelLower.includes("o1")
  ) {
    return 128000; // Standard OpenAI context window (128k)
  }
  if (provider === "xai" || modelLower.includes("grok")) {
    return 128000; // Standard Grok context window (128k)
  }

  return CLOUD_MODEL_CONTEXT_REGISTRY.default;
}
