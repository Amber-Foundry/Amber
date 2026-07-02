import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { PrivacyBadge } from "./PrivacyBadge";

describe("PrivacyBadge Component", () => {
  it("renders correct classes for standard tiers", () => {
    const { container, rerender } = render(<PrivacyBadge tier="open" />);
    expect(container.querySelector(".privacy-badge")).toHaveClass("open");

    rerender(<PrivacyBadge tier="local_only" />);
    expect(container.querySelector(".privacy-badge")).toHaveClass("local_only");

    rerender(<PrivacyBadge tier="locked" />);
    expect(container.querySelector(".privacy-badge")).toHaveClass("locked");

    rerender(<PrivacyBadge tier="redacted" />);
    expect(container.querySelector(".privacy-badge")).toHaveClass("redacted");
  });

  it("falls back to open tier for unknown values", () => {
    const { container } = render(<PrivacyBadge tier="super_secret_unsupported_tier" />);
    expect(container.querySelector(".privacy-badge")).toHaveClass("open");
  });

  it("applies the correct CSS class names, including custom className prop", () => {
    const { container } = render(<PrivacyBadge tier="local_only" className="custom-suffix" />);
    const badge = container.querySelector(".privacy-badge");
    expect(badge).toHaveClass("privacy-badge");
    expect(badge).toHaveClass("local_only");
    expect(badge).toHaveClass("custom-suffix");
  });
});
