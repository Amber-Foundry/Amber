import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import ErrorBoundary from "./ErrorBoundary";

// A dummy component that throws an error when rendered
function CrashingComponent({ shouldCrash }: { shouldCrash: boolean }) {
  if (shouldCrash) {
    throw new Error("Simulated component crash!");
  }
  return <div>Component is fine.</div>;
}

describe("ErrorBoundary Component", () => {
  let originalConsoleError: typeof console.error;

  beforeEach(() => {
    originalConsoleError = console.error;
    // Suppress console.error inside tests to prevent cluttering test output
    console.error = vi.fn();
  });

  afterEach(() => {
    console.error = originalConsoleError;
  });

  it("renders children normally under the happy path", () => {
    render(
      <ErrorBoundary>
        <CrashingComponent shouldCrash={false} />
      </ErrorBoundary>
    );

    expect(screen.getByText("Component is fine.")).toBeInTheDocument();
    expect(screen.queryByText("Simulated component crash!")).not.toBeInTheDocument();
  });

  it("catches rendering crashes, logs to console, and renders the error fallback UI", () => {
    render(
      <ErrorBoundary>
        <CrashingComponent shouldCrash={true} />
      </ErrorBoundary>
    );

    // Should display the error message in the fallback view
    expect(screen.getByText("Simulated component crash!")).toBeInTheDocument();
    expect(screen.queryByText("Component is fine.")).not.toBeInTheDocument();

    // Verify console.error was triggered
    expect(console.error).toHaveBeenCalled();
  });
});
