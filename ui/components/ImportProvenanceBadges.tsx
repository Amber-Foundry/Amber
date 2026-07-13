import type { Node } from "../types/generated/Node";
import {
  getImportOcrConfidence,
  getImportSourceLabel,
  importHasUnstructuredTables,
  isPdfImportNode,
} from "../utils/importDocument";
import styles from "../style/components/ImportProvenanceBadges.module.css";

type ImportProvenanceBadgesProps = {
  node: Node;
  allNodes?: Node[];
};

export default function ImportProvenanceBadges({
  node,
  allNodes = [],
}: ImportProvenanceBadgesProps) {
  if (!isPdfImportNode(node)) {
    return null;
  }

  const sourceLabel = getImportSourceLabel(node);
  const ocrConfidence = getImportOcrConfidence(node, allNodes);
  const tablesWarn = importHasUnstructuredTables(node, allNodes);

  return (
    <div className={styles.row} onClick={(e) => e.stopPropagation()}>
      <span className={styles.provenance} title={`Imported: ${sourceLabel}`}>
        Imported: {sourceLabel}
      </span>
      {ocrConfidence != null && (
        <span className={styles.ocrBadge}>OCR {Math.round(ocrConfidence * 100)}%</span>
      )}
      {tablesWarn && <span className={styles.tablesBadge}>tables unstructured</span>}
    </div>
  );
}
