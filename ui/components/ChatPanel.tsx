import React, {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useRef,
  type KeyboardEvent,
} from "react";
import ReactMarkdown from "react-markdown";
import {
  remarkPluginsStable,
  rehypePluginsStable,
  createMarkdownComponents,
  preprocessMathDelimiters,
  preprocessWikiLinks,
  ExistingNodesContext,
} from "../utils/markdownUtils";
import type { ContextAssemblerScope } from "../constants/contextBudget";
import type { Vault } from "../ipc";
import {
  chatAppendMessage,
  clearChatHistory,
  getChatHistory,
  chatEditAndTruncate,
  chatSetOffTheRecord,
  chatIsOffTheRecord,
  chatUpdateSessionSummary,
  type ChatMessage,
} from "../services/chat";
import { chatWithScope, getAllNodes } from "../services/nodes";
import { getSetting } from "../services/settings";
import { listVaults } from "../services/vaults";
import { extractMemoryIfReady, extractMemoryForce } from "../services/memoryAgent";
import {
  getLlmModel,
  getLlmProvider,
  getLmStudioEndpoint,
  getOllamaEndpoint,
  getLlmMode,
  setLlmMode,
  getApiKey,
} from "../utils/settings";
import { useUIStore } from "../utils/store";
import { chatConvertTemporaryToMemory, chatExtractPdfText } from "../ipc";
import { browseImportPdf, startOcrModelDownload } from "../services/import";
import {
  FileIcon,
  AlertIcon,
  ArrowUpIcon,
  ChevronDownIcon,
  GlobeIcon,
  FolderIcon,
  MessageIcon,
  InboxIcon,
  GraduationIcon,
  GearIcon,
  ComputerIcon,
  CloudIcon,
  ZapIcon,
  BrainIcon,
  HourglassIcon,
  EyeOffIcon,
  BarChartIcon,
  TrendingUpIcon,
  CloseIcon,
  SparklesIcon,
} from "./icons";

// Memoized individual message bubble — prevents re-rendering existing messages
// when unrelated parent state (e.g. input text) changes. Each bubble only
// re-renders when its own content, editing state, or copy state changes.
type ChatMessageBubbleProps = {
  message: ChatMessage;
  index: number;
  isEditing: boolean;
  editingContent: string;
  isCopied: boolean;
  editInputRef: React.RefObject<HTMLTextAreaElement | null>;
  onSetEditingContent: (value: string) => void;
  onCancelEdit: () => void;
  onSaveEdit: (index: number, content: string) => void;
  onCopyMessage: (content: string, id: string) => void;
  onRetryMessage: (index: number) => void;
  onStartEdit: (messageId: string, content: string) => void;
  chartsEnabled: boolean;
  onSelectNode?: (nodeId: string) => void;
  existingNodeIds: Set<string> | null;
  isRedactedUnlocked: boolean;
};

const ChatMessageBubble = React.memo(function ChatMessageBubble({
  message,
  index,
  isEditing,
  editingContent,
  isCopied,
  editInputRef,
  onSetEditingContent,
  onCancelEdit,
  onSaveEdit,
  onCopyMessage,
  onRetryMessage,
  onStartEdit,
  chartsEnabled,
  onSelectNode,
  existingNodeIds,
  isRedactedUnlocked,
}: ChatMessageBubbleProps) {
  const bubbleContentRef = useRef<HTMLDivElement>(null);
  const [isOverflowing, setIsOverflowing] = useState(false);
  const [isCollapsed, setIsCollapsed] = useState(true);

  useEffect(() => {
    if (bubbleContentRef.current && message.role === "user" && !isEditing) {
      // scrollHeight > 245 corresponds to more than 10 lines of text
      const hasOverflow = bubbleContentRef.current.scrollHeight > 245;
      setIsOverflowing(hasOverflow);
      setIsCollapsed(true);
    }
  }, [message.content, message.role, isEditing]);

  const markdownComponents = React.useMemo(() => {
    return createMarkdownComponents(chartsEnabled, onSelectNode, isRedactedUnlocked);
  }, [chartsEnabled, onSelectNode, isRedactedUnlocked]);

  const preprocessedMessage = React.useMemo(() => {
    const wLinks = preprocessWikiLinks(message.content);
    return preprocessMathDelimiters(wLinks);
  }, [message.content]);

  const markdownBody = React.useMemo(
    () => (
      <ReactMarkdown
        remarkPlugins={remarkPluginsStable}
        rehypePlugins={rehypePluginsStable}
        components={markdownComponents}
      >
        {preprocessedMessage}
      </ReactMarkdown>
    ),
    [markdownComponents, preprocessedMessage]
  );

  return (
    <article className={`chat-message chat-message-${message.role}`}>
      {message.role === "assistant" && <div className="chat-avatar">MV</div>}
      <div className="chat-bubble-container">
        <div className="chat-bubble">
          {isEditing ? (
            <div className="chat-bubble-edit-container">
              <textarea
                ref={editInputRef}
                className="chat-bubble-edit-textarea"
                value={editingContent}
                onChange={(e) => onSetEditingContent(e.target.value)}
                autoFocus
              />
              <div className="chat-bubble-edit-actions">
                <button
                  type="button"
                  className="chat-bubble-edit-btn cancel"
                  onClick={onCancelEdit}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="chat-bubble-edit-btn save"
                  onClick={() => void onSaveEdit(index, editingContent)}
                >
                  Save
                </button>
              </div>
            </div>
          ) : (
            <>
              <div
                ref={bubbleContentRef}
                className={`chat-bubble-content-wrapper ${
                  message.role === "user" && isOverflowing && isCollapsed ? "collapsed" : ""
                }`}
              >
                <ExistingNodesContext.Provider value={existingNodeIds}>
                  {markdownBody}
                </ExistingNodesContext.Provider>
              </div>
              {message.role === "user" && isOverflowing && (
                <button
                  type="button"
                  className="chat-show-more-btn"
                  onClick={() => setIsCollapsed(!isCollapsed)}
                >
                  {isCollapsed ? "Show more" : "Show less"}
                </button>
              )}
            </>
          )}
          {message.isStreaming && <span className="streaming-cursor" />}
        </div>
        {message.role === "assistant" && !message.isStreaming && (
          <div className="chat-bubble-actions">
            <button
              type="button"
              className="chat-action-btn"
              onClick={() => onCopyMessage(message.content, message.id)}
              title="Copy message"
            >
              {isCopied ? (
                <svg
                  width="15"
                  height="15"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <polyline points="20 6 9 17 4 12"></polyline>
                </svg>
              ) : (
                <svg
                  width="15"
                  height="15"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
                </svg>
              )}
            </button>
            <button
              type="button"
              className="chat-action-btn"
              onClick={() => onRetryMessage(index)}
              title="Retry response"
            >
              <svg
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <polyline points="23 4 23 10 17 10"></polyline>
                <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"></path>
              </svg>
            </button>
          </div>
        )}
        {message.role === "user" && !isEditing && (
          <div className="chat-bubble-actions">
            <button
              type="button"
              className="chat-action-btn"
              onClick={() => onCopyMessage(message.content, message.id)}
              title="Copy message"
            >
              {isCopied ? (
                <svg
                  width="15"
                  height="15"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <polyline points="20 6 9 17 4 12"></polyline>
                </svg>
              ) : (
                <svg
                  width="15"
                  height="15"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
                </svg>
              )}
            </button>
            <button
              type="button"
              className="chat-action-btn"
              onClick={() => onStartEdit(message.id, message.content)}
              title="Edit message"
            >
              <svg
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M12 20h9"></path>
                <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"></path>
              </svg>
            </button>
            <button
              type="button"
              className="chat-action-btn"
              onClick={() => onRetryMessage(index)}
              title="Retry response"
            >
              <svg
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <polyline points="23 4 23 10 17 10"></polyline>
                <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"></path>
              </svg>
            </button>
          </div>
        )}
      </div>
    </article>
  );
});

async function resolveLlmConfig(): Promise<{
  provider: string;
  endpoint: string;
  model: string;
}> {
  const provider = getLlmProvider();
  let endpoint = "";
  if (provider === "lmstudio") {
    endpoint = getLmStudioEndpoint();
  } else if (provider === "ollama") {
    endpoint = getOllamaEndpoint();
  } else if (["openai", "anthropic", "google", "xai"].includes(provider)) {
    endpoint = await getApiKey(provider);
  }
  const model = getLlmModel();
  return { provider, endpoint, model };
}

type AttachedDoc = {
  id: string;
  filePath: string;
  sourceName: string;
  pageCount: number;
  text: string;
  ocrConfidence: number | null;
  needsOcrModels: boolean;
  promptInjectionFlagged: boolean;
  pageTokenEstimates: number[];
  isOverridden: boolean;
  includedPageCount: number;
  isTruncated: boolean;
};

type ChatPanelProps = {
  selectedNodeIds: string[];
  scope: ContextAssemblerScope;
  selectedVaultId: string | null;
  onSelectVault: (vaultId: string | null) => void;
  onOpenSettings?: () => void;
  onModalToggle?: (isOpen: boolean) => void;
  onSelectNode?: (nodeId: string) => void;
  onRefreshPendingCount?: () => void;
  isRedactedUnlocked: boolean;
  nodeRefreshKey?: number;
  visible?: boolean;
  activeSessionId?: string | null;
  onActivateSession?: (sessionId: string) => void;
};

const computeAttachedDocsBudget = (docs: AttachedDoc[]): AttachedDoc[] => {
  if (docs.length === 0) return [];

  const maxTokens = 6000;
  const processed = docs.map((d) => ({
    ...d,
    includedPageCount: d.pageCount,
    isTruncated: false,
  }));

  let remainingBudget = maxTokens;
  const activeIndices = new Set(processed.keys());

  const includedPages = processed.map((d) => d.pageCount);

  let iterations = 0;
  while (activeIndices.size > 0 && iterations < 10) {
    iterations++;
    const share = Math.floor(remainingBudget / activeIndices.size);
    let budgetReleased = false;

    for (const idx of Array.from(activeIndices)) {
      const fullDocTokens = processed[idx].pageTokenEstimates.reduce((a, b) => a + b, 0);
      if (fullDocTokens <= share) {
        remainingBudget -= fullDocTokens;
        includedPages[idx] = processed[idx].pageCount;
        activeIndices.delete(idx);
        budgetReleased = true;
      }
    }

    if (!budgetReleased) {
      for (const idx of Array.from(activeIndices)) {
        let acc = 0;
        let pCount = 0;
        for (const pageTokens of processed[idx].pageTokenEstimates) {
          if (acc + pageTokens <= share) {
            acc += pageTokens;
            pCount++;
          } else {
            break;
          }
        }
        if (pCount === 0 && processed[idx].pageCount > 0) {
          pCount = 1;
        }
        includedPages[idx] = pCount;
        processed[idx].isTruncated = pCount < processed[idx].pageCount;
      }
      break;
    }
  }

  return processed.map((d, idx) => ({
    ...d,
    includedPageCount: includedPages[idx],
    isTruncated: includedPages[idx] < d.pageCount,
  }));
};

function ChatPanel({
  selectedNodeIds,
  scope,
  selectedVaultId,
  onSelectVault,
  onOpenSettings,
  onModalToggle,
  onSelectNode,
  onRefreshPendingCount,
  isRedactedUnlocked,
  nodeRefreshKey,
  visible = true,
  activeSessionId,
  onActivateSession,
}: ChatPanelProps) {
  const [isOffTheRecord, setIsOffTheRecord] = useState(false);
  const [attachedDocs, setAttachedDocs] = useState<AttachedDoc[]>([]);
  const [isAttaching, setIsAttaching] = useState(false);
  const [isDownloadingOcr, setIsDownloadingOcr] = useState(false);

  const attachedDocsRef = useRef<AttachedDoc[]>([]);
  useEffect(() => {
    attachedDocsRef.current = attachedDocs;
  }, [attachedDocs]);
  const MAX_RENDERED_MESSAGES = 60;
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [status, setStatus] = useState("");
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [editingMessageId, setEditingMessageId] = useState<string | null>(null);
  const [editingContent, setEditingContent] = useState("");
  const [existingNodeIds, setExistingNodeIds] = useState<Set<string> | null>(null);
  const sessionId = isOffTheRecord ? "temporary-session" : activeSessionId || "default-session";

  const handleAttachPdf = async () => {
    setIsAttaching(true);
    setStatus("");
    try {
      const filePath = await browseImportPdf();
      if (!filePath) {
        setIsAttaching(false);
        return;
      }

      if (attachedDocs.some((d) => d.filePath === filePath)) {
        setStatus("Document is already attached.");
        setIsAttaching(false);
        return;
      }

      setStatus("Extracting document content...");
      const res = await chatExtractPdfText(filePath);
      if ("err" in res) {
        setStatus(`Extraction failed: ${res.err}`);
        setIsAttaching(false);
        return;
      }

      const doc = res.ok;
      const newDoc: AttachedDoc = {
        id: crypto.randomUUID(),
        filePath,
        sourceName: doc.sourceName,
        pageCount: doc.pageCount,
        text: doc.text,
        ocrConfidence: doc.ocrConfidence,
        needsOcrModels: doc.needsOcrModels,
        promptInjectionFlagged: doc.promptInjectionFlagged,
        pageTokenEstimates: doc.pageTokenEstimates,
        isOverridden: false,
        includedPageCount: doc.pageCount,
        isTruncated: false,
      };

      setAttachedDocs((prev) => [...prev, newDoc]);
      setStatus("");
    } catch (err) {
      console.error(err);
      setStatus("Failed to attach PDF.");
    } finally {
      setIsAttaching(false);
    }
  };

  const handleRemoveAttachment = (id: string) => {
    setAttachedDocs((prev) => prev.filter((d) => d.id !== id));
  };

  const handleOverrideAttachment = (id: string) => {
    setAttachedDocs((prev) => prev.map((d) => (d.id === id ? { ...d, isOverridden: true } : d)));
  };

  const handleDownloadOcr = async () => {
    setIsDownloadingOcr(true);
    setStatus("Downloading OCR models...");
    try {
      await startOcrModelDownload();
      setStatus("OCR models downloaded. Re-processing attachments...");
      const updated = [];
      for (const doc of attachedDocs) {
        if (doc.needsOcrModels) {
          const res = await chatExtractPdfText(doc.filePath);
          if ("ok" in res) {
            const fresh = res.ok;
            updated.push({
              ...doc,
              text: fresh.text,
              ocrConfidence: fresh.ocrConfidence,
              needsOcrModels: fresh.needsOcrModels,
              promptInjectionFlagged: fresh.promptInjectionFlagged,
              pageTokenEstimates: fresh.pageTokenEstimates,
            });
          } else {
            updated.push(doc);
          }
        } else {
          updated.push(doc);
        }
      }
      setAttachedDocs(updated);
      setStatus("");
    } catch (err) {
      console.error(err);
      setStatus("Failed to download OCR models.");
    } finally {
      setIsDownloadingOcr(false);
    }
  };

  const handleToggleOtr = useCallback(async () => {
    const next = !isOffTheRecord;
    const nextSessionId = next ? "temporary-session" : activeSessionId || "default-session";
    setIsOffTheRecord(next);
    try {
      await chatSetOffTheRecord(next);
      if (sessionId === "temporary-session") {
        await clearChatHistory(sessionId);
      }
      const history = await getChatHistory(nextSessionId);
      setMessages(history);
      setInput("");
      setStatus("");
      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
    } catch (err) {
      console.error("Failed to toggle off-the-record:", err);
      setIsOffTheRecord(!next); // rollback frontend
      void chatSetOffTheRecord(!next).catch(console.error); // rollback backend
    }
  }, [
    isOffTheRecord,
    sessionId,
    activeSessionId,
    setIsOffTheRecord,
    setMessages,
    setInput,
    setStatus,
  ]);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    chatIsOffTheRecord()
      .then((ok) => setIsOffTheRecord(ok))
      .catch((err) => console.error("Failed to fetch off-the-record state:", err));
    getAllNodes(isRedactedUnlocked)
      .then((nodes) => {
        if (active) {
          setExistingNodeIds(new Set(nodes.map((n) => n.id)));
        }
      })
      .catch((err) => {
        console.error("Failed to fetch nodes in ChatPanel:", err);
      });
    return () => {
      active = false;
    };
  }, [nodeRefreshKey, isRedactedUnlocked, visible]);

  const [userName, setUserName] = useState("Lisa");
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [agentMode, setAgentMode] = useState<"Recall/Chat" | "Ingest/Memory" | "Onboarding">(
    "Recall/Chat"
  );
  const [currentProvider, setCurrentProvider] = useState(() => getLlmProvider());
  const [currentModel, setCurrentModel] = useState(() => getLlmModel());
  const [currentMode, setCurrentMode] = useState(() => getLlmMode());
  const chartsEnabled = useUIStore((state) => state.chat.chartsEnabled);
  const setChatChartsEnabled = useUIStore((state) => state.setChatChartsEnabled);
  const [showChartsConfirmModal, setShowChartsConfirmModal] = useState(false);

  const [isExtracting, setIsExtracting] = useState(false);

  useEffect(() => {
    onModalToggle?.(showChartsConfirmModal);
  }, [showChartsConfirmModal, onModalToggle]);
  const [activeDropdown, setActiveDropdown] = useState<"vault" | "mode" | "model" | null>(null);

  const handleToggleCharts = useCallback(() => {
    if (chartsEnabled) {
      setChatChartsEnabled(false);
    } else {
      setShowChartsConfirmModal(true);
    }
  }, [chartsEnabled, setChatChartsEnabled]);

  const handleForceExtract = useCallback(async () => {
    if (isExtracting || isSending) return;
    setIsExtracting(true);
    try {
      const { provider, endpoint, model } = await resolveLlmConfig();
      await extractMemoryForce(provider, endpoint, model);
      onRefreshPendingCount?.();
    } catch (err) {
      console.error("Manual extraction failed:", err);
      setStatus(String(err));
    } finally {
      setIsExtracting(false);
    }
  }, [isExtracting, isSending, onRefreshPendingCount, setIsExtracting, setStatus]);

  const [isConverting, setIsConverting] = useState(false);

  const handleConvertToMemory = useCallback(async () => {
    if (isConverting || isSending) return;
    setIsConverting(true);
    try {
      const { provider, endpoint, model } = await resolveLlmConfig();
      const result = await chatConvertTemporaryToMemory(provider, endpoint, model, activeSessionId);
      if ("err" in result) {
        setStatus(`Conversion failed: ${result.err}`);
        return;
      }
      const { sessionId: savedSessionId, changeset, extractionError } = result.ok;
      onActivateSession?.(savedSessionId);
      setIsOffTheRecord(false);
      setInput("");
      setStatus(
        extractionError
          ? `Conversation saved, but memory extraction failed: ${extractionError}`
          : ""
      );
      if (changeset) {
        onRefreshPendingCount?.();
      }
      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
    } catch (err) {
      console.error("Convert to memory failed:", err);
      setStatus(String(err));
    } finally {
      setIsConverting(false);
    }
  }, [
    isConverting,
    isSending,
    activeSessionId,
    onActivateSession,
    onRefreshPendingCount,
    setIsConverting,
    setIsOffTheRecord,
    setInput,
    setStatus,
  ]);

  const threadEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const editInputRef = useRef<HTMLTextAreaElement>(null);
  const messagesRef = useRef(messages);
  const hiddenMessageCountRef = useRef(0);
  const isSendingRef = useRef(isSending);
  const executeLlmResponseRef = useRef<(prompt: string) => Promise<void>>(async () => {});

  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.style.height = "auto";
      const scrollHeight = inputRef.current.scrollHeight;
      const maxHeight = messages.length === 0 ? 250 : 160;
      if (scrollHeight > maxHeight) {
        inputRef.current.style.height = `${maxHeight}px`;
        inputRef.current.style.overflowY = "auto";
      } else {
        inputRef.current.style.height = `${scrollHeight}px`;
        inputRef.current.style.overflowY = "hidden";
      }
    }
  }, [input, messages.length]);

  useEffect(() => {
    if (editInputRef.current) {
      editInputRef.current.style.height = "auto";
      const scrollHeight = editInputRef.current.scrollHeight;
      const maxHeight = 180;
      if (scrollHeight > maxHeight) {
        editInputRef.current.style.height = `${maxHeight}px`;
        editInputRef.current.style.overflowY = "auto";
      } else {
        editInputRef.current.style.height = `${scrollHeight}px`;
        editInputRef.current.style.overflowY = "hidden";
      }
    }
  }, [editingContent, editingMessageId]);

  useEffect(() => {
    threadEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, isSending]);

  const timeOfDay = useMemo(() => {
    const hour = new Date().getHours();
    if (hour < 12) return "morning";
    if (hour < 18) return "afternoon";
    return "evening";
  }, []);

  const selectedVaultName = useMemo(() => {
    if (!selectedVaultId) return "Root Graph";
    const match = vaults.find((v) => v.id === selectedVaultId);
    return match ? match.name : "Root Graph";
  }, [selectedVaultId, vaults]);

  const activeModelDisplay = useMemo(() => {
    const isCloud = ["openai", "anthropic", "google", "xai"].includes(currentProvider);
    if (isCloud) {
      let niceName = currentProvider.charAt(0).toUpperCase() + currentProvider.slice(1);
      if (currentProvider === "openai") niceName = "OpenAI";
      if (currentProvider === "anthropic") niceName = "Anthropic";
      if (currentProvider === "google") niceName = "Google Gemini";
      if (currentProvider === "xai") niceName = "xAI Grok";
      return `Cloud: ${currentModel || "Model"} (${niceName})`;
    }
    const niceName = currentProvider === "ollama" ? "Ollama" : "LM Studio";
    return `Local: ${currentModel || "Model"} (${niceName})`;
  }, [currentProvider, currentModel]);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const name = await getSetting("displayName");
        if (active && name && name.trim()) {
          setUserName(name.trim());
        }
      } catch (e) {
        console.error("Failed to load user name:", e);
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    function handleSettingsChange() {
      setCurrentProvider(getLlmProvider());
      setCurrentModel(getLlmModel());
      setCurrentMode(getLlmMode());
    }
    window.addEventListener("mindvault:llm-settings-changed", handleSettingsChange);
    return () => {
      window.removeEventListener("mindvault:llm-settings-changed", handleSettingsChange);
    };
  }, []);

  useEffect(() => {
    let active = true;
    const handleChatExternalUpdated = async () => {
      try {
        const offTheRecord = await chatIsOffTheRecord();
        if (!active) return;
        setIsOffTheRecord(offTheRecord);
      } catch (err) {
        console.error("Failed to fetch off-the-record state on update:", err);
      }
    };
    window.addEventListener("mindvault:chat-external-updated", handleChatExternalUpdated);
    return () => {
      active = false;
      window.removeEventListener("mindvault:chat-external-updated", handleChatExternalUpdated);
    };
  }, []);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const data = await listVaults();
        if (active) {
          setVaults(data);
        }
      } catch (e) {
        console.error("Failed to load vaults:", e);
      }
    })();
    return () => {
      active = false;
    };
  }, [selectedVaultId]);

  useEffect(() => {
    if (!activeDropdown) return;
    function handleGlobalClick() {
      setActiveDropdown(null);
    }
    const timer = setTimeout(() => {
      window.addEventListener("click", handleGlobalClick);
    }, 0);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("click", handleGlobalClick);
    };
  }, [activeDropdown]);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const history = await getChatHistory(sessionId);
        if (!active) return;
        setMessages(history);
        setStatus("");
      } catch (error) {
        if (!active) return;
        setStatus(String(error));
      }
    })();
    return () => {
      active = false;
    };
  }, [sessionId]);

  const canSend = useMemo(() => input.trim().length > 0 && !isSending, [input, isSending]);
  const visibleMessages = useMemo(() => {
    if (messages.length <= MAX_RENDERED_MESSAGES) {
      return messages;
    }
    return messages.slice(-MAX_RENDERED_MESSAGES);
  }, [messages]);
  const hiddenMessageCount = Math.max(0, messages.length - visibleMessages.length);

  useEffect(() => {
    messagesRef.current = messages;
    hiddenMessageCountRef.current = hiddenMessageCount;
    isSendingRef.current = isSending;
  });

  const executeLlmResponse = useCallback(
    async (promptText: string) => {
      setStatus("");
      setIsSending(true);

      try {
        const { provider, endpoint, model } = await resolveLlmConfig();

        let executionPrompt = promptText;
        if (agentMode === "Ingest/Memory") {
          executionPrompt = `[Agent Mode: Ingest/Memory] Please extract, deduplicate, and store the following input as a new Node in the memory system. Do not generate a long conversational response, just confirm storage details or output a brief success summary:\n\n${promptText}`;
        } else if (agentMode === "Onboarding") {
          executionPrompt = `[Agent Mode: Onboarding] Act as the Onboarding Agent. Conduct an interview and ask clarifying questions to help build initial context for the user:\n\n${promptText}`;
        }

        const processed = computeAttachedDocsBudget(attachedDocsRef.current);
        const safeDocs = processed.filter((d) => !d.promptInjectionFlagged || d.isOverridden);
        let attachedText: string | null = null;
        if (safeDocs.length > 0) {
          const docTexts = safeDocs.map((d) => {
            const pages = d.text.split("\n\n--- PAGE_BREAK ---\n\n");
            return pages.slice(0, d.includedPageCount).join("\n\n");
          });
          attachedText = docTexts.join("\n\n");
        }

        const aiResponse = await chatWithScope(
          selectedNodeIds,
          scope,
          provider,
          endpoint,
          model,
          executionPrompt,
          chartsEnabled,
          isRedactedUnlocked,
          sessionId,
          attachedText
        );

        const aiMsgId = crypto.randomUUID();
        const aiMsg: ChatMessage = {
          id: aiMsgId,
          role: "assistant",
          content: aiResponse,
          created_at: new Date().toISOString(),
        };

        setMessages((prev) => [...prev, aiMsg]);
        setAttachedDocs([]); // clear attachments on successful send
        await chatAppendMessage(aiMsgId, "assistant", aiResponse, sessionId);

        // Fire-and-forget background extraction check (non-blocking for the user)
        extractMemoryIfReady(provider, endpoint, model, sessionId)
          .then((changeset) => {
            if (changeset && onRefreshPendingCount) {
              onRefreshPendingCount();
            }
          })
          .catch((err) => {
            console.error("Background memory extraction check failed:", err);
          });
      } catch (error) {
        setStatus(String(error));
      } finally {
        setIsSending(false);
      }
    },
    [
      selectedNodeIds,
      scope,
      chartsEnabled,
      isRedactedUnlocked,
      agentMode,
      onRefreshPendingCount,
      sessionId,
      setStatus,
      setIsSending,
      setMessages,
    ]
  );

  useEffect(() => {
    executeLlmResponseRef.current = executeLlmResponse;
  }, [executeLlmResponse]);

  const budgetedDocs = useMemo(() => computeAttachedDocsBudget(attachedDocs), [attachedDocs]);
  const anyNeedsOcr = budgetedDocs.some((d) => d.needsOcrModels);

  const attachmentChipsJsx =
    budgetedDocs.length > 0 ? (
      <div className="chat-attachment-chips">
        {budgetedDocs.map((doc) => (
          <div
            key={doc.id}
            className={`chat-attachment-chip${doc.promptInjectionFlagged && !doc.isOverridden ? " flagged" : ""}`}
          >
            <span className="chip-icon">
              <FileIcon size={14} />
            </span>
            <span className="chip-name">{doc.sourceName}</span>
            <span className="chip-pages">
              {doc.isTruncated
                ? `pp. 1–${doc.includedPageCount} of ${doc.pageCount}`
                : `${doc.pageCount} pg`}
            </span>
            {doc.isTruncated && (
              <span
                className="chip-truncated"
                title="Document was truncated to fit the token budget"
              >
                <AlertIcon size={12} /> Truncated
              </span>
            )}
            {doc.needsOcrModels && (
              <span className="chip-ocr-warn" title="OCR models needed for full extraction">
                <AlertIcon size={12} /> OCR
              </span>
            )}
            {doc.promptInjectionFlagged && !doc.isOverridden && (
              <span className="chip-injection-warn">
                <AlertIcon size={12} /> Hidden instructions detected
                <button
                  type="button"
                  className="chip-override-btn"
                  onClick={() => handleOverrideAttachment(doc.id)}
                  title="Keep this attachment despite the warning"
                >
                  Keep
                </button>
              </span>
            )}
            <button
              type="button"
              className="chip-remove-btn"
              onClick={() => handleRemoveAttachment(doc.id)}
              aria-label={`Remove ${doc.sourceName}`}
            >
              <CloseIcon size={14} />
            </button>
          </div>
        ))}
        {anyNeedsOcr && (
          <button
            type="button"
            className="chip-download-ocr-btn"
            onClick={() => void handleDownloadOcr()}
            disabled={isDownloadingOcr}
          >
            {isDownloadingOcr ? "Downloading OCR..." : "Download OCR Models"}
          </button>
        )}
      </div>
    ) : null;

  async function executeSendMessage(promptText: string) {
    if (isSending || !promptText.trim()) return;

    const userMsgId = crypto.randomUUID();
    const userMsg: ChatMessage = {
      id: userMsgId,
      role: "user",
      content: promptText,
      created_at: new Date().toISOString(),
    };

    const isFirstMessage = messages.length === 0;

    setMessages((prev) => [...prev, userMsg]);

    try {
      await chatAppendMessage(userMsgId, "user", promptText, sessionId);
      if (isFirstMessage && sessionId !== "temporary-session") {
        const summary = promptText.length > 40 ? promptText.substring(0, 40) + "..." : promptText;
        await chatUpdateSessionSummary(sessionId, summary);
      }
      window.dispatchEvent(new CustomEvent("mindvault:chat-external-updated"));
      await executeLlmResponse(promptText);
    } catch (error) {
      setStatus(String(error));
    }
  }

  async function handleSend() {
    if (!canSend) return;
    const promptText = input.trim();
    setInput("");
    await executeSendMessage(promptText);
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void handleSend();
    }
  }

  const handleCopyMessage = useCallback(async (content: string, id: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedMessageId(id);
      setTimeout(() => setCopiedMessageId(null), 2000);
    } catch (err) {
      console.error("Failed to copy", err);
    }
  }, []);

  const handleStartEdit = useCallback((messageId: string, content: string) => {
    setEditingMessageId(messageId);
    setEditingContent(content);
  }, []);

  const handleCancelEdit = useCallback(() => {
    setEditingMessageId(null);
    setEditingContent("");
  }, []);

  const handleSaveEdit = useCallback(
    async (visibleIndex: number, newContent: string) => {
      if (isSendingRef.current || !newContent.trim()) return;

      // The UI passes an index relative to visibleMessages (a tail slice of messages).
      // Offset by hiddenMessageCount to get the correct index into the full messages array.
      const index = visibleIndex + hiddenMessageCountRef.current;
      const msgs = messagesRef.current;
      const userMsg = msgs[index];
      if (userMsg.content === newContent) {
        setEditingMessageId(null);
        return;
      }

      const deleteIds = msgs.slice(index + 1).map((m) => m.id);

      try {
        await chatEditAndTruncate(userMsg.id, newContent, deleteIds, sessionId);
        setMessages((prev) => {
          const updated = [...prev.slice(0, index)];
          updated.push({
            ...userMsg,
            content: newContent,
          });
          return updated;
        });
        setEditingMessageId(null);
        await executeLlmResponseRef.current(newContent);
      } catch (error) {
        setStatus(String(error));
      }
    },

    [setStatus, setMessages, setEditingMessageId, sessionId]
  );

  const handleRetryMessage = useCallback(
    async (visibleIndex: number) => {
      if (isSendingRef.current) return;

      // The UI passes an index relative to visibleMessages (a tail slice of messages).
      // Offset by hiddenMessageCount to get the correct index into the full messages array.
      const msgs = messagesRef.current;
      const index = visibleIndex + hiddenMessageCountRef.current;

      // Find the user message index
      let userIndex = -1;
      if (msgs[index].role === "user") {
        userIndex = index;
      } else {
        // Find the last user message before this assistant message
        for (let i = index - 1; i >= 0; i--) {
          if (msgs[i].role === "user") {
            userIndex = i;
            break;
          }
        }
      }

      if (userIndex !== -1) {
        const userMsg = msgs[userIndex];
        const deleteIds = msgs.slice(userIndex + 1).map((m) => m.id);

        try {
          await chatEditAndTruncate(userMsg.id, userMsg.content, deleteIds, sessionId);
          setMessages((prev) => prev.slice(0, userIndex + 1));
          await executeLlmResponseRef.current(userMsg.content);
        } catch (error) {
          setStatus(String(error));
        }
      }
    },

    [setStatus, setMessages, sessionId]
  );

  function toggleDropdown(type: "vault" | "mode" | "model") {
    setActiveDropdown((prev) => (prev === type ? null : type));
  }

  function handleSelectVault(vaultId: string | null) {
    onSelectVault(vaultId);
    setActiveDropdown(null);
  }

  function handleSelectMode(mode: "Recall/Chat" | "Ingest/Memory" | "Onboarding") {
    setAgentMode(mode);
    setActiveDropdown(null);
  }

  function handleSelectModeSettings(mode: "local" | "cloud" | "hybrid") {
    setLlmMode(mode);
    setCurrentMode(mode);
    setActiveDropdown(null);
    if (onOpenSettings) {
      onOpenSettings();
    }
  }

  if (messages.length === 0) {
    return (
      <section className="chat-panel zen-dashboard">
        <div className="zen-container">
          <h1 className="zen-greeting">
            Good {timeOfDay}, <span className="zen-username">{userName}</span>
          </h1>
          {attachmentChipsJsx}
          <div className="zen-search-wrapper">
            <button
              type="button"
              className="chat-attach-btn"
              onClick={() => void handleAttachPdf()}
              disabled={isAttaching || isSending}
              title="Attach a PDF document"
              aria-label="Attach PDF"
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
              </svg>
            </button>
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleInputKeyDown}
              placeholder="Ask Amber..."
              className="zen-search-input"
              autoFocus
              disabled={isSending}
              rows={1}
            />
            <button
              type="button"
              className="zen-search-submit"
              onClick={() => void handleSend()}
              disabled={!input.trim() || isSending}
              aria-label="Send query"
            >
              <ArrowUpIcon size={18} />
            </button>
          </div>

          <div className="zen-pills-row">
            {/* Pill 1: Active Vault */}
            <div className="zen-pill-container" onClick={(e) => e.stopPropagation()}>
              <button
                type="button"
                className={`zen-pill ${activeDropdown === "vault" ? "active" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  toggleDropdown("vault");
                }}
              >
                <span className="zen-pill-icon">
                  <FolderIcon size={16} />
                </span>
                <span className="zen-pill-label">Vault:</span>
                <span className="zen-pill-value">{selectedVaultName}</span>
                <span className="zen-pill-chevron">
                  <ChevronDownIcon size={14} />
                </span>
              </button>
              {activeDropdown === "vault" && (
                <div className="zen-dropdown">
                  <div className="zen-dropdown-header">Select Scoped Memory Domain</div>
                  <button
                    type="button"
                    className={`zen-dropdown-item ${!selectedVaultId ? "selected" : ""}`}
                    onClick={() => handleSelectVault(null)}
                  >
                    <span className="item-icon">
                      <GlobeIcon size={16} />
                    </span>
                    <span className="item-text">Root Graph (Global Context)</span>
                  </button>
                  {vaults.map((v) => (
                    <button
                      key={v.id}
                      type="button"
                      className={`zen-dropdown-item ${selectedVaultId === v.id ? "selected" : ""}`}
                      onClick={() => handleSelectVault(v.id)}
                    >
                      <span className="item-icon">
                        <FolderIcon size={16} />
                      </span>
                      <span className="item-text">{v.name}</span>
                      <span className="item-badge">{v.privacyTier.replace("_", " ")}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>

            {/* Pill 2: Agent Mode */}
            <div className="zen-pill-container" onClick={(e) => e.stopPropagation()}>
              <button
                type="button"
                className={`zen-pill ${activeDropdown === "mode" ? "active" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  toggleDropdown("mode");
                }}
              >
                <span className="zen-pill-icon">
                  <SparklesIcon size={16} />
                </span>
                <span className="zen-pill-label">Mode:</span>
                <span className="zen-pill-value">{agentMode}</span>
                <span className="zen-pill-chevron">
                  <ChevronDownIcon size={14} />
                </span>
              </button>
              {activeDropdown === "mode" && (
                <div className="zen-dropdown animate-fade-in">
                  <div className="zen-dropdown-header">Select Intent / Execution State</div>
                  <button
                    type="button"
                    className={`zen-dropdown-item ${agentMode === "Recall/Chat" ? "selected" : ""}`}
                    onClick={() => handleSelectMode("Recall/Chat")}
                  >
                    <span className="item-icon">
                      <MessageIcon size={16} />
                    </span>
                    <div className="item-details">
                      <div className="item-title">Recall/Chat</div>
                      <div className="item-desc">
                        MACRL routing to fetch relevant Nodes & converse
                      </div>
                    </div>
                  </button>
                  <button
                    type="button"
                    className={`zen-dropdown-item ${agentMode === "Ingest/Memory" ? "selected" : ""}`}
                    onClick={() => handleSelectMode("Ingest/Memory")}
                  >
                    <span className="item-icon">
                      <InboxIcon size={16} />
                    </span>
                    <div className="item-details">
                      <div className="item-title">Ingest/Memory</div>
                      <div className="item-desc">
                        Extract, deduplicate, and store input as new Node
                      </div>
                    </div>
                  </button>
                  <button
                    type="button"
                    className={`zen-dropdown-item ${agentMode === "Onboarding" ? "selected" : ""}`}
                    onClick={() => handleSelectMode("Onboarding")}
                  >
                    <span className="item-icon">
                      <GraduationIcon size={16} />
                    </span>
                    <div className="item-details">
                      <div className="item-title">Onboarding</div>
                      <div className="item-desc">
                        Trigger Onboarding Agent interview for context
                      </div>
                    </div>
                  </button>
                </div>
              )}
            </div>

            {/* Pill 3: Model */}
            <div className="zen-pill-container" onClick={(e) => e.stopPropagation()}>
              <button
                type="button"
                className={`zen-pill ${activeDropdown === "model" ? "active" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  toggleDropdown("model");
                }}
              >
                <span className="zen-pill-icon">
                  <GearIcon size={16} />
                </span>
                <span className="zen-pill-label">Model:</span>
                <span className="zen-pill-value">{activeModelDisplay}</span>
                <span className="zen-pill-chevron">
                  <ChevronDownIcon size={14} />
                </span>
              </button>
              {activeDropdown === "model" && (
                <div className="zen-dropdown models-dropdown">
                  <div className="zen-dropdown-header">Select Model Environment</div>

                  <button
                    type="button"
                    className={`zen-dropdown-item ${currentMode === "local" ? "selected" : ""}`}
                    onClick={() => handleSelectModeSettings("local")}
                  >
                    <span className="item-icon">
                      <ComputerIcon size={16} />
                    </span>
                    <div className="item-details">
                      <div className="item-title">Local Models</div>
                      <div className="item-desc">
                        Secure, offline models via Ollama or LM Studio
                      </div>
                    </div>
                  </button>

                  <button
                    type="button"
                    className={`zen-dropdown-item ${currentMode === "cloud" ? "selected" : ""}`}
                    onClick={() => handleSelectModeSettings("cloud")}
                  >
                    <span className="item-icon">
                      <CloudIcon size={16} />
                    </span>
                    <div className="item-details">
                      <div className="item-title">Cloud Models</div>
                      <div className="item-desc">
                        High-performance AI via OpenAI, Anthropic, Google, xAI
                      </div>
                    </div>
                  </button>

                  <button
                    type="button"
                    className={`zen-dropdown-item ${currentMode === "hybrid" ? "selected" : ""}`}
                    onClick={() => handleSelectModeSettings("hybrid")}
                  >
                    <span className="item-icon">
                      <ZapIcon size={16} />
                    </span>
                    <div className="item-details">
                      <div className="item-title">Hybrid Mode</div>
                      <div className="item-desc">
                        Run both Cloud and Local models simultaneously
                      </div>
                    </div>
                  </button>
                </div>
              )}
            </div>

            {/* Pill 4: Extract Memory */}
            <div className="zen-pill-container">
              <button
                type="button"
                className={`zen-pill extract-pill ${isExtracting ? "active" : ""}`}
                onClick={() => void handleForceExtract()}
                disabled={isExtracting || isSending}
              >
                <span className="zen-pill-icon">
                  {isExtracting ? <HourglassIcon size={16} /> : <BrainIcon size={16} />}
                </span>
                <span className="zen-pill-label">{isExtracting ? "Extracting..." : "Extract"}</span>
              </button>
            </div>

            {/* Pill 5: Toggle Off the Record */}
            <div className="zen-pill-container">
              <button
                className={`otr-toggle-btn ${isOffTheRecord ? "active" : ""}`}
                onClick={handleToggleOtr}
                title={
                  isOffTheRecord
                    ? "Disable Off the Record (enable memory)"
                    : "Enable Off the Record (private brainstorm)"
                }
              >
                <span className="otr-banner-icon">
                  {isOffTheRecord ? <EyeOffIcon size={16} /> : <BrainIcon size={16} />}
                </span>
                {isOffTheRecord ? "Off the Record" : "Mind Sync Active"}
              </button>
            </div>

            {/* Pill 6: Interactive Charts */}
            <div className="zen-pill-container">
              <button
                type="button"
                className={`zen-pill charts-pill ${chartsEnabled ? "active" : ""}`}
                onClick={handleToggleCharts}
              >
                <span className="zen-pill-icon">
                  {chartsEnabled ? <BarChartIcon size={16} /> : <TrendingUpIcon size={16} />}
                </span>
                <span className="zen-pill-label">Charts:</span>
                <span className="zen-pill-value">{chartsEnabled ? "ON" : "OFF"}</span>
              </button>
            </div>
          </div>
          {status && <p className="chat-status">{status}</p>}
        </div>
        {showChartsConfirmModal && (
          <div className="charts-modal-overlay" onClick={() => setShowChartsConfirmModal(false)}>
            <div className="charts-modal-card" onClick={(e) => e.stopPropagation()}>
              <div className="charts-modal-header">
                <span className="charts-modal-icon">
                  <AlertIcon size={20} />
                </span>
                <h3 className="charts-modal-title">Enable Experimental Charts?</h3>
              </div>
              <div className="charts-modal-body">
                <p className="charts-modal-text warning-lead">
                  Chart generation is highly experimental and does not perform well with
                  smaller/local LLMs (e.g. Ollama 7B/8B), which frequently output invalid JSON
                  schemas.
                </p>
                <p className="charts-modal-text recommendation-note">
                  For reliable, high-quality interactive plots and mathematical visualizations, we
                  strongly recommend choosing advanced cloud models such as{" "}
                  <strong>Claude 4.6 Sonnet</strong> or <strong>GPT-5.4</strong> or larger local
                  models.
                </p>
                <p className="charts-modal-question">Would you like to enable charts anyway?</p>
              </div>
              <div className="charts-modal-actions">
                <button
                  type="button"
                  className="charts-modal-btn cancel-btn"
                  onClick={() => setShowChartsConfirmModal(false)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="charts-modal-btn confirm-btn"
                  onClick={() => {
                    setChatChartsEnabled(true);
                    setShowChartsConfirmModal(false);
                  }}
                >
                  Enable Charts
                </button>
              </div>
            </div>
          </div>
        )}
      </section>
    );
  }

  return (
    <section className="chat-panel chat-active">
      <div className="chat-thread">
        {hiddenMessageCount > 0 ? (
          <p className="chat-history-trim-note">
            Showing latest {visibleMessages.length} messages ({hiddenMessageCount} older hidden for
            speed).
          </p>
        ) : null}
        <div className="chat-messages-container">
          {visibleMessages.map((message, index) => (
            <ChatMessageBubble
              key={message.id}
              message={message}
              index={index}
              isEditing={editingMessageId === message.id}
              editingContent={editingContent}
              isCopied={copiedMessageId === message.id}
              editInputRef={editInputRef}
              onSetEditingContent={setEditingContent}
              onCancelEdit={handleCancelEdit}
              onSaveEdit={handleSaveEdit}
              onCopyMessage={handleCopyMessage}
              onRetryMessage={handleRetryMessage}
              onStartEdit={handleStartEdit}
              chartsEnabled={chartsEnabled}
              onSelectNode={onSelectNode}
              existingNodeIds={existingNodeIds}
              isRedactedUnlocked={isRedactedUnlocked}
            />
          ))}
          {isSending && (
            <article className="chat-message chat-message-assistant loading-message">
              <div className="chat-avatar">MV</div>
              <div className="chat-bubble loading-bubble">
                <div className="dot-pulse-container">
                  <span className="dot-pulse" />
                  <span className="dot-pulse delay-1" />
                  <span className="dot-pulse delay-2" />
                </div>
              </div>
            </article>
          )}
          <div ref={threadEndRef} />
        </div>
      </div>

      <div className="chat-input-container">
        {/* Compact Persistent status pills bar above bottom text input area */}
        <div className="zen-pills-row compact">
          {/* Pill 1: Active Vault */}
          <div className="zen-pill-container" onClick={(e) => e.stopPropagation()}>
            <button
              type="button"
              className={`zen-pill ${activeDropdown === "vault" ? "active" : ""}`}
              onClick={(e) => {
                e.stopPropagation();
                toggleDropdown("vault");
              }}
            >
              <span className="zen-pill-icon">
                <FolderIcon size={16} />
              </span>
              <span className="zen-pill-label">Vault:</span>
              <span className="zen-pill-value">{selectedVaultName}</span>
              <span className="zen-pill-chevron">
                <ChevronDownIcon size={16} />
              </span>
            </button>
            {activeDropdown === "vault" && (
              <div className="zen-dropdown">
                <div className="zen-dropdown-header">Select Scoped Memory Domain</div>
                <button
                  type="button"
                  className={`zen-dropdown-item ${!selectedVaultId ? "selected" : ""}`}
                  onClick={() => handleSelectVault(null)}
                >
                  <span className="item-icon">
                    <GlobeIcon size={16} />
                  </span>
                  <span className="item-text">Root Graph (Global Context)</span>
                </button>
                {vaults.map((v) => (
                  <button
                    key={v.id}
                    type="button"
                    className={`zen-dropdown-item ${selectedVaultId === v.id ? "selected" : ""}`}
                    onClick={() => handleSelectVault(v.id)}
                  >
                    <span className="item-icon">
                      <FolderIcon size={16} />
                    </span>
                    <span className="item-text">{v.name}</span>
                    <span className="item-badge">{v.privacyTier.replace("_", " ")}</span>
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Pill 2: Agent Mode */}
          <div className="zen-pill-container" onClick={(e) => e.stopPropagation()}>
            <button
              type="button"
              className={`zen-pill ${activeDropdown === "mode" ? "active" : ""}`}
              onClick={(e) => {
                e.stopPropagation();
                toggleDropdown("mode");
              }}
            >
              <span className="zen-pill-icon">
                <SparklesIcon size={16} />
              </span>
              <span className="zen-pill-label">Mode:</span>
              <span className="zen-pill-value">{agentMode}</span>
              <span className="zen-pill-chevron">
                <ChevronDownIcon size={16} />
              </span>
            </button>
            {activeDropdown === "mode" && (
              <div className="zen-dropdown animate-fade-in">
                <div className="zen-dropdown-header">Select Intent / Execution State</div>
                <button
                  type="button"
                  className={`zen-dropdown-item ${agentMode === "Recall/Chat" ? "selected" : ""}`}
                  onClick={() => handleSelectMode("Recall/Chat")}
                >
                  <span className="item-icon">
                    <MessageIcon size={16} />
                  </span>
                  <div className="item-details">
                    <div className="item-title">Recall/Chat</div>
                    <div className="item-desc">
                      MACRL routing to fetch relevant Nodes & converse
                    </div>
                  </div>
                </button>
                <button
                  type="button"
                  className={`zen-dropdown-item ${agentMode === "Ingest/Memory" ? "selected" : ""}`}
                  onClick={() => handleSelectMode("Ingest/Memory")}
                >
                  <span className="item-icon">
                    <InboxIcon size={16} />
                  </span>
                  <div className="item-details">
                    <div className="item-title">Ingest/Memory</div>
                    <div className="item-desc">
                      Extract, deduplicate, and store input as new Node
                    </div>
                  </div>
                </button>
                <button
                  type="button"
                  className={`zen-dropdown-item ${agentMode === "Onboarding" ? "selected" : ""}`}
                  onClick={() => handleSelectMode("Onboarding")}
                >
                  <span className="item-icon">
                    <GraduationIcon size={16} />
                  </span>
                  <div className="item-details">
                    <div className="item-title">Onboarding</div>
                    <div className="item-desc">Trigger Onboarding Agent interview for context</div>
                  </div>
                </button>
              </div>
            )}
          </div>

          {/* Pill 3: Model */}
          <div className="zen-pill-container" onClick={(e) => e.stopPropagation()}>
            <button
              type="button"
              className={`zen-pill ${activeDropdown === "model" ? "active" : ""}`}
              onClick={(e) => {
                e.stopPropagation();
                toggleDropdown("model");
              }}
            >
              <span className="zen-pill-icon">
                <GearIcon size={16} />
              </span>
              <span className="zen-pill-label">Model:</span>
              <span className="zen-pill-value">{activeModelDisplay}</span>
              <span className="zen-pill-chevron">
                <ChevronDownIcon size={16} />
              </span>
            </button>
            {activeDropdown === "model" && (
              <div className="zen-dropdown models-dropdown">
                <div className="zen-dropdown-header">Select Model Environment</div>

                <button
                  type="button"
                  className={`zen-dropdown-item ${currentMode === "local" ? "selected" : ""}`}
                  onClick={() => handleSelectModeSettings("local")}
                >
                  <span className="item-icon">
                    <ComputerIcon size={16} />
                  </span>
                  <div className="item-details">
                    <div className="item-title">Local Models</div>
                    <div className="item-desc">Secure, offline models via Ollama or LM Studio</div>
                  </div>
                </button>

                <button
                  type="button"
                  className={`zen-dropdown-item ${currentMode === "cloud" ? "selected" : ""}`}
                  onClick={() => handleSelectModeSettings("cloud")}
                >
                  <span className="item-icon">
                    <CloudIcon size={16} />
                  </span>
                  <div className="item-details">
                    <div className="item-title">Cloud Models</div>
                    <div className="item-desc">
                      High-performance AI via OpenAI, Anthropic, Google, xAI
                    </div>
                  </div>
                </button>

                <button
                  type="button"
                  className={`zen-dropdown-item ${currentMode === "hybrid" ? "selected" : ""}`}
                  onClick={() => handleSelectModeSettings("hybrid")}
                >
                  <span className="item-icon">
                    <ZapIcon size={16} />
                  </span>
                  <div className="item-details">
                    <div className="item-title">Hybrid Mode</div>
                    <div className="item-desc">Run both Cloud and Local models simultaneously</div>
                  </div>
                </button>
              </div>
            )}
          </div>
          {/* Pill 4: Extract Memory */}
          <div className="zen-pill-container">
            <button
              type="button"
              className={`zen-pill extract-pill ${isExtracting ? "active" : ""}`}
              onClick={() => void handleForceExtract()}
              disabled={isExtracting || isSending}
            >
              <span className="zen-pill-icon">
                {isExtracting ? <HourglassIcon size={16} /> : <BrainIcon size={16} />}
              </span>
              <span className="zen-pill-label">{isExtracting ? "Extracting..." : "Extract"}</span>
            </button>
          </div>
          {/* Pill 5: Off the Record Banner */}
          {isOffTheRecord && (
            <div className="otr-convert-banner">
              <span className="otr-banner-label">
                <EyeOffIcon size={16} /> Off the Record
              </span>
              <button
                type="button"
                className="convert-memory-btn"
                onClick={() => void handleConvertToMemory()}
                disabled={isConverting || isSending || messages.length === 0}
                title="Save this temporary brainstorm as a normal conversation"
              >
                {isConverting ? "Saving..." : "Save Brainstorm to Memory"}
              </button>
            </div>
          )}

          {/* Pill 6: Interactive Charts */}
          <div className="zen-pill-container">
            <button
              type="button"
              className={`zen-pill charts-pill ${chartsEnabled ? "active" : ""}`}
              onClick={handleToggleCharts}
            >
              <span className="zen-pill-icon">
                {chartsEnabled ? <BarChartIcon size={16} /> : <TrendingUpIcon size={16} />}
              </span>
              <span className="zen-pill-label">Charts:</span>
              <span className="zen-pill-value">{chartsEnabled ? "ON" : "OFF"}</span>
            </button>
          </div>
        </div>

        {attachmentChipsJsx}
        {/* High-rounded input area matching search bar styles */}
        <div className="chat-input-wrapper">
          <button
            type="button"
            className="chat-attach-btn"
            onClick={() => void handleAttachPdf()}
            disabled={isAttaching || isSending}
            title="Attach a PDF document"
            aria-label="Attach PDF"
          >
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
            </svg>
          </button>
          <textarea
            ref={inputRef}
            value={input}
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={handleInputKeyDown}
            placeholder="Ask Amber..."
            disabled={isSending}
            rows={1}
          />
          <button
            type="button"
            className="chat-input-submit"
            onClick={() => void handleSend()}
            disabled={!canSend}
            aria-label="Send query"
          >
            <ArrowUpIcon size={18} />
          </button>
        </div>
      </div>
      {status && <p className="chat-status">{status}</p>}
      {showChartsConfirmModal && (
        <div className="charts-modal-overlay" onClick={() => setShowChartsConfirmModal(false)}>
          <div className="charts-modal-card" onClick={(e) => e.stopPropagation()}>
            <div className="charts-modal-header">
              <span className="charts-modal-icon">
                <AlertIcon size={20} />
              </span>
              <h3 className="charts-modal-title">Enable Experimental Charts?</h3>
            </div>
            <div className="charts-modal-body">
              <p className="charts-modal-text warning-lead">
                Chart generation is highly experimental and does not perform well with smaller/local
                LLMs (e.g. Ollama 7B/8B), which frequently output invalid JSON schemas.
              </p>
              <p className="charts-modal-text recommendation-note">
                For reliable, high-quality interactive plots and mathematical visualizations, we
                strongly recommend choosing advanced cloud models such as{" "}
                <strong>Claude 4.6 Sonnet</strong> or <strong>GPT-5.4</strong> or larger local
                models.
              </p>
              <p className="charts-modal-question">Would you like to enable charts anyway?</p>
            </div>
            <div className="charts-modal-actions">
              <button
                type="button"
                className="charts-modal-btn cancel-btn"
                onClick={() => setShowChartsConfirmModal(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="charts-modal-btn confirm-btn"
                onClick={() => {
                  setChatChartsEnabled(true);
                  setShowChartsConfirmModal(false);
                }}
              >
                Enable Charts
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

export default ChatPanel;
