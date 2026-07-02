import type { ReactNode } from "react";
import styles from "../style/components/AdvancedLlmSettings.module.css";

type AdvancedLlmSettingsProps = {
  children: ReactNode;
};

export default function AdvancedLlmSettings({ children }: AdvancedLlmSettingsProps) {
  return (
    <details className={styles["advanced-llm-settings"]}>
      <summary className={styles["advanced-llm-settings-summary"]}>
        <span className={styles["advanced-llm-settings-title"]}>
          Advanced — I configure my own models (Ollama, LM Studio, Cloud)
        </span>
      </summary>
      <div className={styles["advanced-llm-settings-body"]}>{children}</div>
    </details>
  );
}
