const USE_MOCK = import.meta.env.VITE_USE_IMPORT_MOCK !== "false";

//TEMP TYPES TO BE REPLACED ON MERGE WITH 2.4 BACKEND

export type ImportJobStatus = {
  id: string;
  status: string;
  sourceName: string;
  totalPages: number;
  digitalPages: number;
  ocrPages: number;
  hybridPages: number;
  avgOcrConfidence: number;
  tablesDetectedUnpreserved: number;
  extractionPath: string;
  rasterizationDPI: number;
  error: string;
};

const MOCK_STATUS: ImportJobStatus = {
  id: "mock-job-id",
  status: "completed",
  sourceName: "mock-source",
  totalPages: 100,
  digitalPages: 80,
  ocrPages: 15,
  hybridPages: 5,
  avgOcrConfidence: 0.95,
  tablesDetectedUnpreserved: 2,
  extractionPath: "mock-extraction-path",
  rasterizationDPI: 300,
  error: "",
};

export async function startImportJob(
  sourceName: string,
  sourcePath: string,
  extractionPath: string,
  rasterizationDPI: number
): Promise<ImportJobStatus> {
  if (USE_MOCK) {
    MOCK_STATUS.sourceName = sourceName;
    MOCK_STATUS.extractionPath = extractionPath;
    MOCK_STATUS.rasterizationDPI = rasterizationDPI;
    return MOCK_STATUS;
  } else {
    // Implement the actual API call to start the import job here
    throw new Error("startImportJob not implemented for non-mock mode" + sourcePath);
  }
}

export async function getImportJobStatus(jobId: string): Promise<ImportJobStatus> {
  if (USE_MOCK) {
    return MOCK_STATUS;
  } else {
    // Implement the actual API call to get the import job status here
    throw new Error("getImportJobStatus not implemented for non-mock mode" + jobId);
  }
}
export async function listImportJobs(): Promise<ImportJobStatus[]> {
  if (USE_MOCK) {
    return [MOCK_STATUS, MOCK_STATUS, MOCK_STATUS];
  } else {
    // Implement the actual API call to list the import jobs here
    throw new Error("listImportJobs not implemented for non-mock mode");
  }
}

export async function cancelImportJob(jobId: string): Promise<void> {
  if (!USE_MOCK) {
    // Implement the actual API call to cancel the import job here
    throw new Error("cancelImportJob not implemented for non-mock mode" + jobId);
  }
}

export async function startOcrModelDownload(jobId: string): Promise<void> {
  if (USE_MOCK) {
    MOCK_STATUS.status = "completed";
    MOCK_STATUS.error = "";
    return;
  } else {
    throw new Error("startOcrModelDownload not implemented for non-mock mode" + jobId);
  }
}
