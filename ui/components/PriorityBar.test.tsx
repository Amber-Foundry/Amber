import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import PriorityBar from "./PriorityBar";

describe("PriorityBar Component", () => {
  it("renders width percentage and labels correctly based on finite score values", () => {
    const { container } = render(<PriorityBar score={0.65} />);

    // Check width of fill bar
    const fill = container.querySelector(".priority-bar-fill");
    expect(fill).toHaveStyle({ width: "65%" });

    // Check score text label
    expect(screen.getByText("0.65")).toBeInTheDocument();
  });

  it("falls back to default 1.0 (100% width) for null or non-finite values", () => {
    const { container, rerender } = render(<PriorityBar score={null} />);
    const fillNull = container.querySelector(".priority-bar-fill");
    expect(fillNull).toHaveStyle({ width: "100%" });
    expect(screen.getByText("1.00")).toBeInTheDocument();

    rerender(<PriorityBar score={Infinity} />);
    const fillInfinity = container.querySelector(".priority-bar-fill");
    expect(fillInfinity).toHaveStyle({ width: "100%" });
  });

  it("applies the correct styling classes based on the threshold rules", () => {
    const { container, rerender } = render(<PriorityBar score={0.2} />);
    expect(container.querySelector(".priority-bar-fill")).toHaveClass("priority-bar-low");

    rerender(<PriorityBar score={0.4} />);
    expect(container.querySelector(".priority-bar-fill")).toHaveClass("priority-bar-low");

    rerender(<PriorityBar score={0.5} />);
    expect(container.querySelector(".priority-bar-fill")).toHaveClass("priority-bar-mid");

    rerender(<PriorityBar score={0.8} />);
    expect(container.querySelector(".priority-bar-fill")).toHaveClass("priority-bar-mid");

    rerender(<PriorityBar score={0.95} />);
    expect(container.querySelector(".priority-bar-fill")).toHaveClass("priority-bar-high");
  });

  it("toggles compact mode and score label display correctly based on props", () => {
    const { container, rerender } = render(
      <PriorityBar score={0.7} compact={true} showLabel={false} />
    );

    // Check compact class
    expect(container.querySelector(".priority-bar")).toHaveClass("priority-bar-compact");

    // Label should be absent
    expect(screen.queryByText("0.70")).not.toBeInTheDocument();

    // Rerender with compact=false and showLabel=true
    rerender(<PriorityBar score={0.7} compact={false} showLabel={true} />);
    expect(container.querySelector(".priority-bar")).not.toHaveClass("priority-bar-compact");
    expect(screen.getByText("0.70")).toBeInTheDocument();
  });
});
