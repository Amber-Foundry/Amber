import { describe, it, expect } from "vitest";
import { shouldOmitSpatialConnector } from "./privacy";

describe("shouldOmitSpatialConnector", () => {
  it("omits connectors for redacted tiers when locked out", () => {
    expect(shouldOmitSpatialConnector("redacted", false)).toBe(true);
  });

  it("shows connectors for redacted tiers after unlock", () => {
    expect(shouldOmitSpatialConnector("redacted", true)).toBe(false);
  });

  it("shows connectors for non-redacted tiers regardless of unlock state", () => {
    expect(shouldOmitSpatialConnector("open", false)).toBe(false);
    expect(shouldOmitSpatialConnector("locked", false)).toBe(false);
    expect(shouldOmitSpatialConnector("local_only", false)).toBe(false);
  });
});
