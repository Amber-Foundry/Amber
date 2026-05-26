use crate::memory_agent::parser::{CandidateAction, CandidateNode};
use crate::memory_agent::similarity::{
    classify_similarity, jaccard_similarity, jaccard_similarity_pretokenized, tokenize,
    SimilarityClass,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The type of action proposed by an individual item in a changeset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangesetItemType {
    /// Add a completely new node to the knowledge base.
    Add,
    /// Update an existing node with new/divergent information.
    Update,
    /// Merge a candidate with an existing node to combine highly similar info.
    Merge,
    /// Delete an existing node from the knowledge base.
    Delete,
}

/// An individual proposed action (Add, Update, Merge, or Delete) inside a changeset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingChangesetItem {
    /// The action type (Add, Update, Merge, or Delete).
    pub item_type: ChangesetItemType,
    /// The ID of the target node, if updating, merging, or deleting.
    pub target_node_id: Option<String>,
    /// Serialized JSON string of the proposed node data.
    pub proposed_data: String,
    /// Serialized JSON string of the existing node data, if applicable.
    pub existing_data: Option<String>,
    /// Calculated Jaccard text similarity score, if matched with an existing node.
    pub similarity: Option<f64>,
    /// The ID of the node to merge this item with, if applicable.
    pub merge_with_id: Option<String>,
}

/// A collection of pending proposed changes to the knowledge base extracted from an LLM session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingChangeset {
    /// The chat session ID associated with this background extraction.
    pub session_id: String,
    /// The name of the LLM used to compile the candidates.
    pub model_used: Option<String>,
    /// The individual proposed items in the changeset.
    pub items: Vec<PendingChangesetItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ProposedNodeData {
    pub title: String,
    pub summary: String,
    pub detail: Option<String>,
    pub node_type: Option<String>,
    pub target_vault_key: Option<String>,
    pub vault_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub confidence: f64,
    pub action: CandidateAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substantial_change: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ExistingNodeData {
    pub id: String,
    pub vault_id: String,
    pub title: String,
    pub summary: String,
    pub detail: Option<String>,
    pub node_type: String,
}

struct DbNode {
    id: String,
    vault_id: String,
    title: String,
    summary: String,
    node_type: String,
}

fn fetch_node_detail(conn: &Connection, node_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT detail FROM nodes WHERE id = ?1;",
        [node_id],
        |row| row.get(0),
    )
    .map_err(|err| format!("Failed to fetch node detail for {node_id}: {err}"))
}

/// Load non-deleted, non-archived nodes from the database and compare them against candidates
/// using Jaccard token-overlap to build a pending changeset of Add, Update, Merge, and Delete tasks.
pub fn build_changeset(
    conn: &Connection,
    candidates: &[CandidateNode],
    session_id: &str,
) -> Result<PendingChangeset, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, vault_id, title, summary, node_type
             FROM nodes
             WHERE deleted_at IS NULL AND is_archived = 0;",
        )
        .map_err(|err| format!("Failed to prepare nodes query: {err}"))?;

    let node_rows = stmt
        .query_map([], |row| {
            Ok(DbNode {
                id: row.get(0)?,
                vault_id: row.get(1)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                node_type: row.get(4)?,
            })
        })
        .map_err(|err| format!("Failed to execute nodes query: {err}"))?;

    // Pre-tokenize existing nodes once (N tokenizations) to avoid re-tokenizing
    let mut existing_nodes: Vec<(DbNode, HashSet<String>)> = Vec::new();
    for row_res in node_rows {
        let node = row_res.map_err(|err| format!("Failed to parse database node: {err}"))?;
        let tokens = tokenize(&format!("{} {}", node.title, node.summary));
        existing_nodes.push((node, tokens));
    }

    let mut items = Vec::new();

    for candidate in candidates {
        // Skip low-confidence candidates
        if candidate.confidence < 0.3 {
            continue;
        }

        // Pre-tokenize each candidate once (M tokenizations total across all candidates)
        let candidate_tokens = tokenize(&format!("{} {}", candidate.title, candidate.summary));

        // Find best similarity match using pre-tokenized sets
        let mut best_match: Option<(&DbNode, f64)> = None;

        for (existing, existing_tokens) in &existing_nodes {
            let score = jaccard_similarity_pretokenized(&candidate_tokens, existing_tokens);
            if let Some((_, best_score)) = best_match {
                if score > best_score {
                    best_match = Some((existing, score));
                }
            } else {
                best_match = Some((existing, score));
            }
        }

        match candidate.action {
            CandidateAction::Delete => {
                if let Some((best_node, score)) = best_match {
                    if score >= 0.50 {
                        let detail = fetch_node_detail(conn, &best_node.id)?;
                        let proposed = ProposedNodeData {
                            title: candidate.title.clone(),
                            summary: candidate.summary.clone(),
                            detail: candidate.detail.clone(),
                            node_type: candidate.node_type.clone(),
                            target_vault_key: candidate.target_vault_key.clone(),
                            vault_id: None,
                            tags: candidate.tags.clone(),
                            confidence: candidate.confidence,
                            action: candidate.action,
                            substantial_change: None,
                        };
                        let proposed_str = serde_json::to_string(&proposed).map_err(|err| {
                            format!("Failed to serialize proposed deletion data: {err}")
                        })?;

                        let existing_ser = ExistingNodeData {
                            id: best_node.id.clone(),
                            vault_id: best_node.vault_id.clone(),
                            title: best_node.title.clone(),
                            summary: best_node.summary.clone(),
                            detail,
                            node_type: best_node.node_type.clone(),
                        };
                        let existing_str = serde_json::to_string(&existing_ser).map_err(|err| {
                            format!("Failed to serialize existing deletion data: {err}")
                        })?;

                        items.push(PendingChangesetItem {
                            item_type: ChangesetItemType::Delete,
                            target_node_id: Some(best_node.id.clone()),
                            proposed_data: proposed_str,
                            existing_data: Some(existing_str),
                            similarity: Some(score),
                            merge_with_id: None,
                        });
                    }
                }
            }
            CandidateAction::Add | CandidateAction::Update => {
                let resolved_vault_id = candidate
                    .target_vault_key
                    .as_deref()
                    .and_then(crate::onboarding::vault_id_for_category_key)
                    .unwrap_or("vault_root_graph")
                    .to_string();

                if let Some((best_node, score)) = best_match {
                    let classification = classify_similarity(score);
                    match classification {
                        SimilarityClass::New => {
                            let proposed = ProposedNodeData {
                                title: candidate.title.clone(),
                                summary: candidate.summary.clone(),
                                detail: candidate.detail.clone(),
                                node_type: candidate.node_type.clone(),
                                target_vault_key: candidate.target_vault_key.clone(),
                                vault_id: Some(resolved_vault_id),
                                tags: candidate.tags.clone(),
                                confidence: candidate.confidence,
                                action: candidate.action,
                                substantial_change: None,
                            };
                            let proposed_str = serde_json::to_string(&proposed).map_err(|err| {
                                format!("Failed to serialize proposed new data: {err}")
                            })?;

                            items.push(PendingChangesetItem {
                                item_type: ChangesetItemType::Add,
                                target_node_id: None,
                                proposed_data: proposed_str,
                                existing_data: None,
                                similarity: None,
                                merge_with_id: None,
                            });
                        }
                        SimilarityClass::DuplicateFlag => {
                            // Substantial-change detection on detail alone
                            let detail = fetch_node_detail(conn, &best_node.id)?;
                            let candidate_detail = candidate.detail.as_deref().unwrap_or("").trim();
                            let existing_detail = detail.as_deref().unwrap_or("").trim();

                            let mut is_substantial = false;
                            if !candidate_detail.is_empty() || !existing_detail.is_empty() {
                                let detail_score =
                                    jaccard_similarity(candidate_detail, existing_detail);
                                if detail_score < 0.30 {
                                    is_substantial = true;
                                }
                            }

                            let mut proposed = ProposedNodeData {
                                title: candidate.title.clone(),
                                summary: candidate.summary.clone(),
                                detail: candidate.detail.clone(),
                                node_type: candidate.node_type.clone(),
                                target_vault_key: candidate.target_vault_key.clone(),
                                vault_id: Some(resolved_vault_id),
                                tags: candidate.tags.clone(),
                                confidence: candidate.confidence,
                                action: candidate.action,
                                substantial_change: None,
                            };

                            let existing_ser = ExistingNodeData {
                                id: best_node.id.clone(),
                                vault_id: best_node.vault_id.clone(),
                                title: best_node.title.clone(),
                                summary: best_node.summary.clone(),
                                detail,
                                node_type: best_node.node_type.clone(),
                            };
                            let existing_str =
                                serde_json::to_string(&existing_ser).map_err(|err| {
                                    format!("Failed to serialize existing merge data: {err}")
                                })?;

                            if is_substantial {
                                proposed.substantial_change = Some(true);
                                let proposed_str =
                                    serde_json::to_string(&proposed).map_err(|err| {
                                        format!(
                                            "Failed to serialize proposed merge/update data: {err}"
                                        )
                                    })?;

                                items.push(PendingChangesetItem {
                                    item_type: ChangesetItemType::Update,
                                    target_node_id: Some(best_node.id.clone()),
                                    proposed_data: proposed_str,
                                    existing_data: Some(existing_str),
                                    similarity: Some(score),
                                    merge_with_id: None,
                                });
                            } else {
                                let proposed_str =
                                    serde_json::to_string(&proposed).map_err(|err| {
                                        format!("Failed to serialize proposed merge data: {err}")
                                    })?;

                                items.push(PendingChangesetItem {
                                    item_type: ChangesetItemType::Merge,
                                    target_node_id: None,
                                    proposed_data: proposed_str,
                                    existing_data: Some(existing_str),
                                    similarity: Some(score),
                                    merge_with_id: Some(best_node.id.clone()),
                                });
                            }
                        }
                        SimilarityClass::Update => {
                            let detail = fetch_node_detail(conn, &best_node.id)?;
                            let proposed = ProposedNodeData {
                                title: candidate.title.clone(),
                                summary: candidate.summary.clone(),
                                detail: candidate.detail.clone(),
                                node_type: candidate.node_type.clone(),
                                target_vault_key: candidate.target_vault_key.clone(),
                                vault_id: Some(resolved_vault_id),
                                tags: candidate.tags.clone(),
                                confidence: candidate.confidence,
                                action: candidate.action,
                                substantial_change: None,
                            };
                            let proposed_str = serde_json::to_string(&proposed).map_err(|err| {
                                format!("Failed to serialize proposed update data: {err}")
                            })?;

                            let existing_ser = ExistingNodeData {
                                id: best_node.id.clone(),
                                vault_id: best_node.vault_id.clone(),
                                title: best_node.title.clone(),
                                summary: best_node.summary.clone(),
                                detail,
                                node_type: best_node.node_type.clone(),
                            };
                            let existing_str =
                                serde_json::to_string(&existing_ser).map_err(|err| {
                                    format!("Failed to serialize existing update data: {err}")
                                })?;

                            items.push(PendingChangesetItem {
                                item_type: ChangesetItemType::Update,
                                target_node_id: Some(best_node.id.clone()),
                                proposed_data: proposed_str,
                                existing_data: Some(existing_str),
                                similarity: Some(score),
                                merge_with_id: None,
                            });
                        }
                    }
                } else {
                    // No existing nodes at all, classify as New (Add)
                    let proposed = ProposedNodeData {
                        title: candidate.title.clone(),
                        summary: candidate.summary.clone(),
                        detail: candidate.detail.clone(),
                        node_type: candidate.node_type.clone(),
                        target_vault_key: candidate.target_vault_key.clone(),
                        vault_id: Some(resolved_vault_id),
                        tags: candidate.tags.clone(),
                        confidence: candidate.confidence,
                        action: candidate.action,
                        substantial_change: None,
                    };
                    let proposed_str = serde_json::to_string(&proposed)
                        .map_err(|err| format!("Failed to serialize proposed new data: {err}"))?;

                    items.push(PendingChangesetItem {
                        item_type: ChangesetItemType::Add,
                        target_node_id: None,
                        proposed_data: proposed_str,
                        existing_data: None,
                        similarity: None,
                        merge_with_id: None,
                    });
                }
            }
        }
    }

    Ok(PendingChangeset {
        session_id: session_id.to_string(),
        model_used: None,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn setup_test_db() -> Connection {
        let conn = match Connection::open_in_memory() {
            Ok(c) => c,
            Err(e) => panic!("Failed to open in-memory DB: {e}"),
        };
        let create_sql = "
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY,
                vault_id TEXT NOT NULL,
                node_type TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                detail TEXT,
                is_archived INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT
            );
        ";
        if let Err(e) = conn.execute(create_sql, []) {
            panic!("Failed to create nodes table: {e}");
        }
        conn
    }

    #[test]
    fn test_all_adds_when_db_is_empty() {
        let conn = setup_test_db();
        let candidates = vec![CandidateNode {
            title: "Rust programming".to_string(),
            summary: "Rust is a systems programming language".to_string(),
            detail: Some("Focuses on memory safety and speed".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: Some(vec!["rust".to_string(), "systems".to_string()]),
            confidence: 0.9,
            action: CandidateAction::Add,
        }];

        let changeset = match build_changeset(&conn, &candidates, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok changeset but got Err: {e}"),
        };

        assert_eq!(changeset.session_id, "session-123");
        assert_eq!(changeset.items.len(), 1);
        let item = &changeset.items[0];
        assert_eq!(item.item_type, ChangesetItemType::Add);
        assert_eq!(item.target_node_id, None);
        assert_eq!(item.merge_with_id, None);
        assert_eq!(item.similarity, None);
        assert_eq!(item.existing_data, None);

        // Verify proposed data contents
        let proposed: ProposedNodeData = match serde_json::from_str(&item.proposed_data) {
            Ok(p) => p,
            Err(e) => panic!("Failed to parse proposed JSON: {e}"),
        };
        assert_eq!(proposed.title, "Rust programming");
        assert_eq!(proposed.vault_id, Some("vault_learning".to_string()));
    }

    #[test]
    fn test_perfect_similarity_results_in_update() {
        let conn = setup_test_db();
        let insert_sql = "
            INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6);
        ";
        if let Err(e) = conn.execute(
            insert_sql,
            params![
                "node_1",
                "vault_learning",
                "concept",
                "Rust programming",
                "Rust is a systems programming language",
                "Focuses on safety"
            ],
        ) {
            panic!("Insert failed: {e}");
        }

        let candidates = vec![CandidateNode {
            title: "Rust programming".to_string(),
            summary: "Rust is a systems programming language".to_string(),
            detail: Some("Refined description of safety and speed".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.95,
            action: CandidateAction::Add,
        }];

        let changeset = match build_changeset(&conn, &candidates, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok changeset but got Err: {e}"),
        };

        assert_eq!(changeset.items.len(), 1);
        let item = &changeset.items[0];
        assert_eq!(item.item_type, ChangesetItemType::Update);
        assert_eq!(item.target_node_id, Some("node_1".to_string()));
        assert!(item.similarity.is_some());
        let score = item.similarity.unwrap_or(0.0);
        assert!(score > 0.85);
    }

    #[test]
    fn test_substantial_change_upgrades_merge_to_update() {
        let conn = setup_test_db();
        // Insert a node: "Machine learning" with a specific detail
        let insert_sql = "
            INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6);
        ";
        if let Err(e) = conn.execute(
            insert_sql,
            params![
                "node_ml",
                "vault_learning",
                "concept",
                "Machine Learning",
                "A field of computer science about algorithms",
                "Traditional supervised learning algorithms like linear regression"
            ],
        ) {
            panic!("Insert failed: {e}");
        }

        // Candidate has identical title and summary (so similarity on title+summary is high/1.0),
        // but entirely divergent details (deep neural networks and transformer architecture)
        let candidates = vec![CandidateNode {
            title: "Machine Learning".to_string(),
            summary: "A field of computer science about algorithms".to_string(),
            detail: Some(
                "Deep learning neural networks and modern transformer architecture".to_string(),
            ),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.90,
            action: CandidateAction::Add,
        }];

        let changeset = match build_changeset(&conn, &candidates, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok changeset: {e}"),
        };

        assert_eq!(changeset.items.len(), 1);
        let item = &changeset.items[0];
        assert_eq!(item.item_type, ChangesetItemType::Update);
        assert_eq!(item.target_node_id, Some("node_ml".to_string()));
    }

    #[test]
    fn test_merge_zone_with_and_without_substantial_change() {
        let conn = setup_test_db();
        let insert_sql = "
            INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6);
        ";
        // Node 1: "Artificial Intelligence"
        if let Err(e) = conn.execute(
            insert_sql,
            params![
                "node_ai",
                "vault_learning",
                "concept",
                "Artificial Intelligence and cognitive systems",
                "Systems that mimic human intelligence and cognitive behavior",
                "Focuses on symbolic reasoning and expert systems"
            ],
        ) {
            panic!("Insert failed: {e}");
        }

        // Candidate 1 (Close detail similarity):
        // Title+Summary overlap is partial (Merge zone, e.g. similarity ~0.60)
        // Detail overlap is also close: "expert systems and symbolic reasoning"
        let candidates_close = vec![CandidateNode {
            title: "Artificial Intelligence".to_string(),
            summary: "Systems that mimic human intelligence".to_string(),
            detail: Some("Focuses on symbolic reasoning and expert systems".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.80,
            action: CandidateAction::Add,
        }];

        let cs_close = match build_changeset(&conn, &candidates_close, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok: {e}"),
        };
        assert_eq!(cs_close.items.len(), 1);
        assert_eq!(cs_close.items[0].item_type, ChangesetItemType::Merge);
        assert_eq!(cs_close.items[0].merge_with_id, Some("node_ai".to_string()));

        // Candidate 2 (Divergent detail similarity):
        // Title+Summary overlap is partial (Merge zone)
        // Detail is totally different: "reinforcement learning with large scale policy optimization"
        let candidates_divergent = vec![CandidateNode {
            title: "Artificial Intelligence".to_string(),
            summary: "Systems that mimic human intelligence".to_string(),
            detail: Some("reinforcement learning with large scale policy optimization".to_string()),
            node_type: Some("concept".to_string()),
            target_vault_key: Some("learning".to_string()),
            tags: None,
            confidence: 0.80,
            action: CandidateAction::Add,
        }];

        let cs_div = match build_changeset(&conn, &candidates_divergent, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok: {e}"),
        };
        assert_eq!(cs_div.items.len(), 1);
        // Upgraded to Update due to low detail Jaccard (<0.30)!
        assert_eq!(cs_div.items[0].item_type, ChangesetItemType::Update);
        assert_eq!(cs_div.items[0].target_node_id, Some("node_ai".to_string()));

        // Verify proposed_data contains the substantial_change: true flag
        let proposed: ProposedNodeData = match serde_json::from_str(&cs_div.items[0].proposed_data)
        {
            Ok(p) => p,
            Err(e) => panic!("Failed to parse: {e}"),
        };
        assert_eq!(proposed.substantial_change, Some(true));
    }

    #[test]
    fn test_deletion_matches_existing_node_properly() {
        let conn = setup_test_db();
        let insert_sql = "
            INSERT INTO nodes (id, vault_id, node_type, title, summary, detail)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6);
        ";
        if let Err(e) = conn.execute(
            insert_sql,
            params![
                "node_del",
                "vault_personal",
                "fact",
                "User works at Acme Corp",
                "User works at Acme Corp as lead designer",
                None::<String>
            ],
        ) {
            panic!("Insert failed: {e}");
        }

        let candidates = vec![CandidateNode {
            title: "User works at Acme Corp".to_string(),
            summary: "User no longer works at Acme Corp".to_string(),
            detail: None,
            node_type: Some("fact".to_string()),
            target_vault_key: None,
            tags: None,
            confidence: 0.90,
            action: CandidateAction::Delete,
        }];

        let changeset = match build_changeset(&conn, &candidates, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok: {e}"),
        };

        assert_eq!(changeset.items.len(), 1);
        assert_eq!(changeset.items[0].item_type, ChangesetItemType::Delete);
        assert_eq!(
            changeset.items[0].target_node_id,
            Some("node_del".to_string())
        );
    }

    #[test]
    fn test_low_confidence_skipped_and_non_matching_delete_discarded() {
        let conn = setup_test_db();
        let candidates = vec![
            // Skip low-confidence candidate
            CandidateNode {
                title: "Low confidence node".to_string(),
                summary: "Should not be processed".to_string(),
                detail: None,
                node_type: Some("fact".to_string()),
                target_vault_key: None,
                tags: None,
                confidence: 0.25,
                action: CandidateAction::Add,
            },
            // Silently discard delete with no matching node (similarity < 0.5)
            CandidateNode {
                title: "Delete something".to_string(),
                summary: "User said they never did that".to_string(),
                detail: None,
                node_type: Some("fact".to_string()),
                target_vault_key: None,
                tags: None,
                confidence: 0.90,
                action: CandidateAction::Delete,
            },
        ];

        let changeset = match build_changeset(&conn, &candidates, "session-123") {
            Ok(cs) => cs,
            Err(e) => panic!("Expected Ok: {e}"),
        };

        // Both skipped/discarded, items should be empty!
        assert!(changeset.items.is_empty());
    }
}
