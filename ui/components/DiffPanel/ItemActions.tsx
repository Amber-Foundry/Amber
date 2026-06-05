import { useState, useEffect } from "react";
import type { ChangesetItem, Vault } from "../../ipc";
import { listVaults } from "../../services/vaults";
import { createPortal } from "react-dom";

interface ItemActionsProps {
  item: ChangesetItem;
  onCommitItem: (action: "accept" | "dismiss" | "edit", editedData: unknown | null) => void;
}

export default function ItemActions({ item, onCommitItem }: ItemActionsProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [editForm, setEditForm] = useState<Record<string, string>>({});
  const [vaults, setVaults] = useState<Vault[]>([]);

  useEffect(() => {
    let active = true;
    const fetchVaults = async () => {
      try {
        const list = await listVaults();
        if (active) {
          setVaults(list);
        }
      } catch (err) {
        console.error("Failed to load vaults for dropdown:", err);
      }
    };
    if (isEditing) {
      void fetchVaults();
    }
    return () => {
      active = false;
    };
  }, [isEditing]);

  const parseJSON = (str: string | null) => {
    if (!str) return {};
    try {
      return JSON.parse(str);
    } catch {
      return {};
    }
  };

  const [validationError, setValidationError] = useState<string | null>(null);

  const handleOpenEdit = () => {
    const data = parseJSON(item.proposedData);
    setEditForm({
      title: data.title || "",
      summary: data.summary || "",
      detail: data.detail || "",
      tags: (data.tags || []).join(", "),
      vaultId: data.vaultId || data.vault_id || "",
    });
    setValidationError(null);
    setIsEditing(true);
  };

  const handleSaveEdit = () => {
    const trimmedTitle = editForm.title.trim();
    if (!trimmedTitle) {
      setValidationError("Title is required and cannot be empty.");
      return;
    }

    const updatedData = {
      ...parseJSON(item.proposedData),
      title: trimmedTitle,
      summary: editForm.summary.trim() || undefined,
      detail: editForm.detail.trim(),
      tags: editForm.tags
        ? editForm.tags
            .split(",")
            .map((t: string) => t.trim())
            .filter((t: string) => t.length > 0)
        : undefined,
      vaultId: editForm.vaultId.trim() || undefined,
    };

    onCommitItem("edit", updatedData);
    setIsEditing(false);
  };

  if (item.status !== "pending") {
    return null; // Do not show actions for already resolved items
  }

  return (
    <div className="diff-item-actions">
      <button
        className="action-btn accept-btn"
        onClick={() => onCommitItem("accept", null)}
        title="Accept Proposal"
        aria-label="Accept"
      >
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
          <polyline points="20 6 9 17 4 12"></polyline>
        </svg>
      </button>

      {item.itemType.toLowerCase() !== "delete" &&
        item.itemType.toLowerCase() !== "orphan_alert" &&
        item.itemType.toLowerCase() !== "repoint_door" && (
          <button
            className="action-btn edit-btn"
            onClick={handleOpenEdit}
            title="Edit Proposal"
            aria-label="Edit"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
            </svg>
          </button>
        )}

      <button
        className="action-btn dismiss-btn"
        onClick={() => onCommitItem("dismiss", null)}
        title="Dismiss Proposal"
        aria-label="Dismiss"
      >
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
          <line x1="18" y1="6" x2="6" y2="18"></line>
          <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
      </button>

      {isEditing &&
        createPortal(
          <div className="diff-edit-modal-backdrop" onClick={() => setIsEditing(false)}>
            <div className="diff-edit-modal" onClick={(e) => e.stopPropagation()}>
              <h3 style={{ margin: "0 0 16px 0", color: "#bc6c25" }}>Edit Proposed Data</h3>

              {validationError && (
                <div className="edit-validation-error">
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    style={{ flexShrink: 0 }}
                  >
                    <circle cx="12" cy="12" r="10"></circle>
                    <line x1="12" y1="8" x2="12" y2="12"></line>
                    <line x1="12" y1="16" x2="12.01" y2="16"></line>
                  </svg>
                  {validationError}
                </div>
              )}

              <div className="edit-form-group">
                <label>Title</label>
                <input
                  type="text"
                  value={editForm.title}
                  onChange={(e) => {
                    setEditForm({ ...editForm, title: e.target.value });
                    if (e.target.value.trim()) {
                      setValidationError(null);
                    }
                  }}
                  className={validationError ? "has-error" : ""}
                  placeholder="Node Title"
                />
              </div>

              <div className="edit-form-group">
                <label>Summary</label>
                <textarea
                  value={editForm.summary}
                  onChange={(e) => setEditForm({ ...editForm, summary: e.target.value })}
                  placeholder="Brief summary..."
                  rows={2}
                />
              </div>

              <div className="edit-form-group">
                <label>Detail</label>
                <textarea
                  value={editForm.detail}
                  onChange={(e) => setEditForm({ ...editForm, detail: e.target.value })}
                  placeholder="Detailed description..."
                  rows={4}
                />
              </div>

              <div className="edit-form-group">
                <label>Tags (comma-separated)</label>
                <input
                  type="text"
                  value={editForm.tags}
                  onChange={(e) => setEditForm({ ...editForm, tags: e.target.value })}
                  placeholder="tag1, tag2, tag3"
                />
              </div>

              <div className="edit-form-group">
                <label>Target Vault</label>
                <select
                  value={editForm.vaultId}
                  onChange={(e) => setEditForm({ ...editForm, vaultId: e.target.value })}
                >
                  <option value="">Select a vault...</option>
                  {vaults.map((v) => (
                    <option key={v.id} value={v.id}>
                      {v.name}
                    </option>
                  ))}
                </select>
              </div>

              <div className="edit-modal-actions">
                <button className="edit-cancel-btn" onClick={() => setIsEditing(false)}>
                  Cancel
                </button>
                <button className="edit-save-btn" onClick={handleSaveEdit}>
                  Save & Accept
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
    </div>
  );
}
