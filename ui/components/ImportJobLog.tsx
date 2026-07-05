import { useCallback, useEffect, useRef, useState } from "react";
import { listImportJobs, startOcrModelDownload, type ImportJobStatus } from "../services/import";
import ImportJobLogStyles from "../style/components/ImportJobLog.module.css";

const POLL_INTERVAL_MS = 2000;

function formatPageSummary(job: ImportJobStatus): string {
  const parts: string[] = [];
  if (job.digitalPages > 0) parts.push(`${job.digitalPages} digital`);
  if (job.hybridPages > 0) {
    parts.push(`${job.hybridPages} hybrid (embedded images extracted via OCR)`);
  }
  if (job.ocrPages > 0) parts.push(`${job.ocrPages} scanned`);
  return `${job.totalPages} pages — ${parts.join(", ")}`;
}

function jobProgressPercent(job: ImportJobStatus): number {
  if (job.status === "completed") return 100;
  if (job.status === "error" || job.totalPages === 0) return 0;
  const processed = job.digitalPages + job.ocrPages + job.hybridPages;
  return Math.min(100, Math.round((processed / job.totalPages) * 100));
}

// Stringly-typed sentinel match until the 2.4 backend gives us a real
// OcrError enum on the wire.
function isModelNotFoundError(job: ImportJobStatus): boolean {
  return job.status === "error" && job.error.includes("OcrError::ModelNotFound");
}

export default function ImportJobLog() {
  const [jobs, setJobs] = useState<ImportJobStatus[]>([]);
  const [downloadingIds, setDownloadingIds] = useState<Set<string>>(new Set());
  const mountedRef = useRef(false);

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

  // Poll on mount, keep polling on an interval.
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

  // Watch for ModelNotFound errors and kick off the bootstrap download once
  // per job, guarded by downloadingIds so it can't loop.
  useEffect(() => {
    jobs.forEach((job) => {
      if (!isModelNotFoundError(job) || downloadingIds.has(job.id)) return;

      setDownloadingIds((prev) => new Set(prev).add(job.id));
      startOcrModelDownload(job.id)
        .then(() => refreshJobs())
        .finally(() => {
          if (!mountedRef.current) return;
          setDownloadingIds((prev) => {
            const next = new Set(prev);
            next.delete(job.id);
            return next;
          });
        });
    });
  }, [jobs, downloadingIds, refreshJobs]);

  if (jobs.length === 0) {
    return (
      <div className={ImportJobLogStyles.jobLogPanel}>
        <p>No Current Jobs</p>
      </div>
    );
  }

  return (
    <div className={ImportJobLogStyles.jobLogPanel}>
      {jobs.map((job) => (
        <div key={job.id} className={ImportJobLogStyles.jobCard}>
          <div className={ImportJobLogStyles.jobHeader}>
            <span className={ImportJobLogStyles.jobSourceName}>{job.sourceName}</span>
            <span className={ImportJobLogStyles.jobStatusBadge} data-status={job.status}>
              {job.status}
            </span>
          </div>

          {downloadingIds.has(job.id) ? (
            <p className={ImportJobLogStyles.jobNotice}>Downloading OCR models, please wait...</p>
          ) : job.status === "error" ? (
            <p className={ImportJobLogStyles.jobError}>{job.error}</p>
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
      ))}
    </div>
  );
}
