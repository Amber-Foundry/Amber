import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import ScopeIndicator from "./ScopeIndicator";
import { ContextAssemblerScope } from "../constants/contextBudget";
import { setMockInvoker } from "../ipcMockState";

// A stateful test harness to support parent-controlled state transitions
function Harness({
  selectedNodeIds = ["node-1", "node-2"],
  initialScope = "local" as ContextAssemblerScope,
  onScopeChange,
}: {
  selectedNodeIds?: string[];
  initialScope?: ContextAssemblerScope;
  onScopeChange?: (scope: ContextAssemblerScope) => void;
}) {
  const [scope, setScope] = useState<ContextAssemblerScope>(initialScope);

  const handleScopeChange = (newScope: ContextAssemblerScope) => {
    setScope(newScope);
    if (onScopeChange) {
      onScopeChange(newScope);
    }
  };

  return (
    <ScopeIndicator
      selectedNodeIds={selectedNodeIds}
      scope={scope}
      onScopeChange={handleScopeChange}
    />
  );
}

describe("ScopeIndicator Component", () => {
  it("renders selected node count immediately (synchronous smoke test)", async () => {
    setMockInvoker(async (command) => {
      if (command === "debug_assemble_context") return { ok: "" };
      if (command === "llm_count_tokens") return { ok: 0 };
      return { err: "Unknown mock command" };
    });

    render(<Harness selectedNodeIds={["node-1", "node-2", "node-3"]} />);
    expect(screen.getByText("Nodes in Context: 3")).toBeInTheDocument();

    // Wait for the async effects to settle to prevent act warnings
    await waitFor(() => {
      expect(screen.getByText("Estimated Tokens: 0 / 8000")).toBeInTheDocument();
      expect(screen.queryByText("Unable to estimate token usage.")).not.toBeInTheDocument();
    });
  });

  it("calculates and displays estimated tokens correctly on success path", async () => {
    // Setup mock invoker to return ok responses in { ok: value } format
    setMockInvoker(async (command, payload) => {
      if (command === "debug_assemble_context") {
        expect(payload).toEqual({ nodeIds: ["node-1", "node-2"], scope: "local" });
        return { ok: "mock assembler context text content" };
      }
      if (command === "llm_count_tokens") {
        expect(payload).toEqual({ text: "mock assembler context text content" });
        return { ok: 150 };
      }
      return { err: "Unknown mock command" };
    });

    render(<Harness selectedNodeIds={["node-1", "node-2"]} />);

    // Wait for the asynchronous token counting effects to complete
    await waitFor(() => {
      expect(screen.getByText("Estimated Tokens: 150 / 8000")).toBeInTheDocument();
    });
  });

  it("triggers scope change callback and updates active classes", async () => {
    const user = userEvent.setup();
    const onScopeChangeMock = vi.fn();

    setMockInvoker(async (command) => {
      if (command === "debug_assemble_context") return { ok: "mock context" };
      if (command === "llm_count_tokens") return { ok: 50 };
      return { err: "Unknown mock command" };
    });

    render(<Harness initialScope="local" onScopeChange={onScopeChangeMock} />);

    const localBtn = screen.getByRole("button", { name: "Local" });
    const cloudBtn = screen.getByRole("button", { name: "Cloud" });

    expect(localBtn).toHaveClass("active");
    expect(cloudBtn).not.toHaveClass("active");

    // Click cloud button
    await user.click(cloudBtn);

    expect(onScopeChangeMock).toHaveBeenCalledWith("cloud");

    // The harness updates the state, checking classes update correctly
    await waitFor(() => {
      expect(localBtn).not.toHaveClass("active");
      expect(cloudBtn).toHaveClass("active");
    });
  });

  it("adds the .danger class to the progress bar when token budget is exceeded", async () => {
    setMockInvoker(async (command) => {
      if (command === "debug_assemble_context") return { ok: "oversized context content" };
      if (command === "llm_count_tokens") return { ok: 9000 };
      return { err: "Unknown mock command" };
    });

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByText("Estimated Tokens: 9000 / 8000")).toBeInTheDocument();
    });

    const progressBar = screen.getByRole("progressbar");
    expect(progressBar).toHaveClass("danger");
    expect(progressBar).toHaveAttribute("value", "8000"); // clamped to max
  });

  it("displays the correct status message when the IPC fails (error path)", async () => {
    setMockInvoker(async (command) => {
      if (command === "debug_assemble_context") {
        return { err: "Vault unavailable or locked" };
      }
      return { err: "Unknown mock command" };
    });

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByText("Vault unavailable or locked")).toBeInTheDocument();
    });
    expect(screen.getByText("Estimated Tokens: 0 / 8000")).toBeInTheDocument();
  });
});
