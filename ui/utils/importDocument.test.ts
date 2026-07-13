import { describe, expect, it } from "vitest";
import type { Node } from "../types/generated/Node";
import {
  formatImportSectionTitle,
  getImportChunkIndex,
  getImportDocumentId,
  isImportChunkNode,
  isImportDocumentNode,
  listImportDocumentSections,
} from "./importDocument";

function makeNode(overrides: Partial<Node> & Pick<Node, "id" | "title" | "meta">): Node {
  return {
    vaultId: "vault_learning",
    subVaultId: null,
    nodeType: "fact",
    summary: "summary",
    detail: "detail",
    source: "doc.pdf",
    sourceType: "pdf_import",
    privacyTier: null,
    priority: "{}",
    version: 1,
    isArchived: false,
    createdAt: "",
    updatedAt: "",
    lastAccessed: "",
    deletedAt: null,
    ...overrides,
  };
}

describe("importDocument helpers", () => {
  it("detects document and chunk roles", () => {
    const doc = makeNode({
      id: "doc1",
      title: "doc",
      meta: JSON.stringify({ import_role: "document", chunk_total: 2 }),
    });
    const chunk = makeNode({
      id: "c1",
      title: "doc · a (1/2)",
      meta: JSON.stringify({
        import_role: "chunk",
        document_id: "doc1",
        chunk_index: 0,
      }),
    });
    expect(isImportDocumentNode(doc)).toBe(true);
    expect(isImportChunkNode(doc)).toBe(false);
    expect(isImportChunkNode(chunk)).toBe(true);
    expect(getImportDocumentId(chunk)).toBe("doc1");
    expect(getImportChunkIndex(chunk)).toBe(0);
  });

  it("falls back to pdf_import + chunk_index for chunks", () => {
    const legacy = makeNode({
      id: "c2",
      title: "legacy",
      meta: JSON.stringify({ chunk_index: 1 }),
    });
    expect(isImportChunkNode(legacy)).toBe(true);
  });

  it("orders document sections by chunk_index", () => {
    const doc = makeNode({
      id: "doc1",
      title: "doc",
      meta: JSON.stringify({ import_role: "document" }),
    });
    const c1 = makeNode({
      id: "c1",
      title: "1",
      meta: JSON.stringify({
        import_role: "chunk",
        document_id: "doc1",
        chunk_index: 1,
      }),
    });
    const c0 = makeNode({
      id: "c0",
      title: "0",
      meta: JSON.stringify({
        import_role: "chunk",
        document_id: "doc1",
        chunk_index: 0,
      }),
    });
    expect(listImportDocumentSections("doc1", [doc, c1, c0]).map((n) => n.id)).toEqual([
      "c0",
      "c1",
    ]);
  });

  it("formats short section titles without the PDF stem", () => {
    expect(formatImportSectionTitle("CSE824 HW1 · # CSE 824: Homework 1 (1/5)", 1, 5)).toBe(
      "# CSE 824: Homework 1 (1/5)"
    );
    expect(
      formatImportSectionTitle("AllAmendments_US · Amendment V Passed by Congress (3/18)", 3, 18)
    ).toBe("Amendment V Passed by Congress (3/18)");
  });
});
