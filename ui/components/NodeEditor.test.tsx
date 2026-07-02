import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import NodeEditor from "./NodeEditor";
import {
  nodeOpen,
  nodeRedacted,
  nodeLocked,
  vaultOpen,
  vaultLocked,
} from "../test/fixtures/privacyFixtures";

vi.mock("./NodeEditorDetail", () => ({
  default: () => <div data-testid="node-editor-detail">Detail</div>,
}));

vi.mock("../services/nodes", () => ({
  getNode: vi.fn(),
  getAllNodes: vi.fn(),
  updateNode: vi.fn(),
  touchNode: vi.fn(),
  deleteNode: vi.fn(),
  refreshAllPriorityScores: vi.fn(),
}));

vi.mock("../services/vaults", () => ({
  listVaults: vi.fn(),
}));

vi.mock("../services/tags", () => ({
  listTags: vi.fn(),
  getNodeTags: vi.fn(),
  addNodeTag: vi.fn(),
  removeNodeTag: vi.fn(),
  createTag: vi.fn(),
}));

vi.mock("../services/doors", () => ({
  listOutgoingDoors: vi.fn(),
  listIncomingDoors: vi.fn(),
  createDoor: vi.fn(),
  deleteDoor: vi.fn(),
  repointDoor: vi.fn(),
}));

vi.mock("../services/auth", () => ({
  isAuthSetup: vi.fn(),
  setMasterPassword: vi.fn(),
  verifyMasterPassword: vi.fn(),
}));

import {
  getNode,
  getAllNodes,
  updateNode,
  touchNode,
  refreshAllPriorityScores,
} from "../services/nodes";
import { listVaults } from "../services/vaults";
import { listTags, getNodeTags } from "../services/tags";
import { listOutgoingDoors, listIncomingDoors } from "../services/doors";
import { isAuthSetup } from "../services/auth";

const mockGetNode = vi.mocked(getNode);
const mockGetAllNodes = vi.mocked(getAllNodes);
const mockUpdateNode = vi.mocked(updateNode);
const mockTouchNode = vi.mocked(touchNode);
const mockListVaults = vi.mocked(listVaults);
const mockListTags = vi.mocked(listTags);
const mockGetNodeTags = vi.mocked(getNodeTags);
const mockListOutgoingDoors = vi.mocked(listOutgoingDoors);
const mockListIncomingDoors = vi.mocked(listIncomingDoors);
const mockIsAuthSetup = vi.mocked(isAuthSetup);
const mockUpdateNodeFn = vi.mocked(updateNode);
const mockRefreshAllPriorityScores = vi.mocked(refreshAllPriorityScores);

function renderEditor(selectedNodeId: string) {
  return render(
    <NodeEditor
      selectedNodeId={selectedNodeId}
      refreshKey={0}
      isRedactedUnlocked={false}
      setIsRedactedUnlocked={vi.fn()}
    />
  );
}

function setupNodeMocks(node: typeof nodeOpen, vaults = [vaultOpen]) {
  mockGetNode.mockResolvedValue(node);
  mockGetAllNodes.mockResolvedValue([node]);
  mockListVaults.mockResolvedValue(vaults);
  mockListTags.mockResolvedValue({ data: [], error: null });
  mockGetNodeTags.mockResolvedValue({ data: [], error: null });
  mockListOutgoingDoors.mockResolvedValue({ data: [], error: null });
  mockListIncomingDoors.mockResolvedValue({ data: [], error: null });
  mockTouchNode.mockResolvedValue(true);
  mockIsAuthSetup.mockResolvedValue({ data: true, error: null });
  mockUpdateNodeFn.mockImplementation(async (input) => ({
    ...node,
    title: input.title ?? node.title,
    summary: input.summary ?? node.summary,
    detail: input.detail ?? node.detail,
    privacyTier: input.privacyTier ?? node.privacyTier,
  }));
  mockRefreshAllPriorityScores.mockResolvedValue(0);
}

describe("NodeEditor privacy gating", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows redacted lock screen and hides privacy select when locked out", async () => {
    setupNodeMocks(nodeRedacted);

    renderEditor(nodeRedacted.id);

    await waitFor(() => {
      expect(
        screen.getByText("Highly Restricted - Redacted. Enter master password.")
      ).toBeInTheDocument();
    });

    expect(document.querySelector(".redacted-lock-screen")).toBeInTheDocument();
    expect(document.querySelector(".editor-privacy select")).not.toBeInTheDocument();
  });

  it("shows content lock for locked nodes and disables editing", async () => {
    setupNodeMocks(nodeLocked);

    renderEditor(nodeLocked.id);

    await waitFor(() => {
      expect(screen.getByText("Content Protected")).toBeInTheDocument();
    });

    const titleInput = screen.getByPlaceholderText("Title") as HTMLInputElement;
    const summaryInput = screen.getByPlaceholderText("Summary") as HTMLTextAreaElement;

    expect(titleInput.disabled).toBe(true);
    expect(summaryInput.disabled).toBe(true);
    expect(screen.queryByTestId("node-editor-detail")).not.toBeInTheDocument();
  });

  it("disables less restrictive options when parent vault is locked", async () => {
    const nodeInLockedVault = { ...nodeOpen, vaultId: vaultLocked.id };
    setupNodeMocks(nodeInLockedVault, [vaultLocked]);

    renderEditor(nodeInLockedVault.id);

    await waitFor(() => {
      const openOption = screen.getByRole("option", { name: "Open" }) as HTMLOptionElement;
      expect(openOption.disabled).toBe(true);
    });

    const localOnlyOption = screen.getByRole("option", {
      name: "Local-Only",
    }) as HTMLOptionElement;
    const lockedOption = screen.getByRole("option", { name: "Locked" }) as HTMLOptionElement;

    expect(localOnlyOption.disabled).toBe(true);
    expect(lockedOption.disabled).toBe(false);
  });

  it("auto-saves privacy tier changes via updateNode", async () => {
    const user = userEvent.setup();
    setupNodeMocks(nodeOpen);

    renderEditor(nodeOpen.id);

    await waitFor(() => {
      expect(screen.getByText("Privacy")).toBeInTheDocument();
    });

    const privacySelect = document.querySelector(".editor-privacy select") as HTMLSelectElement;
    await user.selectOptions(privacySelect, "local_only");

    await waitFor(
      () => {
        expect(mockUpdateNode).toHaveBeenCalledWith(
          expect.objectContaining({
            id: nodeOpen.id,
            privacyTier: "local_only",
          })
        );
      },
      { timeout: 3000 }
    );
  });
});
