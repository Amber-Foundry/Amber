import { describe, it, expect, vi } from "vitest";
import { parseSQLiteDate, parseJSON } from "../utils/parse";
import {
  parsePriorityJson,
  getPriorityScore,
  getPriorityProfile,
  isFrozen,
  getAccessCount,
} from "../utils/priority";
import type { Node } from "../ipc";

// Helper to construct a compliant mock Node
const createMockNode = (priority: string): Node => ({
  id: "node-123",
  vaultId: "vault-root",
  subVaultId: null,
  nodeType: "concept",
  title: "Test Node",
  summary: "A short summary",
  detail: null,
  source: null,
  sourceType: null,
  privacyTier: "open",
  priority,
  version: 1,
  isArchived: false,
  createdAt: "2026-06-23T00:00:00Z",
  updatedAt: "2026-06-23T00:00:00Z",
  lastAccessed: "2026-06-23T00:00:00Z",
  deletedAt: null,
  meta: "{}",
});

describe("DiffPanel Pure Helpers", () => {
  describe("parseSQLiteDate", () => {
    it("returns a Date instance for null/undefined/empty string defaulting to now", () => {
      vi.useFakeTimers();
      try {
        const mockNow = new Date("2026-06-23T12:00:00Z");
        vi.setSystemTime(mockNow);

        const dateNull = parseSQLiteDate(null);
        const dateUndefined = parseSQLiteDate(undefined);
        const dateEmpty = parseSQLiteDate("");

        expect(dateNull).toBeInstanceOf(Date);
        expect(dateUndefined).toBeInstanceOf(Date);
        expect(dateEmpty).toBeInstanceOf(Date);

        expect(dateNull.getTime()).toBe(mockNow.getTime());
        expect(dateUndefined.getTime()).toBe(mockNow.getTime());
        expect(dateEmpty.getTime()).toBe(mockNow.getTime());
      } finally {
        vi.useRealTimers();
      }
    });

    it("parses standardized ISO 8601 strings", () => {
      const dateStr = "2026-06-23T12:00:00.000Z";
      const date = parseSQLiteDate(dateStr);
      expect(date.toISOString()).toBe(dateStr);
    });

    it("normalizes and parses SQLite format space strings without T", () => {
      const sqliteDateStr = "2026-06-23 12:00:00";
      const date = parseSQLiteDate(sqliteDateStr);
      expect(date.toISOString()).toBe("2026-06-23T12:00:00.000Z");
    });
  });

  describe("parseJSON", () => {
    it("returns empty object for falsy arguments", () => {
      expect(parseJSON(null)).toEqual({});
      expect(parseJSON(undefined)).toEqual({});
      expect(parseJSON("")).toEqual({});
    });

    it("parses valid JSON strings correctly", () => {
      const validJSON = '{"key":"value","number":123}';
      expect(parseJSON(validJSON)).toEqual({ key: "value", number: 123 });
    });

    it("catches syntax errors and returns an empty object fallback", () => {
      const invalidJSON = '{"key": "value"'; // incomplete
      expect(parseJSON(invalidJSON)).toEqual({});
    });

    it("returns empty object fallback for primitives and arrays", () => {
      expect(parseJSON("null")).toEqual({});
      expect(parseJSON("123")).toEqual({});
      expect(parseJSON("true")).toEqual({});
      expect(parseJSON('["item1", "item2"]')).toEqual({});
    });
  });
});

describe("PriorityDashboard Pure Helpers", () => {
  describe("parsePriorityJson", () => {
    it("safely handles invalid JSON and returns empty object", () => {
      expect(parsePriorityJson("invalid")).toEqual({});
    });

    it("safely handles non-object JSON values and returns empty object", () => {
      expect(parsePriorityJson("123")).toEqual({});
      expect(parsePriorityJson("null")).toEqual({});
      expect(parsePriorityJson('"string"')).toEqual({});
      expect(parsePriorityJson("[1, 2, 3]")).toEqual({});
    });

    it("parses valid priority json objects", () => {
      expect(parsePriorityJson('{"score":0.75,"profile":"high"}')).toEqual({
        score: 0.75,
        profile: "high",
      });
    });
  });

  describe("getPriorityScore", () => {
    it("returns score value when valid", () => {
      const node = createMockNode('{"score":0.45}');
      expect(getPriorityScore(node)).toBe(0.45);
    });

    it("defaults to 1.0 if score is not a number", () => {
      const nodeMissing = createMockNode('{"profile":"low"}');
      const nodeInvalid = createMockNode('{"score":"high"}');
      expect(getPriorityScore(nodeMissing)).toBe(1.0);
      expect(getPriorityScore(nodeInvalid)).toBe(1.0);
    });

    it("defaults to 1.0 if priority string is empty/invalid", () => {
      const node = createMockNode("");
      expect(getPriorityScore(node)).toBe(1.0);
    });
  });

  describe("getPriorityProfile", () => {
    it("returns profile name when valid", () => {
      const node = createMockNode('{"profile":"aggressive"}');
      expect(getPriorityProfile(node)).toBe("aggressive");
    });

    it("defaults to standard if profile is not present or not a string", () => {
      const nodeMissing = createMockNode('{"score":0.5}');
      const nodeInvalid = createMockNode('{"profile":true}');
      expect(getPriorityProfile(nodeMissing)).toBe("standard");
      expect(getPriorityProfile(nodeInvalid)).toBe("standard");
    });
  });

  describe("isFrozen", () => {
    it("returns true if frozen property is explicitly true", () => {
      const nodeTrue = createMockNode('{"frozen":true}');
      const nodeFalse = createMockNode('{"frozen":false}');
      const nodeMissing = createMockNode('{"score":0.2}');

      expect(isFrozen(nodeTrue)).toBe(true);
      expect(isFrozen(nodeFalse)).toBe(false);
      expect(isFrozen(nodeMissing)).toBe(false);
    });
  });

  describe("getAccessCount", () => {
    it("returns specific access count matching the key", () => {
      const node = createMockNode('{"access_count_30active":15,"access_count_90active":42}');
      expect(getAccessCount(node, "access_count_30active")).toBe(15);
      expect(getAccessCount(node, "access_count_90active")).toBe(42);
    });

    it("defaults to 0 if key is missing or not a finite number", () => {
      const node = createMockNode('{"access_count_30active":"many"}');
      expect(getAccessCount(node, "access_count_30active")).toBe(0);
      expect(getAccessCount(node, "access_count_90active")).toBe(0);
    });
  });
});
