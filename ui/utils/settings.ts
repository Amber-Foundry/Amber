import { settingsGet, settingsSet } from "../ipc.ts";
import { unwrapIpcResult } from "../services/ipcResult.ts";

const LLM_PROVIDER_KEY = "mindvault.llm.provider";
const OLLAMA_ENDPOINT_KEY = "mindvault.llm.ollama.endpoint";
const LMSTUDIO_ENDPOINT_KEY = "mindvault.llm.lmstudio.endpoint";
const CHAT_CONTEXT_AUTO_KEY = "mindvault.llm.chat_context_auto";
const CHAT_CONTEXT_LIMIT_KEY = "mindvault.llm.chat_context_limit";
const DEFAULT_PROVIDER = "ollama";
const DEFAULT_OLLAMA_ENDPOINT = "http://localhost:11434";
const DEFAULT_LMSTUDIO_ENDPOINT = "http://localhost:1234";

export function getLlmProvider(): string {
  const value = window.localStorage.getItem(LLM_PROVIDER_KEY);
  if (!value || !value.trim()) {
    return DEFAULT_PROVIDER;
  }
  const normalized = value.trim().toLowerCase();
  if (["ollama", "lmstudio", "openai", "anthropic", "google", "xai"].includes(normalized)) {
    return normalized;
  }
  return DEFAULT_PROVIDER;
}

export function setLlmProvider(provider: string, skipEvent = false): void {
  const normalized = provider.trim().toLowerCase();
  const next = ["ollama", "lmstudio", "openai", "anthropic", "google", "xai"].includes(normalized)
    ? normalized
    : DEFAULT_PROVIDER;
  window.localStorage.setItem(LLM_PROVIDER_KEY, next);
  void settingsSet(LLM_PROVIDER_KEY, next).catch((err) => {
    console.error("Failed to persist LLM provider in SQLite:", err);
  });
  if (!skipEvent) {
    window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
  }
}

export function getOllamaEndpoint(): string {
  const value = window.localStorage.getItem(OLLAMA_ENDPOINT_KEY);
  if (!value || !value.trim()) {
    return DEFAULT_OLLAMA_ENDPOINT;
  }
  return value;
}

export function setOllamaEndpoint(url: string): void {
  const normalized = url.trim();
  const val = normalized || DEFAULT_OLLAMA_ENDPOINT;
  window.localStorage.setItem(OLLAMA_ENDPOINT_KEY, val);
  void settingsSet(OLLAMA_ENDPOINT_KEY, val).catch((err) => {
    console.error("Failed to persist Ollama endpoint in SQLite:", err);
  });
}

export function getLmStudioEndpoint(): string {
  const value = window.localStorage.getItem(LMSTUDIO_ENDPOINT_KEY);
  if (!value || !value.trim()) {
    return DEFAULT_LMSTUDIO_ENDPOINT;
  }
  return value;
}

export function setLmStudioEndpoint(url: string): void {
  const normalized = url.trim();
  const val = normalized || DEFAULT_LMSTUDIO_ENDPOINT;
  window.localStorage.setItem(LMSTUDIO_ENDPOINT_KEY, val);
  void settingsSet(LMSTUDIO_ENDPOINT_KEY, val).catch((err) => {
    console.error("Failed to persist LM Studio endpoint in SQLite:", err);
  });
}

export function getLlmModel(provider?: string): string {
  const p = provider || getLlmProvider();
  const providerKey = `mindvault.llm.${p}.model`;
  return window.localStorage.getItem(providerKey) || "";
}

export function setLlmModel(provider: string, model: string): void {
  const trimmed = model.trim();
  const providerKey = `mindvault.llm.${provider}.model`;
  window.localStorage.setItem(providerKey, trimmed);
  void settingsSet(providerKey, trimmed).catch((err) => {
    console.error(`Failed to persist LLM model for ${provider} in SQLite:`, err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const LLM_MODE_KEY = "mindvault.llm.mode";

export function getLlmMode(): "local" | "cloud" | "hybrid" {
  const val = window.localStorage.getItem(LLM_MODE_KEY);
  if (val === "cloud" || val === "hybrid") return val;
  return "local";
}

export function setLlmMode(mode: "local" | "cloud" | "hybrid"): void {
  window.localStorage.setItem(LLM_MODE_KEY, mode);
  void settingsSet(LLM_MODE_KEY, mode).catch((err) => {
    console.error("Failed to persist LLM mode in SQLite:", err);
  });
  // Synchronize provider to matching group
  const currentProvider = getLlmProvider();
  if (mode === "local") {
    if (!["ollama", "lmstudio"].includes(currentProvider)) {
      setLlmProvider("ollama", true);
    }
  } else if (mode === "cloud") {
    if (!["openai", "anthropic", "google", "xai"].includes(currentProvider)) {
      setLlmProvider("openai", true);
    }
  }
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const IMPORT_EXTRACTION_MODE_KEY = "amber.import.extractionMode";
export type ImportExtractionMode = "ai" | "fast";

export function getImportExtractionMode(): ImportExtractionMode {
  const value = window.localStorage.getItem(IMPORT_EXTRACTION_MODE_KEY);
  return value === "ai" ? "ai" : "fast";
}

export function setImportExtractionMode(mode: ImportExtractionMode): void {
  const next = mode === "ai" ? "ai" : "fast";
  window.localStorage.setItem(IMPORT_EXTRACTION_MODE_KEY, next);
  void settingsSet(IMPORT_EXTRACTION_MODE_KEY, next).catch((err) => {
    console.error("Failed to persist import extraction mode in SQLite:", err);
  });
}

export async function getApiKey(provider: string): Promise<string> {
  const value = await unwrapIpcResult(settingsGet(`mindvault.llm.${provider}.apikey`));
  return value || "";
}

export async function setApiKey(provider: string, key: string): Promise<void> {
  await unwrapIpcResult(settingsSet(`mindvault.llm.${provider}.apikey`, key.trim()));
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const CHARTS_ENABLED_KEY = "mindvault.llm.charts.enabled";
const CHAT_CHARTS_ENABLED_KEY = "mindvault.llm.charts.chat.enabled";
const NODE_EDITOR_CHARTS_ENABLED_KEY = "mindvault.llm.charts.nodeeditor.enabled";

export function getChartsEnabled(): boolean {
  const value = window.localStorage.getItem(CHARTS_ENABLED_KEY);
  return value === "true";
}

export function setChartsEnabled(enabled: boolean): void {
  const val = enabled ? "true" : "false";
  window.localStorage.setItem(CHARTS_ENABLED_KEY, val);
  void settingsSet(CHARTS_ENABLED_KEY, val).catch((err) => {
    console.error("Failed to persist charts enabled in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

export function getChatChartsEnabled(): boolean {
  const value = window.localStorage.getItem(CHAT_CHARTS_ENABLED_KEY);
  return value === "true";
}

export function setChatChartsEnabled(enabled: boolean): void {
  const val = enabled ? "true" : "false";
  window.localStorage.setItem(CHAT_CHARTS_ENABLED_KEY, val);
  void settingsSet(CHAT_CHARTS_ENABLED_KEY, val).catch((err) => {
    console.error("Failed to persist chat charts enabled in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

export function getNodeEditorChartsEnabled(): boolean {
  const value = window.localStorage.getItem(NODE_EDITOR_CHARTS_ENABLED_KEY);
  return value === "true";
}

export function setNodeEditorChartsEnabled(enabled: boolean): void {
  const val = enabled ? "true" : "false";
  window.localStorage.setItem(NODE_EDITOR_CHARTS_ENABLED_KEY, val);
  void settingsSet(NODE_EDITOR_CHARTS_ENABLED_KEY, val).catch((err) => {
    console.error("Failed to persist node editor charts enabled in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const PLANTUML_SERVER_KEY = "mindvault.llm.plantuml.server";
const DEFAULT_PLANTUML_SERVER = "https://www.plantuml.com/plantuml";

export function getPlantUmlServer(): string {
  const value = window.localStorage.getItem(PLANTUML_SERVER_KEY);
  if (!value || !value.trim()) {
    return DEFAULT_PLANTUML_SERVER;
  }
  return value.trim();
}

export function setPlantUmlServer(url: string): void {
  let normalized = url.trim();
  if (!normalized) {
    normalized = DEFAULT_PLANTUML_SERVER;
  } else {
    // 1. If it doesn't start with http:// or https://, prepend https:// by default
    if (!/^https?:\/\//i.test(normalized)) {
      normalized = "https://" + normalized;
    }

    // 2. Validate URL format and restrict to HTTP/HTTPS schemes to prevent arbitrary injection
    try {
      const parsed = new URL(normalized);
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
        normalized = DEFAULT_PLANTUML_SERVER;
      } else {
        // Strip trailing slashes to ensure clean path concatenation later
        normalized = parsed.origin + parsed.pathname.replace(/\/+$/, "");
      }
    } catch {
      normalized = DEFAULT_PLANTUML_SERVER;
    }
  }

  window.localStorage.setItem(PLANTUML_SERVER_KEY, normalized);
  void settingsSet(PLANTUML_SERVER_KEY, normalized).catch((err) => {
    console.error("Failed to persist PlantUML server in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const PLANTUML_CONSENT_KEY = "mindvault.plantuml.consent";

// Module-scoped flag for session-only consent (not persisted).
let sessionPlantUmlConsent = false;

export function getPlantUmlConsent(): "disabled" | "session" | "always" {
  const persisted = window.localStorage.getItem(PLANTUML_CONSENT_KEY);
  if (persisted === "always") return "always";
  if (sessionPlantUmlConsent) return "session";
  return "disabled";
}

export function setPlantUmlConsent(value: "disabled" | "session" | "always"): void {
  if (value === "session") {
    sessionPlantUmlConsent = true;
    window.localStorage.setItem(PLANTUML_CONSENT_KEY, "disabled");
    void settingsSet(PLANTUML_CONSENT_KEY, "disabled").catch((err) => {
      console.error("Failed to clear PlantUML consent in SQLite:", err);
    });
  } else {
    sessionPlantUmlConsent = false;
    window.localStorage.setItem(PLANTUML_CONSENT_KEY, value);
    void settingsSet(PLANTUML_CONSENT_KEY, value).catch((err) => {
      console.error("Failed to persist PlantUML consent in SQLite:", err);
    });
  }
  window.dispatchEvent(new CustomEvent("mindvault:plantuml-consent-changed"));
}

export function getChatContextAuto(): boolean {
  const value = window.localStorage.getItem(CHAT_CONTEXT_AUTO_KEY);
  if (value === null) {
    return true;
  }
  return value === "true";
}

export function setChatContextAuto(val: boolean): void {
  const strVal = val ? "true" : "false";
  window.localStorage.setItem(CHAT_CONTEXT_AUTO_KEY, strVal);
  void settingsSet(CHAT_CONTEXT_AUTO_KEY, strVal).catch((err) => {
    console.error("Failed to persist chat context auto in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

export function getChatContextLimit(): number {
  const value = window.localStorage.getItem(CHAT_CONTEXT_LIMIT_KEY);
  if (!value) {
    return 6000;
  }
  const parsed = parseInt(value, 10);
  return isNaN(parsed) ? 6000 : parsed;
}

export function setChatContextLimit(val: number): void {
  const strVal = val.toString();
  window.localStorage.setItem(CHAT_CONTEXT_LIMIT_KEY, strVal);
  void settingsSet(CHAT_CONTEXT_LIMIT_KEY, strVal).catch((err) => {
    console.error("Failed to persist chat context limit in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const LOCAL_CONTEXT_OVERRIDES_KEY = "mindvault.llm.local_context_overrides";

export function getLocalModelContextOverrides(): Record<string, number> {
  const value = window.localStorage.getItem(LOCAL_CONTEXT_OVERRIDES_KEY);
  if (!value) return {};
  try {
    return JSON.parse(value);
  } catch {
    return {};
  }
}

export function setLocalModelContextOverride(model: string, limit: number): void {
  const current = getLocalModelContextOverrides();
  if (limit <= 0) {
    delete current[model];
  } else {
    current[model] = limit;
  }
  const strVal = JSON.stringify(current);
  window.localStorage.setItem(LOCAL_CONTEXT_OVERRIDES_KEY, strVal);
  void settingsSet(LOCAL_CONTEXT_OVERRIDES_KEY, strVal).catch((err) => {
    console.error("Failed to persist local context overrides in SQLite:", err);
  });
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

// Self-executing initialization block to restore all settings from the SQLite database into localStorage at startup.
// This resolves issues where localStorage is cleared or flaky in Tauri.
void (async () => {
  const keys = [
    LLM_PROVIDER_KEY,
    LLM_MODE_KEY,
    IMPORT_EXTRACTION_MODE_KEY,
    OLLAMA_ENDPOINT_KEY,
    LMSTUDIO_ENDPOINT_KEY,
    CHARTS_ENABLED_KEY,
    CHAT_CHARTS_ENABLED_KEY,
    NODE_EDITOR_CHARTS_ENABLED_KEY,
    PLANTUML_SERVER_KEY,
    PLANTUML_CONSENT_KEY,
    CHAT_CONTEXT_AUTO_KEY,
    CHAT_CONTEXT_LIMIT_KEY,
    LOCAL_CONTEXT_OVERRIDES_KEY,
    "mindvault.llm.ollama.model",
    "mindvault.llm.lmstudio.model",
    "mindvault.llm.openai.model",
    "mindvault.llm.anthropic.model",
    "mindvault.llm.google.model",
    "mindvault.llm.xai.model",
  ];

  try {
    const promises = keys.map(async (key) => {
      try {
        const res = await settingsGet(key);
        if (res && "ok" in res && res.ok !== null) {
          window.localStorage.setItem(key, res.ok);
        }
      } catch (err) {
        console.error(`Failed to load setting ${key} from SQLite:`, err);
      }
    });
    await Promise.all(promises);
    window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
    window.dispatchEvent(new CustomEvent("mindvault:plantuml-consent-changed"));
  } catch (e) {
    console.error("Failed to initialize settings from DB:", e);
  }
})();
