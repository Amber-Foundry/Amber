import { useEffect, useMemo, useState } from "react";
import type { OnboardingNodeCommitInput, OnboardingProposedNode } from "../ipc";
import { onboardingCommit, onboardingExtractProposals } from "../ipc";
import { unwrapIpcResult } from "../services/ipcResult";
import { getLlmModels } from "../services/nodes";
import ModelSetupPanel from "./ModelSetupPanel";
import { getRecommendedStacks } from "../services/modelSetup";
import { listVaults } from "../services/vaults";
import { setSetting } from "../services/settings";
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

const STEPS = ["Model Setup", "Basics", "LLM setup", "Review"] as const;
const MODEL_SETUP_STEP_INDEX = 0;
const BASICS_STEP_INDEX = 1;
const LLM_STEP_INDEX = 2;
const REVIEW_STEP_INDEX = STEPS.indexOf("Review");

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
    label: "What should Amber call you?",
    placeholder: "Kevin",
  },
  {
    key: "useMindVaultFor",
    label: "What do you mainly want Amber to help with?",
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
  if (step === MODEL_SETUP_STEP_INDEX) {
    return "Check your hardware and pick the best local model path. You can skip this and configure it later.";
  }
  if (step === BASICS_STEP_INDEX) {
    return "Answer fixed onboarding questions. This data is sent when you proceed to Review.";
  }
  if (step === LLM_STEP_INDEX) {
    return "Configure the same provider/endpoint/model used by chat. Extraction runs when you move to Review.";
  }
  return `Review/edit staged proposals before commit. Currently staged: ${stagedCount}.`;
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

type EditableStagedProposal = {
  rowId: string;
  title: string;
  summary: string;
  detail: string;
  nodeType: string;
  sourceType: string;
  vaultId: string;
  category?: string;
  targetVaultKey?: string;
};

function newRowId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `onb_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}

type StatusMessage = { text: string; kind: "info" | "error" };

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
  const [statusMessage, setStatusMessage] = useState<StatusMessage | null>(null);
  const [llmBusy, setLlmBusy] = useState(false);
  const [extractBusy, setExtractBusy] = useState(false);
  const [commitBusy, setCommitBusy] = useState(false);
  const [hasExtracted, setHasExtracted] = useState(false);
  const [extractionFailed, setExtractionFailed] = useState(false);
  const [stagedProposals, setStagedProposals] = useState<EditableStagedProposal[]>([]);
  const [vaultNameById, setVaultNameById] = useState<Record<string, string>>({});

  const endpoint = provider === "ollama" ? ollamaEndpoint : lmStudioEndpoint;
  const shellBusy = busy || llmBusy || extractBusy || commitBusy;
  const isLastStep = currentStep === STEPS.length - 1;
  const canRunExtraction = selectedModel.trim().length > 0 && endpoint.trim().length > 0;
  const answersJson = useMemo(() => buildAnswersJson(answers), [answers]);
  const unresolvedCount = useMemo(
    () => stagedProposals.filter((proposal) => !proposal.vaultId.trim()).length,
    [stagedProposals]
  );
  const vaultOptions = useMemo(() => {
    const merged = new Map<string, string>(Object.entries(KNOWN_VAULT_DISPLAY_NAMES));
    for (const [id, name] of Object.entries(vaultNameById)) {
      merged.set(id, name);
    }
    return Array.from(merged.entries())
      .map(([id, name]) => ({ id, name }))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [vaultNameById]);
  const availableVaultIds = useMemo(
    () => new Set(vaultOptions.map((vault) => vault.id)),
    [vaultOptions]
  );

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
          if (!vault.parentVaultId) {
            next[vault.id] = vault.name;
          }
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
    setExtractionFailed(false);

    const restoredModel = getLlmModel(nextProvider);
    setSelectedModelState(restoredModel);
    setLlmModel(nextProvider, restoredModel);
  }

  function setSelectedModel(nextModel: string) {
    setSelectedModelState(nextModel);
    setLlmModel(provider, nextModel);
    setHasExtracted(false);
    setExtractionFailed(false);
  }

  async function handleStackSelected(stackId: string) {
    try {
      const recommendedStacks = await getRecommendedStacks();
      const chosen = recommendedStacks.find((s) => s.id === stackId);
      if (chosen?.modelName) {
        setSelectedModel(chosen.modelName);
      }
    } catch (err) {
      console.warn("Failed to apply recommended stack settings:", err);
    }
    await goNext();
  }

  function onAnswerChange(field: keyof BasicsAnswers, value: string) {
    setAnswers((current) => ({ ...current, [field]: value }));
    setHasExtracted(false);
    setExtractionFailed(false);
  }

  async function testConnectionAndFetchModels() {
    if (provider === "ollama") {
      setOllamaEndpoint(ollamaEndpoint);
    } else {
      setLmStudioEndpoint(lmStudioEndpoint);
    }
    setLlmBusy(true);
    setStatusMessage(null);
    try {
      const fetchedModels = await getLlmModels(provider, endpoint.trim());
      setModels(fetchedModels);
      if (fetchedModels.length === 0) {
        setStatusMessage({ text: "Connected, but no models were returned.", kind: "error" });
      } else {
        const nextModel =
          selectedModel && fetchedModels.includes(selectedModel) ? selectedModel : fetchedModels[0];
        setSelectedModel(nextModel);
        setStatusMessage({
          text: `Connected. Found ${fetchedModels.length} model(s).`,
          kind: "info",
        });
      }
    } catch (error) {
      setStatusMessage({
        text: error instanceof Error ? error.message : String(error),
        kind: "error",
      });
    } finally {
      setLlmBusy(false);
    }
  }

  async function runExtractionOnce() {
    if (!canRunExtraction) {
      setStatusMessage({ text: "Configure endpoint + model before extraction.", kind: "error" });
      return false;
    }
    if (
      !answers.displayName.trim() &&
      !answers.useMindVaultFor.trim() &&
      !answers.workContext.trim() &&
      !answers.interests.trim()
    ) {
      setStatusMessage({
        text: "Fill at least one Basics answer before extraction.",
        kind: "error",
      });
      return false;
    }

    setExtractBusy(true);
    setStatusMessage(null);
    try {
      const extracted = await unwrapIpcResult(
        onboardingExtractProposals(answersJson, provider, endpoint.trim(), selectedModel.trim())
      );
      const editableRows: EditableStagedProposal[] = extracted.map((proposal) =>
        mapExtractedProposalToEditable(proposal)
      );
      setStagedProposals(editableRows);
      setHasExtracted(true);
      setExtractionFailed(false);
      setStatusMessage({
        text: `Extraction complete. Staged ${editableRows.length} proposal(s).`,
        kind: "info",
      });
      return true;
    } catch (error) {
      setExtractionFailed(true);
      setStatusMessage({
        text: error instanceof Error ? error.message : String(error),
        kind: "error",
      });
      return false;
    } finally {
      setExtractBusy(false);
    }
  }

  function goBack() {
    setCurrentStep((value) => Math.max(0, value - 1));
  }

  function mapExtractedProposalToEditable(
    proposal: OnboardingProposedNode
  ): EditableStagedProposal {
    const resolvedVaultId = proposal.resolvedVaultId ?? "";
    return {
      rowId: newRowId(),
      title: proposal.title ?? "",
      summary: proposal.summary ?? "",
      detail: proposal.detail ?? "",
      nodeType: proposal.nodeType ?? "concept",
      sourceType: "onboarding",
      vaultId: availableVaultIds.has(resolvedVaultId) ? resolvedVaultId : "",
      category: proposal.category,
      targetVaultKey: proposal.targetVaultKey,
    };
  }

  function updateStagedProposal(
    rowId: string,
    field: "title" | "summary" | "detail" | "nodeType" | "vaultId",
    value: string
  ) {
    setStagedProposals((current) =>
      current.map((proposal) =>
        proposal.rowId === rowId ? { ...proposal, [field]: value } : proposal
      )
    );
  }

  function removeStagedProposal(rowId: string) {
    setStagedProposals((current) => current.filter((proposal) => proposal.rowId !== rowId));
  }

  async function commitOnboardingAndFinish() {
    if (unresolvedCount > 0) {
      setStatusMessage({
        text: `Resolve vault assignment for ${unresolvedCount} proposal(s) before commit.`,
        kind: "error",
      });
      return;
    }
    const hasInvalidText = stagedProposals.some(
      (proposal) => !proposal.title.trim() || !proposal.summary.trim()
    );
    if (hasInvalidText) {
      setStatusMessage({
        text: "Each proposal must have a title and summary before commit.",
        kind: "error",
      });
      return;
    }
    const hasUnknownVault = stagedProposals.some(
      (proposal) => !availableVaultIds.has(proposal.vaultId.trim())
    );
    if (hasUnknownVault) {
      setStatusMessage({
        text: "One or more selected vaults does not exist in this database. Re-select vaults and try again.",
        kind: "error",
      });
      return;
    }

    const payload: OnboardingNodeCommitInput[] = stagedProposals.map((proposal) => ({
      vaultId: proposal.vaultId.trim(),
      title: proposal.title.trim(),
      summary: proposal.summary.trim(),
      detail: proposal.detail.trim() ? proposal.detail.trim() : undefined,
      nodeType: proposal.nodeType.trim() ? proposal.nodeType.trim() : undefined,
      sourceType: proposal.sourceType.trim() ? proposal.sourceType.trim() : "onboarding",
    }));

    setCommitBusy(true);
    setStatusMessage(null);
    try {
      if (answers.displayName.trim()) {
        await setSetting("displayName", answers.displayName.trim());
      }
      await unwrapIpcResult(onboardingCommit(payload));
      setStatusMessage({ text: "Onboarding committed. Opening Amber…", kind: "info" });
      await onComplete();
    } catch (error) {
      setStatusMessage({
        text: error instanceof Error ? error.message : String(error),
        kind: "error",
      });
    } finally {
      setCommitBusy(false);
    }
  }

  async function goNext() {
    if (provider === "ollama") {
      setOllamaEndpoint(ollamaEndpoint);
    } else {
      setLmStudioEndpoint(lmStudioEndpoint);
    }
    if (currentStep === LLM_STEP_INDEX) {
      setLlmProvider(provider);
      if (selectedModel.trim()) {
        setLlmModel(provider, selectedModel.trim());
      }
    }
    if (isLastStep) {
      await commitOnboardingAndFinish();
      return;
    }
    if (currentStep === REVIEW_STEP_INDEX - 1 && !hasExtracted) {
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
            <h1>Welcome to Amber</h1>
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
          {currentStep === MODEL_SETUP_STEP_INDEX ? (
            <div className="onboarding-llm-grid">
              <ModelSetupPanel
                variant="onboarding"
                onStackSelected={(stackId) => void handleStackSelected(stackId)}
              />
              <div className="onboarding-hint">
                Pick a recommended stack to continue, or use the skip button below if you want to
                set this up later.
              </div>
            </div>
          ) : null}
          {currentStep === BASICS_STEP_INDEX ? (
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
          {currentStep === LLM_STEP_INDEX ? (
            <div className="onboarding-llm-grid">
              <details className="onboarding-advanced-details" open>
                <summary>Advanced LLM settings</summary>
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
                      const next = event.target.value;
                      if (provider === "ollama") {
                        setOllamaEndpointState(next);
                      } else {
                        setLmStudioEndpointState(next);
                      }
                      setModels([]);
                      setStatusMessage(null);
                      setHasExtracted(false);
                      setExtractionFailed(false);
                    }}
                    onBlur={() => {
                      if (provider === "ollama") {
                        setOllamaEndpoint(ollamaEndpoint);
                      } else {
                        setLmStudioEndpoint(lmStudioEndpoint);
                      }
                    }}
                    placeholder={
                      provider === "ollama" ? "http://localhost:11434" : "http://localhost:1234"
                    }
                    disabled={shellBusy}
                  />
                </label>
                <label className="onboarding-field">
                  <span>Model</span>
                  <input
                    type="text"
                    value={selectedModel}
                    onChange={(event) => setSelectedModel(event.target.value)}
                    placeholder="Choose a model name"
                    disabled={shellBusy}
                  />
                </label>
                <button
                  type="button"
                  className="onboarding-primary"
                  onClick={() => void testConnectionAndFetchModels()}
                  disabled={shellBusy}
                >
                  {llmBusy ? "Testing connection…" : "Test connection & fetch models"}
                </button>
                {models.length > 0 ? (
                  <div className="onboarding-model-results" aria-label="Available models">
                    {models.map((model) => (
                      <button
                        type="button"
                        key={model}
                        className={model === selectedModel ? "active" : ""}
                        onClick={() => setSelectedModel(model)}
                        disabled={shellBusy}
                      >
                        {model}
                      </button>
                    ))}
                  </div>
                ) : null}
                {statusMessage ? (
                  <p
                    className={`onboarding-status ${statusMessage.kind}`}
                    role={statusMessage.kind === "error" ? "alert" : "status"}
                  >
                    {statusMessage.text}
                  </p>
                ) : null}
              </details>
            </div>
          ) : null}
          {currentStep === REVIEW_STEP_INDEX ? (
            <div className="onboarding-review-list" aria-label="Staged onboarding proposals">
              {stagedProposals.length === 0 ? (
                <p className="onboarding-hint">
                  No staged proposals. You can still commit and finish with zero nodes.
                </p>
              ) : (
                stagedProposals.map((proposal, index) => (
                  <article key={proposal.rowId} className="onboarding-proposal-card">
                    <div className="onboarding-proposal-header">
                      <h3>Proposal {index + 1}</h3>
                      <small>{proposal.nodeType || "concept"}</small>
                    </div>
                    <label className="onboarding-field">
                      <span>Title</span>
                      <input
                        type="text"
                        value={proposal.title}
                        onChange={(event) =>
                          updateStagedProposal(proposal.rowId, "title", event.target.value)
                        }
                        disabled={shellBusy}
                      />
                    </label>
                    <label className="onboarding-field">
                      <span>Summary</span>
                      <textarea
                        rows={2}
                        value={proposal.summary}
                        onChange={(event) =>
                          updateStagedProposal(proposal.rowId, "summary", event.target.value)
                        }
                        disabled={shellBusy}
                      />
                    </label>
                    <label className="onboarding-field">
                      <span>Detail (optional)</span>
                      <textarea
                        rows={2}
                        value={proposal.detail}
                        onChange={(event) =>
                          updateStagedProposal(proposal.rowId, "detail", event.target.value)
                        }
                        disabled={shellBusy}
                      />
                    </label>
                    <label className="onboarding-field">
                      <span>Vault</span>
                      <select
                        value={proposal.vaultId}
                        onChange={(event) =>
                          updateStagedProposal(proposal.rowId, "vaultId", event.target.value)
                        }
                        disabled={shellBusy}
                      >
                        <option value="">Choose vault</option>
                        {vaultOptions.map((vault) => (
                          <option key={vault.id} value={vault.id}>
                            {vault.name} ({vault.id})
                          </option>
                        ))}
                      </select>
                    </label>
                    <p className="onboarding-meta">
                      Vault target:{" "}
                      {proposal.vaultId
                        ? `${vaultDisplayLabel(
                            proposal.vaultId,
                            vaultNameById[proposal.vaultId] ?? null
                          )} (${proposal.vaultId})`
                        : "Unmapped — resolve before saving"}
                    </p>
                    <p className="onboarding-meta">
                      Category/key source: {proposal.targetVaultKey ?? proposal.category ?? "none"}
                    </p>
                    <div className="onboarding-inline-actions">
                      <button
                        type="button"
                        className="onboarding-inline-button"
                        onClick={() => removeStagedProposal(proposal.rowId)}
                        disabled={shellBusy}
                      >
                        Remove proposal
                      </button>
                    </div>
                  </article>
                ))
              )}
            </div>
          ) : null}

          {statusMessage ? (
            <p className={statusMessage.kind === "error" ? "onboarding-error" : "onboarding-hint"}>
              {statusMessage.text}
            </p>
          ) : null}
          {errorMessage ? <p className="onboarding-error">{errorMessage}</p> : null}
        </div>

        <footer className="onboarding-actions">
          <button type="button" onClick={goBack} disabled={currentStep === 0 || shellBusy}>
            Back
          </button>
          {currentStep === REVIEW_STEP_INDEX - 1 && extractionFailed ? (
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setHasExtracted(true);
                setExtractionFailed(false);
                setCurrentStep(REVIEW_STEP_INDEX);
              }}
              disabled={shellBusy}
            >
              Skip extraction
            </button>
          ) : null}
          <button
            type="button"
            className="primary"
            onClick={() => void goNext()}
            disabled={shellBusy || (isLastStep && unresolvedCount > 0)}
          >
            {currentStep === MODEL_SETUP_STEP_INDEX
              ? "I’ll set this up later"
              : isLastStep
                ? "Commit and finish onboarding"
                : "Next"}
          </button>
        </footer>
      </div>
    </section>
  );
}

export default OnboardingShell;
