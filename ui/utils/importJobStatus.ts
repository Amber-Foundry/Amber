import type { ImportJobStatus } from "../types/generated/ImportJobStatus";

export const ROOT_GRAPH_VAULT_ID = "vault_root_graph";

const TERMINAL_SUCCESS = new Set(["staged", "committed"]);
const TERMINAL_FAILURE = new Set(["failed"]);
const TERMINAL_RESOLVED = new Set(["staged", "committed", "dismissed"]);
const ACTIVE = new Set(["pending", "extracting"]);

export function isImportJobComplete(job: Pick<ImportJobStatus, "status">): boolean {
  return TERMINAL_SUCCESS.has(job.status);
}

/** Staged / committed / dismissed — finished enough to show Job Log actions. */
export function isImportJobResolved(job: Pick<ImportJobStatus, "status">): boolean {
  return TERMINAL_RESOLVED.has(job.status);
}

export function isImportJobFailed(job: Pick<ImportJobStatus, "status">): boolean {
  return TERMINAL_FAILURE.has(job.status);
}

export function isImportJobActive(job: Pick<ImportJobStatus, "status">): boolean {
  return ACTIVE.has(job.status);
}

export function formatImportJobStatusLabel(status: string): string {
  switch (status) {
    case "pending":
      return "Queued";
    case "extracting":
      return "Extracting";
    case "staged":
      return "Review";
    case "committed":
      return "Committed";
    case "dismissed":
      return "Dismissed";
    case "failed":
      return "Failed";
    default:
      return status;
  }
}

/** Honest progress: page loop caps at 85%; LLM/build phase ~90%; 100% only when staged/committed/dismissed. */
export function importJobProgressPercent(job: ImportJobStatus): number {
  if (job.status === "dismissed" || isImportJobComplete(job)) return 100;
  if (isImportJobFailed(job) || job.totalPages === 0) return 0;
  if (job.currentPage < job.totalPages) {
    return Math.min(85, Math.round((job.currentPage / job.totalPages) * 85));
  }
  // Pages done, still extracting (LLM / changeset).
  return 90;
}

export function formatImportJobPhaseSummary(job: ImportJobStatus): string {
  if (job.totalPages === 0) {
    return "Waiting for page analysis…";
  }
  if (isImportJobActive(job) && job.currentPage < job.totalPages) {
    return `Reading pages ${job.currentPage}/${job.totalPages}…`;
  }
  if (isImportJobActive(job) && job.currentPage >= job.totalPages) {
    return "Building proposals…";
  }

  const parts: string[] = [];
  if (job.digitalPages > 0) parts.push(`${job.digitalPages} digital`);
  if (job.hybridPages > 0) {
    parts.push(`${job.hybridPages} hybrid (embedded images extracted via OCR)`);
  }
  if (job.ocrPages > 0) parts.push(`${job.ocrPages} scanned`);
  return `${job.totalPages} pages — ${parts.join(", ") || "no page breakdown yet"}`;
}

export function formatImportDestinationLabel(
  targetVaultId: string | null | undefined,
  vaultNameById: Map<string, string>
): string {
  if (!targetVaultId) {
    return "Destination unknown";
  }
  if (targetVaultId === ROOT_GRAPH_VAULT_ID) {
    return "Root Graph";
  }
  return vaultNameById.get(targetVaultId) ?? targetVaultId;
}
