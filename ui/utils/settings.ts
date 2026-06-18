import { settingsGet, settingsSet } from "../ipc.ts";
import { unwrapIpcResult } from "../services/ipcResult.ts";

const LLM_PROVIDER_KEY = "mindvault.llm.provider";
const OLLAMA_ENDPOINT_KEY = "mindvault.llm.ollama.endpoint";
const LMSTUDIO_ENDPOINT_KEY = "mindvault.llm.lmstudio.endpoint";
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
  window.localStorage.setItem(OLLAMA_ENDPOINT_KEY, normalized || DEFAULT_OLLAMA_ENDPOINT);
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
  window.localStorage.setItem(LMSTUDIO_ENDPOINT_KEY, normalized || DEFAULT_LMSTUDIO_ENDPOINT);
}

export function getLlmModel(provider?: string): string {
  const p = provider || getLlmProvider();
  const providerKey = `mindvault.llm.${p}.model`;
  return window.localStorage.getItem(providerKey) || "";
}

export function setLlmModel(provider: string, model: string): void {
  window.localStorage.setItem(`mindvault.llm.${provider}.model`, model.trim());
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
  window.localStorage.setItem(CHARTS_ENABLED_KEY, enabled ? "true" : "false");
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

export function getChatChartsEnabled(): boolean {
  const value = window.localStorage.getItem(CHAT_CHARTS_ENABLED_KEY);
  return value === "true";
}

export function setChatChartsEnabled(enabled: boolean): void {
  window.localStorage.setItem(CHAT_CHARTS_ENABLED_KEY, enabled ? "true" : "false");
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

export function getNodeEditorChartsEnabled(): boolean {
  const value = window.localStorage.getItem(NODE_EDITOR_CHARTS_ENABLED_KEY);
  return value === "true";
}

export function setNodeEditorChartsEnabled(enabled: boolean): void {
  window.localStorage.setItem(NODE_EDITOR_CHARTS_ENABLED_KEY, enabled ? "true" : "false");
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
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}

const TEMPORARY_SESSION_RETENTION_KEY = "temporary_session_retention";

export async function getTemporarySessionRetention(): Promise<"immediate" | "7_days"> {
  try {
    const value = await unwrapIpcResult(settingsGet(TEMPORARY_SESSION_RETENTION_KEY));
    if (value === "7_days") {
      return "7_days";
    }
  } catch (e) {
    console.warn("Failed to get temporary session retention setting:", e);
  }
  return "immediate";
}

export async function setTemporarySessionRetention(
  retention: "immediate" | "7_days"
): Promise<void> {
  await unwrapIpcResult(settingsSet(TEMPORARY_SESSION_RETENTION_KEY, retention));
  window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
}
