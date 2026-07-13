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
  const meta = parseNodeMeta(node);
  return (meta.import_role ?? meta.importRole) === "document";
}

export function isImportChunkNode(node: Pick<Node, "meta" | "sourceType">): boolean {
  const meta = parseNodeMeta(node);
  const role = meta.import_role ?? meta.importRole;
  if (role === "document") {
    return false;
  }
  if (role === "chunk") {
    return true;
  }
  const chunkIndex = meta.chunk_index ?? meta.chunkIndex;
  return node.sourceType === "pdf_import" && chunkIndex != null;
}

export function getImportDocumentId(node: Pick<Node, "meta">): string | null {
  const meta = parseNodeMeta(node);
  return meta.document_id ?? meta.documentId ?? null;
}

export function getImportChunkIndex(node: Pick<Node, "meta">): number | null {
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
