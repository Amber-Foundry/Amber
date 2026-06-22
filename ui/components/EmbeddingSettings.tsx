// TEMP — to be replaced with ../../types/generated
type EmbeddingStatus = {
  activeModel: string;
  tier: string;
  backend: string;
  coveragePercent: number;
  lastComputedAt: string | null;
  jaccardFallbackActive: boolean;
  reembedInProgress: boolean;
};
// END TEMP

type EmbeddingSettingsProps = {
  status: EmbeddingStatus | null;
  loading: boolean;
  onReembed: () => void;
  onCancelReembed: () => void;
};

function formatRelativeTime(isoString: string | null): string {
  if (!isoString) return "Never";

  const diff = Date.now() - new Date(isoString).getTime();
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
  onReembed,
  onCancelReembed,
}: EmbeddingSettingsProps) {
  if (loading) {
    return (
      <div className="embedding-settings-panel">
        <p className="embedding-settings-item">Loading embedding status…</p>
      </div>
    );
  }

  if (!status) {
    return (
      <div className="embedding-settings-panel">
        <p className="embedding-settings-item">No embedding status available.</p>
      </div>
    );
  }

  const coverageClamped = Math.min(100, Math.max(0, status.coveragePercent));

  return (
    <div className="embedding-settings-panel">
      <h1 className="embedding-settings-title">Embedding Settings</h1>

      {/* Jaccard fallback warning */}
      {status.jaccardFallbackActive && (
        <div className="embedding-jaccard-warning">
          <span className="embedding-jaccard-warning-icon">⚠</span>
          <span>
            Using text overlap for dedup — embedding model not available. Download a model for
            better memory matching.
          </span>
        </div>
      )}

      {/* Model info */}
      <p className="embedding-settings-item">
        <span className="embedding-settings-label">Model</span>
        {status.activeModel}
      </p>
      <p className="embedding-settings-item">
        <span className="embedding-settings-label">Tier</span>
        {status.tier}
      </p>
      <p className="embedding-settings-item">
        <span className="embedding-settings-label">Backend</span>
        {status.backend}
      </p>

      {/* Coverage progress */}
      <div className="embedding-coverage">
        <div className="embedding-coverage-header">
          <span className="embedding-settings-label">Coverage</span>
          <span className="embedding-coverage-pct">{coverageClamped}%</span>
        </div>
        <div className="embedding-progress-track">
          <div
            className="embedding-progress-fill"
            style={{ width: `${coverageClamped}%` }}
            role="progressbar"
            aria-valuenow={coverageClamped}
            aria-valuemin={0}
            aria-valuemax={100}
          />
        </div>
        <p className="embedding-settings-item embedding-last-computed">
          Last computed: {formatRelativeTime(status.lastComputedAt)}
        </p>
      </div>

      {/* Actions */}
      <div className="embedding-settings-actions">
        {status.reembedInProgress ? (
          <>
            <div className="embedding-spinner-row">
              <span className="embedding-spinner" aria-label="Re-embedding in progress" />
              <span className="embedding-settings-item">Re-embedding…</span>
            </div>
            <button className="embedding-settings-button" onClick={onCancelReembed}>
              Cancel
            </button>
          </>
        ) : (
          <button className="embedding-settings-button" onClick={onReembed}>
            Re-embed Memories
          </button>
        )}
      </div>
    </div>
  );
}
