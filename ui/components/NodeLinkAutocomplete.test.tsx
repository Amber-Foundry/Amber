import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import NodeLinkAutocomplete from "./NodeLinkAutocomplete";
import type { Node } from "../types/generated/Node";

vi.mock("../services/nodes", () => ({
  searchNodes: vi.fn(),
}));

import { searchNodes } from "../services/nodes";

const mockNodes: Node[] = [
  {
    id: "node-1",
    vaultId: "v1",
    subVaultId: null,
    nodeType: "standard",
    title: "Rust Programming",
    summary: "Intro to systems coding",
    detail: "Rust rules",
    source: null,
    sourceType: null,
    privacyTier: "open",
    priority: "normal",
    version: 1,
    isArchived: false,
    createdAt: "2026",
    updatedAt: "2026",
    lastAccessed: "2026",
    deletedAt: null,
    meta: "{}",
  },
  {
    id: "node-2",
    vaultId: "v1",
    subVaultId: null,
    nodeType: "standard",
    title: "React Testing Library",
    summary: "Component rendering validation",
    detail: "RTL rules",
    source: null,
    sourceType: null,
    privacyTier: "open",
    priority: "normal",
    version: 1,
    isArchived: false,
    createdAt: "2026",
    updatedAt: "2026",
    lastAccessed: "2026",
    deletedAt: null,
    meta: "{}",
  },
  {
    id: "node-3",
    vaultId: "v1",
    subVaultId: null,
    nodeType: "standard",
    title: "Vite Bundler",
    summary: "Fast build tooling",
    detail: "Vite is speedy",
    source: null,
    sourceType: null,
    privacyTier: "open",
    priority: "normal",
    version: 1,
    isArchived: false,
    createdAt: "2026",
    updatedAt: "2026",
    lastAccessed: "2026",
    deletedAt: null,
    meta: "{}",
  },
];

describe("NodeLinkAutocomplete Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders suggestion items from preloaded nodes and filters them by query on title or summary", () => {
    const onSelectMock = vi.fn();
    const onCloseMock = vi.fn();

    // No query - shows all preloaded nodes
    const { rerender } = render(
      <NodeLinkAutocomplete
        query=""
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
        nodes={mockNodes}
      />
    );

    expect(screen.getByText("Rust Programming")).toBeInTheDocument();
    expect(screen.getByText("React Testing Library")).toBeInTheDocument();
    expect(screen.getByText("Vite Bundler")).toBeInTheDocument();

    // Query "rust" - filters by title
    rerender(
      <NodeLinkAutocomplete
        query="rust"
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
        nodes={mockNodes}
      />
    );

    expect(screen.getByText("Rust Programming")).toBeInTheDocument();
    expect(screen.queryByText("React Testing Library")).not.toBeInTheDocument();

    // Query "validation" - filters by summary
    rerender(
      <NodeLinkAutocomplete
        query="validation"
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
        nodes={mockNodes}
      />
    );

    expect(screen.getByText("React Testing Library")).toBeInTheDocument();
    expect(screen.queryByText("Rust Programming")).not.toBeInTheDocument();
  });

  it("renders null (renders nothing) when nodes list is empty", () => {
    const { container } = render(
      <NodeLinkAutocomplete
        query=""
        position={{ top: 100, left: 200 }}
        onSelect={vi.fn()}
        onClose={vi.fn()}
        nodes={[]}
      />
    );
    expect(container.firstChild).toBeNull();
  });

  it("handles mouse selection and triggers onSelect callback", async () => {
    const user = userEvent.setup();
    const onSelectMock = vi.fn();
    const onCloseMock = vi.fn();

    render(
      <NodeLinkAutocomplete
        query=""
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
        nodes={mockNodes}
      />
    );

    const option = screen.getByText("Vite Bundler");
    await user.click(option);

    expect(onSelectMock).toHaveBeenCalledWith(mockNodes[2]);
  });

  it("handles keyboard events: ArrowDown, ArrowUp, Enter, Tab, and Escape", async () => {
    const user = userEvent.setup();
    const onSelectMock = vi.fn();
    const onCloseMock = vi.fn();

    const { unmount } = render(
      <NodeLinkAutocomplete
        query=""
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
        nodes={mockNodes}
      />
    );

    // Initial selected index is 0 ("Rust Programming")
    // Press ArrowDown to highlight "React Testing Library" (index 1)
    await user.keyboard("{ArrowDown}");

    // Press ArrowUp to highlight back to "Rust Programming" (index 0)
    await user.keyboard("{ArrowUp}");

    // Press Tab to confirm selection on index 0
    await user.keyboard("{Tab}");
    expect(onSelectMock).toHaveBeenCalledWith(mockNodes[0]);
    onSelectMock.mockClear();

    unmount();

    // Rerender to test Enter key
    render(
      <NodeLinkAutocomplete
        query=""
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
        nodes={mockNodes}
      />
    );

    // Press ArrowDown twice to select "Vite Bundler" (index 2)
    await user.keyboard("{ArrowDown}");
    await user.keyboard("{ArrowDown}");
    await user.keyboard("{Enter}");
    expect(onSelectMock).toHaveBeenCalledWith(mockNodes[2]);

    // Press Escape to close
    await user.keyboard("{Escape}");
    expect(onCloseMock).toHaveBeenCalled();
  });

  it("dynamically imports service and queries searchNodes when no preloaded nodes are provided", async () => {
    vi.mocked(searchNodes).mockResolvedValue([mockNodes[0]]);
    const onSelectMock = vi.fn();
    const onCloseMock = vi.fn();

    render(
      <NodeLinkAutocomplete
        query="dynamic-query"
        position={{ top: 100, left: 200 }}
        onSelect={onSelectMock}
        onClose={onCloseMock}
      />
    );

    await waitFor(() => {
      expect(searchNodes).toHaveBeenCalledWith("dynamic-query");
      expect(screen.getByText("Rust Programming")).toBeInTheDocument();
    });
  });
});
