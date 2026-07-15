import type { ImportExtractionPreview } from "../types/generated/ImportExtractionPreview";
import type { ImportJobStatus } from "../types/generated/ImportJobStatus";
import type { ImportStartJobInput } from "../types/generated/ImportStartJobInput";
import { invokeTyped, chatExtractPdfText } from "../ipc";
import type { ChatPdfExtraction } from "../ipc";
import {
  getApiKey,
  getLmStudioEndpoint,
  getLlmModel,
  getLlmProvider,
  getOllamaEndpoint,
} from "../utils/settings";
import { unwrapIpcResult } from "./ipcResult.ts";

export type { ImportExtractionPreview } from "../types/generated/ImportExtractionPreview";
export type { ImportJobStatus } from "../types/generated/ImportJobStatus";
export type { ImportStartJobInput } from "../types/generated/ImportStartJobInput";

const USE_MOCK = import.meta.env.VITE_USE_IMPORT_MOCK !== "false";

const MOCK_STATUS: ImportJobStatus = {
  id: "mock-job-id",
  status: "staged",
  sourceName: "mock-source.pdf",
  targetVaultId: "vault_root_graph",
  changesetId: null,
  nodeCount: 0,
  totalPages: 100,
  currentPage: 100,
  digitalPages: 80,
  ocrPages: 15,
  hybridPages: 5,
  avgOcrConfidence: 0.95,
  tablesDetectedUnpreserved: 2,
  extractionPath: "hybrid",
  rasterizationDpi: 300,
  error: null,
};

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

export async function buildImportStartInput(opts: {
  filePath: string;
  targetVaultId: string;
  useLlmExtraction: boolean;
  rasterizationDpi?: number;
}): Promise<ImportStartJobInput> {
  const rasterizationDpi = opts.rasterizationDpi ?? 300;
  if (!opts.useLlmExtraction) {
    return {
      filePath: opts.filePath,
      targetVaultId: opts.targetVaultId,
      rasterizationDpi,
      useLlmExtraction: false,
      provider: null,
      endpoint: null,
      model: null,
    };
  }

  const { provider, endpoint, model } = await resolveLlmConfig();
  return {
    filePath: opts.filePath,
    targetVaultId: opts.targetVaultId,
    rasterizationDpi,
    useLlmExtraction: true,
    provider,
    endpoint: endpoint || null,
    model: model || null,
  };
}

export async function browseImportPdf(): Promise<string | null> {
  if (USE_MOCK) {
    return "C:\\mock\\sample.pdf";
  }
  return unwrapIpcResult(invokeTyped<string | null>("import_browse_pdf"));
}

export async function chatAttachPdf(): Promise<ChatPdfExtraction | null> {
  const filePath = await browseImportPdf();
  if (!filePath) {
    return null;
  }
  if (USE_MOCK) {
    return {
      sourceName: filePath.split(/[/\\]/).pop() || "mock-source.pdf",
      pageCount: 1,
      text: "This is mock extracted text from the PDF attachment.",
      ocrConfidence: 1.0,
      needsOcrModels: false,
      promptInjectionFlagged: false,
      pageTokenEstimates: [20],
    };
  }
  return unwrapIpcResult(chatExtractPdfText(filePath));
}

export async function startImportJob(input: ImportStartJobInput): Promise<ImportJobStatus> {
  if (USE_MOCK) {
    MOCK_STATUS.sourceName = input.filePath.split(/[/\\]/).pop() || "mock-source.pdf";
    MOCK_STATUS.rasterizationDpi = input.rasterizationDpi;
    MOCK_STATUS.status = "staged";
    MOCK_STATUS.error = null;
    return { ...MOCK_STATUS };
  }
  return unwrapIpcResult(invokeTyped<ImportJobStatus>("import_start_job", { input }));
}

export async function getImportJobStatus(jobId: string): Promise<ImportJobStatus | null> {
  if (USE_MOCK) {
    return { ...MOCK_STATUS, id: jobId };
  }
  return unwrapIpcResult(invokeTyped<ImportJobStatus | null>("import_get_status", { jobId }));
}

export async function listImportJobs(limit = 20): Promise<ImportJobStatus[]> {
  if (USE_MOCK) {
    return [{ ...MOCK_STATUS }];
  }
  return unwrapIpcResult(invokeTyped<ImportJobStatus[]>("import_list_jobs", { limit }));
}

export async function cancelImportJob(): Promise<void> {
  if (USE_MOCK) {
    MOCK_STATUS.status = "failed";
    MOCK_STATUS.error = "Cancelled by user";
    return;
  }
  await unwrapIpcResult(invokeTyped<void>("import_cancel_job"));
}

export async function startOcrModelDownload(): Promise<void> {
  if (USE_MOCK) {
    MOCK_STATUS.status = "failed";
    MOCK_STATUS.error = "OCR models ready — retry import.";
    return;
  }
  await unwrapIpcResult(invokeTyped<void>("ocr_download_models"));
}

export async function getImportExtractionPreview(jobId: string): Promise<ImportExtractionPreview> {
  if (USE_MOCK) {
    return {
      jobId,
      sourceName: MOCK_STATUS.sourceName,
      markdown: "# Mock extraction\n\nSample markdown from Fast Import.",
      status: MOCK_STATUS.status,
      totalPages: MOCK_STATUS.totalPages,
      digitalPages: MOCK_STATUS.digitalPages,
      ocrPages: MOCK_STATUS.ocrPages,
      hybridPages: MOCK_STATUS.hybridPages,
      changesetId: MOCK_STATUS.changesetId,
    };
  }
  return unwrapIpcResult(
    invokeTyped<ImportExtractionPreview>("import_get_extraction_preview", { jobId })
  );
}
