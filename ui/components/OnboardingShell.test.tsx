import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import OnboardingShell from "./OnboardingShell";

// Mock Tauri/IPC operations
vi.mock("../ipc", () => ({
  onboardingCommit: vi.fn(),
  onboardingExtractProposals: vi.fn(),
}));

vi.mock("../services/ipcResult", () => ({
  unwrapIpcResult: vi.fn((promiseOrVal) => {
    if (promiseOrVal instanceof Promise) {
      return promiseOrVal.then((res) => {
        if (res && res.err) throw new Error(res.err);
        return res.ok;
      });
    }
    if (promiseOrVal && promiseOrVal.err) throw new Error(promiseOrVal.err);
    return promiseOrVal.ok;
  }),
}));

vi.mock("../services/nodes", () => ({
  getLlmModels: vi.fn(),
}));

vi.mock("../services/vaults", () => ({
  listVaults: vi.fn(),
}));

vi.mock("../services/settings", () => ({
  setSetting: vi.fn(),
}));

// Mock settings storage utility helpers
vi.mock("../utils/settings", () => {
  let provider = "ollama";
  let ollamaEndpoint = "http://localhost:11434";
  let lmStudioEndpoint = "http://localhost:1234";
  let selectedModel = "";

  return {
    getLlmProvider: vi.fn(() => provider),
    setLlmProvider: vi.fn((val) => {
      provider = val;
    }),
    getOllamaEndpoint: vi.fn(() => ollamaEndpoint),
    setOllamaEndpoint: vi.fn((val) => {
      ollamaEndpoint = val;
    }),
    getLmStudioEndpoint: vi.fn(() => lmStudioEndpoint),
    setLmStudioEndpoint: vi.fn((val) => {
      lmStudioEndpoint = val;
    }),
    getLlmModel: vi.fn(() => selectedModel),
    setLlmModel: vi.fn((_p, val) => {
      selectedModel = val;
    }),
  };
});

import { onboardingCommit, onboardingExtractProposals } from "../ipc";
import { getLlmModels } from "../services/nodes";
import { listVaults } from "../services/vaults";
import { setSetting } from "../services/settings";

const mockOnboardingCommit = vi.mocked(onboardingCommit);
const mockOnboardingExtractProposals = vi.mocked(onboardingExtractProposals);
const mockGetLlmModels = vi.mocked(getLlmModels);
const mockListVaults = vi.mocked(listVaults);
const mockSetSetting = vi.mocked(setSetting);

describe("OnboardingShell Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListVaults.mockResolvedValue([
      {
        id: "vault_personal",
        name: "Personal",
        icon: null,
        description: null,
        privacyTier: "open",
        priorityProfile: "standard",
        summaryNodeId: null,
        sortOrder: 0,
        createdAt: "",
        updatedAt: "",
        deletedAt: null,
        meta: "{}",
        uiMetadata: "{}",
      },
      {
        id: "vault_work",
        name: "Work",
        icon: null,
        description: null,
        privacyTier: "open",
        priorityProfile: "standard",
        summaryNodeId: null,
        sortOrder: 1,
        createdAt: "",
        updatedAt: "",
        deletedAt: null,
        meta: "{}",
        uiMetadata: "{}",
      },
    ]);
  });

  it("renders steps, navigates to basics and llm setup, and skip onboarding handles skip action", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const onSkip = vi.fn();

    render(
      <OnboardingShell onComplete={onComplete} onSkip={onSkip} busy={false} errorMessage={null} />
    );

    // Initial step 0 is Model Setup
    expect(screen.getByText("Welcome to MindVault")).toBeInTheDocument();
    expect(screen.getByText("1. Model Setup")).toBeInTheDocument();

    // Click skip onboarding
    const skipBtn = screen.getByText("Skip onboarding");
    await user.click(skipBtn);
    expect(onSkip).toHaveBeenCalledTimes(1);

    // Advance from Model Setup to Basics
    await user.click(screen.getByText("I’ll set this up later"));
    expect(screen.getByText("2. Basics")).toBeInTheDocument();

    // Enter name and click next
    const nameInput = screen.getByPlaceholderText("Kevin") as HTMLInputElement;
    await user.type(nameInput, "Aashish");

    const nextBtn = screen.getByText("Next");
    await user.click(nextBtn);

    // Moves to step 2 (LLM setup)
    expect(screen.getByText("3. LLM setup")).toBeInTheDocument();
  });

  it("handles LLM setup validation if endpoint or model is missing or if basics is empty", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const onSkip = vi.fn();

    render(
      <OnboardingShell onComplete={onComplete} onSkip={onSkip} busy={false} errorMessage={null} />
    );

    // Step 0 -> Step 1 (Basics)
    await user.click(screen.getByText("I’ll set this up later"));
    expect(screen.getByText("2. Basics")).toBeInTheDocument();

    // Step 1 -> Step 2 (LLM setup) leaving answers blank
    await user.click(screen.getByText("Next"));
    expect(screen.getByText("3. LLM setup")).toBeInTheDocument();

    // Try going next on Step 2 (without selected model)
    await user.click(screen.getByText("Next"));
    expect(screen.getByText("Configure endpoint + model before extraction.")).toBeInTheDocument();
  });

  it("tests LLM connection, fetches models, and allows extraction to transition to review", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const onSkip = vi.fn();

    mockGetLlmModels.mockResolvedValue(["llama3", "mistral"]);
    mockOnboardingExtractProposals.mockResolvedValue({
      ok: [
        {
          title: "New Staged Concept",
          summary: "This is a summary",
          detail: "This is a detail description",
          nodeType: "concept",
          resolvedVaultId: "vault_personal",
          category: "Work",
          targetVaultKey: "Vault",
        },
      ],
    });

    render(
      <OnboardingShell onComplete={onComplete} onSkip={onSkip} busy={false} errorMessage={null} />
    );

    // Step 0: Model Setup -> Step 1: Basics
    await user.click(screen.getByText("I’ll set this up later"));

    // Step 1: Basics -> Step 2: LLM Setup
    const nameInput = screen.getByPlaceholderText("Kevin") as HTMLInputElement;
    await user.type(nameInput, "Evan");
    await user.click(screen.getByText("Next"));

    // Step 2: LLM Setup
    expect(screen.getByText("3. LLM setup")).toBeInTheDocument();

    // Click connection check button
    const testConnectionBtn = screen.getByText(/test connection & fetch models/i);
    await user.click(testConnectionBtn);

    expect(mockGetLlmModels).toHaveBeenCalledWith("ollama", "http://localhost:11434");

    // Model dropdown options should populate
    await waitFor(() => {
      expect(screen.getByText("Connected. Found 2 model(s).")).toBeInTheDocument();
    });

    const modelSelect = screen.getByRole("combobox") as HTMLSelectElement;
    await user.selectOptions(modelSelect, "llama3");

    // Click next (triggers extraction first)
    await user.click(screen.getByText("Next"));

    expect(mockOnboardingExtractProposals).toHaveBeenCalledTimes(1);

    // Transitioned to review step
    await waitFor(() => {
      expect(screen.getByText("4. Review")).toBeInTheDocument();
    });

    // Should display extraction success message
    expect(screen.getByText(/Extraction complete. Staged 1 proposal\(s\)./)).toBeInTheDocument();
    expect(screen.getByText("Proposal 1")).toBeInTheDocument();
  });

  it("allows editing staged proposals, checks commit validations, and commits successfully", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const onSkip = vi.fn();

    mockGetLlmModels.mockResolvedValue(["llama3"]);
    mockOnboardingExtractProposals.mockResolvedValue({
      ok: [
        {
          title: "Proposal A",
          summary: "Summary A",
          nodeType: "task",
        },
      ],
    });
    mockOnboardingCommit.mockResolvedValue({ ok: true });

    render(
      <OnboardingShell onComplete={onComplete} onSkip={onSkip} busy={false} errorMessage={null} />
    );

    // Step 0 -> Step 1 (Basics)
    await user.click(screen.getByText("I’ll set this up later"));

    // Fill basics and go to Step 2 (LLM setup)
    const nameInput = screen.getByPlaceholderText("Kevin") as HTMLInputElement;
    await user.type(nameInput, "Kevin");
    await user.click(screen.getByText("Next"));

    // Set model and go to Step 3 (Review)
    await user.click(screen.getByText(/test connection & fetch models/i));
    await waitFor(() => {
      expect(screen.getByRole("combobox")).toBeInTheDocument();
    });
    await user.selectOptions(screen.getByRole("combobox"), "llama3");
    await user.click(screen.getByText("Next"));

    await waitFor(() => {
      expect(screen.getByText("4. Review")).toBeInTheDocument();
    });

    // Check that commit button is disabled since vault is not chosen (unresolvedCount > 0)
    const commitBtn = screen.getByText("Commit and finish onboarding") as HTMLButtonElement;
    expect(commitBtn.disabled).toBe(true);

    // Select vault for the proposal to resolve assignment
    const vaultSelect = screen.getByRole("combobox") as HTMLSelectElement;
    await user.selectOptions(vaultSelect, "vault_personal");

    // Now the commit button should be enabled
    expect(commitBtn.disabled).toBe(false);

    // Edit title to empty to test invalid text validation
    const titleInput = screen.getByDisplayValue("Proposal A") as HTMLInputElement;
    await user.clear(titleInput);
    await user.click(commitBtn);

    expect(
      screen.getByText("Each proposal must have a title and summary before commit.")
    ).toBeInTheDocument();

    // Put title back and commit
    await user.type(titleInput, "Valid Proposal Title");
    await user.click(commitBtn);

    // Verify commit payload contains correct settings and proposals
    expect(mockSetSetting).toHaveBeenCalledWith("displayName", "Kevin");
    expect(mockOnboardingCommit).toHaveBeenCalledWith([
      {
        vaultId: "vault_personal",
        title: "Valid Proposal Title",
        summary: "Summary A",
        detail: undefined,
        nodeType: "task",
        sourceType: "onboarding",
      },
    ]);

    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });
  });

  it("renders Skip extraction button when extraction fails, allowing manual skip", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const onSkip = vi.fn();

    // Mock extraction failure
    mockGetLlmModels.mockResolvedValue(["llama3"]);
    mockOnboardingExtractProposals.mockResolvedValue({
      err: "Failed to connect to LLM server",
    });

    render(
      <OnboardingShell onComplete={onComplete} onSkip={onSkip} busy={false} errorMessage={null} />
    );

    // Step 0 -> Step 1 (Basics)
    await user.click(screen.getByText("I’ll set this up later"));

    // Step 1: Fill basics answer and move to Step 2
    const nameInput = screen.getByPlaceholderText("Kevin") as HTMLInputElement;
    await user.type(nameInput, "Aashish");
    await user.click(screen.getByText("Next"));

    // Step 2: LLM Setup. Set model.
    await user.click(screen.getByText(/test connection & fetch models/i));
    await waitFor(() => {
      expect(screen.getByRole("combobox")).toBeInTheDocument();
    });
    await user.selectOptions(screen.getByRole("combobox"), "llama3");

    // Click Next (triggers extraction which fails)
    await user.click(screen.getByText("Next"));

    // Wait for the failure message and the Skip Extraction button to appear
    await waitFor(() => {
      expect(screen.getByText("Failed to connect to LLM server")).toBeInTheDocument();
    });
    const skipExtractionBtn = screen.getByText("Skip extraction");
    expect(skipExtractionBtn).toBeInTheDocument();

    // Click Skip Extraction
    await user.click(skipExtractionBtn);

    // Should transition to step 3 (Review) with 0 staged proposals
    expect(screen.getByText("4. Review")).toBeInTheDocument();
    expect(
      screen.getByText("No staged proposals. You can still commit and finish with zero nodes.")
    ).toBeInTheDocument();
  });

  it("allows navigating back to previous steps using the Back button", async () => {
    const user = userEvent.setup();
    render(
      <OnboardingShell onComplete={vi.fn()} onSkip={vi.fn()} busy={false} errorMessage={null} />
    );

    // Verify Back button is disabled on Step 0 (Model Setup)
    const backBtn = screen.getByRole("button", { name: "Back" }) as HTMLButtonElement;
    expect(backBtn.disabled).toBe(true);

    // Move to Step 1 (Basics)
    await user.click(screen.getByText("I’ll set this up later"));
    expect(screen.getByText("2. Basics")).toBeInTheDocument();

    // Now Back button should be enabled
    expect(backBtn.disabled).toBe(false);

    // Click Back
    await user.click(backBtn);

    // Should return to Step 0 (Model Setup)
    expect(screen.getByText("1. Model Setup")).toBeInTheDocument();
    expect(backBtn.disabled).toBe(true);
  });
});
