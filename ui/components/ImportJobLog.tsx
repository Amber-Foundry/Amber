import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelImportJob,
  getImportExtractionPreview,
  listImportJobs,
  startOcrModelDownload,
  type ImportExtractionPreview,
  type ImportJobStatus,
} from "../services/import";
import { listVaults } from "../services/vaults";
import { toAppError } from "../services/ipcResult";
import ImportJobLogStyles from "../style/components/ImportJobLog.module.css";
import type { ImportStartJobInput } from "../types/generated/ImportStartJobInput";
import {
  formatImportDestinationLabel,
  formatImportJobPhaseSummary,
  formatImportJobStatusLabel,
  importJobProgressPercent,
  isImportJobActive,
  isImportJobFailed,
  isImportJobResolved,
} from "../utils/importJobStatus";

const POLL_INTERVAL_MS = 2000;

function isModelNotFoundError(job: ImportJobStatus): boolean {
  return isImportJobFailed(job) && (job.error?.includes("OCR model not found") ?? false);
}

type ImportJobLogProps = {
  refreshKey?: number;
  onOpenChangeset?: (changesetId: string) => void;
  onActiveJobsChange?: (hasActive: boolean) => void;
  onRetry?: (input: ImportStartJobInput) => void;
};

export default function ImportJobLog({
  refreshKey = 0,
  onOpenChangeset,
  onActiveJobsChange,
  onRetry,
}: ImportJobLogProps) {
  const [jobs, setJobs] = useState<ImportJobStatus[]>([]);
  const [vaultNameById, setVaultNameById] = useState<Map<string, string>>(() => new Map());
  const [jobParams, setJobParams] = useState<Record<string, ImportStartJobInput>>(() => {
    try {
      return JSON.parse(localStorage.getItem("mindvault.import.job_params") || "{}");
    } catch {
      return {};
    }
  });
  const [isDownloadingModels, setIsDownloadingModels] = useState(false);
  const [hasAttemptedDownload, setHasAttemptedDownload] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [preview, setPreview] = useState<ImportExtractionPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [previewLoadingJobId, setPreviewLoadingJobId] = useState<string | null>(null);
  const mountedRef = useRef(false);
  const downloadStartedRef = useRef(false);

  const refreshJobs = useCallback(async () => {
    try {
      const result = await listImportJobs();
      if (mountedRef.current) {
        setJobs(result);
        try {
          const parsed = JSON.parse(localStorage.getItem("mindvault.import.job_params") || "{}");
          setJobParams(parsed);
        } catch {
          // Ignore
        }
      }
    } catch {
      // Swallow polling errors — the log just skips this tick.
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    const intervalId = setInterval(refreshJobs, POLL_INTERVAL_MS);
    const initialFetchId = setTimeout(refreshJobs, 0);
    return () => {
      mountedRef.current = false;
      clearInterval(intervalId);
      clearTimeout(initialFetchId);
    };
  }, [refreshJobs]);

  useEffect(() => {
    if (refreshKey <= 0) return;
    const timer = setTimeout(() => {
      void refreshJobs();
    }, 0);
    return () => clearTimeout(timer);
  }, [refreshKey, refreshJobs]);

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void listVaults()
        .then((vaults) => {
          if (cancelled || !mountedRef.current) return;
          setVaultNameById(new Map(vaults.map((v) => [v.id, v.name])));
        })
        .catch(() => {
          // Destination falls back to id / Root Graph hardcode.
        });
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    onActiveJobsChange?.(jobs.some(isImportJobActive));
  }, [jobs, onActiveJobsChange]);

  useEffect(() => {
    if (downloadStartedRef.current || hasAttemptedDownload) return;
    if (!jobs.some(isModelNotFoundError)) return;

    let cancelled = false;
    const timer = setTimeout(() => {
      if (cancelled || downloadStartedRef.current || hasAttemptedDownload) return;
      downloadStartedRef.current = true;
      setIsDownloadingModels(true);
      startOcrModelDownload()
        .then(() => refreshJobs())
        .finally(() => {
          if (mountedRef.current) {
            setIsDownloadingModels(false);
            setHasAttemptedDownload(true);
          }
        });
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [jobs, hasAttemptedDownload, refreshJobs]);

  const handleCancel = useCallback(async () => {
    if (cancelling) return;
    setCancelling(true);
    try {
      await cancelImportJob();
      await refreshJobs();
    } finally {
      if (mountedRef.current) setCancelling(false);
    }
  }, [cancelling, refreshJobs]);

  const handleViewExtraction = useCallback(async (jobId: string) => {
    setPreviewLoadingJobId(jobId);
    setPreviewError(null);
    try {
      const result = await getImportExtractionPreview(jobId);
      if (mountedRef.current) {
        setPreview(result);
      }
    } catch (error) {
      if (mountedRef.current) {
        setPreviewError(toAppError(error).message || "Failed to load extraction");
      }
    } finally {
      if (mountedRef.current) {
        setPreviewLoadingJobId(null);
      }
    }
  }, []);

  const closePreview = useCallback(() => {
    setPreview(null);
    setPreviewError(null);
  }, []);

  const openProposals = useCallback(
    (changesetId: string) => {
      onOpenChangeset?.(changesetId);
      closePreview();
    },
    [closePreview, onOpenChangeset]
  );

  if (jobs.length === 0) {
    return (
      <div className={ImportJobLogStyles.jobLogPanel}>
        <p>No Current Jobs</p>
        <p className={ImportJobLogStyles.jobHint}>
          Fast Import creates chunked memory adds from extracted text — use View Extraction after a
          job stages to read the full document text.
        </p>
      </div>
    );
  }

  return (
    <div className={ImportJobLogStyles.jobLogPanel}>
      {jobs.map((job) => {
        const modelMissing = isModelNotFoundError(job);
        const canReview = isImportJobResolved(job);
        const destination = formatImportDestinationLabel(job.targetVaultId, vaultNameById);
        return (
          <div key={job.id} className={ImportJobLogStyles.jobCard}>
            <div className={ImportJobLogStyles.jobHeader}>
              <span className={ImportJobLogStyles.jobSourceName}>{job.sourceName}</span>
              <div className={ImportJobLogStyles.jobHeaderActions}>
                {isImportJobActive(job) && (
                  <button
                    type="button"
                    className={ImportJobLogStyles.cancelBtn}
                    onClick={() => void handleCancel()}
                    disabled={cancelling}
                  >
                    {cancelling ? "Cancelling…" : "Cancel"}
                  </button>
                )}
                <span className={ImportJobLogStyles.jobStatusBadge} data-status={job.status}>
                  {isImportJobActive(job) && <span className={ImportJobLogStyles.spinner} />}
                  {formatImportJobStatusLabel(job.status)}
                </span>
              </div>
            </div>

            <p className={ImportJobLogStyles.jobDestination}>
              Destination: {destination}
              {jobParams[job.id] != null && (
                <>
                  {" • "}
                  <span className={ImportJobLogStyles.modeLabel}>
                    {jobParams[job.id].useLlmExtraction ? "AI Extraction" : "Fast Import"}
                  </span>
                </>
              )}
            </p>

            {isDownloadingModels && modelMissing ? (
              <p className={ImportJobLogStyles.jobNotice}>Downloading OCR models, please wait...</p>
            ) : isImportJobFailed(job) ? (
              <div className={ImportJobLogStyles.jobErrorContainer}>
                <p className={ImportJobLogStyles.jobError}>
                  {hasAttemptedDownload && modelMissing
                    ? "Models downloaded — retry import."
                    : (job.error ?? "Import failed")}
                </p>
                {jobParams[job.id] && onRetry && (
                  <button
                    type="button"
                    className={ImportJobLogStyles.retryBtn}
                    onClick={() => onRetry(jobParams[job.id])}
                    title="Retry Import"
                  >
                    <svg
                      className={ImportJobLogStyles.retryIcon}
                      viewBox="0 0 24 24"
                      width="12"
                      height="12"
                    >
                      <path
                        fill="currentColor"
                        d="M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08c-.82 2.33-3.04 4-5.65 4-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
                      />
                    </svg>
                    <span>Retry</span>
                  </button>
                )}
              </div>
            ) : (
              <>
                <div className={ImportJobLogStyles.progressTrack}>
                  <div
                    className={ImportJobLogStyles.progressFill}
                    style={{ width: `${importJobProgressPercent(job)}%` }}
                  />
                </div>
                <p className={ImportJobLogStyles.jobSummary}>{formatImportJobPhaseSummary(job)}</p>
                {job.status === "staged" && (
                  <p className={ImportJobLogStyles.jobHint}>
                    Not in the vault yet — open proposals to accept.
                  </p>
                )}
                <div className={ImportJobLogStyles.jobBadges}>
                  <span className={ImportJobLogStyles.confidenceBadge}>
                    OCR confidence: {Math.round(job.avgOcrConfidence * 100)}%
                  </span>
                  {job.tablesDetectedUnpreserved > 0 && (
                    <span className={ImportJobLogStyles.warningBadge}>
                      {job.tablesDetectedUnpreserved} unstructured table
                      {job.tablesDetectedUnpreserved > 1 ? "s" : ""}
                    </span>
                  )}
                </div>
                {canReview && (
                  <div className={ImportJobLogStyles.jobActions}>
                    <button
                      type="button"
                      className={ImportJobLogStyles.actionBtn}
                      onClick={() => void handleViewExtraction(job.id)}
                      disabled={previewLoadingJobId === job.id}
                    >
                      {previewLoadingJobId === job.id ? "Loading…" : "View Extraction"}
                    </button>
                    {job.changesetId && onOpenChangeset && (
                      <button
                        type="button"
                        className={ImportJobLogStyles.actionBtn}
                        onClick={() => openProposals(job.changesetId!)}
                      >
                        Open proposals
                      </button>
                    )}
                  </div>
                )}
              </>
            )}
          </div>
        );
      })}

      {(preview || previewError) && (
        <div
          className={ImportJobLogStyles.previewOverlay}
          role="dialog"
          aria-modal="true"
          aria-label="Extraction preview"
        >
          <div className={ImportJobLogStyles.previewModal}>
            <div className={ImportJobLogStyles.previewHeader}>
              <div>
                <h3 className={ImportJobLogStyles.previewTitle}>
                  {preview?.sourceName ?? "Extraction"}
                </h3>
                {preview && (
                  <p className={ImportJobLogStyles.previewMeta}>
                    {preview.totalPages} pages · full extracted text (not a document archive)
                  </p>
                )}
              </div>
              <button
                type="button"
                className={ImportJobLogStyles.previewClose}
                onClick={closePreview}
              >
                Close
              </button>
            </div>
            {previewError ? (
              <p className={ImportJobLogStyles.jobError}>{previewError}</p>
            ) : (
              <pre className={ImportJobLogStyles.previewBody}>{preview?.markdown}</pre>
            )}
            <div className={ImportJobLogStyles.previewFooter}>
              {preview?.changesetId && onOpenChangeset && (
                <button
                  type="button"
                  className={ImportJobLogStyles.actionBtn}
                  onClick={() => openProposals(preview.changesetId!)}
                >
                  Review in Diff
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
