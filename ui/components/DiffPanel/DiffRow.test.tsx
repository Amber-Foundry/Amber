import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import DiffRow from "./DiffRow";
import type { ChangesetItem } from "../../ipc";

// Mock listVaults from ui/services/vaults
vi.mock("../../services/vaults", () => ({
  listVaults: vi.fn(),
}));

import { listVaults } from "../../services/vaults";

const mockListVaults = vi.mocked(listVaults);

const createMockChangesetItem = (overrides: Partial<ChangesetItem>): ChangesetItem => ({
  id: "item-123",
  changesetId: "cs-123",
  itemType: "ADD",
  targetNodeId: null,
  proposedData: "{}",
  existingData: null,
  similarity: null,
  mergeWithId: null,
  doorId: null,
  status: "pending",
  reviewedAt: null,
  sortOrder: 1,
  crossVaultAnomaly: false,
  anomalyWarning: null,
  ...overrides,
});

describe("DiffRow Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListVaults.mockResolvedValue([
      {
        id: "vault-1",
        name: "Work Vault",
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
        id: "vault-2",
        name: "Personal Vault",
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

  it("renders ADD operation proposed fields", () => {
    const proposed = {
      title: "New Note",
      summary: "This is a summary",
      detail: "Details here",
      tags: ["work", "important"],
      vaultId: "vault-1",
    };
    const item = createMockChangesetItem({
      itemType: "ADD",
      proposedData: JSON.stringify(proposed),
    });

    render(<DiffRow item={item} onCommitItem={vi.fn()} />);

    expect(screen.getByText("ADD")).toBeInTheDocument();
    expect(screen.getByText("New Note")).toBeInTheDocument();
    expect(screen.getByText("This is a summary")).toBeInTheDocument();
    expect(screen.getByText("Details here")).toBeInTheDocument();
    expect(screen.getByText("work")).toBeInTheDocument();
    expect(screen.getByText("important")).toBeInTheDocument();
    expect(screen.getByText(/vault-1/)).toBeInTheDocument();
  });

  it("renders UPDATE operation with existing and proposed changes (including diffs)", () => {
    const existing = {
      title: "Old Title",
      summary: "Old Summary",
      detail: "Same detail",
      tags: ["tag-old"],
    };
    const proposed = {
      title: "New Title",
      summary: "New Summary",
      detail: "Same detail",
      tags: ["tag-new"],
    };
    const item = createMockChangesetItem({
      itemType: "UPDATE",
      existingData: JSON.stringify(existing),
      proposedData: JSON.stringify(proposed),
    });

    render(<DiffRow item={item} onCommitItem={vi.fn()} />);

    expect(screen.getByText("UPDATE")).toBeInTheDocument();
    // Existing values in Current State
    expect(screen.getByText("Old Title")).toBeInTheDocument();
    expect(screen.getByText("Old Summary")).toBeInTheDocument();

    // Diffs rendered (inserted and deleted segments are separate text nodes)
    expect(screen.getAllByText("Old")).toHaveLength(2);
    expect(screen.getAllByText("New")).toHaveLength(2);
    expect(screen.getAllByText("Title").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Summary").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Same detail")).toHaveLength(2);
  });

  it("renders DELETE operation with deletion context and reason evidence", () => {
    const existing = {
      title: "Obsolete Note",
      summary: "Old context details",
    };
    const proposed = {
      summary: "This node is redundant now because Kevin said so.",
    };
    const item = createMockChangesetItem({
      itemType: "DELETE",
      existingData: JSON.stringify(existing),
      proposedData: JSON.stringify(proposed),
    });

    render(<DiffRow item={item} onCommitItem={vi.fn()} />);

    expect(screen.getByText("DELETE")).toBeInTheDocument();
    expect(screen.getByText("Obsolete Note")).toBeInTheDocument();
    expect(
      screen.getByText("This node is redundant now because Kevin said so.")
    ).toBeInTheDocument();
  });

  it("renders REPOINT_DOOR connection mapping details", () => {
    const item = createMockChangesetItem({
      itemType: "REPOINT_DOOR",
      doorId: "door-777",
      targetNodeId: "node-888",
    });

    render(<DiffRow item={item} onCommitItem={vi.fn()} />);

    expect(screen.getByText("ORPHAN")).toBeInTheDocument();
    expect(screen.getByText(/#door-777/)).toBeInTheDocument();
    expect(screen.getByText(/#node-888/)).toBeInTheDocument();
  });

  it("renders crossVaultAnomaly warning banner when true", () => {
    const item = createMockChangesetItem({
      crossVaultAnomaly: true,
      anomalyWarning: "Custom warning: writing to high-sensitivity work vault!",
    });

    render(<DiffRow item={item} onCommitItem={vi.fn()} />);

    expect(screen.getByText(/Security Warning: Mismatched Vault Sensitivity!/)).toBeInTheDocument();
    expect(
      screen.getByText("Custom warning: writing to high-sensitivity work vault!")
    ).toBeInTheDocument();
  });

  it("triggers onCommitItem with accept action when Accept button is clicked", async () => {
    const user = userEvent.setup();
    const onCommit = vi.fn();
    const item = createMockChangesetItem({ id: "item-abc", status: "pending" });

    render(<DiffRow item={item} onCommitItem={onCommit} />);

    const acceptBtn = screen.getByLabelText("Accept");
    await user.click(acceptBtn);

    expect(onCommit).toHaveBeenCalledWith("item-abc", "accept", null);
  });

  it("triggers onCommitItem with dismiss action when Dismiss button is clicked", async () => {
    const user = userEvent.setup();
    const onCommit = vi.fn();
    const item = createMockChangesetItem({ id: "item-abc", status: "pending" });

    render(<DiffRow item={item} onCommitItem={onCommit} />);

    const dismissBtn = screen.getByLabelText("Dismiss");
    await user.click(dismissBtn);

    expect(onCommit).toHaveBeenCalledWith("item-abc", "dismiss", null);
  });

  describe("Editing Flow Modal", () => {
    it("opens edit modal, validates empty title, loads vaults, and saves edit payload", async () => {
      const user = userEvent.setup();
      const onCommit = vi.fn();
      const proposed = {
        title: "Editable Task",
        summary: "To be edited summary",
        detail: "Initial detail text",
        tags: ["initial-tag"],
        vaultId: "vault-1",
      };
      const item = createMockChangesetItem({
        id: "item-edit-1",
        itemType: "ADD",
        proposedData: JSON.stringify(proposed),
        status: "pending",
      });

      render(<DiffRow item={item} onCommitItem={onCommit} />);

      // Verify the edit modal is not open initially
      expect(screen.queryByText("Edit Proposed Data")).not.toBeInTheDocument();

      // Click the edit action button
      const editBtn = screen.getByLabelText("Edit");
      await user.click(editBtn);

      // Verify edit modal is open (via Portal in document.body)
      expect(screen.getByText("Edit Proposed Data")).toBeInTheDocument();
      expect(listVaults).toHaveBeenCalledTimes(1);

      // Form inputs should be prefilled
      const titleInput = screen.getByPlaceholderText("Node Title") as HTMLInputElement;
      const summaryTextarea = screen.getByPlaceholderText(
        "Brief summary..."
      ) as HTMLTextAreaElement;
      const detailTextarea = screen.getByPlaceholderText(
        "Detailed description..."
      ) as HTMLTextAreaElement;
      const tagsInput = screen.getByPlaceholderText("tag1, tag2, tag3") as HTMLInputElement;
      const vaultSelect = screen.getByRole("combobox") as HTMLSelectElement;

      expect(titleInput.value).toBe("Editable Task");
      expect(summaryTextarea.value).toBe("To be edited summary");
      expect(detailTextarea.value).toBe("Initial detail text");
      expect(tagsInput.value).toBe("initial-tag");

      // Verify vault list options are loaded
      await waitFor(() => {
        expect(screen.getByText("Work Vault")).toBeInTheDocument();
        expect(screen.getByText("Personal Vault")).toBeInTheDocument();
      });
      expect(vaultSelect.value).toBe("vault-1");

      // Test empty title validation
      await user.clear(titleInput);
      const saveBtn = screen.getByText("Save & Accept");
      await user.click(saveBtn);

      // Validation error message should show up
      expect(screen.getByText("Title is required and cannot be empty.")).toBeInTheDocument();
      expect(onCommit).not.toHaveBeenCalled();

      // Enter new values and save
      await user.type(titleInput, "Updated Title");
      await user.clear(summaryTextarea);
      await user.type(summaryTextarea, "Updated Summary");
      await user.clear(detailTextarea);
      await user.type(detailTextarea, "Updated Details");
      await user.clear(tagsInput);
      await user.type(tagsInput, "tag-new-1, tag-new-2");
      await user.selectOptions(vaultSelect, "vault-2");

      await user.click(saveBtn);

      // Modal should be closed and callback triggered with edit action and updatedData
      expect(screen.queryByText("Edit Proposed Data")).not.toBeInTheDocument();
      expect(onCommit).toHaveBeenCalledWith("item-edit-1", "edit", {
        title: "Updated Title",
        summary: "Updated Summary",
        detail: "Updated Details",
        tags: ["tag-new-1", "tag-new-2"],
        vaultId: "vault-2",
      });
    });

    it("cancels out of the edit modal without triggering callbacks", async () => {
      const user = userEvent.setup();
      const onCommit = vi.fn();
      const item = createMockChangesetItem({
        id: "item-edit-2",
        itemType: "ADD",
        proposedData: JSON.stringify({ title: "Keep Me" }),
        status: "pending",
      });

      render(<DiffRow item={item} onCommitItem={onCommit} />);

      const editBtn = screen.getByLabelText("Edit");
      await user.click(editBtn);

      expect(screen.getByText("Edit Proposed Data")).toBeInTheDocument();

      const cancelBtn = screen.getByText("Cancel");
      await user.click(cancelBtn);

      expect(screen.queryByText("Edit Proposed Data")).not.toBeInTheDocument();
      expect(onCommit).not.toHaveBeenCalled();
    });
  });
});
