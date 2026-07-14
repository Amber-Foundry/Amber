import { type ChangesetItem } from "../../ipc";
import ItemActions from "./ItemActions";
import { AlertIcon, FolderIcon, LinkIcon } from "../icons";

interface DiffRowProps {
  item: ChangesetItem;
  onCommitItem: (
    itemId: string,
    action: "accept" | "dismiss" | "edit",
    editedData: unknown | null
  ) => void;
}

interface ProposedData {
  title?: string;
  summary?: string;
  detail?: string;
  tags?: string[];
  vaultId?: string;
  vault_id?: string;
  _amended?: { at?: string; similarity?: number; reason?: string };
}

interface ExistingData {
  title?: string;
  summary?: string;
  detail?: string;
  tags?: string[];
  vaultId?: string;
  vault_id?: string;
}

interface DiffToken {
  type: "match" | "insert" | "delete";
  text: string;
}

// Custom Word-Level LCS Diffing Utility
const diffCache = new Map<string, DiffToken[]>();
const MAX_CACHE_SIZE = 1000;

function setCache(key: string, value: DiffToken[]) {
  if (diffCache.size >= MAX_CACHE_SIZE) {
    diffCache.clear();
  }
  diffCache.set(key, value);
}

function diffLines(oldStr: string, newStr: string): DiffToken[] {
  const oldLines = oldStr
    .split("\n")
    .map((line, idx, arr) => line + (idx < arr.length - 1 ? "\n" : ""));
  const newLines = newStr
    .split("\n")
    .map((line, idx, arr) => line + (idx < arr.length - 1 ? "\n" : ""));

  // If the number of lines is also extremely large (e.g., > 300 lines),
  // fall back to displaying the text directly without highlighting to prevent hangs.
  if (oldLines.length > 300 || newLines.length > 300) {
    return [{ type: "match", text: newStr }];
  }

  const cols = newLines.length + 1;
  const dp = new Int32Array((oldLines.length + 1) * cols);

  for (let i = 1; i <= oldLines.length; i++) {
    for (let j = 1; j <= newLines.length; j++) {
      if (oldLines[i - 1] === newLines[j - 1]) {
        dp[i * cols + j] = dp[(i - 1) * cols + (j - 1)] + 1;
      } else {
        dp[i * cols + j] = Math.max(dp[(i - 1) * cols + j], dp[i * cols + (j - 1)]);
      }
    }
  }

  const tokens: DiffToken[] = [];
  let i = oldLines.length;
  let j = newLines.length;

  while (i > 0 || j > 0) {
    if (i > 0 && j > 0 && oldLines[i - 1] === newLines[j - 1]) {
      tokens.push({ type: "match", text: oldLines[i - 1] });
      i--;
      j--;
    } else if (j > 0 && (i === 0 || dp[i * cols + (j - 1)] >= dp[(i - 1) * cols + j])) {
      tokens.push({ type: "insert", text: newLines[j - 1] });
      j--;
    } else {
      tokens.push({ type: "delete", text: oldLines[i - 1] });
      i--;
    }
  }

  return tokens.reverse();
}

function diffWords(oldStr: string, newStr: string): DiffToken[] {
  const cleanOld = oldStr || "";
  const cleanNew = newStr || "";

  const cacheKey = `${cleanOld}\0${cleanNew}`;
  const cached = diffCache.get(cacheKey);
  if (cached) return cached;

  if (!cleanOld) {
    const result: DiffToken[] = [{ type: "insert", text: cleanNew }];
    setCache(cacheKey, result);
    return result;
  }
  if (!cleanNew) {
    const result: DiffToken[] = [{ type: "delete", text: cleanOld }];
    setCache(cacheKey, result);
    return result;
  }

  // Prevent O(N*M) quadratic complexity in LCS diffing by setting a threshold limit.
  // If either string exceeds 300 words, we fall back to a line-by-line diff.
  const oldWordCount = cleanOld.split(/\s+/).filter(Boolean).length;
  const newWordCount = cleanNew.split(/\s+/).filter(Boolean).length;

  if (oldWordCount > 300 || newWordCount > 300) {
    const result = diffLines(cleanOld, cleanNew);
    setCache(cacheKey, result);
    return result;
  }

  const oldWords = cleanOld.split(/(\s+)/);
  const newWords = cleanNew.split(/(\s+)/);

  const cols = newWords.length + 1;
  const dp = new Int32Array((oldWords.length + 1) * cols);

  for (let i = 1; i <= oldWords.length; i++) {
    for (let j = 1; j <= newWords.length; j++) {
      if (oldWords[i - 1] === newWords[j - 1]) {
        dp[i * cols + j] = dp[(i - 1) * cols + (j - 1)] + 1;
      } else {
        dp[i * cols + j] = Math.max(dp[(i - 1) * cols + j], dp[i * cols + (j - 1)]);
      }
    }
  }

  const tokens: DiffToken[] = [];
  let i = oldWords.length;
  let j = newWords.length;

  while (i > 0 || j > 0) {
    if (i > 0 && j > 0 && oldWords[i - 1] === newWords[j - 1]) {
      tokens.push({ type: "match", text: oldWords[i - 1] });
      i--;
      j--;
    } else if (j > 0 && (i === 0 || dp[i * cols + (j - 1)] >= dp[(i - 1) * cols + j])) {
      tokens.push({ type: "insert", text: newWords[j - 1] });
      j--;
    } else {
      tokens.push({ type: "delete", text: oldWords[i - 1] });
      i--;
    }
  }

  const result = tokens.reverse();
  setCache(cacheKey, result);
  return result;
}

export default function DiffRow({ item, onCommitItem }: DiffRowProps) {
  // Safe JSON parsing
  const parseJSON = (str: string | null) => {
    if (!str) return {};
    try {
      return JSON.parse(str);
    } catch {
      return {};
    }
  };

  const proposed: ProposedData = parseJSON(item.proposedData);
  const existing: ExistingData = parseJSON(item.existingData);
  const typeUpper = item.itemType.toUpperCase();

  // Color mapping classes for badges
  let badgeClass = "badge-add";
  if (typeUpper === "UPDATE") badgeClass = "badge-update";
  else if (typeUpper === "MERGE") badgeClass = "badge-merge";
  else if (typeUpper === "DELETE") badgeClass = "badge-delete";
  else if (typeUpper === "REPOINT_DOOR" || typeUpper === "ORPHAN_ALERT")
    badgeClass = "badge-orphan";

  // Safe Tag Diff calculation
  const getTagsDiff = (oldTagsList?: string[], newTagsList?: string[]) => {
    const oldTags = oldTagsList || [];
    const newTags = newTagsList || [];
    const oldSet = new Set(oldTags.map((t) => t.toLowerCase()));
    const newSet = new Set(newTags.map((t) => t.toLowerCase()));

    const allTags: { text: string; type: "added" | "deleted" | "unchanged" }[] = [];

    oldTags.forEach((t) => {
      if (!newSet.has(t.toLowerCase())) {
        allTags.push({ text: t, type: "deleted" });
      }
    });

    newTags.forEach((t) => {
      if (oldSet.has(t.toLowerCase())) {
        allTags.push({ text: t, type: "unchanged" });
      } else {
        allTags.push({ text: t, type: "added" });
      }
    });

    return allTags;
  };

  // Inline Word diff renderer
  const renderTextDiff = (oldText?: string, newText?: string) => {
    const tokens = diffWords(oldText || "", newText || "");
    return (
      <>
        {tokens.map((token, index) => {
          if (token.type === "insert") {
            return (
              <span key={index} className="diff-highlight-insert">
                {token.text}
              </span>
            );
          } else if (token.type === "delete") {
            return (
              <span key={index} className="diff-highlight-delete">
                {token.text}
              </span>
            );
          }
          return <span key={index}>{token.text}</span>;
        })}
      </>
    );
  };

  return (
    <div className={`diff-row-container ${item.crossVaultAnomaly ? "anomaly" : ""}`}>
      {/* DiffRow Header */}
      <div className="diff-row-header">
        <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>
          <span className={`changeset-item-badge ${badgeClass}`}>
            {typeUpper === "REPOINT_DOOR" || typeUpper === "ORPHAN_ALERT" ? "ORPHAN" : typeUpper}
          </span>
          {proposed._amended && (
            <span
              className="diff-amended-badge"
              title={`Amended at ${proposed._amended.at ?? "unknown"}`}
            >
              (amended)
            </span>
          )}
          <span className="changeset-item-status">Status: {item.status}</span>
        </div>
        <ItemActions
          item={item}
          onCommitItem={(action, editedData) => onCommitItem(item.id, action, editedData)}
        />
      </div>

      {item.crossVaultAnomaly && (
        <div className="anomaly-warning-banner">
          <div className="anomaly-warning-icon">
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
          </div>
          <div className="anomaly-warning-content">
            <span className="anomaly-warning-badge">
              <AlertIcon size={14} /> Security Warning: Mismatched Vault Sensitivity!
            </span>
            <p className="anomaly-warning-text">
              {item.anomalyWarning ||
                "This item was extracted from a general open conversation but is slated to be written into a higher sensitivity vault. Review carefully before accepting."}
            </p>
          </div>
        </div>
      )}

      {/* Comparison Viewports */}
      <div className="diff-row-comparison-grid">
        {/* ADD OPERATION */}
        {typeUpper === "ADD" && (
          <div className="diff-card proposed" style={{ gridColumn: "span 2" }}>
            <div className="diff-card-section">
              <span className="diff-card-label">Title</span>
              <strong className="diff-text-value">{proposed.title || "Untitled"}</strong>
            </div>
            {proposed.summary && (
              <div className="diff-card-section">
                <span className="diff-card-label">Summary</span>
                <span className="diff-text-value">{proposed.summary}</span>
              </div>
            )}
            {proposed.detail && (
              <div className="diff-card-section">
                <span className="diff-card-label">Detail</span>
                <span className="diff-text-value">{proposed.detail}</span>
              </div>
            )}
            {proposed.tags && proposed.tags.length > 0 && (
              <div className="diff-card-section">
                <span className="diff-card-label">Tags</span>
                <div className="diff-tags-container">
                  {proposed.tags.map((tag, idx) => (
                    <span key={idx} className="diff-tag tag-added">
                      {tag}
                    </span>
                  ))}
                </div>
              </div>
            )}
            <div className="diff-card-section">
              <span className="diff-card-label">Target Vault</span>
              <span className="diff-text-value" style={{ fontSize: "0.8rem", color: "#bc6c25" }}>
                <FolderIcon size={14} /> {proposed.vaultId || proposed.vault_id || "unknown_vault"}
              </span>
            </div>
          </div>
        )}

        {/* UPDATE & MERGE OPERATIONS */}
        {(typeUpper === "UPDATE" || typeUpper === "MERGE") && (
          <>
            {/* Left Card: Existing state */}
            <div className="diff-card">
              <div
                className="diff-card-label"
                style={{
                  color: "#7d7a75",
                  borderBottom: "1px solid #f2f0ef",
                  paddingBottom: "4px",
                }}
              >
                Current State
              </div>
              <div className="diff-card-section">
                <span className="diff-card-label">Title</span>
                <strong className="diff-text-value">{existing.title || "Untitled"}</strong>
              </div>
              {existing.summary && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Summary</span>
                  <span className="diff-text-value">{existing.summary}</span>
                </div>
              )}
              {existing.detail && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Detail</span>
                  <span className="diff-text-value">{existing.detail}</span>
                </div>
              )}
              {existing.tags && existing.tags.length > 0 && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Tags</span>
                  <div className="diff-tags-container">
                    {existing.tags.map((tag, idx) => (
                      <span key={idx} className="diff-tag">
                        {tag}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </div>

            {/* Right Card: Proposed state */}
            <div className="diff-card proposed">
              <div
                className="diff-card-label"
                style={{
                  color: typeUpper === "UPDATE" ? "#059669" : "#d97706",
                  borderBottom: "1px solid #f2f0ef",
                  paddingBottom: "4px",
                }}
              >
                Proposed {typeUpper === "UPDATE" ? "Changes" : "Merge"}
              </div>
              <div className="diff-card-section">
                <span className="diff-card-label">Title</span>
                <strong className="diff-text-value">
                  {existing.title !== proposed.title
                    ? renderTextDiff(existing.title, proposed.title)
                    : proposed.title}
                </strong>
              </div>
              {(existing.summary || proposed.summary) && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Summary</span>
                  <span className="diff-text-value">
                    {existing.summary !== proposed.summary
                      ? renderTextDiff(existing.summary, proposed.summary)
                      : proposed.summary}
                  </span>
                </div>
              )}
              {(existing.detail || proposed.detail) && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Detail</span>
                  <span className="diff-text-value">
                    {existing.detail !== proposed.detail
                      ? renderTextDiff(existing.detail, proposed.detail)
                      : proposed.detail}
                  </span>
                </div>
              )}
              {(existing.tags || proposed.tags) && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Tags</span>
                  <div className="diff-tags-container">
                    {getTagsDiff(existing.tags, proposed.tags).map((tag, idx) => (
                      <span
                        key={idx}
                        className={`diff-tag ${
                          tag.type === "added"
                            ? "tag-added"
                            : tag.type === "deleted"
                              ? "tag-deleted"
                              : "tag-unchanged"
                        }`}
                      >
                        {tag.text}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </>
        )}

        {/* DELETE OPERATION */}
        {typeUpper === "DELETE" && (
          <>
            {/* Left Card: Existing target */}
            <div className="diff-card">
              <div
                className="diff-card-label"
                style={{
                  color: "#7d7a75",
                  borderBottom: "1px solid #f2f0ef",
                  paddingBottom: "4px",
                }}
              >
                Target Node to Delete
              </div>
              <div className="diff-card-section">
                <span className="diff-card-label">Title</span>
                <strong className="diff-text-value">{existing.title || "Untitled"}</strong>
              </div>
              {existing.summary && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Summary</span>
                  <span className="diff-text-value">{existing.summary}</span>
                </div>
              )}
              {existing.tags && existing.tags.length > 0 && (
                <div className="diff-card-section">
                  <span className="diff-card-label">Tags</span>
                  <div className="diff-tags-container">
                    {existing.tags.map((tag, idx) => (
                      <span key={idx} className="diff-tag">
                        {tag}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </div>

            {/* Right Card: Retraction evidence */}
            <div className="diff-card proposed">
              <div
                className="diff-card-label"
                style={{
                  color: "#dc2626",
                  borderBottom: "1px solid #f2f0ef",
                  paddingBottom: "4px",
                }}
              >
                Retraction Evidence
              </div>
              <div className="retraction-note">
                <strong>
                  <AlertIcon size={14} /> Proposed Deletion Context
                </strong>
                <p style={{ margin: "6px 0 0 0", fontSize: "0.82rem" }}>
                  {proposed.summary ||
                    "No specific deletion reason compiled by the extraction agent."}
                </p>
              </div>
            </div>
          </>
        )}

        {/* REPOINT_DOOR / ORPHAN OPERATION */}
        {(typeUpper === "REPOINT_DOOR" || typeUpper === "ORPHAN_ALERT") && (
          <div className="diff-card proposed" style={{ gridColumn: "span 2" }}>
            <div
              className="diff-card-label"
              style={{ color: "#b45309", borderBottom: "1px solid #f2f0ef", paddingBottom: "4px" }}
            >
              Orphaned Connection Alignment
            </div>
            <div className="orphan-mapping-box">
              <strong>
                <LinkIcon size={14} /> Resolve Orphaned Door Alert
              </strong>
              <p style={{ margin: "6px 0 0 0", fontSize: "0.84rem" }}>
                Proposed to repoint orphaned door ID{" "}
                <code
                  style={{
                    background: "rgba(188, 108, 37, 0.1)",
                    padding: "2px 4px",
                    borderRadius: "4px",
                  }}
                >
                  #{item.doorId || "unknown"}
                </code>{" "}
                to connect with target node ID{" "}
                <code
                  style={{
                    background: "rgba(188, 108, 37, 0.1)",
                    padding: "2px 4px",
                    borderRadius: "4px",
                  }}
                >
                  #{item.targetNodeId || "unknown"}
                </code>{" "}
                to restore link integrity.
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
