/* eslint-disable react-refresh/only-export-components */
import React, { useState } from "react";
import { createPortal } from "react-dom";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import ChartBlock from "../components/ChartBlock";
import MermaidBlock from "../components/MermaidBlock";
import PlantUmlBlock from "../components/PlantUmlBlock";
import LatexBlock from "../components/LatexBlock";
import { getAllNodes } from "../services/nodes";
import "katex/dist/katex.min.css";
import { AlertIcon, ArrowUpRightIcon } from "../components/icons";
import {
  TbBrandPython,
  TbBrandJavascript,
  TbBrandTypescript,
  TbBrandRust,
  TbBrandHtml5,
  TbBrandCss3,
  TbDatabase,
  TbTerminal2,
  TbBraces,
  TbMarkdown,
  TbBrandGolang,
  TbBrandCpp,
  TbCode,
} from "react-icons/tb";

export type CodeBlockProps = {
  language: string;
  code: string;
};

export const remarkPluginsStable = [remarkGfm, remarkMath];
export const rehypePluginsStable = [rehypeKatex];

export function getLanguageIcon(language: string): React.ReactNode {
  const lang = language.toLowerCase().trim();
  if (!lang) return null;

  const size = 15;

  switch (lang) {
    case "python":
    case "py":
      return <TbBrandPython size={size} />;
    case "javascript":
    case "js":
      return <TbBrandJavascript size={size} />;
    case "typescript":
    case "ts":
      return <TbBrandTypescript size={size} />;
    case "rust":
    case "rs":
      return <TbBrandRust size={size} />;
    case "bash":
    case "sh":
    case "shell":
    case "zsh":
      return <TbTerminal2 size={size} />;
    case "json":
      return <TbBraces size={size} />;
    case "html":
      return <TbBrandHtml5 size={size} />;
    case "css":
      return <TbBrandCss3 size={size} />;
    case "sql":
      return <TbDatabase size={size} />;
    case "markdown":
    case "md":
      return <TbMarkdown size={size} />;
    case "go":
    case "golang":
      return <TbBrandGolang size={size} />;
    case "cpp":
    case "c++":
    case "c":
      return <TbBrandCpp size={size} />;
    default:
      return <TbCode size={size} />;
  }
}

export function CodeBlock({ language, code }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy code block:", err);
    }
  };

  return (
    <div className="code-block-wrapper">
      <div className="code-block-header">
        <div className="code-block-lang-container">
          {getLanguageIcon(language)}
          <span className="code-block-lang">{language}</span>
        </div>
        <button
          type="button"
          className="code-block-copy-btn"
          onClick={handleCopy}
          aria-label={copied ? "Copied to clipboard" : "Copy code"}
          title={copied ? "Copied!" : "Copy"}
        >
          {copied ? (
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="20 6 9 17 4 12"></polyline>
            </svg>
          ) : (
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
              <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
            </svg>
          )}
        </button>
      </div>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}

export { preprocessWikiLinks } from "./wikilinkUtils";

/** Detects if the given text has raw LaTeX document structure (checking for \documentclass or \begin{document}) */
export function isRawLatex(text: string): boolean {
  if (!text) return false;
  const trimmed = text.trim();
  return trimmed.includes("\\documentclass") || trimmed.includes("\\begin{document}");
}

/** Normalizes LaTeX math delimiters like \[ ... \] and \( ... \) into standard $$ and $ formats for ReactMarkdown */
export function preprocessMathDelimiters(text: string): string {
  if (!text) return "";
  let processed = text;
  // Replace \\[ or \[ with $$
  processed = processed.replace(/\\\\\[/g, "$$$$\n").replace(/\\\[/g, "$$$$\n");
  processed = processed.replace(/\\\\\]/g, "\n$$$$").replace(/\\\]/g, "\n$$$$");
  // Replace \\( or \( with $
  processed = processed.replace(/\\\\\(/g, "$").replace(/\\\(/g, "$");
  processed = processed.replace(/\\\\\)/g, "$").replace(/\\\)/g, "$");
  return processed;
}
// undefined = no provider (badges validate per link); null = provider loading; Set = ready lookup
export const ExistingNodesContext = React.createContext<Set<string> | null | undefined>(undefined);

function WikiLinkBadge({
  nodeId,
  children,
  onSelectNode,
  isRedactedUnlocked,
}: {
  nodeId: string;
  children: React.ReactNode;
  onSelectNode?: (nodeId: string) => void;
  isRedactedUnlocked?: boolean;
}) {
  const existingNodeIds = React.useContext(ExistingNodesContext);
  const [fetchedExists, setFetchedExists] = React.useState<boolean | null>(null);
  const [modal, setModal] = React.useState<{ title: string; message: string } | null>(null);
  const isSearchQuery = nodeId.startsWith("search:");

  // Extract a clean string representation of children for string templates (alert messages & tooltips)
  const labelText = React.useMemo(() => {
    if (typeof children === "string") return children;
    if (typeof children === "number") return String(children);
    const extractString = (node: React.ReactNode): string => {
      if (!node) return "";
      if (typeof node === "string" || typeof node === "number") return String(node);
      if (Array.isArray(node)) return node.map(extractString).join("");
      if (React.isValidElement(node)) {
        return extractString((node.props as { children?: React.ReactNode }).children);
      }
      return "";
    };
    return extractString(children);
  }, [children]);

  const nodeExists = existingNodeIds ? existingNodeIds.has(nodeId) : fetchedExists;
  const isBroken = !isSearchQuery && nodeExists === false;
  const isLoading = nodeExists === null && !isSearchQuery;

  // Validate node existence on mount (skip for search queries or if context provider is present)
  React.useEffect(() => {
    if (isSearchQuery || existingNodeIds !== undefined) {
      return; // Don't validate search queries or if context provider is present
    }

    let isMounted = true;

    getAllNodes(isRedactedUnlocked)
      .then((nodes) => {
        if (isMounted) {
          const exists = nodes.some((n) => n.id === nodeId);
          setFetchedExists(exists);
        }
      })
      .catch(() => {
        if (isMounted) {
          setFetchedExists(false);
        }
      });

    return () => {
      isMounted = false;
    };
  }, [nodeId, isSearchQuery, existingNodeIds, isRedactedUnlocked]);

  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (isBroken) {
      setModal({
        title: "Broken Node Connection",
        message: `The node "${labelText}" no longer exists in your vault.\n\nYou may need to:\n• Remove this link\n• Create a matching node with this title`,
      });
      return;
    }

    if (onSelectNode) {
      if (isSearchQuery) {
        const query = nodeId.substring(7).trim();
        getAllNodes(isRedactedUnlocked)
          .then((nodes) => {
            const match = nodes.find((n) => n.title.toLowerCase().trim() === query.toLowerCase());
            if (match) {
              onSelectNode(match.id);
            } else {
              setModal({
                title: "Node Not Found",
                message: `No node with title "${query}" exists in your vault.`,
              });
            }
          })
          .catch((err) => console.error("Failed to query nodes for wikilink:", err));
      } else {
        onSelectNode(nodeId);
      }
    }
  };

  return (
    <>
      <button
        type="button"
        className={`wikilink-badge ${isBroken ? "broken" : ""} ${isLoading ? "loading" : ""}`}
        onClick={handleClick}
        title={
          isBroken
            ? `Broken link: Node "${labelText}" not found`
            : isSearchQuery
              ? `Search/Navigate to: ${labelText}`
              : `Navigate to: ${labelText}`
        }
        disabled={isLoading}
      >
        <span className="wikilink-badge-icon">
          {isBroken ? <AlertIcon size={12} /> : <ArrowUpRightIcon size={12} />}
        </span>{" "}
        {children}
      </button>

      {modal &&
        createPortal(
          <div className="diff-edit-modal-backdrop" onClick={() => setModal(null)}>
            <div className="diff-edit-modal" onClick={(e) => e.stopPropagation()}>
              <h3 style={{ margin: "0 0 16px 0", color: "#bc6c25" }}>{modal.title}</h3>
              <p
                style={{
                  margin: "0 0 20px 0",
                  fontSize: "0.95rem",
                  color: "#7d7a75",
                  lineHeight: "1.5",
                  whiteSpace: "pre-wrap",
                }}
              >
                {modal.message}
              </p>
              <div className="edit-modal-actions">
                <button className="edit-save-btn" onClick={() => setModal(null)}>
                  Dismiss
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
    </>
  );
}
/**
 * Creates stable markdown components overrides
 */
export function createMarkdownComponents(
  chartsEnabled: boolean,
  onSelectNode?: (nodeId: string) => void,
  isRedactedUnlocked?: boolean
) {
  return {
    a({ href, children, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) {
      const isNodeLink = href && (href.includes("#node/") || href.includes("mindvault://node/"));
      if (isNodeLink) {
        const nodeId =
          href
            .split(/#node\/|mindvault:\/\/node\//)
            .pop()
            ?.split(/[?#]/)[0] || "";
        const decodedNodeId = decodeURIComponent(nodeId);
        return (
          <WikiLinkBadge
            nodeId={decodedNodeId}
            onSelectNode={onSelectNode}
            isRedactedUnlocked={isRedactedUnlocked}
          >
            {children}
          </WikiLinkBadge>
        );
      }
      return (
        <a href={href} {...props}>
          {children}
        </a>
      );
    },
    pre({
      children,
      ...props
    }: React.ClassAttributes<HTMLPreElement> &
      React.HTMLAttributes<HTMLPreElement> & { node?: unknown }) {
      const childrenArray = React.Children.toArray(children);
      const codeChild = childrenArray.find(
        (
          child
        ): child is React.ReactElement<{
          className?: string;
          children?: React.ReactNode;
        }> => React.isValidElement(child) && child.type === "code"
      );

      if (codeChild) {
        const codeProps = codeChild.props;
        const className = codeProps.className || "";
        const match = /language-([\w-]+)/.exec(className);
        const language = match ? match[1] : "";

        const codeContent = codeProps.children;
        const codeString =
          typeof codeContent === "string"
            ? codeContent
            : Array.isArray(codeContent)
              ? codeContent.join("")
              : String(codeContent || "");

        const isChart =
          chartsEnabled &&
          (language === "chart" || language === "plotly" || language.startsWith("chart-"));

        if (isChart) {
          return <ChartBlock language={language} code={codeString.replace(/\n$/, "")} />;
        }

        if (language === "mermaid") {
          return chartsEnabled ? (
            <MermaidBlock code={codeString.replace(/\n$/, "")} />
          ) : (
            <CodeBlock language={language} code={codeString.replace(/\n$/, "")} />
          );
        }

        if (language === "plantuml" || language === "puml") {
          return chartsEnabled ? (
            <PlantUmlBlock code={codeString.replace(/\n$/, "")} />
          ) : (
            <CodeBlock language={language} code={codeString.replace(/\n$/, "")} />
          );
        }

        if (language === "latex" || language === "tex") {
          return chartsEnabled ? (
            <LatexBlock code={codeString.replace(/\n$/, "")} />
          ) : (
            <CodeBlock language={language} code={codeString.replace(/\n$/, "")} />
          );
        }

        return <CodeBlock language={language} code={codeString.replace(/\n$/, "")} />;
      }

      return <pre {...props}>{children}</pre>;
    },
  };
}

/**
 * Calculates the exact top and left caret coordinates relative to the top-left corner of the textarea's padding box.
 */
export function getCaretCoordinates(
  element: HTMLTextAreaElement,
  position: number
): { top: number; left: number } {
  const div = document.createElement("div");
  document.body.appendChild(div);

  const style = div.style;
  const computed = window.getComputedStyle(element);

  style.whiteSpace = "pre-wrap";
  style.wordBreak = "break-word";
  style.position = "absolute";
  style.visibility = "hidden";

  const properties = [
    "direction",
    "boxSizing",
    "width",
    "height",
    "overflowX",
    "overflowY",
    "borderWidth",
    "borderStyle",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "fontFamily",
    "fontSize",
    "fontWeight",
    "fontStyle",
    "fontVariant",
    "textTransform",
    "wordSpacing",
    "letterSpacing",
    "lineHeight",
  ];

  for (const prop of properties) {
    // @ts-expect-error - dynamic key styling access
    style[prop] = computed[prop];
  }

  style.boxSizing = "content-box";
  const paddingLeft = parseFloat(computed.paddingLeft) || 0;
  const paddingRight = parseFloat(computed.paddingRight) || 0;
  style.width = `${element.clientWidth - paddingLeft - paddingRight}px`;

  const textContent = element.value.substring(0, position);
  div.textContent = textContent;

  const span = document.createElement("span");
  span.textContent = "\u200b";
  div.appendChild(span);

  const borderTop = parseFloat(computed.borderTopWidth) || 0;
  const borderLeft = parseFloat(computed.borderLeftWidth) || 0;
  const lineHeight = parseFloat(computed.lineHeight) || 20;

  const coordinates = {
    top: span.offsetTop + borderTop + lineHeight - element.scrollTop,
    left: span.offsetLeft + borderLeft - element.scrollLeft,
  };

  document.body.removeChild(div);
  return coordinates;
}
