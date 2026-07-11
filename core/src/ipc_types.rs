use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct Vault {
    pub id: String,
    #[ts(optional)]
    pub parent_vault_id: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub privacy_tier: String,
    pub priority_profile: String,
    pub summary_node_id: Option<String>,
    #[ts(type = "number")]
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub meta: String,
    pub ui_metadata: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct VaultCreateInput {
    pub name: String,
    #[serde(default)]
    #[ts(optional)]
    pub parent_vault_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub icon: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub privacy_tier: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub priority_profile: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    #[ts(type = "number")]
    pub sort_order: Option<i64>,
    #[serde(default)]
    #[ts(optional)]
    pub meta: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct VaultUpdateInput {
    pub id: String,
    #[ts(optional)]
    pub name: Option<String>,
    #[ts(optional)]
    pub privacy_tier: Option<String>,
    #[ts(optional)]
    pub priority_profile: Option<String>,
    #[ts(optional)]
    pub icon: Option<String>,
    #[ts(optional)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct Node {
    pub id: String,
    pub vault_id: String,
    pub sub_vault_id: Option<String>,
    pub node_type: String,
    pub title: String,
    pub summary: String,
    pub detail: Option<String>,
    pub source: Option<String>,
    pub source_type: Option<String>,
    pub privacy_tier: Option<String>,
    pub priority: String,
    #[ts(type = "number")]
    pub version: i64,
    pub is_archived: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_accessed: String,
    pub deleted_at: Option<String>,
    pub meta: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct NodeCreateInput {
    pub vault_id: String,
    #[serde(default)]
    #[ts(optional)]
    pub sub_vault_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub node_type: Option<String>,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    #[ts(optional)]
    pub detail: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub source: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub source_type: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub privacy_tier: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub meta: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct NodeUpdateInput {
    pub id: String,
    #[ts(optional)]
    pub vault_id: Option<String>,
    #[ts(optional)]
    pub sub_vault_id: Option<String>,
    #[ts(optional)]
    pub node_type: Option<String>,
    #[ts(optional)]
    pub title: Option<String>,
    #[ts(optional)]
    pub summary: Option<String>,
    #[ts(optional)]
    pub detail: Option<String>,
    #[ts(optional)]
    pub source: Option<String>,
    #[ts(optional)]
    pub source_type: Option<String>,
    #[ts(optional)]
    pub privacy_tier: Option<String>,
    #[ts(optional)]
    pub priority: Option<String>,
    #[ts(optional)]
    pub is_archived: Option<bool>,
    #[ts(optional)]
    pub meta: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct TagCreateInput {
    pub name: String,
    #[ts(optional)]
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct Door {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: Option<String>,
    pub target_vault_id: Option<String>,
    pub label: Option<String>,
    pub status: String,
    pub orphan_reason: Option<String>,
    pub orphan_since: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct DoorCreateInput {
    pub source_node_id: String,
    #[ts(optional)]
    pub target_node_id: Option<String>,
    #[ts(optional)]
    pub target_vault_id: Option<String>,
    #[ts(optional)]
    pub label: Option<String>,
}

/// Serialized onboarding extraction result for IPC / TypeScript. Enriches `onboarding::ProposedNode` with backend-derived metadata for the UI like `resolved_vault_id`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct OnboardingProposedNode {
    pub title: String,
    pub summary: String,
    #[ts(optional)]
    pub detail: Option<String>,
    #[ts(optional)]
    pub category: Option<String>,
    #[ts(optional)]
    pub target_vault_key: Option<String>,
    #[ts(optional)]
    pub tags: Option<Vec<String>>,
    #[ts(optional)]
    pub node_type: Option<String>,
    #[ts(optional)]
    pub resolved_vault_id: Option<String>,
}

/// Payload for committing accepted onboarding rows to persistent nodes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct OnboardingNodeCommitInput {
    pub vault_id: String,
    pub title: String,
    pub summary: String,
    #[ts(optional)]
    pub detail: Option<String>,
    #[ts(optional)]
    pub node_type: Option<String>,
    #[ts(optional)]
    pub source_type: Option<String>,
    #[ts(optional)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct Backlink {
    pub id: String,
    pub target_node_id: String,
    pub source_node_id: String,
    pub door_id: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct Changeset {
    pub id: String,
    pub session_id: Option<String>,
    pub status: String,
    #[ts(type = "number")]
    pub item_count: i64,
    #[ts(type = "number")]
    pub accepted_count: i64,
    #[ts(type = "number")]
    pub dismissed_count: i64,
    pub model_used: Option<String>,
    pub created_at: String,
    pub reviewed_at: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct ChangesetItem {
    pub id: String,
    pub changeset_id: String,
    pub item_type: String,
    pub target_node_id: Option<String>,
    pub proposed_data: String,
    pub existing_data: Option<String>,
    pub similarity: Option<f64>,
    pub merge_with_id: Option<String>,
    pub door_id: Option<String>,
    pub status: String,
    pub reviewed_at: Option<String>,
    #[ts(type = "number")]
    pub sort_order: i64,
    pub cross_vault_anomaly: bool,
    pub anomaly_warning: Option<String>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct ChangesetCommitInput {
    pub changeset_id: String,
    pub item_actions: Vec<ItemReviewAction>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct ItemReviewAction {
    pub item_id: String,
    pub action: String, // "accept", "dismiss", "edit"
    #[ts(type = "unknown")]
    pub edited_data: Option<Value>, // Typed JSON properties for edit payload
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct EmbeddingStatus {
    pub model: String,
    pub tier: String,
    pub backend: String,
    pub coverage_percent: f64,
    pub last_computed_at: Option<String>,
    pub jaccard_fallback_active: bool,
    pub reembed_in_progress: bool,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct EmbeddingReembedInput {
    pub model: String,
    pub tier: String,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct ImportJobStatus {
    pub id: String,
    pub status: String,
    pub source_name: String,
    pub changeset_id: Option<String>,
    pub node_count: i32,
    pub total_pages: i32,
    pub digital_pages: i32,
    pub ocr_pages: i32,
    pub hybrid_pages: i32,
    pub avg_ocr_confidence: f32,
    /// IPC exposes 0 or 1; DB may store a table-block count from the engine.
    pub tables_detected_unpreserved: i32,
    pub extraction_path: Option<String>,
    pub rasterization_dpi: i32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../ui/types/generated/")]
pub struct ImportStartJobInput {
    pub file_path: String,
    pub target_vault_id: Option<String>,
    pub rasterization_dpi: i32,
    pub use_llm_extraction: bool,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_typescript_bindings() {
        if let Err(err) = Vault::export() {
            panic!("failed to export Vault: {err}");
        }
        if let Err(err) = VaultCreateInput::export() {
            panic!("failed to export VaultCreateInput: {err}");
        }
        if let Err(err) = VaultUpdateInput::export() {
            panic!("failed to export VaultUpdateInput: {err}");
        }
        if let Err(err) = Node::export() {
            panic!("failed to export Node: {err}");
        }
        if let Err(err) = NodeCreateInput::export() {
            panic!("failed to export NodeCreateInput: {err}");
        }
        if let Err(err) = NodeUpdateInput::export() {
            panic!("failed to export NodeUpdateInput: {err}");
        }
        if let Err(err) = Tag::export() {
            panic!("failed to export Tag: {err}");
        }
        if let Err(err) = TagCreateInput::export() {
            panic!("failed to export TagCreateInput: {err}");
        }
        if let Err(err) = Door::export() {
            panic!("failed to export Door: {err}");
        }
        if let Err(err) = DoorCreateInput::export() {
            panic!("failed to export DoorCreateInput: {err}");
        }
        if let Err(err) = Backlink::export() {
            panic!("failed to export Backlink: {err}");
        }
        if let Err(err) = OnboardingProposedNode::export() {
            panic!("failed to export OnboardingProposedNode: {err}");
        }
        if let Err(err) = OnboardingNodeCommitInput::export() {
            panic!("failed to export OnboardingNodeCommitInput: {err}");
        }
        if let Err(err) = Changeset::export() {
            panic!("failed to export Changeset: {err}");
        }
        if let Err(err) = ChangesetItem::export() {
            panic!("failed to export ChangesetItem: {err}");
        }
        if let Err(err) = ChangesetCommitInput::export() {
            panic!("failed to export ChangesetCommitInput: {err}");
        }
        if let Err(err) = ItemReviewAction::export() {
            panic!("failed to export ItemReviewAction: {err}");
        }
        if let Err(err) = EmbeddingStatus::export() {
            panic!("failed to export EmbeddingStatus: {err}");
        }
        if let Err(err) = EmbeddingReembedInput::export() {
            panic!("failed to export EmbeddingReembedInput: {err}");
        }
        if let Err(err) = ImportJobStatus::export() {
            panic!("failed to export ImportJobStatus: {err}");
        }
        if let Err(err) = ImportStartJobInput::export() {
            panic!("failed to export ImportStartJobInput: {err}");
        }
    }
}
