import { useEffect, useMemo, useRef, useState } from "react";
import type { HardwareProfile, RecommendedStack } from "../services/modelSetup";
import { getRecommendedStacks, probeHardware, startStackDownload } from "../services/modelSetup";
import styles from "../style/components/ModelSetupPanel.module.css";

type ModelSetupPanelProps = {
  variant?: "onboarding" | "settings"; // minor layout tweaks only
  onStackSelected?: (stackId: string) => void;
};

type PanelState = "idle" | "loading" | "results" | "error";

const PREVIEW_STACKS: RecommendedStack[] = [
  {
    id: "best_quality",
    label: "Best quality",
    modelName: "Qwen2.5-7B-Instruct-Q4_K_M",
    paramsB: 7_000_000_000,
    quant: "FP16",
    vramEstimateGb: 8,
    speedTokS: 28,
    fitType: "Full GPU",
    score: 100,
    license: "Apache-2.0",
    embeddingTier: "quality",
    embeddingModelId: "nomic-embed-text-v1.5",
  },
  {
    id: "balanced",
    label: "Balanced",
    modelName: "Mistral-7B-Instruct-Q4_K_M",
    paramsB: 7_000_000_000,
    quant: "Q4_K_M",
    vramEstimateGb: 5,
    speedTokS: 42,
    fitType: "Partial",
    score: 88,
    license: "Apache-2.0",
    embeddingTier: "standard",
    embeddingModelId: "nomic-embed-text-v1.5",
  },
  {
    id: "fastest",
    label: "Fastest",
    modelName: "Phi-3.5-mini-instruct",
    paramsB: 3_800_000_000,
    quant: "Q4_K_M",
    vramEstimateGb: 3,
    speedTokS: 67,
    fitType: "CPU",
    score: 72,
    license: "MIT",
    embeddingTier: "light",
    embeddingModelId: "all-MiniLM-L6-v2",
  },
];

function tierLabel(tier: HardwareProfile["tier"]): string {
  if (tier === "quality") return "Best quality";
  if (tier === "standard") return "Balanced";
  return "Fastest";
}

function formatHardwareSummary(hardware: HardwareProfile): string {
  const gpu = hardware.gpuName || hardware.platform;
  return `${hardware.ramGb} GB RAM · ${hardware.vramGb === null ? "GPU not detected" : `${hardware.vramGb} GB VRAM`} · ${gpu}`;
}

function buildSubtitle(stacks: RecommendedStack[]): string {
  const reference = stacks[0];
  if (!reference) {
    return "Includes memory search model (auto-selected for your PC).";
  }
  return `Includes memory search model (auto-selected for your PC): ${reference.embeddingTier} • ${reference.embeddingModelId}`;
}

function StackCard({
  stack,
  onDownload,
  busy,
}: {
  stack: RecommendedStack;
  onDownload: (stackId: string) => void;
  busy: boolean;
}) {
  return (
    <article className={styles["model-setup-stack-card"]}>
      <div className={styles["model-setup-stack-card-top"]}>
        <span className={styles["model-setup-stack-label"]}>{stack.label}</span>
        <span className={styles["model-setup-stack-score"]}>Score {stack.score}</span>
      </div>
      <h3 className={styles["model-setup-stack-model"]}>{stack.modelName}</h3>
      <dl className={styles["model-setup-stack-metrics"]}>
        <div>
          <dt>VRAM</dt>
          <dd>{`${stack.vramEstimateGb} GB`}</dd>
        </div>
        <div>
          <dt>Speed</dt>
          <dd>{`${stack.speedTokS} tok/s`}</dd>
        </div>
        <div>
          <dt>Fit</dt>
          <dd>{stack.fitType}</dd>
        </div>
      </dl>
      <button
        type="button"
        className={styles["model-setup-primary-button"]}
        onClick={() => onDownload(stack.id)}
        disabled={busy}
      >
        {busy ? "Preparing…" : "Download and continue"}
      </button>
    </article>
  );
}

function ModelSetupPanel({ variant = "settings", onStackSelected }: ModelSetupPanelProps) {
  const [phase, setPhase] = useState<PanelState>("idle");
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [stacks, setStacks] = useState<RecommendedStack[]>([]);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [toastMessage, setToastMessage] = useState<string | null>(null);
  const [busyStackId, setBusyStackId] = useState<string | null>(null);
  const [isExpanded, setIsExpanded] = useState(true);

  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const orderedStacks = useMemo(() => stacks.slice(0, 3), [stacks]);
  const subtitle = useMemo(() => buildSubtitle(orderedStacks), [orderedStacks]);

  async function runProbe() {
    setPhase("loading");
    setErrorMessage(null);
    setToastMessage(null);
    try {
      const [nextHardware, nextStacks] = await Promise.all([
        probeHardware(),
        getRecommendedStacks(),
      ]);
      if (!mountedRef.current) return;
      setHardware(nextHardware);
      setStacks(nextStacks);
      setPhase("results");
    } catch (error) {
      if (!mountedRef.current) return;
      setErrorMessage(error instanceof Error ? error.message : String(error));
      setPhase("error");
    }
  }

  async function handleDownload(stackId: string) {
    setBusyStackId(stackId);
    setToastMessage(null);
    try {
      await startStackDownload(stackId);
      if (!mountedRef.current) return;
      setToastMessage("Coming soon: download IPC is not wired yet.");
      setBusyStackId(null);
      onStackSelected?.(stackId);
    } catch (error) {
      if (!mountedRef.current) return;
      setToastMessage(error instanceof Error ? error.message : String(error));
      setBusyStackId(null);
    }
  }

  if (phase === "idle") {
    return (
      <section
        className={`${styles["model-setup-panel"]} ${styles[`variant-${variant}`]}`}
        aria-label="Model setup"
      >
        <button
          type="button"
          className={styles["model-setup-hero-button"]}
          onClick={() => void runProbe()}
        >
          Check my hardware and best local models
        </button>
      </section>
    );
  }

  if (phase === "loading") {
    return (
      <section
        className={`${styles["model-setup-panel"]} ${styles[`variant-${variant}`]}`}
        aria-busy="true"
        aria-label="Model setup scan"
      >
        <div className={styles["model-setup-status"]}>
          <span className={styles["model-setup-spinner"]} aria-hidden="true" />
          <span>Scanning your PC…</span>
        </div>
      </section>
    );
  }

  if (phase === "error") {
    return (
      <section
        className={`${styles["model-setup-panel"]} ${styles[`variant-${variant}`]}`}
        role="alert"
        aria-label="Model setup failed"
      >
        <div className={styles["model-setup-error"]}>
          <p className={styles["model-setup-error-title"]}>Couldn’t scan this machine</p>
          <p className={styles["model-setup-error-copy"]}>
            {errorMessage || "The hardware probe failed. Try again to get a fresh recommendation."}
          </p>
          <button
            type="button"
            className={styles["model-setup-primary-button"]}
            onClick={() => void runProbe()}
          >
            Retry scan
          </button>
        </div>
      </section>
    );
  }

  return (
    <section
      className={`${styles["model-setup-panel"]} ${styles[`variant-${variant}`]}`}
      aria-label="Model recommendations"
    >
      {toastMessage ? <p className={styles["model-setup-toast"]}>{toastMessage}</p> : null}
      <div className={styles["model-setup-summary"]}>
        <div className={styles["model-setup-summary-header"]}>
          <div>
            <span className={styles["model-setup-summary-label"]}>Hardware summary</span>
            <p className={styles["model-setup-summary-copy"]}>
              {hardware ? formatHardwareSummary(hardware) : "Hardware unavailable"}
            </p>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
            <span className={styles["model-setup-tier-badge"]}>
              {hardware ? tierLabel(hardware.tier) : "Recommended"}
            </span>
            <button
              type="button"
              className={styles["model-setup-toggle-button"]}
              onClick={() => setIsExpanded((prev) => !prev)}
              aria-expanded={isExpanded}
            >
              {isExpanded ? "Hide recommendations ▲" : "Show recommendations ▼"}
            </button>
          </div>
        </div>
      </div>

      {isExpanded && (
        <>
          <p className={styles["model-setup-subtitle"]}>{subtitle}</p>

          <div className={styles["model-setup-stack-grid"]}>
            {orderedStacks.map((stack) => (
              <StackCard
                key={stack.id}
                stack={stack}
                onDownload={handleDownload}
                busy={busyStackId === stack.id}
              />
            ))}
          </div>
        </>
      )}
    </section>
  );
}

export function ModelSetupPanelPreview() {
  return (
    <div className={styles["model-setup-preview-page"]}>
      <section className={styles["model-setup-preview-block"]}>
        <h2>Idle</h2>
        <ModelSetupPanel variant="settings" />
      </section>
      <section className={styles["model-setup-preview-block"]}>
        <h2>Loading</h2>
        <div
          className={`${styles["model-setup-panel"]} ${styles["variant-settings"]}`}
          aria-busy="true"
        >
          <div className={styles["model-setup-status"]}>
            <span className={styles["model-setup-spinner"]} aria-hidden="true" />
            <span>Scanning your PC…</span>
          </div>
        </div>
      </section>
      <section className={styles["model-setup-preview-block"]}>
        <h2>Results</h2>
        <div className={`${styles["model-setup-panel"]} ${styles["variant-settings"]}`}>
          <div className={styles["model-setup-summary"]}>
            <div>
              <span className={styles["model-setup-summary-label"]}>Hardware summary</span>
              <p className={styles["model-setup-summary-copy"]}>
                32 GB RAM · 12 GB VRAM · Apple M2 Pro
              </p>
            </div>
            <span className={styles["model-setup-tier-badge"]}>Best quality</span>
          </div>
          <p className={styles["model-setup-subtitle"]}>{buildSubtitle(PREVIEW_STACKS)}</p>
          <div className={styles["model-setup-stack-grid"]}>
            {PREVIEW_STACKS.map((stack) => (
              <StackCard key={stack.id} stack={stack} onDownload={() => {}} busy={false} />
            ))}
          </div>
        </div>
      </section>
      <section className={styles["model-setup-preview-block"]}>
        <h2>Error</h2>
        <div
          className={`${styles["model-setup-panel"]} ${styles["variant-settings"]}`}
          role="alert"
        >
          <div className={styles["model-setup-error"]}>
            <p className={styles["model-setup-error-title"]}>Couldn’t scan this machine</p>
            <p className={styles["model-setup-error-copy"]}>
              The hardware probe failed. Try again to get a fresh recommendation.
            </p>
            <button type="button" className={styles["model-setup-primary-button"]}>
              Retry scan
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}

export default ModelSetupPanel;
