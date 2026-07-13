import { useState } from "react";
import type { Node } from "../types/generated/Node";
import {
  getImportChunkIndex,
  formatImportSectionTitle,
  isImportDocumentNode,
  listImportDocumentSections,
} from "../utils/importDocument";
import ImportDocumentSpineStyles from "../style/components/ImportDocumentSpine.module.css";

const COLLAPSED_SECTION_LIMIT = 5;

type ImportDocumentSpineProps = {
  documentNode: Node;
  allNodes: Node[];
  onSelectSection: (nodeId: string) => void;
};

export default function ImportDocumentSpine({
  documentNode,
  allNodes,
  onSelectSection,
}: ImportDocumentSpineProps) {
  const [expanded, setExpanded] = useState(false);

  if (!isImportDocumentNode(documentNode)) {
    return null;
  }

  const sections = listImportDocumentSections(documentNode.id, allNodes);
  if (sections.length === 0) {
    return null;
  }

  const canCollapse = sections.length > COLLAPSED_SECTION_LIMIT;
  const visibleSections =
    canCollapse && !expanded ? sections.slice(0, COLLAPSED_SECTION_LIMIT) : sections;
  const hiddenCount = sections.length - visibleSections.length;

  return (
    <div className={ImportDocumentSpineStyles.spine}>
      <div className={ImportDocumentSpineStyles.spineHeader}>
        <span className={ImportDocumentSpineStyles.spineLabel}>Document sections</span>
        <span className={ImportDocumentSpineStyles.spineCount}>{sections.length} parts</span>
      </div>
      <div className={ImportDocumentSpineStyles.sectionList}>
        {visibleSections.map((section, index) => {
          const chunkIndex = getImportChunkIndex(section);
          const ordinal = (chunkIndex ?? index) + 1;
          const shortTitle = formatImportSectionTitle(section.title, ordinal, sections.length);
          return (
            <div key={section.id} className={ImportDocumentSpineStyles.sectionBlock}>
              {index > 0 && (
                <div className={ImportDocumentSpineStyles.connector} aria-hidden="true">
                  <div className={ImportDocumentSpineStyles.connectorLine} />
                  <span className={ImportDocumentSpineStyles.connectorBadge}>next</span>
                  <div className={ImportDocumentSpineStyles.connectorLine} />
                </div>
              )}
              <button
                type="button"
                className={ImportDocumentSpineStyles.sectionCard}
                onClick={() => onSelectSection(section.id)}
                title={section.title}
              >
                <div className={ImportDocumentSpineStyles.sectionMeta}>
                  <span className={ImportDocumentSpineStyles.sectionIndex}>
                    {ordinal}/{sections.length}
                  </span>
                  <strong className={ImportDocumentSpineStyles.sectionTitle}>{shortTitle}</strong>
                </div>
                {section.summary.trim() && (
                  <p className={ImportDocumentSpineStyles.sectionSummary}>
                    {section.summary.length > 140
                      ? `${section.summary.slice(0, 137)}...`
                      : section.summary}
                  </p>
                )}
              </button>
            </div>
          );
        })}
      </div>
      {canCollapse && (
        <button
          type="button"
          className={ImportDocumentSpineStyles.toggleBtn}
          onClick={() => setExpanded((prev) => !prev)}
          aria-expanded={expanded}
        >
          {expanded
            ? "Show fewer sections"
            : `Show all ${sections.length} sections (${hiddenCount} more)`}
        </button>
      )}
    </div>
  );
}
