import type { Node } from "../types/generated/Node";

type ImportMeta = {
  import_role?: string;
  importRole?: string;
  chunk_index?: number;
  chunkIndex?: number;
  document_id?: string;
  documentId?: string;
  chunk_total?: number;
  chunkTotal?: number;
  avg_ocr_confidence?: number;
  avgOcrConfidence?: number;
  ocr_confidence?: number;
  ocrConfidence?: number;
  tables_unstructured?: boolean;
  tablesUnstructured?: boolean;
  extraction_path?: string;
  extractionPath?: string;
};

export function parseNodeMeta(node: Pick<Node, "meta">): ImportMeta {
  if (!node.meta || node.meta === "{}") {
    return {};
  }
  try {
    return JSON.parse(node.meta) as ImportMeta;
  } catch {
    return {};
  }
}

export function isImportDocumentNode(node: Pick<Node, "meta" | "sourceType">): boolean {
  if (node.sourceType !== "pdf_import") {
    return false;
  }
  const meta = parseNodeMeta(node);
  return (meta.import_role ?? meta.importRole) === "document";
}

export function isImportChunkNode(node: Pick<Node, "meta" | "sourceType">): boolean {
  if (node.sourceType !== "pdf_import") {
    return false;
  }
  const meta = parseNodeMeta(node);
  const role = meta.import_role ?? meta.importRole;
  if (role === "document") {
    return false;
  }
  if (role === "chunk") {
    return true;
  }
  const chunkIndex = meta.chunk_index ?? meta.chunkIndex;
  return chunkIndex != null;
}

export function isPdfImportNode(node: Pick<Node, "meta" | "sourceType">): boolean {
  if (node.sourceType === "pdf_import") {
    return true;
  }
  return isImportDocumentNode(node) || isImportChunkNode(node);
}

export function getImportDocumentId(node: Pick<Node, "meta" | "sourceType">): string | null {
  if (node.sourceType !== "pdf_import") {
    return null;
  }
  const meta = parseNodeMeta(node);
  return meta.document_id ?? meta.documentId ?? null;
}

export function getImportChunkIndex(node: Pick<Node, "meta" | "sourceType">): number | null {
  if (node.sourceType !== "pdf_import") {
    return null;
  }
  const meta = parseNodeMeta(node);
  const value = meta.chunk_index ?? meta.chunkIndex;
  return typeof value === "number" ? value : null;
}

/** Chunks belonging to a document parent, ordered by chunk_index. */
export function listImportDocumentSections(parentId: string, allNodes: Node[]): Node[] {
  return allNodes
    .filter((node) => getImportDocumentId(node) === parentId && isImportChunkNode(node))
    .sort((a, b) => (getImportChunkIndex(a) ?? 0) - (getImportChunkIndex(b) ?? 0));
}

/**
 * Compact section label for the document spine: heading + (n/N), without repeating the PDF stem.
 */
export function formatImportSectionTitle(title: string, ordinal: number, total: number): string {
  let heading = title.trim();
  const indexSuffix = heading.match(/\s*\(\d+\s*\/\s*\d+\)\s*$/);
  if (indexSuffix) {
    heading = heading.slice(0, -indexSuffix[0].length).trim();
  }
  const stemSplit = heading.match(/^(.+?)\s*[·•\-–—]\s+(.+)$/);
  if (stemSplit) {
    heading = stemSplit[2].trim();
  }
  if (!heading) {
    heading = `Section ${ordinal}`;
  }
  return `${heading} (${ordinal}/${total})`;
}

/** Short filename for "Imported: …" (strip path + extension). */
export function getImportSourceLabel(node: Pick<Node, "source" | "title">): string {
  const raw = (node.source ?? "").trim() || node.title.trim();
  const base = raw.replace(/\\/g, "/").split("/").pop() ?? raw;
  const withoutExt = base.replace(/\.(pdf|PDF)$/, "").trim();
  return withoutExt || base || "Document";
}

function readOcrConfidence(meta: ImportMeta): number | null {
  const value =
    meta.avg_ocr_confidence ?? meta.avgOcrConfidence ?? meta.ocr_confidence ?? meta.ocrConfidence;
  if (typeof value !== "number" || Number.isNaN(value) || value <= 0) {
    return null;
  }
  return value;
}

function readTablesFlag(meta: ImportMeta): boolean {
  return Boolean(meta.tables_unstructured ?? meta.tablesUnstructured);
}

/**
 * OCR confidence for badges: parent rollup meta, else mean of section chunk confidences.
 * Returns null when no OCR signal is available (e.g. pure digital).
 */
export function getImportOcrConfidence(
  node: Pick<Node, "id" | "meta" | "sourceType">,
  allNodes: Node[] = []
): number | null {
  if (node.sourceType !== "pdf_import") {
    return null;
  }
  const own = readOcrConfidence(parseNodeMeta(node));
  if (own != null) {
    return own;
  }
  if (!isImportDocumentNode(node)) {
    return null;
  }
  const sections = listImportDocumentSections(node.id, allNodes);
  const values = sections
    .map((section) => readOcrConfidence(parseNodeMeta(section)))
    .filter((v): v is number => v != null);
  if (values.length === 0) {
    return null;
  }
  return values.reduce((sum, v) => sum + v, 0) / values.length;
}

/** True when parent meta or any section chunk flags unstructured tables. */
export function importHasUnstructuredTables(
  node: Pick<Node, "id" | "meta" | "sourceType">,
  allNodes: Node[] = []
): boolean {
  if (node.sourceType !== "pdf_import") {
    return false;
  }
  if (readTablesFlag(parseNodeMeta(node))) {
    return true;
  }
  if (!isImportDocumentNode(node)) {
    return false;
  }
  return listImportDocumentSections(node.id, allNodes).some((section) =>
    readTablesFlag(parseNodeMeta(section))
  );
}
