import { describe, it, expect } from "vitest";
import {
  lookupModel,
  getContextBudgetCeiling,
  UNKNOWN_MODEL_FALLBACK,
  ABSOLUTE_HARD_CAP,
  AUTO_WINDOW_FRACTION,
} from "./modelRegistry";

describe("modelRegistry", () => {
  describe("lookupModel", () => {
    it("resolves exact known flagship models", () => {
      expect(lookupModel("gpt-4o").contextWindow).toBe(128000);
      expect(lookupModel("claude-3-5-sonnet").contextWindow).toBe(200000);
      expect(lookupModel("gemini-1.5-pro").contextWindow).toBe(1000000);
    });

    it("resolves exact base names when provider prefix is present", () => {
      expect(lookupModel("openai/gpt-4o").contextWindow).toBe(128000);
      expect(lookupModel("anthropic/claude-3-5-sonnet").contextWindow).toBe(200000);
    });

    it("uses smart fuzzy fallback matching based on provider name", () => {
      expect(lookupModel("my-custom-gemini-model", "google").contextWindow).toBe(1000000);
      expect(lookupModel("custom-claude-deployment", "anthropic").contextWindow).toBe(200000);
      expect(lookupModel("gpt-4-custom", "openai").contextWindow).toBe(128000);
      expect(lookupModel("custom-grok-deployment", "xai").contextWindow).toBe(128000);
    });

    it("uses smart fuzzy fallback matching based on model ID keywords", () => {
      expect(lookupModel("gemini-3.1-custom").contextWindow).toBe(1000000);
      expect(lookupModel("claude-4-custom").contextWindow).toBe(200000);
      expect(lookupModel("gpt-4-custom").contextWindow).toBe(128000);
      expect(lookupModel("grok-3-custom").contextWindow).toBe(128000);
    });

    it("returns UNKNOWN_MODEL_FALLBACK for completely unrecognized model names", () => {
      expect(lookupModel("random-model-id").contextWindow).toBe(
        UNKNOWN_MODEL_FALLBACK.contextWindow
      );
    });
  });

  describe("getContextBudgetCeiling", () => {
    it("caps large context models at ABSOLUTE_HARD_CAP", () => {
      // gemini-1.5-pro has 1,000,000 window. 75% of it is 750,000. Capped at ABSOLUTE_HARD_CAP (200,000).
      expect(getContextBudgetCeiling("gemini-1.5-pro")).toBe(ABSOLUTE_HARD_CAP);
    });

    it("scales small context models correctly with AUTO_WINDOW_FRACTION", () => {
      // gpt-4 has 8,192 window. 75% of it is 6,144.
      const expected = Math.floor(8192 * AUTO_WINDOW_FRACTION);
      expect(getContextBudgetCeiling("gpt-4")).toBe(expected);
    });

    it("applies fallback scale for unrecognized model ID", () => {
      const expected = Math.floor(UNKNOWN_MODEL_FALLBACK.contextWindow * AUTO_WINDOW_FRACTION);
      expect(getContextBudgetCeiling("random-model-id")).toBe(expected);
    });
  });
});
