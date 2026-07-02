import { useCallback, useEffect, useState } from "react";
import {
  TbClipboard,
  TbClipboardCheck,
  TbSourceCode,
  TbVector,
  TbLink,
  TbShieldLock,
  TbShieldOff,
} from "react-icons/tb";
import { getPlantUmlServer } from "../utils/settings";
import { getPlantUmlConsent, setPlantUmlConsent } from "../utils/settings";

interface PlantUmlBlockProps {
  code: string;
}

export default function PlantUmlBlock({ code }: PlantUmlBlockProps) {
  const [consent, setConsent] = useState(getPlantUmlConsent);
  const [encoded, setEncoded] = useState<string>("");
  const [viewSource, setViewSource] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [copiedSvg, setCopiedSvg] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(() => getPlantUmlConsent() !== "disabled");

  const allowed = consent !== "disabled";
  const isLoading = loading || (!encoded && !error);

  // Listen for external consent changes (e.g. another PlantUML block on the same page or settings hydration)
  useEffect(() => {
    const handler = () => setConsent(getPlantUmlConsent());
    window.addEventListener("mindvault:plantuml-consent-changed", handler);
    window.addEventListener("mindvault:llm-settings-changed", handler);
    return () => {
      window.removeEventListener("mindvault:plantuml-consent-changed", handler);
      window.removeEventListener("mindvault:llm-settings-changed", handler);
    };
  }, []);

  // Only encode + fetch when consent has been granted
  useEffect(() => {
    if (!allowed) return;

    let active = true;

    const debounceTimer = setTimeout(() => {
      async function encodeDiagram() {
        try {
          setLoading(true);
          setError(null);

          // 1. UTF-8 encode
          const encoder = new TextEncoder();
          const utf8 = encoder.encode(code);

          // 2. Compress using Deflate via CompressionStream
          const stream = new Blob([utf8]).stream().pipeThrough(new CompressionStream("deflate"));
          const response = new Response(stream);
          const buffer = await response.arrayBuffer();
          const zlibData = new Uint8Array(buffer);

          // 3. Strip 2-byte zlib header and 4-byte Adler-32 checksum to get raw deflate
          const rawDeflate = zlibData.subarray(2, zlibData.length - 4);

          // 4. Custom PlantUML Base64 encode
          const encodedStr = encode64(rawDeflate);

          if (active) {
            setEncoded(encodedStr);
            setLoading(false);
          }
        } catch (err) {
          console.error("PlantUML Compression Error:", err);
          if (active) {
            setError(err instanceof Error ? err.message : String(err));
            setLoading(false);
          }
        }
      }

      void encodeDiagram();
    }, 600);

    return () => {
      active = false;
      clearTimeout(debounceTimer);
    };
  }, [code, allowed]);

  const handleCopyCode = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy PlantUML source code:", err);
    }
  };

  const handleCopyUrl = async () => {
    if (!encoded) return;
    const server = getPlantUmlServer();
    const diagramUrl = `${server}/svg/${encoded}`;
    try {
      await navigator.clipboard.writeText(diagramUrl);
      setCopiedUrl(true);
      setTimeout(() => setCopiedUrl(false), 2000);
    } catch (err) {
      console.error("Failed to copy diagram URL:", err);
    }
  };

  const handleCopySvg = async () => {
    if (!encoded) return;
    const server = getPlantUmlServer();
    const diagramUrl = `${server}/svg/${encoded}`;
    try {
      const res = await fetch(diagramUrl);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const svgText = await res.text();
      await navigator.clipboard.writeText(svgText);
      setCopiedSvg(true);
      setTimeout(() => setCopiedSvg(false), 2000);
    } catch (err) {
      console.error("Failed to fetch and copy diagram SVG:", err);
    }
  };

  const handleEnableSession = useCallback(() => {
    setLoading(true);
    setEncoded("");
    setError(null);
    setPlantUmlConsent("session");
    setConsent("session");
  }, []);

  const handleAlwaysAllow = useCallback(() => {
    setLoading(true);
    setEncoded("");
    setError(null);
    setPlantUmlConsent("always");
    setConsent("always");
  }, []);

  const handleRevokeConsent = useCallback(() => {
    setPlantUmlConsent("disabled");
    setConsent("disabled");
    setEncoded("");
  }, []);

  const server = getPlantUmlServer();
  const serverHost = (() => {
    try {
      return new URL(server).hostname || server;
    } catch {
      return server;
    }
  })();

  // --- Consent gate: show privacy placeholder ---
  if (!allowed) {
    return (
      <div className="plantuml-block-wrapper">
        <div className="plantuml-block-header">
          <div className="plantuml-block-title-container">
            <TbShieldLock className="plantuml-block-title-icon" size={16} />
            <span className="plantuml-block-title">PlantUML Diagram</span>
          </div>
          <div className="plantuml-block-actions">
            <button
              type="button"
              className="plantuml-action-btn-pill"
              onClick={() => setViewSource((prev) => !prev)}
              title={viewSource ? "Hide Source Code" : "Show Source Code"}
            >
              <TbSourceCode size={14} />
              <span>Source</span>
            </button>
            {viewSource && (
              <button
                type="button"
                className="plantuml-action-btn-icon"
                onClick={handleCopyCode}
                title="Copy PlantUML Code"
                aria-label="Copy code"
              >
                {copied ? <TbClipboardCheck size={14} /> : <TbClipboard size={14} />}
              </button>
            )}
          </div>
        </div>

        {viewSource ? (
          <div className="plantuml-block-source-container">
            <pre>
              <code>{code}</code>
            </pre>
          </div>
        ) : (
          <div className="plantuml-consent-card">
            <TbShieldLock className="plantuml-consent-icon" size={32} />
            <h4 className="plantuml-consent-title">Privacy Notice</h4>
            <p className="plantuml-consent-message">
              Rendering this diagram requires sending its text to an external server (
              <strong>{serverHost}</strong>). No data is sent until you opt in.
            </p>
            <div className="plantuml-consent-actions">
              <button type="button" className="plantuml-consent-btn" onClick={handleEnableSession}>
                Enable for this Session
              </button>
              <button
                type="button"
                className="plantuml-consent-btn plantuml-consent-btn-primary"
                onClick={handleAlwaysAllow}
              >
                Always Allow
              </button>
            </div>
          </div>
        )}
      </div>
    );
  }

  // --- Consented: render normally ---
  const diagramUrl = encoded ? `${server}/svg/${encoded}` : "";

  return (
    <div className="plantuml-block-wrapper">
      <div className={`plantuml-block-header ${error ? "error" : ""}`}>
        <div className="plantuml-block-title-container">
          <TbVector className="plantuml-block-title-icon" size={16} />
          <span className="plantuml-block-title">
            {error ? "PlantUML Diagram (Error)" : "PlantUML Diagram"}
          </span>
        </div>

        <div className="plantuml-block-actions">
          {!error && !isLoading && (
            <>
              <button
                type="button"
                className="plantuml-action-btn-pill"
                onClick={() => setViewSource((prev) => !prev)}
                title={viewSource ? "Show Rendered Diagram" : "Show Source Code"}
              >
                {viewSource ? (
                  <>
                    <TbVector size={14} />
                    <span>Diagram</span>
                  </>
                ) : (
                  <>
                    <TbSourceCode size={14} />
                    <span>Source</span>
                  </>
                )}
              </button>

              {!viewSource && encoded && (
                <>
                  <button
                    type="button"
                    className="plantuml-action-btn-icon"
                    onClick={handleCopySvg}
                    title="Copy Diagram SVG"
                    aria-label="Copy SVG"
                  >
                    {copiedSvg ? <TbClipboardCheck size={14} /> : <TbClipboard size={14} />}
                  </button>

                  <button
                    type="button"
                    className="plantuml-action-btn-icon"
                    onClick={handleCopyUrl}
                    title="Copy Diagram URL"
                    aria-label="Copy URL"
                  >
                    {copiedUrl ? <TbClipboardCheck size={14} /> : <TbLink size={14} />}
                  </button>
                </>
              )}
            </>
          )}

          {(error || viewSource) && (
            <button
              type="button"
              className="plantuml-action-btn-icon"
              onClick={handleCopyCode}
              title="Copy PlantUML Code"
              aria-label="Copy code"
            >
              {copied ? <TbClipboardCheck size={14} /> : <TbClipboard size={14} />}
            </button>
          )}

          {!isLoading && (
            <button
              type="button"
              className="plantuml-action-btn-icon plantuml-revoke-btn"
              onClick={handleRevokeConsent}
              title="Revoke PlantUML rendering consent"
              aria-label="Revoke consent"
            >
              <TbShieldOff size={14} />
            </button>
          )}
        </div>
      </div>

      {isLoading ? (
        <div className="plantuml-block-body loading">
          <div className="plantuml-spinner" />
          <span>Encoding diagram...</span>
        </div>
      ) : error ? (
        <div className="plantuml-block-error-container">
          <div className="plantuml-block-error-message">
            <span className="plantuml-error-text">
              <strong>Encoding Error:</strong> {error}
            </span>
          </div>
          <div className="plantuml-block-error-code">
            <pre>
              <code>{code}</code>
            </pre>
          </div>
        </div>
      ) : viewSource ? (
        <div className="plantuml-block-source-container">
          <pre>
            <code>{code}</code>
          </pre>
        </div>
      ) : (
        <div className="plantuml-block-body">
          <img
            src={diagramUrl}
            alt="PlantUML Diagram"
            className="plantuml-diagram-image"
            loading="lazy"
          />
        </div>
      )}
    </div>
  );
}

// PlantUML custom Base64 encoder
function encode64(data: Uint8Array): string {
  let r = "";
  for (let i = 0; i < data.length; i += 3) {
    if (i + 2 < data.length) {
      r += append3bytes(data[i], data[i + 1], data[i + 2]);
    } else if (i + 1 < data.length) {
      r += append3bytes(data[i], data[i + 1], 0).substring(0, 3);
    } else {
      r += append3bytes(data[i], 0, 0).substring(0, 2);
    }
  }
  return r;
}

function append3bytes(b1: number, b2: number, b3: number): string {
  const c1 = b1 >> 2;
  const c2 = ((b1 & 0x3) << 4) | (b2 >> 4);
  const c3 = ((b2 & 0xf) << 2) | (b3 >> 6);
  const c4 = b3 & 0x3f;
  let r = "";
  r += encode6bit(c1 & 0x3f);
  r += encode6bit(c2 & 0x3f);
  r += encode6bit(c3 & 0x3f);
  r += encode6bit(c4 & 0x3f);
  return r;
}

function encode6bit(b: number): string {
  if (b < 10) {
    return String.fromCharCode(48 + b); // '0' - '9'
  }
  b -= 10;
  if (b < 26) {
    return String.fromCharCode(65 + b); // 'A' - 'Z'
  }
  b -= 26;
  if (b < 26) {
    return String.fromCharCode(97 + b); // 'a' - 'z'
  }
  b -= 26;
  if (b === 0) {
    return "-";
  }
  if (b === 1) {
    return "_";
  }
  return "?";
}
