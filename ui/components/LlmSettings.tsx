import { useEffect, useState } from "react";
import { DEV_ONBOARDING_CHANGED } from "../constants/devEvents";
import { onboardingExtractProposals } from "../ipc";
import { getLlmModels } from "../services/nodes";
import { unwrapIpcResult, AppError } from "../services/ipcResult";
import { getOnboardingComplete, setOnboardingComplete } from "../services/settings";
import {
  getLlmModel,
  getLlmProvider,
  getLmStudioEndpoint,
  getOllamaEndpoint,
  setLlmModel,
  setLlmProvider,
  setLmStudioEndpoint,
  setOllamaEndpoint,
  getLlmMode,
  setLlmMode,
  getApiKey,
  setApiKey,
} from "../utils/settings";
import EmbeddingSettings from "./EmbeddingSettings";
import { cancelReembed, startReembed } from "../services/embedding";

const DEV_SAMPLE_ONBOARDING_ANSWERS = `{
  "displayName": "Dev Tester",
  "useMindVaultFor": "capturing context for projects",
  "workContext": "software engineer on a small team",
  "interests": "running, reading, local LLMs"
}`;

const CLOUD_PROVIDERS = [
  { id: "openai", name: "OpenAI", presets: ["gpt-4o", "gpt-4o-mini", "o1-mini", "o1-preview"] },
  {
    id: "anthropic",
    name: "Anthropic",
    presets: ["claude-3-5-sonnet-20241022", "claude-3-5-haiku-20241022", "claude-3-opus-20240229"],
  },
  {
    id: "google",
    name: "Google Gemini",
    presets: ["gemini-1.5-pro", "gemini-1.5-flash", "gemini-2.0-flash-exp"],
  },
  { id: "xai", name: "xAI Grok", presets: ["grok-2-1212", "grok-beta"] },
];

const LOCAL_PROVIDERS = [
  { id: "ollama", name: "Ollama" },
  { id: "lmstudio", name: "LM Studio" },
];

function EyeIcon({ visible }: { visible: boolean }) {
  if (visible) {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"></path>
        <circle cx="12" cy="12" r="3"></circle>
      </svg>
    );
  }
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"></path>
      <line x1="1" y1="1" x2="23" y2="23"></line>
    </svg>
  );
}

function LocalSettings({
  provider,
  setProvider,
  endpoint,
  setEndpoint,
  onSave,
  onTest,
  status,
  isLoading,
  models,
  selectedModel,
  setSelectedModel,
}: {
  provider: string;
  setProvider: (p: string) => void;
  endpoint: string;
  setEndpoint: (e: string) => void;
  onSave: () => void;
  onTest: () => void;
  status: string;
  isLoading: boolean;
  models: string[];
  selectedModel: string;
  setSelectedModel: (m: string) => void;
}) {
  return (
    <div className="settings-section local-settings">
      <div className="provider-toggle segmented-control">
        {LOCAL_PROVIDERS.map((p) => (
          <button
            key={p.id}
            type="button"
            className={provider === p.id ? "active" : ""}
            onClick={() => setProvider(p.id)}
          >
            {p.name}
          </button>
        ))}
      </div>

      <label className="settings-field">
        <span>Endpoint URL</span>
        <input
          type="text"
          value={endpoint}
          onChange={(event) => setEndpoint(event.target.value)}
          placeholder={provider === "ollama" ? "http://localhost:11434" : "http://localhost:1234"}
        />
      </label>

      <button type="button" className="settings-action" onClick={onTest}>
        {isLoading ? "Testing..." : "Test Connection & Fetch Models"}
      </button>

      <label className="settings-field">
        <span>Model</span>
        <select
          value={selectedModel}
          onChange={(event) => setSelectedModel(event.target.value)}
          disabled={models.length === 0 && !selectedModel}
        >
          {models.length === 0 ? <option value="">No models loaded</option> : null}
          {selectedModel && !models.includes(selectedModel) ? (
            <option value={selectedModel}>{selectedModel} (Saved)</option>
          ) : null}
          {models.map((model: string) => (
            <option key={model} value={model}>
              {model}
            </option>
          ))}
        </select>
      </label>

      <button type="button" className="settings-action save" onClick={onSave}>
        Save Configuration
      </button>

      {status && <p className="pane-status">{status}</p>}
    </div>
  );
}

function CloudSettings({
  provider,
  setProvider,
}: {
  provider: string;
  setProvider: (p: string) => void;
}) {
  const [apiKeyVisible, setApiKeyVisible] = useState(false);

  const providerDef = CLOUD_PROVIDERS.find((p) => p.id === provider) || CLOUD_PROVIDERS[0];
  const defaultModel = providerDef.presets[0] || "";

  const [apiKey, setApiKeyState] = useState("");
  const [status, setStatus] = useState("");
  useEffect(() => {
    let canceled = false;
    void (async () => {
      try {
        const key = await getApiKey(provider);
        if (!canceled) {
          setApiKeyState(key);
          setStatus("");
        }
      } catch (err) {
        if (!canceled) {
          setApiKeyState("");
          const message = err instanceof Error ? err.message : String(err);
          setStatus(`Failed to load API key: ${message}`);
        }
      }
    })();
    return () => {
      canceled = true;
    };
  }, [provider]);

  const [selectedModel, setSelectedModel] = useState(() => getLlmModel(provider) || defaultModel);

  useEffect(() => {
    if (!getLlmModel(provider)) {
      setLlmModel(provider, defaultModel);
    }
  }, [provider, defaultModel]);

  async function handleSave() {
    setStatus("");
    try {
      await setApiKey(provider, apiKey);
      setLlmModel(provider, selectedModel);
      setStatus("Saved cloud configuration.");
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setStatus(`Failed to save cloud configuration: ${message}`);
    }
  }

  return (
    <div className="settings-section cloud-settings">
      <div className="provider-grid">
        {CLOUD_PROVIDERS.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`provider-card ${provider === p.id ? "active" : ""}`}
            onClick={() => {
              setProvider(p.id);
              setLlmProvider(p.id);
            }}
          >
            <span className="provider-name">{p.name}</span>
          </button>
        ))}
      </div>

      <label className="settings-field">
        <span>API Key for {providerDef.name}</span>
        <div className="password-input-wrapper">
          <input
            type={apiKeyVisible ? "text" : "password"}
            value={apiKey}
            onChange={(e) => setApiKeyState(e.target.value)}
            placeholder="sk-..."
          />
          <button
            type="button"
            className="eye-toggle"
            onClick={() => setApiKeyVisible(!apiKeyVisible)}
            aria-label={apiKeyVisible ? "Hide API key" : "Show API key"}
            aria-pressed={apiKeyVisible}
          >
            <EyeIcon visible={apiKeyVisible} />
          </button>
        </div>
      </label>

      <label className="settings-field">
        <span>Model Preset</span>
        <select value={selectedModel} onChange={(e) => setSelectedModel(e.target.value)}>
          {!providerDef.presets.includes(selectedModel) && selectedModel ? (
            <option value={selectedModel}>{selectedModel}</option>
          ) : null}
          {providerDef.presets.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </label>

      <button type="button" className="settings-action save" onClick={handleSave}>
        Save Configuration
      </button>

      {status && <p className="pane-status">{status}</p>}
    </div>
  );
}
///TEMP TYPES
type EmbeddingStatus = {
  activeModel: string;
  tier: string;
  backend: string;
  coveragePercent: number;
  lastComputedAt: string | null;
  jaccardFallbackActive: boolean;
  reembedInProgress: boolean;
};

function LlmSettings() {
  const showDevOnboardingTools = import.meta.env.DEV;
  const [onboardingCompleteLabel, setOnboardingCompleteLabel] = useState<string>("…");
  const [onboardingDevBusy, setOnboardingDevBusy] = useState(false);
  const [extractionAnswersJson, setExtractionAnswersJson] = useState(DEV_SAMPLE_ONBOARDING_ANSWERS);
  const [extractionPreview, setExtractionPreview] = useState("");
  const [extractionBusy, setExtractionBusy] = useState(false);

  const [mode, setModeState] = useState(() => getLlmMode());
  const [localProvider, setLocalProvider] = useState<"ollama" | "lmstudio">(() => {
    const p = getLlmProvider();
    return p === "lmstudio" ? "lmstudio" : "ollama";
  });
  const [cloudProvider, setCloudProvider] = useState<string>(() => {
    const p = getLlmProvider();
    return ["openai", "anthropic", "google", "xai"].includes(p) ? p : "openai";
  });

  const [ollamaEndpoint, setOllamaEndpointState] = useState(() => getOllamaEndpoint());
  const [lmStudioEndpoint, setLmStudioEndpointState] = useState(() => getLmStudioEndpoint());
  const [localModels, setLocalModels] = useState<string[]>([]);
  const [localSelectedModel, setLocalSelectedModel] = useState(() => getLlmModel(localProvider));
  const [localStatus, setLocalStatus] = useState("");
  const [isLoading, setIsLoading] = useState(false);

  const [hybridTab, setHybridTab] = useState<"local" | "cloud">("local");

  const localEndpoint = localProvider === "ollama" ? ollamaEndpoint : lmStudioEndpoint;

  const [embeddingStatus, setEmbeddingStatus] = useState<EmbeddingStatus | null>(null);
  const [embeddingLoading] = useState(true);

  useEffect(() => {
    if (!showDevOnboardingTools) return;
    void (async () => {
      try {
        const done = await getOnboardingComplete();
        setOnboardingCompleteLabel(done ? "complete" : "not complete");
      } catch {
        setOnboardingCompleteLabel("error");
      }
    })();
  }, [showDevOnboardingTools]);

  useEffect(() => {
    let active = true;
    if (!localEndpoint.trim()) return;
    void (async () => {
      try {
        const fetchedModels = await getLlmModels(localProvider, localEndpoint.trim());
        if (active) {
          setLocalModels(fetchedModels);
        }
      } catch (e) {
        console.warn("Auto-fetch models failed:", e);
      }
    })();
    return () => {
      active = false;
    };
  }, [localProvider, localEndpoint]);

  useEffect(() => {
    function handleSettingsChange() {
      setModeState(getLlmMode());
      const p = getLlmProvider();
      if (["ollama", "lmstudio"].includes(p)) {
        setLocalProvider(p as "ollama" | "lmstudio");
        setLocalSelectedModel(getLlmModel(p));
      } else if (["openai", "anthropic", "google", "xai"].includes(p)) {
        setCloudProvider(p);
      }
    }
    window.addEventListener("mindvault:llm-settings-changed", handleSettingsChange);
    return () => {
      window.removeEventListener("mindvault:llm-settings-changed", handleSettingsChange);
    };
  }, []);

  async function refreshOnboardingDevState() {
    try {
      const done = await getOnboardingComplete();
      setOnboardingCompleteLabel(done ? "complete" : "not complete");
    } catch {
      setOnboardingCompleteLabel("error");
    }
    window.dispatchEvent(new CustomEvent(DEV_ONBOARDING_CHANGED));
  }

  async function onDevTestOnboardingExtraction() {
    setExtractionPreview("");
    let parsed: unknown;
    try {
      parsed = JSON.parse(extractionAnswersJson.trim()) as unknown;
    } catch {
      setExtractionPreview("Answers JSON is invalid — fix JSON syntax.");
      return;
    }
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      setExtractionPreview("Answers must be a JSON object at the top level.");
      return;
    }

    setExtractionBusy(true);
    try {
      const p = getLlmProvider();
      let endp = "";
      if (p === "ollama") endp = getOllamaEndpoint();
      else if (p === "lmstudio") endp = getLmStudioEndpoint();
      else endp = await getApiKey(p);
      const m = getLlmModel(p);

      const proposals = await unwrapIpcResult(
        onboardingExtractProposals(extractionAnswersJson.trim(), p, endp.trim(), m.trim())
      );
      setExtractionPreview(JSON.stringify(proposals, null, 2));
      setLocalStatus(`Dev: extraction OK (${proposals.length} proposal(s)).`);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setExtractionPreview(`Error:\n${message}`);
      setLocalStatus("Dev: extraction failed — see Developer panel.");
    } finally {
      setExtractionBusy(false);
    }
  }

  async function onTestLocalConnection() {
    setIsLoading(true);
    setLocalStatus("");
    try {
      const fetchedModels = await getLlmModels(localProvider, localEndpoint.trim());
      setLocalModels(fetchedModels);
      if (fetchedModels.length === 0) {
        setLocalStatus("Connected, but no models were returned.");
      } else {
        const nextModel =
          localSelectedModel && fetchedModels.includes(localSelectedModel)
            ? localSelectedModel
            : fetchedModels[0];
        setLocalSelectedModel(nextModel);
        setLlmModel(localProvider, nextModel);
        setLocalStatus(`Connected. Found ${fetchedModels.length} model(s).`);
      }
    } catch (err) {
      if (err instanceof AppError) {
        setLocalStatus(err.message);
      } else {
        setLocalStatus("Failed to connect to endpoint.");
      }
    }
    setIsLoading(false);
  }

  function onSaveLocalSettings() {
    setLlmProvider(localProvider);
    if (localProvider === "ollama") {
      setOllamaEndpoint(ollamaEndpoint);
      setOllamaEndpointState(getOllamaEndpoint());
      setLlmModel("ollama", localSelectedModel);
      setLocalStatus("Saved Ollama settings.");
    } else {
      setLmStudioEndpoint(lmStudioEndpoint);
      setLmStudioEndpointState(getLmStudioEndpoint());
      setLlmModel("lmstudio", localSelectedModel);
      setLocalStatus("Saved LM Studio settings.");
    }
  }

  function handleModeChange(newMode: "local" | "cloud" | "hybrid") {
    setModeState(newMode);
    setLlmMode(newMode);
  }

  return (
    <aside className="pane pane-right llm-settings">
      <div className="pane-header">
        <h3>⚙️ LLM Settings</h3>
      </div>

      <div className="segmented-control mode-selector">
        <button
          type="button"
          className={mode === "local" ? "active" : ""}
          onClick={() => handleModeChange("local")}
        >
          💻 Local
        </button>
        <button
          type="button"
          className={mode === "cloud" ? "active" : ""}
          onClick={() => handleModeChange("cloud")}
        >
          ☁️ Cloud
        </button>
        <button
          type="button"
          className={mode === "hybrid" ? "active" : ""}
          onClick={() => handleModeChange("hybrid")}
        >
          ⚡ Hybrid
        </button>
      </div>

      <div className="settings-content-wrapper">
        {mode === "local" && (
          <LocalSettings
            provider={localProvider}
            setProvider={(p: string) => {
              setLocalProvider(p as "ollama" | "lmstudio");
              setLlmProvider(p);
              setLocalModels([]);
              setLocalSelectedModel(getLlmModel(p));
              setLocalStatus("");
            }}
            endpoint={localEndpoint}
            setEndpoint={
              localProvider === "ollama" ? setOllamaEndpointState : setLmStudioEndpointState
            }
            onSave={onSaveLocalSettings}
            onTest={onTestLocalConnection}
            status={localStatus}
            isLoading={isLoading}
            models={localModels}
            selectedModel={localSelectedModel}
            setSelectedModel={setLocalSelectedModel}
          />
        )}

        {mode === "cloud" && (
          <CloudSettings
            key={cloudProvider}
            provider={cloudProvider}
            setProvider={setCloudProvider}
          />
        )}

        {mode === "hybrid" && (
          <div className="hybrid-wrapper">
            <div className="hybrid-tabs segmented-control">
              <button
                type="button"
                className={hybridTab === "local" ? "active" : ""}
                onClick={() => {
                  setHybridTab("local");
                  setLlmProvider(localProvider);
                }}
              >
                💻 Local Set
              </button>
              <button
                type="button"
                className={hybridTab === "cloud" ? "active" : ""}
                onClick={() => {
                  setHybridTab("cloud");
                  setLlmProvider(cloudProvider);
                }}
              >
                ☁️ Cloud Set
              </button>
            </div>
            {hybridTab === "local" ? (
              <LocalSettings
                provider={localProvider}
                setProvider={(p: string) => {
                  setLocalProvider(p as "ollama" | "lmstudio");
                  setLlmProvider(p);
                  setLocalModels([]);
                  setLocalSelectedModel(getLlmModel(p));
                  setLocalStatus("");
                }}
                endpoint={localEndpoint}
                setEndpoint={
                  localProvider === "ollama" ? setOllamaEndpointState : setLmStudioEndpointState
                }
                onSave={onSaveLocalSettings}
                onTest={onTestLocalConnection}
                status={localStatus}
                isLoading={isLoading}
                models={localModels}
                selectedModel={localSelectedModel}
                setSelectedModel={setLocalSelectedModel}
              />
            ) : (
              <CloudSettings
                key={cloudProvider}
                provider={cloudProvider}
                setProvider={setCloudProvider}
              />
            )}
            <div className="hybrid-note">
              <p>
                In Hybrid Mode, components dynamically choose whether to route to local or cloud
                depending on task demands. Adjust settings for both here.
              </p>
            </div>
          </div>
        )}
        <EmbeddingSettings
          status={embeddingStatus}
          loading={embeddingLoading}
          onReembed={() => {
            void startReembed().then(() =>
              setEmbeddingStatus((s) => (s ? { ...s, reembedInProgress: true } : s))
            );
          }}
          onCancelReembed={() => {
            void cancelReembed().then(() =>
              setEmbeddingStatus((s) => (s ? { ...s, reembedInProgress: false } : s))
            );
          }}
        />
      </div>

      {showDevOnboardingTools ? (
        <div className="llm-settings-dev" aria-label="Developer onboarding shortcuts">
          <h4 className="llm-settings-dev-title">Developer</h4>
          <p className="llm-settings-dev-line">
            Onboarding: <strong>{onboardingCompleteLabel}</strong>
          </p>
          <div className="llm-settings-dev-actions">
            <button
              type="button"
              className="settings-action"
              disabled={onboardingDevBusy}
              onClick={() => {
                setOnboardingDevBusy(true);
                void (async () => {
                  try {
                    await setOnboardingComplete(false);
                    await refreshOnboardingDevState();
                    setLocalStatus("Onboarding reset: wizard should appear.");
                  } catch (err) {
                    setLocalStatus(err instanceof Error ? err.message : String(err));
                  } finally {
                    setOnboardingDevBusy(false);
                  }
                })();
              }}
            >
              Reset onboarding
            </button>
            <button
              type="button"
              className="settings-action"
              disabled={onboardingDevBusy}
              onClick={() => {
                setOnboardingDevBusy(true);
                void (async () => {
                  try {
                    await setOnboardingComplete(true);
                    await refreshOnboardingDevState();
                    setLocalStatus("Onboarding marked complete.");
                  } catch (err) {
                    setLocalStatus(err instanceof Error ? err.message : String(err));
                  } finally {
                    setOnboardingDevBusy(false);
                  }
                })();
              }}
            >
              Mark onboarding done
            </button>
          </div>

          <label className="settings-field llm-settings-dev-field">
            <span>Test onboarding_extract_proposals (answers JSON)</span>
            <textarea
              className="llm-settings-dev-json-input"
              rows={6}
              spellCheck={false}
              value={extractionAnswersJson}
              onChange={(event) => setExtractionAnswersJson(event.target.value)}
              aria-label="Sample onboarding answers JSON"
            />
          </label>
          <button
            type="button"
            className="settings-action"
            disabled={extractionBusy || onboardingDevBusy}
            onClick={() => void onDevTestOnboardingExtraction()}
          >
            {extractionBusy ? "Running extraction…" : "Run onboarding extraction"}
          </button>
          {extractionPreview ? (
            <pre className="llm-settings-dev-json-preview" tabIndex={0}>
              {extractionPreview}
            </pre>
          ) : null}

          <p className="llm-settings-dev-note">Shown only in dev builds.</p>
        </div>
      ) : null}
    </aside>
  );
}

export default LlmSettings;
