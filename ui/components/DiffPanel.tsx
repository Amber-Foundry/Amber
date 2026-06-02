import { useEffect, useState, useCallback } from "react";
import {
  listPendingChangesets,
  listResolvedChangesets,
  listChangesetItems,
  commitChangeset,
} from "../services/memoryAgent";
import { verifyMasterPassword } from "../services/auth";
import { type Changeset, type ChangesetItem, type ChangesetCommitInput } from "../ipc";
import DiffRow from "./DiffPanel/DiffRow";
import "../style/components/DiffPanel.css";
import "../style/components/DiffPanelActions.css";

function parseSQLiteDate(dateStr: string | null | undefined): Date {
  if (!dateStr) return new Date();
  // If it doesn't contain 'T', replace space with 'T' and append 'Z' for UTC.
  const normalized = dateStr.includes("T") ? dateStr : dateStr.replace(" ", "T") + "Z";
  return new Date(normalized);
}

interface DiffPanelProps {
  onClose: () => void;
  activeChangesetId: string | null;
  onSelectChangeset: (id: string | null) => void;
}

export default function DiffPanel({
  onClose,
  activeChangesetId,
  onSelectChangeset,
}: DiffPanelProps) {
  const [activeTab, setActiveTab] = useState<"pending" | "history">("pending");
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [changesets, setChangesets] = useState<Changeset[]>([]);
  const [changesetNames, setChangesetNames] = useState<Record<string, string>>({});
  const [items, setItems] = useState<ChangesetItem[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isClosing, setIsClosing] = useState(false);
  const [refreshTrigger, setRefreshTrigger] = useState(0);
  const [drawerWidth, setDrawerWidth] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("mindvault-diff-panel-width");
      if (saved) {
        const val = parseInt(saved, 10);
        if (!isNaN(val) && val >= 400) {
          return val;
        }
      }
    } catch {
      // Ignored
    }
    return 480;
  });
  const [isResizing, setIsResizing] = useState(false);

  // Vault locked handling
  const [lockedActionQueue, setLockedActionQueue] = useState<(() => Promise<void>) | null>(null);
  const [showPasswordModal, setShowPasswordModal] = useState(false);
  const [passwordInput, setPasswordInput] = useState("");
  const [passwordError, setPasswordError] = useState("");

  const executeCommit = async (input: ChangesetCommitInput) => {
    try {
      await commitChangeset(input);
      setRefreshTrigger((prev) => prev + 1);
    } catch (err) {
      if (String(err).includes("VAULT_LOCKED")) {
        setLockedActionQueue(() => async () => {
          await executeCommit(input);
        });
        setShowPasswordModal(true);
      } else {
        setError(String(err));
      }
    }
  };

  const handleCommitItem = (
    itemId: string,
    action: "accept" | "dismiss",
    editedData: unknown | null
  ) => {
    if (!activeChangesetId) return;
    void executeCommit({
      changesetId: activeChangesetId,
      itemActions: [{ itemId, action, editedData }],
    });
  };

  const handleBulkAccept = () => {
    if (!activeChangesetId) return;
    const actions = items
      .filter((i) => i.status === "pending")
      .map((i) => ({ itemId: i.id, action: "accept", editedData: null }));
    if (actions.length === 0) return;
    void executeCommit({ changesetId: activeChangesetId, itemActions: actions });
  };

  const handleBulkDismiss = () => {
    if (!activeChangesetId) return;
    const actions = items
      .filter((i) => i.status === "pending")
      .map((i) => ({ itemId: i.id, action: "dismiss", editedData: null }));
    if (actions.length === 0) return;
    void executeCommit({ changesetId: activeChangesetId, itemActions: actions });
  };

  const handlePasswordSubmit = async () => {
    setPasswordError("");
    const result = await verifyMasterPassword(passwordInput);
    if (result.error) {
      setPasswordError(result.error.message);
      return;
    }
    if (result.data) {
      setShowPasswordModal(false);
      setPasswordInput("");
      if (lockedActionQueue) {
        await lockedActionQueue();
        setLockedActionQueue(null);
      }
    } else {
      setPasswordError("Invalid password");
    }
  };

  const handleClose = () => {
    setIsClosing(true);
    setTimeout(() => {
      onClose();
    }, 250);
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
  };

  // Resizing mouse movement listeners
  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      const newWidth = window.innerWidth - e.clientX;
      const minWidth = 400;
      const maxWidth = window.innerWidth * 0.95;
      setDrawerWidth(Math.max(minWidth, Math.min(maxWidth, newWidth)));
    };

    const handleMouseUp = () => {
      setIsResizing(false);
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    document.body.style.userSelect = "none";

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      document.body.style.userSelect = "";
    };
  }, [isResizing]);

  // Save width to localStorage when it changes, debounced or just when not resizing
  useEffect(() => {
    if (!isResizing) {
      try {
        localStorage.setItem("mindvault-diff-panel-width", String(drawerWidth));
      } catch {
        // Ignored
      }
    }
  }, [isResizing, drawerWidth]);

  // Listen to external seed updates for dynamic UI refresh
  useEffect(() => {
    const handleSeeded = () => {
      setRefreshTrigger((prev) => prev + 1);
    };
    window.addEventListener("mindvault-changeset-seeded", handleSeeded);
    return () => {
      window.removeEventListener("mindvault-changeset-seeded", handleSeeded);
    };
  }, []);

  // Load changesets on mount, tab switch, seed event, or when the activeChangesetId changes to null
  useEffect(() => {
    let active = true;
    const fetchChangesets = async () => {
      setIsLoading(true);
      setError(null);
      try {
        const data =
          activeTab === "pending" ? await listPendingChangesets() : await listResolvedChangesets();
        if (active) {
          setChangesets(data);
        }
      } catch (err) {
        if (active) {
          setError(String(err));
        }
      } finally {
        if (active) {
          setIsLoading(false);
        }
      }
    };

    if (!activeChangesetId) {
      void fetchChangesets();
    }

    return () => {
      active = false;
    };
  }, [activeTab, activeChangesetId, refreshTrigger]);

  // Load items when a changeset is selected
  useEffect(() => {
    let active = true;
    const fetchItems = async () => {
      if (!activeChangesetId) {
        setItems([]);
        return;
      }
      setIsLoading(true);
      setError(null);
      try {
        const data = await listChangesetItems(activeChangesetId);
        if (active) {
          setItems(data);
        }
      } catch (err) {
        if (active) {
          setError(String(err));
        }
      } finally {
        if (active) {
          setIsLoading(false);
        }
      }
    };

    void fetchItems();

    return () => {
      active = false;
    };
  }, [activeChangesetId, refreshTrigger]);

  // Handle Tab Switching
  const handleTabChange = (tab: "pending" | "history") => {
    setActiveTab(tab);
    onSelectChangeset(null);
    setSearchQuery("");
    setSelectedCategory(null);
  };

  // Safe JSON Parsing for proposed/existing data
  const parseJSON = (str: string) => {
    try {
      return JSON.parse(str);
    } catch {
      return {};
    }
  };

  // Helper to extract item summary title
  const getItemTitle = useCallback((item: ChangesetItem) => {
    const data = parseJSON(item.proposedData);
    if (
      item.itemType.toLowerCase() === "repoint_door" ||
      item.itemType.toLowerCase() === "orphan"
    ) {
      return `Repoint door #${item.doorId || "unknown"}`;
    }
    return data.title || data.summary || `Proposal #${item.id.slice(0, 8)}`;
  }, []);

  // Prefetch first item of changesets to build friendly human-readable names
  useEffect(() => {
    let active = true;
    const fetchFriendlyNames = async () => {
      const names: Record<string, string> = {};
      for (const cs of changesets) {
        try {
          const itemsList = await listChangesetItems(cs.id);
          if (itemsList.length > 0) {
            // Find content proposals first (ADD, UPDATE, MERGE)
            const contentItem = itemsList.find(
              (i) =>
                i.itemType.toLowerCase() === "add" ||
                i.itemType.toLowerCase() === "update" ||
                i.itemType.toLowerCase() === "merge"
            );
            const primaryItem = contentItem || itemsList[0];
            const primaryTitle = getItemTitle(primaryItem);

            if (itemsList.length > 1) {
              names[cs.id] =
                `${primaryTitle} & ${itemsList.length - 1} other${itemsList.length - 1 > 1 ? "s" : ""}`;
            } else {
              names[cs.id] = primaryTitle;
            }
          } else {
            names[cs.id] = `Empty Changeset #${cs.id.slice(0, 8)}`;
          }
        } catch (err) {
          console.error("Failed to load items for changeset friendly name:", err);
          names[cs.id] = `Changeset #${cs.id.slice(0, 8)}`;
        }
      }
      if (active) {
        setChangesetNames(names);
      }
    };
    if (changesets.length > 0) {
      void fetchFriendlyNames();
    }
    return () => {
      active = false;
    };
  }, [changesets, getItemTitle]);

  // Filter Changesets
  const filteredChangesets = changesets.filter((cs) => {
    const matchSearch =
      cs.id.toLowerCase().includes(searchQuery.toLowerCase()) ||
      (cs.modelUsed && cs.modelUsed.toLowerCase().includes(searchQuery.toLowerCase()));

    if (activeTab === "pending") {
      // Must have actual pending proposals left to review
      return matchSearch && cs.itemCount > 0 && cs.itemCount > cs.acceptedCount + cs.dismissedCount;
    }
    return matchSearch;
  });

  // Filter Changeset Items
  const filteredItems = items.filter((item) => {
    const title = getItemTitle(item).toLowerCase();
    const parsed = parseJSON(item.proposedData);
    const summary = (parsed.summary || "").toLowerCase();
    const detail = (parsed.detail || "").toLowerCase();
    const matchSearch =
      title.includes(searchQuery.toLowerCase()) ||
      summary.includes(searchQuery.toLowerCase()) ||
      detail.includes(searchQuery.toLowerCase());

    const matchCategory =
      !selectedCategory || item.itemType.toLowerCase() === selectedCategory.toLowerCase();

    return matchSearch && matchCategory;
  });

  return (
    <div className={`diff-panel-backdrop ${isClosing ? "closing" : ""}`} onClick={handleClose}>
      <div
        className={`diff-panel-drawer ${isClosing ? "closing" : ""}`}
        style={{
          width: `${drawerWidth}px`,
          transition: isResizing ? "none" : undefined,
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Resize Handle */}
        <div
          className={`diff-panel-drawer-resize-handle ${isResizing ? "resizing" : ""}`}
          onMouseDown={handleMouseDown}
        />

        {/* Header */}
        <div className="diff-panel-header">
          <span className="diff-panel-title">
            {activeChangesetId ? "Changeset Details" : "Memory Proposals"}
          </span>
          <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
            {activeChangesetId && items.some((i) => i.status === "pending") && (
              <>
                <button
                  className="diff-panel-bulk-btn"
                  style={{ color: "#dc2626" }}
                  onClick={handleBulkDismiss}
                >
                  Dismiss All
                </button>
                <button
                  className="diff-panel-bulk-btn"
                  style={{ color: "#059669" }}
                  onClick={handleBulkAccept}
                >
                  Accept All
                </button>
              </>
            )}
            <button className="diff-panel-close-btn" onClick={handleClose} aria-label="Close panel">
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>
        </div>

        {/* Tab Buttons (Only shown if not drilling down into a changeset) */}
        {!activeChangesetId && (
          <div className="diff-panel-tabs">
            <button
              className={`diff-panel-tab-btn ${activeTab === "pending" ? "active" : ""}`}
              onClick={() => handleTabChange("pending")}
            >
              Pending
            </button>
            <button
              className={`diff-panel-tab-btn ${activeTab === "history" ? "active" : ""}`}
              onClick={() => handleTabChange("history")}
            >
              History
            </button>
          </div>
        )}

        {/* Filters */}
        <div className="diff-panel-filters">
          <div className="diff-panel-search-box">
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="#6b7280"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              type="text"
              placeholder={activeChangesetId ? "Search changeset items..." : "Search changesets..."}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>

          {/* Category badges (Only shown when drilling down into changeset items) */}
          {activeChangesetId && (
            <div className="diff-panel-category-filters">
              <button
                className={`category-filter-btn ${selectedCategory === null ? "active" : ""}`}
                onClick={() => setSelectedCategory(null)}
              >
                ALL
              </button>
              <button
                className={`category-filter-btn ${selectedCategory === "add" ? "active" : ""}`}
                onClick={() => setSelectedCategory("add")}
              >
                ADD
              </button>
              <button
                className={`category-filter-btn ${selectedCategory === "update" ? "active" : ""}`}
                onClick={() => setSelectedCategory("update")}
              >
                UPDATE
              </button>
              <button
                className={`category-filter-btn ${selectedCategory === "merge" ? "active" : ""}`}
                onClick={() => setSelectedCategory("merge")}
              >
                MERGE
              </button>
              <button
                className={`category-filter-btn ${selectedCategory === "delete" ? "active" : ""}`}
                onClick={() => setSelectedCategory("delete")}
              >
                DELETE
              </button>
              <button
                className={`category-filter-btn ${selectedCategory === "repoint_door" || selectedCategory === "orphan" ? "active" : ""}`}
                onClick={() => setSelectedCategory("repoint_door")}
              >
                ORPHAN
              </button>
            </div>
          )}
        </div>

        {/* Content Area */}
        <div className="diff-panel-content">
          {isLoading && <div className="changeset-list-empty">Loading data...</div>}

          {error && (
            <div className="changeset-list-empty" style={{ color: "#ef4444" }}>
              Error: {error}
            </div>
          )}

          {!isLoading && !error && (
            <>
              {/* CHANGESET LIST MODE */}
              {!activeChangesetId && (
                <>
                  {filteredChangesets.length === 0 ? (
                    <div className="changeset-list-empty">
                      <svg
                        width="32"
                        height="32"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      >
                        <polyline points="22 12 16 12 14 15 10 15 8 12 2 12" />
                        <path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
                      </svg>
                      <span>No changesets found</span>
                    </div>
                  ) : (
                    filteredChangesets.map((cs) => (
                      <div
                        key={cs.id}
                        className={`changeset-card ${activeChangesetId === cs.id ? "active" : ""}`}
                        onClick={() => onSelectChangeset(cs.id)}
                      >
                        <div className="changeset-card-header">
                          <span
                            className="changeset-card-id"
                            style={{ fontSize: "0.95rem", fontWeight: "700" }}
                          >
                            {changesetNames[cs.id] || `Changeset #${cs.id.slice(0, 8)}`}
                          </span>
                          <span
                            className={`changeset-card-status status-${cs.status.toLowerCase()}`}
                          >
                            {cs.status}
                          </span>
                        </div>
                        <div
                          style={{
                            fontSize: "0.72rem",
                            color: "#a3a09a",
                            marginTop: "-4px",
                            marginBottom: "6px",
                          }}
                        >
                          ID: #{cs.id.slice(0, 8)}
                        </div>
                        <div className="changeset-card-details">
                          <div>Proposals: {cs.itemCount}</div>
                          <div>Model: {cs.modelUsed || "Unknown"}</div>
                          <div>Accepted: {cs.acceptedCount}</div>
                          <div>Dismissed: {cs.dismissedCount}</div>
                        </div>
                        <div className="changeset-card-time">
                          <svg
                            width="12"
                            height="12"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="2"
                            strokeLinecap="round"
                            strokeLinejoin="round"
                          >
                            <circle cx="12" cy="12" r="10" />
                            <polyline points="12 6 12 12 16 14" />
                          </svg>
                          {parseSQLiteDate(cs.createdAt).toLocaleString()}
                        </div>
                        {activeTab === "history" && (
                          <div
                            className="changeset-card-backup"
                            style={{
                              fontSize: "0.75rem",
                              color: "#bc6c25",
                              marginTop: "8px",
                              paddingTop: "8px",
                              borderTop: "1px dashed rgba(188, 108, 37, 0.12)",
                              display: "flex",
                              alignItems: "center",
                              gap: "6px",
                            }}
                          >
                            <span>
                              📂 backups/mindvault-pre-changeset-
                              {parseSQLiteDate(cs.reviewedAt || cs.createdAt).getTime()}
                              .db
                            </span>
                          </div>
                        )}
                      </div>
                    ))
                  )}
                </>
              )}

              {/* CHANGESET ITEMS DRILL-DOWN MODE */}
              {activeChangesetId && (
                <>
                  <div style={{ marginBottom: "12px" }}>
                    <button className="category-filter-btn" onClick={() => onSelectChangeset(null)}>
                      ← Back to Changesets
                    </button>
                  </div>

                  {filteredItems.length === 0 ? (
                    <div className="changeset-list-empty">
                      <span>No proposal items match the filters</span>
                    </div>
                  ) : (
                    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
                      {/* Detailed diff cards will render here in Commit 4. 
                          For Commit 3, we render a highly polished list summary with type badges. */}
                      {filteredItems.map((item) => (
                        <div
                          key={item.id}
                          className={item.status !== "pending" ? "diff-row-resolved" : ""}
                          style={{
                            transition: "opacity 0.5s ease-out, transform 0.5s ease-out",
                            opacity: item.status !== "pending" ? 0.4 : 1,
                          }}
                        >
                          <DiffRow item={item} onCommitItem={handleCommitItem} />
                        </div>
                      ))}
                    </div>
                  )}
                </>
              )}
            </>
          )}
        </div>
      </div>

      {showPasswordModal && (
        <div className="diff-edit-modal-backdrop" onClick={() => setShowPasswordModal(false)}>
          <div className="diff-edit-modal" onClick={(e) => e.stopPropagation()}>
            <h3 style={{ margin: "0 0 16px 0", color: "#dc2626" }}>Vault Locked</h3>
            <p style={{ margin: "0 0 16px 0", fontSize: "0.9rem", color: "#7d7a75" }}>
              This proposal targets a redacted vault that is currently locked. Please enter your
              master password to complete the commit.
            </p>
            <div className="edit-form-group">
              <label>Master Password</label>
              <input
                type="password"
                value={passwordInput}
                onChange={(e) => setPasswordInput(e.target.value)}
                placeholder="Enter your master password..."
                onKeyDown={(e) => {
                  if (e.key === "Enter") handlePasswordSubmit();
                }}
                autoFocus
              />
            </div>
            {passwordError && (
              <div style={{ color: "#dc2626", fontSize: "0.85rem", marginTop: "4px" }}>
                {passwordError}
              </div>
            )}
            <div className="edit-modal-actions">
              <button className="edit-cancel-btn" onClick={() => setShowPasswordModal(false)}>
                Cancel
              </button>
              <button className="edit-save-btn" onClick={handlePasswordSubmit}>
                Unlock & Commit
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
