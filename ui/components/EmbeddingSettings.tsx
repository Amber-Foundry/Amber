import type { EmbeddingStatus } from "../types/generated";
import styles from "../style/components/EmbeddingSettings.module.css";

type EmbeddingSettingsProps = {
  status: EmbeddingStatus | null;
  loading: boolean;
  model: string;
  syncState: "idle" | "running" | "complete" | "error";
  syncError: string;
  onReembed: () => void;
  onCancelReembed: () => void;
};

function formatRelativeTime(isoString: string | null): string {
  if (!isoString) return "Never";

  const timestamp = new Date(isoString).getTime();
  if (Number.isNaN(timestamp)) return "Never";

  const diff = Date.now() - timestamp;
  const minutes = Math.floor(diff / 60_000);

  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;

  const hours = Math.floor(minutes / 60);

  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

export default function EmbeddingSettings({
  status,
  loading,
  model,
  syncState,
  syncError,
  onReembed,
  onCancelReembed,
}: EmbeddingSettingsProps) {
  const coverageLabelId = "embedding-coverage-label";
  const lastComputedId = "embedding-last-computed";
  const isServiceUnavailable = syncState === "error" && !status;
  const isEmptyState = !loading && !status && !isServiceUnavailable;

  if (loading) {
    return (
      <div className={styles["embedding-settings-panel"]} aria-busy="true">
        <p className={styles["embedding-settings-title"]}>Embedding Settings</p>
        <p className={styles["embedding-settings-item"]}>Loading embedding status…</p>
      </div>
    );
  }

  if (isServiceUnavailable) {
    return (
      <div className={styles["embedding-settings-panel"]}>
        <p className={styles["embedding-settings-title"]}>Embedding Settings</p>
        <div
          className={styles["embedding-state-card"]}
          role="alert"
          aria-label="Embedding IPC unreachable"
        >
          <p className={styles["embedding-state-title"]}>Embedding service unavailable</p>
          <p className={styles["embedding-state-copy"]}>
            {syncError || "The embedding IPC could not be reached. Coverage data is unavailable."}
          </p>
        </div>
      </div>
    );
  }

  if (isEmptyState) {
    return (
      <div className={styles["embedding-settings-panel"]}>
        <p className={styles["embedding-settings-title"]}>Embedding Settings</p>
        <div className={styles["embedding-state-card"]} aria-label="No embedding status available">
          <p className={styles["embedding-state-title"]}>No embedding data yet</p>
          <p className={styles["embedding-state-copy"]}>
            Start a re-embed to generate coverage metrics for your memories.
          </p>
        </div>
      </div>
    );
  }

  const embeddingStatus = status;

  if (!embeddingStatus) {
    return null;
  }

  const coverageClamped = Math.min(100, Math.max(0, embeddingStatus.coveragePercent));
  const modelChanged = embeddingStatus.model !== model;

  return (
    <div className={styles["embedding-settings-panel"]}>
      <h1 className={styles["embedding-settings-title"]}>Embedding Settings</h1>

      {embeddingStatus.jaccardFallbackActive && (
        <div
          className={styles["embedding-jaccard-warning"]}
          role="status"
          aria-live="polite"
          aria-label="Embedding fallback warning"
        >
          <span className={styles["embedding-jaccard-warning-icon"]} aria-hidden="true">
            ⚠
          </span>
          <span>
            Using text overlap for dedup — embedding model not available. Download a model for
            better memory matching.
          </span>
        </div>
      )}

      <p className={styles["embedding-settings-item"]}>
        <span className={styles["embedding-settings-label"]}>Model</span>
        {embeddingStatus.model}
      </p>
      <p className={styles["embedding-settings-item"]}>
        <span className={styles["embedding-settings-label"]}>Tier</span>
        {embeddingStatus.tier}
      </p>
      <p className={styles["embedding-settings-item"]}>
        <span className={styles["embedding-settings-label"]}>Backend</span>
        {embeddingStatus.backend}
      </p>

      <div className={styles["embedding-coverage"]}>
        <div className={styles["embedding-coverage-header"]}>
          <span id={coverageLabelId} className={styles["embedding-settings-label"]}>
            Coverage
          </span>
          <span className={styles["embedding-coverage-pct"]}>{coverageClamped}%</span>
        </div>
        <div
          className={styles["embedding-progress-track"]}
          role="progressbar"
          aria-label={`Embedding coverage for ${embeddingStatus.model}`}
          aria-labelledby={coverageLabelId}
          aria-describedby={lastComputedId}
          aria-valuenow={coverageClamped}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className={styles["embedding-progress-fill"]}
            style={{ width: `${coverageClamped}%` }}
            aria-hidden="true"
          />
        </div>
        <p id={lastComputedId} className={styles["embedding-last-computed"]}>
          Last computed: {formatRelativeTime(embeddingStatus.lastComputedAt)}
        </p>
      </div>

      <div className={styles["embedding-settings-actions"]}>
        {syncState === "running" ? (
          <div className={styles["embedding-spinner-row"]} aria-live="polite">
            <span className={styles["embedding-spinner"]} aria-hidden="true" />
            <span className={styles["embedding-settings-item"]}>Polling embedding status…</span>
          </div>
        ) : syncState === "complete" ? (
          <p className={styles["embedding-settings-item"]}>Embedding status updated.</p>
        ) : syncState === "error" ? (
          <p className={styles["embedding-settings-item"]}>{syncError}</p>
        ) : null}

        {embeddingStatus.coveragePercent < 100 ||
        modelChanged ||
        embeddingStatus.reembedInProgress ? (
          <>
            {embeddingStatus.reembedInProgress ? (
              <>
                <div className={styles["embedding-spinner-row"]} aria-live="polite">
                  <span className={styles["embedding-spinner"]} aria-hidden="true" />
                  <span className={styles["embedding-settings-item"]}>Re-embedding…</span>
                </div>
                <button
                  type="button"
                  className={styles["embedding-settings-button"]}
                  aria-label="Cancel the in-progress embedding re-index"
                  onClick={onCancelReembed}
                  disabled={loading}
                >
                  Cancel
                </button>
              </>
            ) : (
              <button
                type="button"
                className={styles["embedding-settings-button"]}
                aria-label="Re-embed memories with the current embedding model"
                onClick={onReembed}
                disabled={syncState === "running" || loading}
              >
                Re-embed Memories
              </button>
            )}
          </>
        ) : (
          <p className={styles["embedding-settings-item"]}>All memories are embedded.</p>
        )}
      </div>
    </div>
  );
}
