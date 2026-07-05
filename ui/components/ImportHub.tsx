import React, { useState, useEffect, useRef } from "react";
import ImportHubStyles from "../style/components/ImportHub.module.css";
import ImportJobLog from "./ImportJobLog";
//TEMP IMPORT, REPLACE WITH GENERATED TYPES ON MERGE WITH 2.4 BACKEND
//import { ImportJobStatus } from "../services/import";

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

  // Find the label of the currently selected option
  const selectedOption = options.find((opt) => opt.value === selectedValue);

  // Close dropdown if user clicks outside of the element
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
      {/* Dropdown Header/Trigger */}
      <button
        className={ImportHubStyles.extractionDropdownHeader}
        type="button"
        onClick={() => setIsOpen(!isOpen)}
      >
        {selectedOption ? selectedOption.label : placeholder}
      </button>

      {/* Dropdown Menu Options */}
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

export default function ImportHub() {
  const ExtractionModeOptions: ExtractionModeOption[] = [
    { label: "AI Extraction", value: "ai" },
    { label: "Fast Import", value: "fast" },
  ];
  // Mock vault options for demonstration purposes. Replace with actual vault options as needed.
  const VaultOptions: ExtractionModeOption[] = [
    { label: "Vault A", value: "vault_a" },
    { label: "Vault B", value: "vault_b" },
  ];
  const [selectedFramework, setSelectedFramework] = useState<string | number | null>(null);
  const [selectedVault, setSelectedVault] = useState<string | number | null>(null);
  const isReady = Boolean(selectedFramework && selectedVault);
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
          options={VaultOptions}
          selectedValue={selectedVault}
          placeholder="Select Vault"
          onChange={setSelectedVault}
        />
      </div>
      <div
        className={`${ImportHubStyles.importDropZone} ${!isReady ? ImportHubStyles.importDropZoneDisabled : ""}`}
        aria-disabled={!isReady}
      >
        <p>
          {isReady ? (
            <>
              Drag & drop files here or{" "}
              <span
                className={ImportHubStyles.browseBtn}
                onClick={() => document.getElementById("file-input")?.click()}
              >
                browse
              </span>
            </>
          ) : (
            "Select an extraction mode and vault to enable import"
          )}
        </p>
        <input
          type="file"
          id="file-input"
          multiple
          disabled={!isReady}
          style={{ display: "none" }}
        />
      </div>

      <div className="pane-header">
        <span className="sidebar-subtitle">Job Log</span>
      </div>

      <ImportJobLog />
    </div>
  );
}
