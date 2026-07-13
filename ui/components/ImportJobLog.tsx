import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelImportJob,
  listImportJobs,
  startOcrModelDownload,
  type ImportJobStatus,
} from "../services/import";
import ImportJobLogStyles from "../style/components/ImportJobLog.module.css";

const POLL_INTERVAL_MS = 2000;

const TERMINAL_SUCCESS = new Set(["staged", "committed"]);
const TERMINAL_FAILURE = new Set(["failed"]);
const ACTIVE = new Set(["pending", "extracting"]);

function isJobComplete(job: ImportJobStatus): boolean {
  return TERMINAL_SUCCESS.has(job.status);
}

function isJobFailed(job: ImportJobStatus): boolean {
  return TERMINAL_FAILURE.has(job.status);
}

function isJobActive(job: ImportJobStatus): boolean {
  return ACTIVE.has(job.status);
}

function formatPageSummary(job: ImportJobStatus): string {
  if (job.totalPages === 0) {
    return "Waiting for page analysis…";
  }
  const parts: string[] = [];
  if (job.digitalPages > 0) parts.push(`${job.digitalPages} digital`);
  if (job.hybridPages > 0) {
    parts.push(`${job.hybridPages} hybrid (embedded images extracted via OCR)`);
  }
  if (job.ocrPages > 0) parts.push(`${job.ocrPages} scanned`);
  return `${job.totalPages} pages — ${parts.join(", ") || "no page breakdown yet"}`;
}

function jobProgressPercent(job: ImportJobStatus): number {
  if (isJobComplete(job)) return 100;
  if (isJobFailed(job) || job.totalPages === 0) return 0;
  const processed = job.digitalPages + job.ocrPages + job.hybridPages;
  return Math.min(100, Math.round((processed / job.totalPages) * 100));
}

function isModelNotFoundError(job: ImportJobStatus): boolean {
  return isJobFailed(job) && (job.error?.includes("OCR model not found") ?? false);
}

type ImportJobLogProps = {
  refreshKey?: number;
};

export default function ImportJobLog({ refreshKey = 0 }: ImportJobLogProps) {
  const [jobs, setJobs] = useState<ImportJobStatus[]>([]);
  const [isDownloadingModels, setIsDownloadingModels] = useState(false);
  const [hasAttemptedDownload, setHasAttemptedDownload] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const mountedRef = useRef(false);
  const downloadStartedRef = useRef(false);

  const refreshJobs = useCallback(async () => {
    try {
      const result = await listImportJobs();
      if (mountedRef.current) {
        setJobs(result);
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

  if (jobs.length === 0) {
    return (
      <div className={ImportJobLogStyles.jobLogPanel}>
        <p>No Current Jobs</p>
      </div>
    );
  }

  return (
    <div className={ImportJobLogStyles.jobLogPanel}>
      {jobs.map((job) => {
        const modelMissing = isModelNotFoundError(job);
        return (
          <div key={job.id} className={ImportJobLogStyles.jobCard}>
            <div className={ImportJobLogStyles.jobHeader}>
              <span className={ImportJobLogStyles.jobSourceName}>{job.sourceName}</span>
              <div className={ImportJobLogStyles.jobHeaderActions}>
                {isJobActive(job) && (
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
                  {job.status}
                </span>
              </div>
            </div>

            {isDownloadingModels && modelMissing ? (
              <p className={ImportJobLogStyles.jobNotice}>Downloading OCR models, please wait...</p>
            ) : isJobFailed(job) ? (
              <p className={ImportJobLogStyles.jobError}>
                {hasAttemptedDownload && modelMissing
                  ? "Models downloaded — retry import."
                  : (job.error ?? "Import failed")}
              </p>
            ) : (
              <>
                <div className={ImportJobLogStyles.progressTrack}>
                  <div
                    className={ImportJobLogStyles.progressFill}
                    style={{ width: `${jobProgressPercent(job)}%` }}
                  />
                </div>
                <p className={ImportJobLogStyles.jobSummary}>{formatPageSummary(job)}</p>
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
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}
