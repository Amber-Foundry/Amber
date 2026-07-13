import { describe, expect, it } from "vitest";
import type { ImportJobStatus } from "../types/generated/ImportJobStatus";
import {
  formatImportDestinationLabel,
  formatImportJobPhaseSummary,
  formatImportJobStatusLabel,
  importJobProgressPercent,
  isImportJobActive,
  ROOT_GRAPH_VAULT_ID,
} from "./importJobStatus";

function job(overrides: Partial<ImportJobStatus> = {}): ImportJobStatus {
  return {
    id: "j1",
    status: "extracting",
    sourceName: "doc.pdf",
    targetVaultId: "vault_learning",
    changesetId: null,
    nodeCount: 0,
    totalPages: 10,
    currentPage: 3,
    digitalPages: 10,
    ocrPages: 0,
    hybridPages: 0,
    avgOcrConfidence: 1,
    tablesDetectedUnpreserved: 0,
    extractionPath: null,
    rasterizationDpi: 300,
    error: null,
    ...overrides,
  };
}

describe("importJobProgressPercent", () => {
  it("stays below 100 while classification counts equal total pages during extract", () => {
    const mid = job({ currentPage: 5, digitalPages: 10, ocrPages: 0, hybridPages: 0 });
    expect(importJobProgressPercent(mid)).toBeLessThan(100);
    expect(importJobProgressPercent(mid)).toBe(43); // round(5/10*85)
  });

  it("holds ~90% after pages finish while still extracting", () => {
    expect(importJobProgressPercent(job({ currentPage: 10 }))).toBe(90);
  });

  it("is 100% only when staged or committed", () => {
    expect(importJobProgressPercent(job({ status: "staged", currentPage: 10 }))).toBe(100);
    expect(importJobProgressPercent(job({ status: "committed", currentPage: 10 }))).toBe(100);
  });
});

describe("formatImportJobPhaseSummary", () => {
  it("shows reading / building phases while active", () => {
    expect(formatImportJobPhaseSummary(job({ currentPage: 2 }))).toBe("Reading pages 2/10…");
    expect(formatImportJobPhaseSummary(job({ currentPage: 10 }))).toBe("Building proposals…");
  });
});

describe("formatImportJobStatusLabel", () => {
  it("maps pipeline statuses to review-friendly labels", () => {
    expect(formatImportJobStatusLabel("pending")).toBe("Queued");
    expect(formatImportJobStatusLabel("staged")).toBe("Review");
    expect(formatImportJobStatusLabel("committed")).toBe("Committed");
    expect(formatImportJobStatusLabel("dismissed")).toBe("Dismissed");
  });
});

describe("importJobProgressPercent dismissed", () => {
  it("treats dismissed as complete progress", () => {
    expect(importJobProgressPercent(job({ status: "dismissed", currentPage: 10 }))).toBe(100);
  });
});

describe("formatImportDestinationLabel", () => {
  it("labels Root Graph and resolves vault names", () => {
    const map = new Map([["vault_learning", "Learning"]]);
    expect(formatImportDestinationLabel(ROOT_GRAPH_VAULT_ID, map)).toBe("Root Graph");
    expect(formatImportDestinationLabel("vault_learning", map)).toBe("Learning");
  });
});

describe("isImportJobActive", () => {
  it("treats pending and extracting as active", () => {
    expect(isImportJobActive(job({ status: "pending" }))).toBe(true);
    expect(isImportJobActive(job({ status: "extracting" }))).toBe(true);
    expect(isImportJobActive(job({ status: "staged" }))).toBe(false);
  });
});
