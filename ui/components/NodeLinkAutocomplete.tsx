import { useEffect, useMemo, useState, useRef } from "react";
import type { Node } from "../types/generated/Node";
import { FileIcon } from "./icons";

type NodeLinkAutocompleteProps = {
  query: string;
  position: { top: number; left: number };
  onSelect: (node: Node) => void;
  onClose: () => void;
  nodes?: Node[];
};

export default function NodeLinkAutocomplete({
  query,
  position,
  onSelect,
  onClose,
  nodes: preloadedNodes,
}: NodeLinkAutocompleteProps) {
  const [asyncNodes, setAsyncNodes] = useState<Node[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const filteredPreloadedNodes = useMemo(() => {
    if (!preloadedNodes) {
      return null;
    }

    const normalizedQuery = query.trim().toLowerCase();
    const results = !normalizedQuery
      ? preloadedNodes
      : preloadedNodes.filter(
          (node) =>
            node.title.toLowerCase().includes(normalizedQuery) ||
            (node.summary && node.summary.toLowerCase().includes(normalizedQuery))
        );

    return results.slice(0, 8);
  }, [query, preloadedNodes]);

  useEffect(() => {
    if (preloadedNodes) {
      return;
    }

    let active = true;
    async function fetchMatches() {
      try {
        const { searchNodes } = await import("../services/nodes");
        const results = await searchNodes(query);
        if (active) {
          // Limit to top 8 matches for visual sanity and quick lists
          setAsyncNodes(results.slice(0, 8));
          setSelectedIndex(0);
        }
      } catch (err) {
        console.error("Failed to query autocomplete nodes:", err);
      }
    }
    fetchMatches();
    return () => {
      active = false;
    };
  }, [query, preloadedNodes]);

  const nodes = filteredPreloadedNodes ?? asyncNodes;

  useEffect(() => {
    if (nodes.length === 0) return;

    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        setSelectedIndex((prev) => (prev + 1) % nodes.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        setSelectedIndex((prev) => (prev - 1 + nodes.length) % nodes.length);
      } else if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        e.stopPropagation();
        onSelect(nodes[selectedIndex]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    }

    // Capture keyboard events strictly to prevent default text area behaviors (newline on enter, cursor jump on arrows)
    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [nodes, selectedIndex, onSelect, onClose]);

  // Adjust positioning to make sure it doesn't overflow screen boundaries
  useEffect(() => {
    if (!containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;

    if (rect.right > viewportWidth) {
      containerRef.current.style.left = `${position.left - (rect.right - viewportWidth) - 16}px`;
    }
    if (rect.bottom > viewportHeight) {
      // Shift it upwards if it goes below the screen fold
      containerRef.current.style.top = `${position.top - rect.height - 24}px`;
    }
  }, [nodes, position]);

  if (nodes.length === 0) return null;

  return (
    <div
      ref={containerRef}
      className="wikilink-dropdown"
      style={{
        position: "absolute",
        top: `${position.top}px`,
        left: `${position.left}px`,
      }}
    >
      <div className="wikilink-dropdown-header">Link to Node</div>
      {nodes.map((node, index) => (
        <button
          key={node.id}
          type="button"
          className={`wikilink-option ${index === selectedIndex ? "selected" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
          }}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onSelect(node);
          }}
          onMouseEnter={() => setSelectedIndex(index)}
        >
          <span className="wikilink-option-icon">
            <FileIcon size={14} />
          </span>
          <div className="wikilink-option-text">
            <strong>{node.title}</strong>
            {node.summary ? <small>{node.summary}</small> : <small>No summary available</small>}
          </div>
        </button>
      ))}
    </div>
  );
}
