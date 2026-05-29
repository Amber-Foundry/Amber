import React, { Suspense, useEffect, useMemo, useState } from "react";
import { Mafs, Coordinates, Plot } from "mafs";
import "mafs/core.css";
import { preprocessExpression, tokenize, parseToRpn, evaluateRpn } from "../utils/mathParser";
import {
  TbMath,
  TbChartArea,
  TbAlertCircle,
  TbArrowsMove,
  TbSearch,
  TbRectangle,
  TbLasso,
  TbPlus,
  TbMinus,
  TbArrowsMaximize,
  TbHome,
} from "react-icons/tb";
import { jsonrepair } from "jsonrepair";

// Cast third-party Mafs component to bypass non-standard props type errors

const MafsAny = Mafs as unknown as React.FC<Record<string, unknown>>;

// Lazy load Plotly to maintain fast startup speed
const PlotlyRenderer = React.lazy(() => import("./PlotlyRenderer"));

// Set of direct Plotly trace types to automatically wrap and route to Plotly
const PLOTLY_TRACE_TYPES = new Set([
  "bar",
  "scatter",
  "pie",
  "line",
  "heatmap",
  "box",
  "violin",
  "histogram",
  "funnel",
  "waterfall",
  "treemap",
  "sunburst",
  "scatterpolar",
  "barpolar",
  "scatter3d",
]);

const DEFAULT_DOMAIN_X: [number, number] = [-5, 5];
const DEFAULT_DOMAIN_Y: [number, number] = [-5, 5];

interface ChartBlockProps {
  language: string;
  code: string;
}

interface ParsedMathExpression {
  rpn: string[];
  color: string;
  label: string;
}

interface ParsedChartSchema {
  type: "function" | "plotly";
  title?: string;
  domainX?: [number, number];
  domainY?: [number, number];
  data?: unknown[];
  layout?: {
    xaxis?: Record<string, unknown>;
    yaxis?: Record<string, unknown>;
    [key: string]: unknown;
  };
  config?: Record<string, unknown>;
  [key: string]: unknown;
}

export default function ChartBlock({ language, code }: ChartBlockProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy chart specification:", err);
    }
  };

  const [debouncedCode, setDebouncedCode] = useState(code);

  React.useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedCode(code);
    }, 600);
    return () => clearTimeout(timer);
  }, [code]);

  // Run JSON validation and schema routing inside useMemo to avoid cascading renders
  const { parsedData, mathExpressions, error } = useMemo(() => {
    let json: unknown;
    try {
      json = JSON.parse(debouncedCode);
    } catch (err) {
      // Fallback: Attempt parsing with jsonrepair
      try {
        const repaired = jsonrepair(debouncedCode);
        json = JSON.parse(repaired);
      } catch (repairErr) {
        return {
          parsedData: null,
          mathExpressions: [],
          error: `JSON Parsing Failed: ${(err as Error).message} (Repair failed: ${(repairErr as Error).message})`,
        };
      }
    }

    if (!json || typeof json !== "object") {
      return {
        parsedData: null,
        mathExpressions: [],
        error: "Invalid JSON: Specification must be a JSON object.",
      };
    }

    let obj = json as Record<string, unknown>;

    // Wrap raw Plotly trace type schemas into standard Plotly schema definitions
    if (typeof obj.type === "string" && PLOTLY_TRACE_TYPES.has(obj.type)) {
      const { title, layout, config, ...traceData } = obj;
      obj = {
        type: "plotly",
        data: [traceData],
        layout: layout && typeof layout === "object" ? { title, ...layout } : { title },
        config: config && typeof config === "object" ? config : undefined,
      };
    }

    // 1. Math/Function plot schema detection
    const isMath =
      obj.type === "function" || obj.type === "math" || "expression" in obj || "expressions" in obj;

    if (isMath) {
      try {
        let expressionsInput: { expression: string; color: string; label: string }[] = [];
        if (obj.expression) {
          if (typeof obj.expression !== "string") {
            throw new Error("Mathematical expression must be a non-empty string.");
          }
          expressionsInput = [
            {
              expression: obj.expression,
              color: typeof obj.color === "string" ? obj.color : "#b56a37",
              label: typeof obj.label === "string" ? obj.label : obj.expression,
            },
          ];
        } else if (Array.isArray(obj.expressions)) {
          expressionsInput = obj.expressions.map((e: unknown) => {
            if (typeof e === "string") {
              return { expression: e, color: "#b56a37", label: e };
            }
            if (e && typeof e === "object") {
              const eObj = e as Record<string, unknown>;
              const expr = eObj.expression;
              if (typeof expr !== "string") {
                throw new Error("Mathematical expression must be a non-empty string.");
              }
              return {
                expression: expr,
                color: typeof eObj.color === "string" ? eObj.color : "#b56a37",
                label: typeof eObj.label === "string" ? eObj.label : expr,
              };
            }
            throw new Error("Invalid expression item format.");
          });
        } else {
          return {
            parsedData: null,
            mathExpressions: [],
            error:
              "Math Chart Specification must contain either an 'expression' or 'expressions' array.",
          };
        }

        // Validate and pre-parse all math expressions to catch syntax errors
        const parsedExprs: ParsedMathExpression[] = expressionsInput.map((item) => {
          const prepped = preprocessExpression(item.expression);
          const tokens = tokenize(prepped);
          const rpn = parseToRpn(tokens);
          return {
            rpn,
            color: item.color,
            label: item.label,
          };
        });

        const parsed: ParsedChartSchema = {
          ...obj,
          type: "function",
        };

        return {
          parsedData: parsed,
          mathExpressions: parsedExprs,
          error: null,
        };
      } catch (err) {
        return {
          parsedData: null,
          mathExpressions: [],
          error: `Math Parsing Exception: ${(err as Error).message}`,
        };
      }
    }

    // 2. Plotly schema detection
    const isPlotly = obj.type === "plotly" || "data" in obj;
    if (isPlotly) {
      if (!Array.isArray(obj.data)) {
        return {
          parsedData: null,
          mathExpressions: [],
          error: "Plotly Specification must contain an array in the 'data' property.",
        };
      }

      const parsed: ParsedChartSchema = {
        ...obj,
        type: "plotly",
      };

      return {
        parsedData: parsed,
        mathExpressions: [],
        error: null,
      };
    }

    // 3. Fallback for valid JSON but unrecognized schema
    return {
      parsedData: null,
      mathExpressions: [],
      error: "Unrecognized Chart Schema: JSON must specify either a Math Plot or a Plotly Chart.",
    };
  }, [debouncedCode]);

  // Domain bounding boxes
  const domainX = parsedData?.domainX || DEFAULT_DOMAIN_X;
  const domainY = parsedData?.domainY || DEFAULT_DOMAIN_Y;
  const domainXMin = domainX[0];
  const domainXMax = domainX[1];
  const domainYMin = domainY[0];
  const domainYMax = domainY[1];

  // Controlled viewport state
  const [viewBoxX, setViewBoxX] = useState<[number, number]>(domainX);
  const [viewBoxY, setViewBoxY] = useState<[number, number]>(domainY);
  const [resetKey, setResetKey] = useState(0);
  const [activeMode, setActiveMode] = useState<"pan" | "zoomArea" | "selectBox" | "lasso">("pan");

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      setViewBoxX([domainXMin, domainXMax]);
      setViewBoxY([domainYMin, domainYMax]);
      setResetKey((prev) => prev + 1);
    });

    return () => cancelAnimationFrame(frame);
  }, [domainXMin, domainXMax, domainYMin, domainYMax]);

  // Fallback rendering structure (rendered exactly like MindVault's CodeBlock)
  if (error) {
    return (
      <div className="chart-block-wrapper error-fallback">
        <div className="chart-block-header error">
          <div className="chart-block-title-container">
            <TbAlertCircle size={16} color="#c94a4a" />
            <span className="chart-block-title text-error">Chart Rendering Fallback</span>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
            <span
              className="code-block-lang"
              style={{
                color: "#c94a4a",
                fontSize: "0.72rem",
                opacity: 0.65,
                textTransform: "lowercase",
                fontWeight: 500,
              }}
            >
              {language}
            </span>
            <button
              type="button"
              className="code-block-copy-btn"
              onClick={handleCopy}
              aria-label={copied ? "Copied" : "Copy specification"}
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
        </div>
        <div className="chart-block-fallback-info">
          <span className="error-message">{error}</span>
        </div>
        <div className="code-block-wrapper" style={{ margin: 0, borderRadius: 0, border: "none" }}>
          <pre>
            <code>{code}</code>
          </pre>
        </div>
      </div>
    );
  }

  if (!parsedData) {
    return (
      <div className="chart-block-wrapper loading">
        <div className="chart-block-body" style={{ minHeight: "150px" }}>
          <div className="dot-pulse-container">
            <span className="dot-pulse" />
            <span className="dot-pulse delay-1" />
            <span className="dot-pulse delay-2" />
          </div>
        </div>
      </div>
    );
  }

  // Dynamic axis grid lines & label calculation
  const getTickStep = (domain: [number, number]): number => {
    const range = Math.abs(domain[1] - domain[0]);
    if (range <= 12) return 1;
    if (range <= 30) return 2;
    if (range <= 60) return 5;
    if (range <= 150) return 10;
    if (range <= 300) return 20;
    if (range <= 600) return 50;
    if (range <= 1500) return 100;
    return Math.ceil(range / 10);
  };

  const stepX = getTickStep(viewBoxX);
  const stepY = getTickStep(viewBoxY);

  const formatLabel = (n: number) => {
    const rounded = Math.round(n * 10000) / 10000;
    return rounded.toString();
  };

  // Zoom in by 30% around center
  const handleZoomIn = () => {
    setViewBoxX((prev) => {
      const mid = (prev[0] + prev[1]) / 2;
      const halfSpan = ((prev[1] - prev[0]) / 2) * 0.7;
      return [mid - halfSpan, mid + halfSpan];
    });
    setViewBoxY((prev) => {
      const mid = (prev[0] + prev[1]) / 2;
      const halfSpan = ((prev[1] - prev[0]) / 2) * 0.7;
      return [mid - halfSpan, mid + halfSpan];
    });
  };

  // Zoom out by 30% around center
  const handleZoomOut = () => {
    setViewBoxX((prev) => {
      const mid = (prev[0] + prev[1]) / 2;
      const halfSpan = ((prev[1] - prev[0]) / 2) * 1.43;
      return [mid - halfSpan, mid + halfSpan];
    });
    setViewBoxY((prev) => {
      const mid = (prev[0] + prev[1]) / 2;
      const halfSpan = ((prev[1] - prev[0]) / 2) * 1.43;
      return [mid - halfSpan, mid + halfSpan];
    });
  };

  // Autoscale Y based on math expressions evaluated across current viewBoxX range
  const handleAutoscale = () => {
    let minY = Infinity;
    let maxY = -Infinity;
    const steps = 100;
    const xStart = viewBoxX[0];
    const xEnd = viewBoxX[1];
    const xStep = (xEnd - xStart) / steps;

    for (let i = 0; i <= steps; i++) {
      const x = xStart + i * xStep;
      for (const item of mathExpressions) {
        try {
          const y = evaluateRpn(item.rpn, x);
          if (!isNaN(y) && isFinite(y)) {
            if (y < minY) minY = y;
            if (y > maxY) maxY = y;
          }
        } catch {
          // ignore
        }
      }
    }

    if (minY !== Infinity && maxY !== -Infinity) {
      const spanY = maxY - minY;
      const padY = spanY === 0 ? 1 : spanY * 0.15; // 15% vertical safety padding
      setViewBoxY([minY - padY, maxY + padY]);
    }
  };

  // Reset view to original domain properties
  const handleReset = () => {
    setViewBoxX(domainX);
    setViewBoxY(domainY);
    setResetKey((prev) => prev + 1);
  };

  // Securely evaluate coordinates for rendering x
  const evaluateForX = (rpn: string[], xVal: number) => {
    try {
      const res = evaluateRpn(rpn, xVal);
      return isNaN(res) || !isFinite(res) ? NaN : res;
    } catch {
      return NaN;
    }
  };

  // Securely resolve title for Plotly visualizations without type unsafe access
  const resolvePlotlyTitle = (): string => {
    const layout = parsedData.layout;
    if (!layout) return "";
    const title = layout.title;
    if (typeof title === "string") return title;
    if (title && typeof title === "object") {
      const titleObj = title as Record<string, unknown>;
      if (typeof titleObj.text === "string") return titleObj.text;
    }
    return "";
  };

  const chartTitle =
    parsedData.type === "function"
      ? parsedData.title || "Mathematical Equation"
      : parsedData.title || resolvePlotlyTitle() || "Data Visualization";

  return (
    <div className="chart-block-wrapper">
      <div className="chart-block-header">
        <div className="chart-block-title-container">
          {parsedData.type === "function" ? <TbMath size={16} /> : <TbChartArea size={16} />}
          <span className="chart-block-title">{chartTitle}</span>
        </div>
        <button
          type="button"
          className="code-block-copy-btn"
          onClick={handleCopy}
          aria-label={copied ? "Copied" : "Copy specification"}
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

      <div className="chart-block-body">
        {parsedData.type === "function" ? (
          <div
            className="mafs-container"
            style={{ width: "100%", display: "flex", flexDirection: "column", gap: "12px" }}
          >
            <div
              className="mafs-plot-area"
              style={{
                width: "100%",
                borderRadius: "8px",
                overflow: "hidden",
                border: "1px solid rgba(188, 108, 37, 0.08)",
                position: "relative",
              }}
            >
              {/* Premium Floating Glassmorphic Toolbar Overlay */}
              <div className="mafs-toolbar">
                <button
                  type="button"
                  className={`mafs-toolbar-btn ${activeMode === "zoomArea" ? "active" : ""}`}
                  onClick={() => setActiveMode("zoomArea")}
                  title="Zoom Area"
                  aria-label="Zoom Area"
                >
                  <TbSearch size={15} />
                </button>
                <button
                  type="button"
                  className={`mafs-toolbar-btn ${activeMode === "pan" ? "active" : ""}`}
                  onClick={() => setActiveMode("pan")}
                  title="Pan Tool"
                  aria-label="Pan Tool"
                >
                  <TbArrowsMove size={15} />
                </button>
                <button
                  type="button"
                  className={`mafs-toolbar-btn ${activeMode === "selectBox" ? "active" : ""}`}
                  onClick={() => setActiveMode("selectBox")}
                  title="Box Select"
                  aria-label="Box Select"
                >
                  <TbRectangle size={15} />
                </button>
                <button
                  type="button"
                  className={`mafs-toolbar-btn ${activeMode === "lasso" ? "active" : ""}`}
                  onClick={() => setActiveMode("lasso")}
                  title="Lasso Select"
                  aria-label="Lasso Select"
                >
                  <TbLasso size={15} />
                </button>

                <div className="mafs-toolbar-divider" />

                <button
                  type="button"
                  className="mafs-toolbar-btn"
                  onClick={handleZoomIn}
                  title="Zoom In"
                  aria-label="Zoom In"
                >
                  <TbPlus size={15} />
                </button>
                <button
                  type="button"
                  className="mafs-toolbar-btn"
                  onClick={handleZoomOut}
                  title="Zoom Out"
                  aria-label="Zoom Out"
                >
                  <TbMinus size={15} />
                </button>
                <button
                  type="button"
                  className="mafs-toolbar-btn"
                  onClick={handleAutoscale}
                  title="Autoscale (Fit Y-axis)"
                  aria-label="Autoscale"
                >
                  <TbArrowsMaximize size={15} />
                </button>
                <button
                  type="button"
                  className="mafs-toolbar-btn"
                  onClick={handleReset}
                  title="Reset View (Home)"
                  aria-label="Reset View"
                >
                  <TbHome size={15} />
                </button>
              </div>

              <MafsAny
                key={resetKey}
                viewBox={{ x: viewBoxX, y: viewBoxY }}
                zoom={true}
                pan={true}
                height={300}
                width="auto"
                theme={{ background: "transparent" }}
              >
                <Coordinates.Cartesian
                  xAxis={{
                    lines: stepX,
                    labels: formatLabel,
                  }}
                  yAxis={{
                    lines: stepY,
                    labels: formatLabel,
                  }}
                />
                {mathExpressions.map((item, idx) => (
                  <Plot.OfX key={idx} y={(x) => evaluateForX(item.rpn, x)} color={item.color} />
                ))}
              </MafsAny>
            </div>
            {mathExpressions.length > 0 && (
              <div
                className="mafs-legend"
                style={{
                  display: "flex",
                  flexWrap: "wrap",
                  gap: "12px",
                  fontSize: "0.78rem",
                  padding: "0 4px",
                }}
              >
                {mathExpressions.map((item, idx) => (
                  <div key={idx} style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                    <span
                      style={{
                        display: "inline-block",
                        width: "12px",
                        height: "4px",
                        backgroundColor: item.color,
                        borderRadius: "2px",
                      }}
                    />
                    <span style={{ fontWeight: 500, color: "rgba(27, 26, 23, 0.75)" }}>
                      {item.label}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : (
          <Suspense
            fallback={
              <div
                className="plotly-lazy-loading"
                style={{
                  display: "flex",
                  justifyContent: "center",
                  alignItems: "center",
                  minHeight: "300px",
                  width: "100%",
                }}
              >
                <div className="dot-pulse-container">
                  <span className="dot-pulse" />
                  <span className="dot-pulse delay-1" />
                  <span className="dot-pulse delay-2" />
                </div>
              </div>
            }
          >
            <PlotlyRenderer
              data={parsedData.data || []}
              layout={parsedData.layout}
              config={parsedData.config}
            />
          </Suspense>
        )}
      </div>
    </div>
  );
}
export type { ChartBlockProps };
