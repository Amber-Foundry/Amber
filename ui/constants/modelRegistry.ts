export interface ModelRegistryEntry {
  contextWindow: number;
}

export const MODEL_REGISTRY: Record<string, ModelRegistryEntry> = {
  // OpenAI
  "gpt-4o": { contextWindow: 128000 },
  "gpt-4o-mini": { contextWindow: 128000 },
  "gpt-4-turbo": { contextWindow: 128000 },
  "gpt-4": { contextWindow: 8192 },
  "gpt-3.5-turbo": { contextWindow: 16385 },
  o1: { contextWindow: 128000 },
  "o1-mini": { contextWindow: 128000 },
  "o1-preview": { contextWindow: 128000 },
  "o3-mini": { contextWindow: 200000 },

  // Anthropic
  "claude-3-5-sonnet": { contextWindow: 200000 },
  "claude-3-5-haiku": { contextWindow: 200000 },
  "claude-3-opus": { contextWindow: 200000 },
  "claude-3-sonnet": { contextWindow: 200000 },
  "claude-3-haiku": { contextWindow: 200000 },
  "claude-3-5-sonnet-20241022": { contextWindow: 200000 },
  "claude-3-5-haiku-20241022": { contextWindow: 200000 },

  // Google
  "gemini-1.5-pro": { contextWindow: 1000000 },
  "gemini-1.5-flash": { contextWindow: 1000000 },
  "gemini-1.0-pro": { contextWindow: 32768 },
  "gemini-3.1-flash-lite": { contextWindow: 1000000 },

  // xAI
  "grok-2": { contextWindow: 128000 },
  "grok-beta": { contextWindow: 128000 },
};

export const UNKNOWN_MODEL_FALLBACK: ModelRegistryEntry = {
  contextWindow: 8000,
};

export const AUTO_WINDOW_FRACTION = 0.75;
export const ABSOLUTE_HARD_CAP = 200000;

export function lookupModel(modelId: string, provider?: string): ModelRegistryEntry {
  const modelClean = modelId.trim();

  // 1. Try exact match
  if (MODEL_REGISTRY[modelClean]) {
    return MODEL_REGISTRY[modelClean];
  }

  // 2. Try match on base name (e.g. provider/model-name)
  const baseName = modelClean.split("/").pop() || "";
  if (MODEL_REGISTRY[baseName]) {
    return MODEL_REGISTRY[baseName];
  }

  // 3. Smart fuzzy fallback based on provider / model name content
  const modelLower = modelClean.toLowerCase();
  const providerLower = provider?.toLowerCase() || "";

  if (providerLower === "google" || modelLower.includes("gemini")) {
    return { contextWindow: 1000000 };
  }
  if (providerLower === "anthropic" || modelLower.includes("claude")) {
    return { contextWindow: 200000 };
  }
  if (
    providerLower === "openai" ||
    modelLower.includes("gpt-4") ||
    modelLower.includes("gpt-3") ||
    modelLower.includes("o1") ||
    modelLower.includes("o3")
  ) {
    return { contextWindow: 128000 };
  }
  if (providerLower === "xai" || modelLower.includes("grok")) {
    return { contextWindow: 128000 };
  }

  return UNKNOWN_MODEL_FALLBACK;
}

export function getContextBudgetCeiling(modelId: string, provider?: string): number {
  const entry = lookupModel(modelId, provider);
  return Math.min(Math.floor(entry.contextWindow * AUTO_WINDOW_FRACTION), ABSOLUTE_HARD_CAP);
}
