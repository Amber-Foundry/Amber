import { useEffect, useMemo, useState } from "react";
import type { OnboardingProposedNode } from "../ipc";
import { onboardingExtractProposals } from "../ipc";
import { unwrapIpcResult } from "../services/ipcResult";
import { getLlmModels } from "../services/nodes";
import { listVaults } from "../services/vaults";
import {
  getLlmModel,
  getLlmProvider,
  getLmStudioEndpoint,
  getOllamaEndpoint,
  setLlmModel,
  setLlmProvider,
  setLmStudioEndpoint,
  setOllamaEndpoint,
} from "../utils/settings";

type OnboardingShellProps = {
  onComplete: () => Promise<void>;
  onSkip: () => Promise<void>;
  busy: boolean;
  errorMessage: string | null;
};

const STEPS = ["Basics", "LLM setup", "Review", "Done"] as const;

type Provider = "ollama" | "lmstudio";

type BasicsAnswers = {
  displayName: string;
  useMindVaultFor: string;
  workContext: string;
  interests: string;
};

const QUESTION_FIELDS: Array<{
  key: keyof BasicsAnswers;
  label: string;
  placeholder: string;
  multiline?: boolean;
}> = [
  {
    key: "displayName",
    label: "What should MindVault call you?",
    placeholder: "Kevin",
  },
  {
    key: "useMindVaultFor",
    label: "What do you mainly want MindVault to help with?",
    placeholder: "Remember stuff for work and keep notes about things I care about.",
    multiline: true,
  },
  {
    key: "workContext",
    label: "What is your work/study context?",
    placeholder: "I work at a small company. Mostly emails and meetings.",
    multiline: true,
  },
  {
    key: "interests",
    label: "What are your main interests right now?",
    placeholder: "Cooking, weekend hikes, and a book club.",
    multiline: true,
  },
];

function stepDescription(step: number, stagedCount: number): string {
  if (step === 0) {
    return "Answer fixed onboarding questions. This data is sent when you proceed to Review.";
  }
  if (step === 1) {
    return "Configure the same provider/endpoint/model used by chat. Extraction runs when you move to Review.";
  }
  if (step === 2) {
    return `Review staged proposals before any DB writes. Currently staged: ${stagedCount}.`;
  }
  return "Extraction is complete for this run. Finish onboarding to open MindVault.";
}

/** Seed + migration vault IDs — used when `vault_list` names are not loaded yet. */
const KNOWN_VAULT_DISPLAY_NAMES: Record<string, string> = {
  vault_root_graph: "Root Graph",
  vault_credentials: "Credentials",
  vault_personal: "Personal",
  vault_work: "Work",
  vault_learning: "Learning",
  vault_health: "Health",
  vault_finance: "Finance",
};

function vaultDisplayLabel(vaultId: string | null, nameFromList: string | null): string {
  if (!vaultId) {
    return "";
  }
  const trimmed = nameFromList?.trim();
  if (trimmed) {
    return trimmed;
  }
  return KNOWN_VAULT_DISPLAY_NAMES[vaultId] ?? "Unknown vault";
}

function buildAnswersJson(answers: BasicsAnswers): string {
  const payload = {
    displayName: answers.displayName.trim(),
    useMindVaultFor: answers.useMindVaultFor.trim(),
    workContext: answers.workContext.trim(),
    interests: answers.interests.trim(),
  };
  return JSON.stringify(payload, null, 2);
}

function OnboardingShell({ onComplete, onSkip, busy, errorMessage }: OnboardingShellProps) {
  const [currentStep, setCurrentStep] = useState(0);
  const [answers, setAnswers] = useState<BasicsAnswers>({
    displayName: "",
    useMindVaultFor: "",
    workContext: "",
    interests: "",
  });
  const [provider, setProviderState] = useState<Provider>(() => {
    return getLlmProvider() === "lmstudio" ? "lmstudio" : "ollama";
  });
  const [ollamaEndpoint, setOllamaEndpointState] = useState(() => getOllamaEndpoint());
  const [lmStudioEndpoint, setLmStudioEndpointState] = useState(() => getLmStudioEndpoint());
  const [selectedModel, setSelectedModelState] = useState(() => getLlmModel());
  const [models, setModels] = useState<string[]>([]);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [llmBusy, setLlmBusy] = useState(false);
  const [extractBusy, setExtractBusy] = useState(false);
  const [hasExtracted, setHasExtracted] = useState(false);
  const [stagedProposals, setStagedProposals] = useState<OnboardingProposedNode[]>([]);
  const [vaultNameById, setVaultNameById] = useState<Record<string, string>>({});

  const endpoint = provider === "ollama" ? ollamaEndpoint : lmStudioEndpoint;
  const shellBusy = busy || llmBusy || extractBusy;
  const isLastStep = currentStep === STEPS.length - 1;
  const canRunExtraction = selectedModel.trim().length > 0 && endpoint.trim().length > 0;
  const answersJson = useMemo(() => buildAnswersJson(answers), [answers]);

  const heading = useMemo(() => `${currentStep + 1}. ${STEPS[currentStep]}`, [currentStep]);
  const description = useMemo(
    () => stepDescription(currentStep, stagedProposals.length),
    [currentStep, stagedProposals.length]
  );

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const vaults = await listVaults();
        if (cancelled) {
          return;
        }
        const next: Record<string, string> = {};
        for (const vault of vaults) {
          next[vault.id] = vault.name;
        }
        setVaultNameById(next);
      } catch {
        if (!cancelled) {
          setVaultNameById({});
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  function setProvider(nextProvider: Provider) {
    setProviderState(nextProvider);
    setLlmProvider(nextProvider);
    setModels([]);
    setStatusMessage(null);
    setHasExtracted(false);
  }

  function setSelectedModel(nextModel: string) {
    setSelectedModelState(nextModel);
    setLlmModel(nextModel);
    setHasExtracted(false);
  }

  function onAnswerChange(field: keyof BasicsAnswers, value: string) {
    setAnswers((current) => ({ ...current, [field]: value }));
    setHasExtracted(false);
  }

  async function testConnectionAndFetchModels() {
    setLlmBusy(true);
    setStatusMessage(null);
    try {
      const fetchedModels = await getLlmModels(provider, endpoint.trim());
      setModels(fetchedModels);
      if (fetchedModels.length === 0) {
        setStatusMessage("Connected, but no models were returned.");
      } else {
        const nextModel =
          selectedModel && fetchedModels.includes(selectedModel) ? selectedModel : fetchedModels[0];
        setSelectedModel(nextModel);
        setStatusMessage(`Connected. Found ${fetchedModels.length} model(s).`);
      }
    } catch (error) {
      setStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setLlmBusy(false);
    }
  }

  function saveLlmSettings() {
    setLlmProvider(provider);
    if (provider === "ollama") {
      setOllamaEndpoint(ollamaEndpoint);
      setStatusMessage("Saved Ollama settings.");
    } else {
      setLmStudioEndpoint(lmStudioEndpoint);
      setStatusMessage("Saved LM Studio settings.");
    }
  }

  async function runExtractionOnce() {
    if (!canRunExtraction) {
      setStatusMessage("Configure endpoint + model before extraction.");
      return false;
    }
    if (
      !answers.displayName.trim() &&
      !answers.useMindVaultFor.trim() &&
      !answers.workContext.trim() &&
      !answers.interests.trim()
    ) {
      setStatusMessage("Fill at least one Basics answer before extraction.");
      return false;
    }

    setExtractBusy(true);
    setStatusMessage(null);
    try {
      const extracted = await unwrapIpcResult(
        onboardingExtractProposals(answersJson, provider, endpoint.trim(), selectedModel.trim())
      );
      setStagedProposals(extracted);
      setHasExtracted(true);
      setStatusMessage(`Extraction complete. Staged ${extracted.length} proposal(s).`);
      return true;
    } catch (error) {
      setStatusMessage(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      setExtractBusy(false);
    }
  }

  function goBack() {
    setCurrentStep((value) => Math.max(0, value - 1));
  }

  async function goNext() {
    if (currentStep === 1 && !hasExtracted) {
      const ok = await runExtractionOnce();
      if (!ok) {
        return;
      }
    }
    setCurrentStep((value) => Math.min(STEPS.length - 1, value + 1));
  }

  return (
    <section className="onboarding-shell" aria-label="Onboarding wizard">
      <div className="onboarding-card">
        <header className="onboarding-header">
          <div>
            <p className="onboarding-eyebrow">First Run Setup</p>
            <h1>Welcome to MindVault</h1>
          </div>
          <button
            type="button"
            className="onboarding-skip"
            onClick={() => void onSkip()}
            disabled={shellBusy}
          >
            Skip onboarding
          </button>
        </header>

        <ol className="onboarding-stepper">
          {STEPS.map((step, index) => (
            <li
              key={step}
              className={`onboarding-step ${index === currentStep ? "active" : ""} ${index < currentStep ? "done" : ""}`}
            >
              <span>{index + 1}</span>
              <small>{step}</small>
            </li>
          ))}
        </ol>

        <div className="onboarding-content">
          <h2>{heading}</h2>
          <p>{description}</p>
          {currentStep === 0 ? (
            <div className="onboarding-form-grid">
              {QUESTION_FIELDS.map((field) => (
                <label className="onboarding-field" key={field.key}>
                  <span>{field.label}</span>
                  {field.multiline ? (
                    <textarea
                      rows={3}
                      value={answers[field.key]}
                      onChange={(event) => onAnswerChange(field.key, event.target.value)}
                      placeholder={field.placeholder}
                      disabled={shellBusy}
                    />
                  ) : (
                    <input
                      type="text"
                      value={answers[field.key]}
                      onChange={(event) => onAnswerChange(field.key, event.target.value)}
                      placeholder={field.placeholder}
                      disabled={shellBusy}
                    />
                  )}
                </label>
              ))}
            </div>
          ) : null}
          {currentStep === 1 ? (
            <div className="onboarding-llm-grid">
              <div className="provider-toggle" role="radiogroup" aria-label="LLM provider">
                <label>
                  <input
                    type="radio"
                    name="onboarding-llm-provider"
                    checked={provider === "ollama"}
                    onChange={() => setProvider("ollama")}
                    disabled={shellBusy}
                  />
                  Ollama
                </label>
                <label>
                  <input
                    type="radio"
                    name="onboarding-llm-provider"
                    checked={provider === "lmstudio"}
                    onChange={() => setProvider("lmstudio")}
                    disabled={shellBusy}
                  />
                  LM Studio
                </label>
              </div>
              <label className="onboarding-field">
                <span>Endpoint URL</span>
                <input
                  type="text"
                  value={endpoint}
                  onChange={(event) => {
                    const nextValue = event.target.value;
                    if (provider === "ollama") {
                      setOllamaEndpointState(nextValue);
                    } else {
                      setLmStudioEndpointState(nextValue);
                    }
                    setHasExtracted(false);
                  }}
                  placeholder={
                    provider === "ollama" ? "http://localhost:11434" : "http://localhost:1234"
                  }
                  disabled={shellBusy}
                />
              </label>
              <div className="onboarding-inline-actions">
                <button
                  type="button"
                  className="onboarding-inline-button"
                  onClick={() => void testConnectionAndFetchModels()}
                  disabled={shellBusy}
                >
                  {llmBusy ? "Testing..." : "Test Connection & Fetch Models"}
                </button>
                <button
                  type="button"
                  className="onboarding-inline-button"
                  onClick={saveLlmSettings}
                  disabled={shellBusy}
                >
                  Save LLM Settings
                </button>
              </div>
              <label className="onboarding-field">
                <span>Model</span>
                <select
                  value={selectedModel}
                  onChange={(event) => setSelectedModel(event.target.value)}
                  disabled={shellBusy}
                >
                  <option value="">
                    {models.length === 0
                      ? "Use saved model or fetch available models"
                      : "Select model"}
                  </option>
                  {models.map((model) => (
                    <option key={model} value={model}>
                      {model}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          ) : null}
          {currentStep === 2 ? (
            <div className="onboarding-review-list" aria-label="Staged onboarding proposals">
              {stagedProposals.length === 0 ? (
                <p className="onboarding-hint">No staged proposals yet.</p>
              ) : (
                stagedProposals.map((proposal, index) => (
                  <article key={`${proposal.title}-${index}`} className="onboarding-proposal-card">
                    <div className="onboarding-proposal-header">
                      <h3>{proposal.title}</h3>
                      <small>{proposal.nodeType ?? "concept"}</small>
                    </div>
                    <p>{proposal.summary}</p>
                    <p className="onboarding-meta">
                      Vault target:{" "}
                      {proposal.resolvedVaultId
                        ? `${vaultDisplayLabel(
                            proposal.resolvedVaultId,
                            vaultNameById[proposal.resolvedVaultId] ?? null
                          )} (${proposal.resolvedVaultId})`
                        : "Unmapped — resolve before saving"}
                    </p>
                    <p className="onboarding-meta">
                      Category/key source: {proposal.targetVaultKey ?? proposal.category ?? "none"}
                    </p>
                  </article>
                ))
              )}
            </div>
          ) : null}
          {currentStep === 3 ? (
            <div className="onboarding-done-note">
              <p>
                Extraction run: <strong>{hasExtracted ? "yes" : "no"}</strong>
              </p>
              <p>
                Staged proposals: <strong>{stagedProposals.length}</strong> (no database writes
                yet).
              </p>
            </div>
          ) : null}
          {statusMessage ? <p className="onboarding-hint">{statusMessage}</p> : null}
          {errorMessage ? <p className="onboarding-error">{errorMessage}</p> : null}
        </div>

        <footer className="onboarding-actions">
          <button type="button" onClick={goBack} disabled={currentStep === 0 || shellBusy}>
            Back
          </button>
          {isLastStep ? (
            <button
              type="button"
              className="primary"
              onClick={() => void onComplete()}
              disabled={shellBusy}
            >
              Finish onboarding
            </button>
          ) : (
            <button
              type="button"
              className="primary"
              onClick={() => void goNext()}
              disabled={shellBusy}
            >
              Next
            </button>
          )}
        </footer>
      </div>
    </section>
  );
}

export default OnboardingShell;
