import { useState } from "react";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import DiffPanel from "./DiffPanel";
import { Changeset, ChangesetItem } from "../ipc";

// Mock the services before imports
vi.mock("../services/memoryAgent", () => ({
  listPendingChangesets: vi.fn(),
  listResolvedChangesets: vi.fn(),
  listChangesetItems: vi.fn(),
  commitChangeset: vi.fn(),
}));

vi.mock("../services/auth", () => ({
  verifyMasterPassword: vi.fn(),
}));

import {
  listPendingChangesets,
  listResolvedChangesets,
  listChangesetItems,
  commitChangeset,
} from "../services/memoryAgent";
import { verifyMasterPassword } from "../services/auth";

// Stateful test harness to support active changesets propagation
function Harness({
  initialChangesetId = null as string | null,
  onClose = () => {},
  onRefreshPendingCount = () => {},
}: {
  initialChangesetId?: string | null;
  onClose?: () => void;
  onRefreshPendingCount?: () => void;
}) {
  const [activeChangesetId, setActiveChangesetId] = useState<string | null>(initialChangesetId);

  return (
    <DiffPanel
      onClose={onClose}
      activeChangesetId={activeChangesetId}
      onSelectChangeset={setActiveChangesetId}
      onRefreshPendingCount={onRefreshPendingCount}
    />
  );
}

const mockPendingChangesets: Changeset[] = [
  {
    id: "cs-1",
    sessionId: "session-abc",
    status: "Pending",
    itemCount: 2,
    acceptedCount: 0,
    dismissedCount: 0,
    modelUsed: "gpt-4o",
    createdAt: "2026-06-23 18:00:00",
    reviewedAt: null,
    summary: "Refined testing architecture",
  },
  {
    id: "cs-3",
    sessionId: "session-xyz",
    status: "Pending",
    itemCount: 1,
    acceptedCount: 0,
    dismissedCount: 0,
    modelUsed: "ollama-llama3",
    createdAt: "2026-06-23 18:30:00",
    reviewedAt: null,
    summary: "Another changes list",
  },
];

const mockResolvedChangesets: Changeset[] = [
  {
    id: "cs-2",
    sessionId: "session-123",
    status: "Committed",
    itemCount: 1,
    acceptedCount: 1,
    dismissedCount: 0,
    modelUsed: "ollama-llama3",
    createdAt: "2026-06-23 17:00:00",
    reviewedAt: "2026-06-23 17:05:00",
    summary: "Old updates committed",
  },
];

const mockChangesetItems: ChangesetItem[] = [
  {
    id: "item-1",
    changesetId: "cs-1",
    itemType: "add",
    targetNodeId: null,
    proposedData: JSON.stringify({
      title: "New Component Tests",
      summary: "Add comprehensive unit tests.",
      detail: "Colocating component tests alongside source files.",
    }),
    existingData: null,
    similarity: null,
    mergeWithId: null,
    doorId: null,
    status: "pending",
    reviewedAt: null,
    sortOrder: 1,
    crossVaultAnomaly: false,
    anomalyWarning: null,
  },
  {
    id: "item-2",
    changesetId: "cs-1",
    itemType: "update",
    targetNodeId: "node-xyz",
    proposedData: JSON.stringify({
      title: "Settings Configuration",
      summary: "Adjust local storage parsing.",
      detail: "Added validation hooks.",
    }),
    existingData: JSON.stringify({
      title: "Settings",
      summary: "Local storage parsing.",
      detail: "Original validation.",
    }),
    similarity: 0.85,
    mergeWithId: null,
    doorId: null,
    status: "pending",
    reviewedAt: null,
    sortOrder: 2,
    crossVaultAnomaly: false,
    anomalyWarning: null,
  },
];

describe("DiffPanel Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();

    // Default mock resolves
    vi.mocked(listPendingChangesets).mockResolvedValue(mockPendingChangesets);
    vi.mocked(listResolvedChangesets).mockResolvedValue(mockResolvedChangesets);
    vi.mocked(listChangesetItems).mockResolvedValue(mockChangesetItems);
    vi.mocked(commitChangeset).mockResolvedValue(true);
    vi.mocked(verifyMasterPassword).mockResolvedValue({ data: true, error: null });
  });

  it("renders pending changesets on mount and filters them via search", async () => {
    render(<Harness />);

    // Renders the changeset cards
    await waitFor(() => {
      expect(screen.getByText("Refined testing architecture")).toBeInTheDocument();
      expect(screen.getByText("Another changes list")).toBeInTheDocument();
    });

    const searchInput = screen.getByPlaceholderText("Search changesets...");

    // Type query to filter by model (since search filters by id and modelUsed)
    fireEvent.change(searchInput, { target: { value: "gpt-4o" } });

    // cs-1 matches "gpt-4o", cs-3 does not
    expect(screen.getByText("Refined testing architecture")).toBeInTheDocument();
    expect(screen.queryByText("Another changes list")).not.toBeInTheDocument();
  });

  it("switches tabs and fetches history correctly", async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByText("Refined testing architecture")).toBeInTheDocument();
    });

    const historyTabBtn = screen.getByRole("button", { name: "History" });
    fireEvent.click(historyTabBtn);

    // Listens to resolved changesets
    await waitFor(() => {
      expect(screen.getByText("Old updates committed")).toBeInTheDocument();
    });
    expect(screen.queryByText("Refined testing architecture")).not.toBeInTheDocument();
    expect(listResolvedChangesets).toHaveBeenCalledTimes(1);
  });

  it("drills down into changeset items and applies category badges filtering", async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByText("Refined testing architecture")).toBeInTheDocument();
    });

    // Click on the changeset card to drill down
    const card = screen.getByText("Refined testing architecture");
    fireEvent.click(card);

    // Verify changeset items render
    await waitFor(() => {
      expect(screen.getByText("New Component Tests")).toBeInTheDocument();
      // "Settings" is present in both Current State title and Proposed title changes, so use getAllByText
      expect(screen.getAllByText("Settings").length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText("Configuration")).toBeInTheDocument();
    });

    // Exposes category badges inside drill-down view
    const addBadge = screen.getByRole("button", { name: "ADD" });
    const updateBadge = screen.getByRole("button", { name: "UPDATE" });
    const allBadge = screen.getByRole("button", { name: "ALL" });

    // Click ADD to filter
    fireEvent.click(addBadge);

    expect(screen.getByText("New Component Tests")).toBeInTheDocument();
    expect(screen.queryByText("Configuration")).not.toBeInTheDocument();

    // Click UPDATE to filter
    fireEvent.click(updateBadge);

    expect(screen.queryByText("New Component Tests")).not.toBeInTheDocument();
    expect(screen.getByText("Configuration")).toBeInTheDocument();

    // Reset filter
    fireEvent.click(allBadge);

    expect(screen.getByText("New Component Tests")).toBeInTheDocument();
    expect(screen.getByText("Configuration")).toBeInTheDocument();
  });

  it("handles Bulk Accept all action with correct payload formatting", async () => {
    const user = userEvent.setup();
    render(<Harness initialChangesetId="cs-1" />);

    // Verify items loaded
    await waitFor(() => {
      expect(screen.getByText("New Component Tests")).toBeInTheDocument();
    });

    const acceptAllBtn = screen.getByRole("button", { name: "Accept All" });
    await user.click(acceptAllBtn);

    await waitFor(() => {
      expect(commitChangeset).toHaveBeenCalledWith({
        changesetId: "cs-1",
        itemActions: [
          { itemId: "item-1", action: "accept", editedData: null },
          { itemId: "item-2", action: "accept", editedData: null },
        ],
      });
    });
  });

  it("shows the password modal on VAULT_LOCKED rejection, handles failures, and retries on success", async () => {
    const user = userEvent.setup();

    // First call rejects with VAULT_LOCKED, next resolves
    vi.mocked(commitChangeset)
      .mockRejectedValueOnce(new Error("Database operation failed: VAULT_LOCKED"))
      .mockResolvedValue(true);

    // verifyMasterPassword returns false on first attempt, true on second
    vi.mocked(verifyMasterPassword)
      .mockResolvedValueOnce({ data: false, error: null })
      .mockResolvedValueOnce({ data: true, error: null });

    render(<Harness initialChangesetId="cs-1" />);

    // Verify items loaded
    await waitFor(() => {
      expect(screen.getByText("New Component Tests")).toBeInTheDocument();
    });

    const acceptAllBtn = screen.getByRole("button", { name: "Accept All" });
    await user.click(acceptAllBtn);

    // Expect the passcode modal to show
    await waitFor(() => {
      expect(screen.getByText("Vault Locked")).toBeInTheDocument();
    });

    const passwordInput = screen.getByPlaceholderText("Enter your master password...");
    const unlockBtn = screen.getByRole("button", { name: "Unlock & Commit" });

    // Submit invalid password
    await user.type(passwordInput, "wrong-password");
    await user.click(unlockBtn);

    // Expect password error
    await waitFor(() => {
      expect(screen.getByText("Invalid password")).toBeInTheDocument();
    });
    expect(commitChangeset).toHaveBeenCalledTimes(1); // Not retried yet

    // Clear and type correct password
    await user.clear(passwordInput);
    await user.type(passwordInput, "correct-password");
    await user.click(unlockBtn);

    // Modal should close and commitChangeset is retried
    await waitFor(() => {
      expect(screen.queryByText("Vault Locked")).not.toBeInTheDocument();
    });

    expect(commitChangeset).toHaveBeenCalledTimes(2); // Retried
  });
});
