import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, beforeEach } from "vitest";
import PlantUmlBlock from "./PlantUmlBlock";
import { setPlantUmlConsent } from "../utils/settings";

describe("PlantUmlBlock Component", () => {
  beforeEach(() => {
    setPlantUmlConsent("disabled");
    window.localStorage.clear();
  });

  it("renders privacy notice when consent is disabled", () => {
    render(<PlantUmlBlock code="@startuml\nA -> B\n@enduml" />);

    expect(screen.getByText("Privacy Notice")).toBeInTheDocument();
    expect(screen.getByText("Enable for this Session")).toBeInTheDocument();
    expect(screen.getByText("Always Allow")).toBeInTheDocument();
  });

  it("updates consent when clicking Enable for this Session", async () => {
    const user = userEvent.setup();
    render(<PlantUmlBlock code="@startuml\nA -> B\n@enduml" />);

    await user.click(screen.getByText("Enable for this Session"));

    // Privacy notice should disapear and transition to diagram view
    expect(screen.queryByText("Privacy Notice")).not.toBeInTheDocument();
    expect(screen.getByText("Encoding diagram...")).toBeInTheDocument();
  });

  it("updates consent state when mindvault:llm-settings-changed event fires", () => {
    render(<PlantUmlBlock code="@startuml\nA -> B\n@enduml" />);

    expect(screen.getByText("Privacy Notice")).toBeInTheDocument();

    // Simulate async settings hydration updating localStorage and emitting event
    act(() => {
      setPlantUmlConsent("always");
      window.dispatchEvent(new CustomEvent("mindvault:llm-settings-changed"));
    });

    // Privacy notice should disappear
    expect(screen.queryByText("Privacy Notice")).not.toBeInTheDocument();
  });

  it("updates consent state when mindvault:plantuml-consent-changed event fires", () => {
    render(<PlantUmlBlock code="@startuml\nA -> B\n@enduml" />);

    expect(screen.getByText("Privacy Notice")).toBeInTheDocument();

    act(() => {
      setPlantUmlConsent("always");
      window.dispatchEvent(new CustomEvent("mindvault:plantuml-consent-changed"));
    });

    expect(screen.queryByText("Privacy Notice")).not.toBeInTheDocument();
  });
});
