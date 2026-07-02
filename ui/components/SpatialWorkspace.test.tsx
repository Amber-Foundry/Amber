import { render, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SpatialWorkspace from "./SpatialWorkspace";
import {
  vaultOpen,
  nodeOpen,
  nodeRedacted,
  doorOpenToRedacted,
} from "../test/fixtures/privacyFixtures";

vi.mock("../services/vaults", () => ({
  listVaults: vi.fn(),
  createVault: vi.fn(),
  deleteVault: vi.fn(),
  updateVault: vi.fn(),
  updateVaultPosition: vi.fn(),
  updateVaultColorTheme: vi.fn(),
}));

vi.mock("../services/nodes", () => ({
  getNodes: vi.fn(),
  createNode: vi.fn(),
  deleteNode: vi.fn(),
  updateNode: vi.fn(),
}));

vi.mock("../services/doors", () => ({
  listAllDoors: vi.fn(),
}));

vi.mock("../services/auth", () => ({
  isAuthSetup: vi.fn(),
  setMasterPassword: vi.fn(),
  verifyMasterPassword: vi.fn(),
}));

import { listVaults } from "../services/vaults";
import { getNodes } from "../services/nodes";
import { listAllDoors } from "../services/doors";

const mockListVaults = vi.mocked(listVaults);
const mockGetNodes = vi.mocked(getNodes);
const mockListAllDoors = vi.mocked(listAllDoors);

function renderWorkspace(isRedactedUnlocked: boolean) {
  return render(
    <SpatialWorkspace
      selectedVaultId={null}
      selectedNodeId={null}
      onSelectVault={vi.fn()}
      onSelectNode={vi.fn()}
      refreshKey={0}
      isRedactedUnlocked={isRedactedUnlocked}
      setIsRedactedUnlocked={vi.fn()}
    />
  );
}

describe("SpatialWorkspace connector privacy", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListVaults.mockResolvedValue([vaultOpen]);
    mockGetNodes.mockResolvedValue([nodeOpen, nodeRedacted]);
    mockListAllDoors.mockResolvedValue({ data: [doorOpenToRedacted], error: null });
  });

  it("omits connectors involving locked-out redacted nodes", async () => {
    const { container } = renderWorkspace(false);

    await waitFor(() => {
      expect(container.querySelectorAll(".spatial-connection-line").length).toBe(0);
    });
  });

  it("renders connectors after redacted unlock", async () => {
    const { container } = renderWorkspace(true);

    await waitFor(() => {
      expect(container.querySelectorAll(".spatial-connection-line").length).toBe(1);
    });
  });
});
