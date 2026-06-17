import React, { useState, useEffect, useRef, useTransition } from "react";
import ReactMarkdown from "react-markdown";
import {
  remarkPluginsStable,
  rehypePluginsStable,
  createMarkdownComponents,
  preprocessWikiLinks,
  getCaretCoordinates,
  isRawLatex,
  preprocessMathDelimiters,
  ExistingNodesContext,
} from "../utils/markdownUtils";
import NodeLinkAutocomplete from "./NodeLinkAutocomplete";
import type { Node } from "../types/generated/Node";
import { createDoor, listOutgoingDoors } from "../services/doors";
import { useUIStore } from "../utils/store";
import LatexBlock from "./LatexBlock";

type NodeEditorDetailProps = {
  value: string;
  onChange: (val: string) => void;
  disabled?: boolean;
  placeholder?: string;
  chartsEnabled?: boolean;
  onExpand?: () => void;
  onSelectNode?: (nodeId: string) => void;
  nodeId?: string;
  onRefreshDoors?: () => void;
  existingNodeIds?: Set<string>;
  isRedactedUnlocked?: boolean;
};

export default function NodeEditorDetail({
  value,
  onChange,
  disabled = false,
  placeholder = "Detail — type [[ to link to another node",
  chartsEnabled: propChartsEnabled,
  onExpand,
  onSelectNode,
  nodeId,
  onRefreshDoors,
  existingNodeIds,
  isRedactedUnlocked,
}: NodeEditorDetailProps) {
  const storeChartsEnabled = useUIStore((state) => state.nodeEditor.chartsEnabled);
  const setNodeEditorChartsEnabled = useUIStore((state) => state.setNodeEditorChartsEnabled);
  const chartsEnabled = propChartsEnabled !== undefined ? propChartsEnabled : storeChartsEnabled;
  const [activeTab, setActiveTab] = useState<"edit" | "preview">("preview");
  const [localValue, setLocalValue] = useState(value);
  const [debouncedPreviewValue, setDebouncedPreviewValue] = useState(value);
  const [, startTransition] = useTransition();

  // Autocomplete state
  const [wikilinkOpen, setWikilinkOpen] = useState(false);
  const [wikilinkQuery, setWikilinkQuery] = useState("");
  const [wikilinkCursorPos, setWikilinkCursorPos] = useState(0);
  const [wikilinkDropdownPos, setWikilinkDropdownPos] = useState<{
    top: number;
    left: number;
  } | null>(null);

  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    /* eslint-disable react-hooks/set-state-in-effect */
    if (value !== localValue) {
      setLocalValue(value);
    }
    /* eslint-enable react-hooks/set-state-in-effect */
  }, [value, localValue]);

  // Debounce the heavy markdown preview translation to prevent visual input lags while typing
  useEffect(() => {
    if (activeTab !== "preview") return;
    const timer = setTimeout(() => {
      startTransition(() => {
        setDebouncedPreviewValue(localValue);
      });
    }, 250);
    return () => clearTimeout(timer);
  }, [localValue, activeTab]);

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    setLocalValue(val);
    onChange(val);
  };

  const handleKeyUp = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const ta = e.currentTarget;
    const pos = ta.selectionStart;
    const text = ta.value;

    const before = text.slice(0, pos);
    const openIdx = before.lastIndexOf("[[");
    if (openIdx === -1) {
      if (wikilinkOpen) setWikilinkOpen(false);
      return;
    }

    const segment = before.slice(openIdx + 2);
    if (segment.includes("]]")) {
      if (wikilinkOpen) setWikilinkOpen(false);
      return;
    }

    setWikilinkQuery(segment);
    setWikilinkCursorPos(pos);
    setWikilinkOpen(true);

    const coords = getCaretCoordinates(ta, pos);
    setWikilinkDropdownPos(coords);
  };

  const handleSelectAutocomplete = (targetNode: Node) => {
    const ta = textareaRef.current;
    if (!ta) return;

    const text = localValue;
    const pos = wikilinkCursorPos;
    const before = text.slice(0, pos);
    const openIdx = before.lastIndexOf("[[");
    if (openIdx === -1) return;

    // Use confirmed tag output syntax [[Title|node_id]]
    const replacement = `[[${targetNode.title}|${targetNode.id}]]`;
    const newText = text.slice(0, openIdx) + replacement + text.slice(pos);

    setLocalValue(newText);
    onChange(newText);
    setWikilinkOpen(false);
    setWikilinkQuery("");

    // Create the outgoing connection door immediately if nodeId matches
    if (nodeId && targetNode.id !== nodeId) {
      void (async () => {
        try {
          const currentOutgoing = await listOutgoingDoors(nodeId);
          const alreadyExists = (currentOutgoing.data ?? []).some(
            (d) => d.targetNodeId === targetNode.id
          );
          if (!alreadyExists) {
            const res = await createDoor({
              sourceNodeId: nodeId,
              targetNodeId: targetNode.id,
            });
            if (!res.error && onRefreshDoors) {
              onRefreshDoors();
            }
          }
        } catch (err) {
          console.error("Failed to auto-create door on autocomplete select:", err);
        }
      })();
    }

    const newCursorPos = openIdx + replacement.length;
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(newCursorPos, newCursorPos);
    });
  };

  const preprocessedMarkdown = React.useMemo(() => {
    const wLinks = preprocessWikiLinks(debouncedPreviewValue);
    return preprocessMathDelimiters(wLinks);
  }, [debouncedPreviewValue]);

  const markdownComponents = React.useMemo(() => {
    return createMarkdownComponents(chartsEnabled, onSelectNode, isRedactedUnlocked);
  }, [chartsEnabled, onSelectNode, isRedactedUnlocked]);

  const markdownBody = React.useMemo(
    () => (
      <ReactMarkdown
        remarkPlugins={remarkPluginsStable}
        rehypePlugins={rehypePluginsStable}
        components={markdownComponents}
      >
        {preprocessedMarkdown}
      </ReactMarkdown>
    ),
    [markdownComponents, preprocessedMarkdown]
  );

  return (
    <div className="node-editor-detail-container">
      <div className="node-editor-detail-header">
        <div className="node-editor-detail-tabs">
          <button
            type="button"
            className={`detail-tab-btn ${activeTab === "edit" ? "active" : ""}`}
            onClick={() => setActiveTab("edit")}
          >
            Edit
          </button>
          <button
            type="button"
            className={`detail-tab-btn ${activeTab === "preview" ? "active" : ""}`}
            onClick={() => setActiveTab("preview")}
          >
            Preview
          </button>
        </div>
        <div
          className="detail-header-actions"
          style={{ display: "flex", alignItems: "center", gap: "8px" }}
        >
          <button
            type="button"
            className={`charts-toggle-btn ${chartsEnabled ? "active" : ""}`}
            onClick={() => setNodeEditorChartsEnabled(!chartsEnabled)}
            title="Toggle interactive charts render workspace assets"
          >
            📊 Charts: {chartsEnabled ? "ON" : "OFF"}
          </button>
          {onExpand && (
            <button
              type="button"
              className="detail-expand-btn"
              onClick={onExpand}
              title="Expand editor to full center canvas focus"
            >
              <span className="expand-icon">⛶</span> Focus Canvas
            </button>
          )}
        </div>
      </div>

      <div className="node-editor-detail-body">
        {activeTab === "edit" ? (
          <div
            className="wikilink-wrapper"
            style={{ position: "relative", width: "100%", height: "100%" }}
          >
            <textarea
              ref={textareaRef}
              className="editor-detail monospace-editor"
              value={localValue}
              onChange={handleChange}
              onKeyUp={handleKeyUp}
              onScroll={(e) => {
                if (wikilinkOpen) {
                  const ta = e.currentTarget;
                  const coords = getCaretCoordinates(ta, ta.selectionStart);
                  setWikilinkDropdownPos(coords);
                }
              }}
              onBlur={() => {
                // Slight timeout allows click event on absolute panel options to resolve first
                setTimeout(() => setWikilinkOpen(false), 200);
              }}
              placeholder={placeholder}
              disabled={disabled}
            />
            {wikilinkOpen && wikilinkDropdownPos && (
              <NodeLinkAutocomplete
                query={wikilinkQuery}
                position={wikilinkDropdownPos}
                onSelect={handleSelectAutocomplete}
                onClose={() => setWikilinkOpen(false)}
              />
            )}
          </div>
        ) : (
          <div className="editor-detail-preview paper-preview">
            {isRawLatex(preprocessedMarkdown) ? (
              chartsEnabled ? (
                <LatexBlock code={preprocessedMarkdown} />
              ) : (
                <pre
                  style={{
                    margin: 0,
                    padding: "16px",
                    background: "rgba(0, 0, 0, 0.03)",
                    border: "1px solid rgba(188, 108, 37, 0.15)",
                    borderRadius: "6px",
                    overflow: "auto",
                    fontFamily: "monospace",
                    whiteSpace: "pre-wrap",
                    fontSize: "0.9rem",
                    color: "var(--text-black)",
                  }}
                >
                  <code>{preprocessedMarkdown}</code>
                </pre>
              )
            ) : (
              <ExistingNodesContext.Provider value={existingNodeIds}>
                {markdownBody}
              </ExistingNodesContext.Provider>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
