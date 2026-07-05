import React, { useEffect, useState } from "react";
import {
  chatListSessions,
  chatCreateSession,
  chatDeleteSession,
  chatUpdateSessionSummary,
  chatSetOffTheRecord,
  chatIsOffTheRecord,
  type ChatSession,
} from "../services/chat";

type ChatHistoryPanelProps = {
  activeSessionId: string | null;
  setActiveSessionId: (id: string | null) => void;
};

export default function ChatHistoryPanel({
  activeSessionId,
  setActiveSessionId,
}: ChatHistoryPanelProps) {
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [isOtr, setIsOtr] = useState(false);
  const [loading, setLoading] = useState(true);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editSummary, setEditSummary] = useState("");
  const [isRenaming, setIsRenaming] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const offTheRecord = await chatIsOffTheRecord();
        if (!active) return;
        setIsOtr(offTheRecord);
        const list = await chatListSessions();
        if (!active) return;
        setSessions(list);
      } catch (error) {
        console.error("Failed to load chat sessions:", error);
      } finally {
        if (active) {
          setLoading(false);
        }
      }
    })();

    const handleChatUpdated = () => {
      setRefreshKey((prev) => prev + 1);
    };

    window.addEventListener("mindvault:chat-external-updated", handleChatUpdated);
    return () => {
      active = false;
      window.removeEventListener("mindvault:chat-external-updated", handleChatUpdated);
    };
  }, [refreshKey]);

  const handleCreateSession = async () => {
    try {
      const emptySession = sessions.find((s) => !s.summary || s.summary === "New Conversation");
      if (emptySession) {
        await chatSetOffTheRecord(false);
        setActiveSessionId(emptySession.id);
        window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
        return;
      }

      const newId = crypto.randomUUID();
      await chatCreateSession(newId, "New Conversation");
      await chatSetOffTheRecord(false);
      setActiveSessionId(newId);
      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
    } catch (error) {
      console.error("Failed to create new session:", error);
    }
  };

  const handleSelectSession = async (id: string) => {
    try {
      await chatSetOffTheRecord(false);
      setActiveSessionId(id);
      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
    } catch (error) {
      console.error("Failed to select session:", error);
    }
  };

  const handleDeleteSession = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    const shouldDelete = window.confirm(
      "Are you sure you want to delete this conversation? This will delete all messages in it."
    );
    if (!shouldDelete) return;

    try {
      await chatDeleteSession(id);

      const updatedSessions = sessions.filter((s) => s.id !== id);
      setSessions(updatedSessions);

      if (activeSessionId === id) {
        if (updatedSessions.length > 0) {
          setActiveSessionId(updatedSessions[0].id);
        } else {
          const newId = crypto.randomUUID();
          await chatCreateSession(newId, "New Conversation");
          setActiveSessionId(newId);
        }
      }

      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
    } catch (error) {
      console.error("Failed to delete session:", error);
    }
  };

  const startRename = (e: React.MouseEvent, s: ChatSession) => {
    e.stopPropagation();
    setEditingId(s.id);
    setEditSummary(s.summary || "New Conversation");
  };

  const cancelRename = (e: React.MouseEvent) => {
    e.stopPropagation();
    setEditingId(null);
    setEditSummary("");
  };

  const saveRename = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    if (!editSummary.trim() || isRenaming) return;
    setIsRenaming(true);
    try {
      await chatUpdateSessionSummary(id, editSummary.trim());
      setSessions((prev) =>
        prev.map((s) => (s.id === id ? { ...s, summary: editSummary.trim() } : s))
      );
      setEditingId(null);
      setEditSummary("");
      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
    } catch (error) {
      console.error("Failed to rename session:", error);
      window.alert(
        `Failed to rename conversation: ${error instanceof Error ? error.message : String(error)}`
      );
    } finally {
      setIsRenaming(false);
    }
  };

  const formatTime = (startedAt: string) => {
    try {
      const utcString =
        startedAt.includes("Z") || startedAt.includes("+")
          ? startedAt
          : `${startedAt.replace(" ", "T")}Z`;
      const date = new Date(utcString);

      const options: Intl.DateTimeFormatOptions = {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      };

      if (date.getFullYear() !== new Date().getFullYear()) {
        options.year = "numeric";
      }

      return date.toLocaleDateString(undefined, options);
    } catch {
      return startedAt;
    }
  };

  if (loading) {
    return (
      <div className="chat-history-panel loading">
        <div className="chat-history-header">
          <h2>Conversations</h2>
        </div>
        <p className="chat-history-empty-note">Loading conversations...</p>
      </div>
    );
  }

  return (
    <div className="pane pane-left">
      <div className="pane-header">
        <span className="sidebar-subtitle">Conversations</span>
        <button type="button" className="chat-history-new-btn" onClick={handleCreateSession}>
          ➕ New Chat
        </button>
      </div>

      {sessions.length === 0 ? (
        <p className="chat-history-empty-note">No conversations yet.</p>
      ) : (
        <ul className="chat-history-list">
          {sessions.map((s) => {
            const isActive = activeSessionId === s.id && !isOtr;
            const isEditing = editingId === s.id;

            if (isEditing) {
              return (
                <li key={s.id} className="chat-history-item editing">
                  <div className="chat-history-rename-box">
                    <input
                      type="text"
                      value={editSummary}
                      onChange={(e) => setEditSummary(e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                      autoFocus
                    />
                    <div className="chat-history-rename-actions">
                      <button
                        type="button"
                        className="chat-history-rename-btn cancel"
                        onClick={cancelRename}
                      >
                        ✕
                      </button>
                      <button
                        type="button"
                        className="chat-history-rename-btn save"
                        onClick={(e) => void saveRename(e, s.id)}
                        disabled={isRenaming}
                      >
                        ✓
                      </button>
                    </div>
                  </div>
                </li>
              );
            }

            return (
              <li key={s.id} className="chat-history-item">
                <button
                  type="button"
                  className={`chat-history-item-btn ${isActive ? "active" : ""}`}
                  onClick={() => void handleSelectSession(s.id)}
                >
                  <div className="chat-history-item-content">
                    <span className="chat-history-session-title">
                      {s.summary || "New Conversation"}
                    </span>
                    <span className="chat-history-session-time">{formatTime(s.startedAt)}</span>
                  </div>
                  <div className="chat-history-item-actions">
                    <button
                      type="button"
                      className="chat-history-action-icon edit"
                      onClick={(e) => startRename(e, s)}
                      title="Rename conversation"
                    >
                      ✏️
                    </button>
                    <button
                      type="button"
                      className="chat-history-action-icon delete"
                      onClick={(e) => void handleDeleteSession(e, s.id)}
                      title="Delete conversation"
                    >
                      🗑️
                    </button>
                  </div>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
