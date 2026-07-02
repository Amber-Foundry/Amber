import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import VaultSidebar from "./VaultSidebar";
import { vaultOpen, vaultRedacted, vaultLocked } from "../test/fixtures/privacyFixtures";

vi.mock("../services/nodes", () => ({
  getAllNodes: vi.fn(),
}));

vi.mock("../services/vaults", () => ({
  listVaults: vi.fn(),
  createVault: vi.fn(),
  deleteVault: vi.fn(),
  updateVault: vi.fn(),
}));

vi.mock("../services/auth", () => ({
  isAuthSetup: vi.fn(),
  setMasterPassword: vi.fn(),
  verifyMasterPassword: vi.fn(),
}));

import { getAllNodes } from "../services/nodes";
import { listVaults, updateVault } from "../services/vaults";
import { isAuthSetup } from "../services/auth";

const mockGetAllNodes = vi.mocked(getAllNodes);
const mockListVaults = vi.mocked(listVaults);
const mockUpdateVault = vi.mocked(updateVault);
const mockIsAuthSetup = vi.mocked(isAuthSetup);

function renderSidebar(isRedactedUnlocked = false) {
  return render(
    <VaultSidebar
      selectedVaultId={null}
      refreshKey={0}
      onSelectVault={vi.fn()}
      onSelectNode={vi.fn()}
      onVaultCreated={vi.fn()}
      onVaultDeleted={vi.fn()}
      onOpenDashboard={vi.fn()}
      onOpenSettings={vi.fn()}
      isRedactedUnlocked={isRedactedUnlocked}
      setIsRedactedUnlocked={vi.fn()}
    />
  );
}

describe("VaultSidebar privacy UI", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetAllNodes.mockResolvedValue([]);
    mockIsAuthSetup.mockResolvedValue({ data: true, error: null });
    mockUpdateVault.mockResolvedValue(vaultOpen);
  });

  it("renders redacted vault names as [REDACTED] when locked out", async () => {
    mockListVaults.mockResolvedValue([vaultRedacted]);

    renderSidebar(false);

    await waitFor(() => {
      expect(screen.getByText("[REDACTED]")).toBeInTheDocument();
    });
    expect(screen.queryByText("Secret Vault")).not.toBeInTheDocument();
  });

  it("renders open vault names normally", async () => {
    mockListVaults.mockResolvedValue([vaultOpen]);

    renderSidebar(false);

    await waitFor(() => {
      expect(screen.getByText("Open Vault")).toBeInTheDocument();
    });
  });

  it("prompts for unlock when saving redacted tier while locked out", async () => {
    const user = userEvent.setup();
    mockListVaults.mockResolvedValue([vaultOpen]);

    renderSidebar(false);

    await waitFor(() => {
      expect(screen.getByText("Open Vault")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Update settings for Open Vault" }));

    await waitFor(() => {
      expect(screen.getByText("Vault Settings")).toBeInTheDocument();
    });

    const privacySelect = screen.getByRole("combobox");
    await user.selectOptions(privacySelect, "redacted");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => {
      expect(
        screen.getByText("Enter your master password to unlock this tier.")
      ).toBeInTheDocument();
    });

    expect(mockUpdateVault).not.toHaveBeenCalled();
  });

  it("shows password field when deleting a locked vault", async () => {
    const user = userEvent.setup();
    mockListVaults.mockResolvedValue([vaultLocked]);

    renderSidebar(false);

    await waitFor(() => {
      expect(screen.getByText("Locked Vault")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Delete Locked Vault" }));

    await waitFor(() => {
      expect(screen.getByText("Delete Vault")).toBeInTheDocument();
    });

    const modal = screen.getByText("Delete Vault").closest(".delete-confirm-modal");
    expect(modal).not.toBeNull();
    expect(within(modal as HTMLElement).getByText("Master Password")).toBeInTheDocument();
    expect(
      within(modal as HTMLElement).getByPlaceholderText("Master password")
    ).toBeInTheDocument();
  });
});
