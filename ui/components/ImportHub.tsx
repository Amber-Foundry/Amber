import React, { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { browseImportPdf, buildImportStartInput, startImportJob } from "../services/import";
import type { ImportStartJobInput } from "../types/generated/ImportStartJobInput";
import { toAppError } from "../services/ipcResult";
import { listVaults } from "../services/vaults";
import ImportHubStyles from "../style/components/ImportHub.module.css";
import {
  getImportExtractionMode,
  setImportExtractionMode,
  setImportJobParam,
  type ImportExtractionMode,
} from "../utils/settings";
import { ROOT_GRAPH_VAULT_ID } from "../utils/importJobStatus";
import ImportJobLog from "./ImportJobLog";

export interface ExtractionModeOption {
  label: string;
  value: string | number;
}
interface ExtractionDropdownProps {
  options: ExtractionModeOption[];
  selectedValue: string | number | null;
  placeholder?: string;
  onChange: (value: string | number) => void;
  disabled?: boolean;
}

export const ExtractionDropdown: React.FC<ExtractionDropdownProps> = ({
  options,
  selectedValue,
  placeholder = "Select an option",
  onChange,
  disabled = false,
}) => {
  const [isOpen, setIsOpen] = useState<boolean>(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const selectedOption = options.find((opt) => opt.value === selectedValue);

  useEffect(() => {
    const handleOutsideClick = (event: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    };

    document.addEventListener("mousedown", handleOutsideClick);
    return () => document.removeEventListener("mousedown", handleOutsideClick);
  }, []);

  const handleOptionClick = (value: string | number) => {
    onChange(value);
    setIsOpen(false);
  };

  return (
    <div className={ImportHubStyles.extractionDropdown} ref={dropdownRef}>
      <button
        className={ImportHubStyles.extractionDropdownHeader}
        type="button"
        disabled={disabled}
        onClick={() => {
          if (!disabled) setIsOpen(!isOpen);
        }}
      >
        {selectedOption ? selectedOption.label : placeholder}
      </button>

      {isOpen && !disabled && (
        <ul className={ImportHubStyles.extractionDropdownMenu}>
          {options.map((option) => (
            <li
              className={ImportHubStyles.extractionDropdownOption}
              key={option.value}
              onClick={() => handleOptionClick(option.value)}
            >
              {option.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

const ExtractionModeOptions: ExtractionModeOption[] = [
  { label: "AI Extraction", value: "ai" },
  { label: "Fast Import", value: "fast" },
];

const BUSY_IMPORT_MESSAGE =
  "An import is already in progress. Cancel it in the Job Log or wait for it to finish.";

function isPdfPath(path: string): boolean {
  return path.toLowerCase().endsWith(".pdf");
}

function vaultOptionLabel(name: string, id: string): string {
  if (id === ROOT_GRAPH_VAULT_ID) {
    return "Root Graph";
  }
  return name;
}

function sortVaultOptions(options: ExtractionModeOption[]): ExtractionModeOption[] {
  return [...options].sort((a, b) => {
    const aRoot = String(a.value) === ROOT_GRAPH_VAULT_ID ? 0 : 1;
    const bRoot = String(b.value) === ROOT_GRAPH_VAULT_ID ? 0 : 1;
    if (aRoot !== bRoot) return aRoot - bRoot;
    return String(a.label).localeCompare(String(b.label));
  });
}

export default function ImportHub({
  onOpenImportChangeset,
}: {
  onOpenImportChangeset?: (changesetId: string) => void;
}) {
  const [selectedFramework, setSelectedFramework] = useState<string | number | null>(() =>
    getImportExtractionMode()
  );
  const [selectedVault, setSelectedVault] = useState<string | number | null>(null);
  const [vaultOptions, setVaultOptions] = useState<ExtractionModeOption[]>([]);
  const [vaultsLoading, setVaultsLoading] = useState(true);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [jobLogRefreshKey, setJobLogRefreshKey] = useState(0);
  const [importBusy, setImportBusy] = useState(false);

  const isReady = Boolean(selectedFramework && selectedVault) && !starting && !importBusy;

  const handleFrameworkChange = useCallback((value: string | number) => {
    const mode: ImportExtractionMode = value === "ai" ? "ai" : "fast";
    setSelectedFramework(mode);
    setImportExtractionMode(mode);
  }, []);

  const handleActiveJobsChange = useCallback((hasActive: boolean) => {
    setImportBusy(hasActive);
    if (!hasActive) {
      setStartError((prev) => (prev === BUSY_IMPORT_MESSAGE ? null : prev));
    }
  }, []);

  const refreshVaultOptions = useCallback(async (): Promise<ExtractionModeOption[]> => {
    const vaults = await listVaults();
    const options = sortVaultOptions(
      vaults.map((vault) => ({
        label: vaultOptionLabel(vault.name, vault.id),
        value: vault.id,
      }))
    );
    setVaultOptions(options);
    setVaultsLoading(false);
    return options;
  }, []);

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void refreshVaultOptions()
        .then((options) => {
          if (cancelled) return;
          setSelectedVault((prev) => {
            if (prev == null) return prev;
            return options.some((opt) => opt.value === prev) ? prev : null;
          });
        })
        .catch((error) => {
          if (cancelled) return;
          setStartError(toAppError(error).message || "Failed to load vaults");
          setVaultsLoading(false);
        });
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [refreshVaultOptions]);

  // Re-fetch when the window regains focus so soft-deleted vaults drop out of the list.
  useEffect(() => {
    const onFocus = () => {
      void refreshVaultOptions()
        .then((options) => {
          setSelectedVault((prev) => {
            if (prev == null) return prev;
            return options.some((opt) => opt.value === prev) ? prev : null;
          });
        })
        .catch(() => {
          // Keep existing options on focus refresh failure.
        });
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refreshVaultOptions]);

  const beginImport = useCallback(
    async (filePath: string) => {
      if (!selectedFramework || !selectedVault || starting || importBusy) {
        if (importBusy) {
          setStartError(BUSY_IMPORT_MESSAGE);
        }
        return;
      }
      if (!isPdfPath(filePath)) {
        setStartError("Only PDF files are supported for import.");
        return;
      }

      setStarting(true);
      setStartError(null);
      try {
        const options = await refreshVaultOptions();
        const vaultId = String(selectedVault);
        if (!options.some((opt) => String(opt.value) === vaultId)) {
          setSelectedVault(null);
          setStartError("Vault no longer available — pick another");
          return;
        }
        const input = await buildImportStartInput({
          filePath,
          targetVaultId: vaultId,
          useLlmExtraction: selectedFramework === "ai",
        });
        const job = await startImportJob(input);
        setImportJobParam(job.id, input);
        setImportBusy(true);
        setJobLogRefreshKey((key) => key + 1);
      } catch (error) {
        const message = toAppError(error).message;
        if (message.toLowerCase().includes("already active")) {
          setStartError(BUSY_IMPORT_MESSAGE);
          setImportBusy(true);
        } else {
          setStartError(message);
        }
      } finally {
        setStarting(false);
      }
    },
    [importBusy, refreshVaultOptions, selectedFramework, selectedVault, starting]
  );
  const handleRetry = useCallback(
    async (input: ImportStartJobInput) => {
      if (starting || importBusy) return;
      setStarting(true);
      setStartError(null);
      try {
        const job = await startImportJob(input);
        setImportJobParam(job.id, input);
        setImportBusy(true);
        setJobLogRefreshKey((key) => key + 1);
      } catch (error) {
        setStartError(toAppError(error).message);
      } finally {
        setStarting(false);
      }
    },
    [importBusy, starting]
  );

  const handleBrowse = useCallback(async () => {
    if (!isReady) {
      if (importBusy) setStartError(BUSY_IMPORT_MESSAGE);
      return;
    }
    try {
      const path = await browseImportPdf();
      if (!path) return;
      await beginImport(path);
    } catch (error) {
      setStartError(toAppError(error).message);
    }
  }, [beginImport, importBusy, isReady]);

  const dragDropStateRef = useRef({
    selectedFramework,
    selectedVault,
    importBusy,
    starting,
    beginImport,
  });

  useEffect(() => {
    dragDropStateRef.current = {
      selectedFramework,
      selectedVault,
      importBusy,
      starting,
      beginImport,
    };
  }, [selectedFramework, selectedVault, importBusy, starting, beginImport]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void getCurrentWindow()
      .onDragDropEvent((event) => {
        if (event.payload.type !== "drop") return;
        const state = dragDropStateRef.current;
        if (
          !state.selectedFramework ||
          !state.selectedVault ||
          state.importBusy ||
          state.starting
        ) {
          if (state.importBusy) setStartError(BUSY_IMPORT_MESSAGE);
          return;
        }
        const pdfPath = event.payload.paths.find(isPdfPath);
        if (!pdfPath) {
          setStartError("Only PDF files are supported for import.");
          return;
        }
        void state.beginImport(pdfPath);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {
        // Non-Tauri / browser preview — drag-drop stays unavailable.
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className="pane pane-left">
      <div className="pane-header">
        <span className="sidebar-subtitle">Import</span>
      </div>
      <div className={ImportHubStyles.dropdownContainer}>
        <ExtractionDropdown
          options={ExtractionModeOptions}
          selectedValue={selectedFramework}
          placeholder="Select Extraction Mode"
          onChange={handleFrameworkChange}
        />
        <ExtractionDropdown
          options={vaultOptions}
          selectedValue={selectedVault}
          placeholder={vaultsLoading ? "Loading vaults…" : "Select Vault"}
          onChange={setSelectedVault}
        />
      </div>
      {selectedFramework === "fast" && (
        <p className={ImportHubStyles.importHint}>
          Fast Import stages chunked memory adds from text — not a full document archive. After
          staging, use View Extraction in the Job Log for the full extracted text.
        </p>
      )}
      {importBusy && (
        <p className={ImportHubStyles.importBusyBanner} role="status">
          {BUSY_IMPORT_MESSAGE}
        </p>
      )}
      <div
        className={`${ImportHubStyles.importDropZone} ${!isReady ? ImportHubStyles.importDropZoneDisabled : ""}`}
        aria-disabled={!isReady}
      >
        <p>
          {starting ? (
            "Starting import…"
          ) : importBusy ? (
            "Import in progress — cancel from the Job Log to start another"
          ) : isReady ? (
            <>
              Drag & drop files here or{" "}
              <span className={ImportHubStyles.browseBtn} onClick={() => void handleBrowse()}>
                browse
              </span>
            </>
          ) : vaultsLoading ? (
            "Loading vaults…"
          ) : vaultOptions.length === 0 ? (
            "Create a vault before importing"
          ) : (
            "Select an extraction mode and vault to enable import"
          )}
        </p>
      </div>

      {startError && <p className={ImportHubStyles.importError}>{startError}</p>}

      <div className="pane-header">
        <span className="sidebar-subtitle">Job Log</span>
      </div>

      <ImportJobLog
        refreshKey={jobLogRefreshKey}
        onOpenChangeset={onOpenImportChangeset}
        onActiveJobsChange={handleActiveJobsChange}
        onRetry={handleRetry}
      />
    </div>
  );
}
