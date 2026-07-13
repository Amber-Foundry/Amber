import React, { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { browseImportPdf, buildImportStartInput, startImportJob } from "../services/import";
import { toAppError } from "../services/ipcResult";
import { listVaults } from "../services/vaults";
import ImportHubStyles from "../style/components/ImportHub.module.css";
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
}

export const ExtractionDropdown: React.FC<ExtractionDropdownProps> = ({
  options,
  selectedValue,
  placeholder = "Select an option",
  onChange,
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
        onClick={() => setIsOpen(!isOpen)}
      >
        {selectedOption ? selectedOption.label : placeholder}
      </button>

      {isOpen && (
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

function isPdfPath(path: string): boolean {
  return path.toLowerCase().endsWith(".pdf");
}

export default function ImportHub() {
  const [selectedFramework, setSelectedFramework] = useState<string | number | null>(null);
  const [selectedVault, setSelectedVault] = useState<string | number | null>(null);
  const [vaultOptions, setVaultOptions] = useState<ExtractionModeOption[]>([]);
  const [vaultsLoading, setVaultsLoading] = useState(true);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [jobLogRefreshKey, setJobLogRefreshKey] = useState(0);

  const isReady = Boolean(selectedFramework && selectedVault) && !starting;

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void listVaults()
        .then((vaults) => {
          if (cancelled) return;
          setVaultOptions(vaults.map((vault) => ({ label: vault.name, value: vault.id })));
        })
        .catch((error) => {
          if (cancelled) return;
          setStartError(toAppError(error).message || "Failed to load vaults");
        })
        .finally(() => {
          if (!cancelled) setVaultsLoading(false);
        });
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);

  const beginImport = useCallback(
    async (filePath: string) => {
      if (!selectedFramework || !selectedVault || starting) return;
      if (!isPdfPath(filePath)) {
        setStartError("Only PDF files are supported for import.");
        return;
      }

      setStarting(true);
      setStartError(null);
      try {
        const input = await buildImportStartInput({
          filePath,
          targetVaultId: String(selectedVault),
          useLlmExtraction: selectedFramework === "ai",
        });
        await startImportJob(input);
        setJobLogRefreshKey((key) => key + 1);
      } catch (error) {
        setStartError(toAppError(error).message);
      } finally {
        setStarting(false);
      }
    },
    [selectedFramework, selectedVault, starting]
  );

  const handleBrowse = useCallback(async () => {
    if (!isReady) return;
    try {
      const path = await browseImportPdf();
      if (!path) return;
      await beginImport(path);
    } catch (error) {
      setStartError(toAppError(error).message);
    }
  }, [beginImport, isReady]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void getCurrentWindow()
      .onDragDropEvent((event) => {
        if (event.payload.type !== "drop") return;
        if (!selectedFramework || !selectedVault) return;
        const pdfPath = event.payload.paths.find(isPdfPath);
        if (!pdfPath) {
          setStartError("Only PDF files are supported for import.");
          return;
        }
        void beginImport(pdfPath);
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
  }, [beginImport, selectedFramework, selectedVault]);

  return (
    <div className="pane pane-left">
      <div className="pane-header">
        <span className="sidebar-subtitle">Import</span>
      </div>
      <div className="dropdown-container">
        <ExtractionDropdown
          options={ExtractionModeOptions}
          selectedValue={selectedFramework}
          placeholder="Select Extraction Mode"
          onChange={setSelectedFramework}
        />
        <ExtractionDropdown
          options={vaultOptions}
          selectedValue={selectedVault}
          placeholder={vaultsLoading ? "Loading vaults…" : "Select Vault"}
          onChange={setSelectedVault}
        />
      </div>
      <div
        className={`${ImportHubStyles.importDropZone} ${!isReady ? ImportHubStyles.importDropZoneDisabled : ""}`}
        aria-disabled={!isReady}
      >
        <p>
          {starting ? (
            "Starting import…"
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

      <ImportJobLog refreshKey={jobLogRefreshKey} />
    </div>
  );
}
