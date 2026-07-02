export type HardwareProfile = {
  ramGb: number;
  vramGb: number | null;
  platform: string;
  gpuName: string | null;
  tier: "light" | "standard" | "quality";
};

export type RecommendedStack = {
  id: string;
  label: "Best quality" | "Balanced" | "Fastest";
  modelName: string;
  paramsB: number;
  quant: string;
  vramEstimateGb: number;
  speedTokS: number;
  fitType: "Full GPU" | "Partial" | "CPU";
  score: number;
  license: string;
  embeddingTier: string; // paired embedding tier from M2.3 registry
  embeddingModelId: string;
};

const USE_MOCK = import.meta.env.VITE_USE_MODEL_SETUP_MOCK !== "false"; // default true until M2.8 IPC lands

export async function probeHardware(): Promise<HardwareProfile> {
  if (!USE_MOCK) {
    // stub — M2.8 model_probe_hardware
  }
  return {
    ramGb: 16,
    vramGb: 8,
    platform: "Windows",
    gpuName: "NVIDIA GeForce RTX 3060",
    tier: "standard",
  };
}
export async function getRecommendedStacks(): Promise<RecommendedStack[]> {
  if (!USE_MOCK) {
    // stub — M2.8 model_recommended_stacks
  }
  return [
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
}
export async function startStackDownload(_stackId: string): Promise<void> {
  // stub — M2.8 model_download_start
}
