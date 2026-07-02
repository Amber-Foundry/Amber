use std::collections::HashSet;
use std::sync::OnceLock;

use rusqlite::Connection;
use tiktoken_rs::CoreBPE;

use crate::privacy::{
    cloud_llm_context_policy, generate_pointer_stub, get_effective_privacy,
    local_llm_context_policy, unrestricted_llm_context_policy, LlmContextPolicy,
};

const ATTENTION_SINK_TOKENS: usize = 50;
const FALLBACK_CHARS_PER_TOKEN_EST: usize = 4;
const SQLITE_IN_CLAUSE_BATCH_SIZE: usize = 900;

fn cl100k_bpe() -> &'static CoreBPE {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    BPE.get_or_init(|| match tiktoken_rs::cl100k_base() {
        Ok(bpe) => bpe,
        Err(err) => panic!("failed to load cl100k tokenizer (tiktoken-rs): {err}"),
    })
}

/// Token count for budgeting using OpenAI `cl100k_base` (tiktoken-compatible).
pub fn count_tokens(text: &str) -> usize {
    cl100k_bpe().encode_with_special_tokens(text).len()
}

fn trim_tail_fallback_chars(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(FALLBACK_CHARS_PER_TOKEN_EST);
    if max_chars == 0 {
        return String::new();
    }
    if max_tokens < ATTENTION_SINK_TOKENS {
        return text.chars().take(max_chars).collect();
    }
    let sink_chars = ATTENTION_SINK_TOKENS.saturating_mul(FALLBACK_CHARS_PER_TOKEN_EST);
    let sink: String = text.chars().take(sink_chars).collect();
    let sink_len = sink.chars().count();
    let remaining_chars = max_chars.saturating_sub(sink_len);
    let tail: String = text.chars().skip(sink_len).take(remaining_chars).collect();
    format!("{sink}{tail}")
}

/// Trim `text` to at most `max_tokens` (cl100k), preserving token boundaries.
fn trim_tail_with_attention_sink(text: &str, max_tokens: usize) -> String {
    let bpe = cl100k_bpe();
    let ids = bpe.encode_with_special_tokens(text);
    if ids.len() <= max_tokens {
        return text.to_string();
    }
    if max_tokens == 0 {
        return String::new();
    }

    let head = &ids[..max_tokens];
    match bpe.decode(head.to_vec()) {
        Ok(decoded) => decoded,
        Err(_) => trim_tail_fallback_chars(text, max_tokens),
    }
}

pub struct AssemblerConfig {
    pub scope: String,
    pub max_tokens: usize,
    pub is_unlocked: bool,
}

struct AssemblerNode {
    id: String,
    title: String,
    summary: String,
    detail: String,
    node_privacy_tier: Option<String>,
    sub_vault_privacy_tier: Option<String>,
    vault_privacy_tier: Option<String>,
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_xml_body(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn fetch_requested_nodes(
    db: &Connection,
    node_ids: &[String],
) -> Result<Vec<AssemblerNode>, crate::AppError> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut nodes = Vec::new();
    let mut seen_ids = HashSet::new();

    for chunk in node_ids.chunks(SQLITE_IN_CLAUSE_BATCH_SIZE) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let query_str = format!(
            "SELECT n.id,
                    n.title,
                    n.summary,
                    COALESCE(n.detail, '') AS detail,
                    n.privacy_tier,
                    n.vault_id,
                    n.priority,
                    sv.privacy_tier,
                    v.privacy_tier
             FROM nodes n
             LEFT JOIN sub_vaults sv
               ON sv.id = n.sub_vault_id
              AND sv.deleted_at IS NULL
             LEFT JOIN vaults v
               ON v.id = n.vault_id
              AND v.deleted_at IS NULL
             WHERE n.id IN ({placeholders})
               AND n.deleted_at IS NULL
               AND n.is_archived = 0;"
        );

        let mut statement = db
            .prepare(&query_str)
            .map_err(|err| format!("Failed preparing assembler query: {err}"))?;

        let params: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let mut rows = statement
            .query(rusqlite::params_from_iter(params))
            .map_err(|err| format!("Failed querying nodes for assembler: {err}"))?;

        while let Some(row) = rows
            .next()
            .map_err(|err| format!("Failed reading assembler row: {err}"))?
        {
            let id: String = row
                .get(0)
                .map_err(|err| format!("Failed decoding id field in assembler: {err}"))?;
            if !seen_ids.insert(id.clone()) {
                continue;
            }
            nodes.push(AssemblerNode {
                id,
                title: row
                    .get(1)
                    .map_err(|err| format!("Failed decoding title field in assembler: {err}"))?,
                summary: row
                    .get(2)
                    .map_err(|err| format!("Failed decoding summary field in assembler: {err}"))?,
                detail: row
                    .get(3)
                    .map_err(|err| format!("Failed decoding detail field in assembler: {err}"))?,
                node_privacy_tier: row.get(4).map_err(|err| {
                    format!("Failed decoding node privacy field in assembler: {err}")
                })?,
                sub_vault_privacy_tier: row.get(7).map_err(|err| {
                    format!("Failed decoding sub-vault privacy field in assembler: {err}")
                })?,
                vault_privacy_tier: row.get(8).map_err(|err| {
                    format!("Failed decoding vault privacy field in assembler: {err}")
                })?,
            });
        }
    }

    let mut nodes_map: std::collections::HashMap<String, AssemblerNode> = nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect();

    let mut sorted_nodes = Vec::with_capacity(node_ids.len());
    for id in node_ids {
        if let Some(node) = nodes_map.remove(id) {
            sorted_nodes.push(node);
        }
    }

    Ok(sorted_nodes)
}

pub fn build_context(
    db: &Connection,
    node_ids: Vec<String>,
    config: AssemblerConfig,
) -> Result<String, crate::AppError> {
    let nodes = fetch_requested_nodes(db, &node_ids)?;

    let mut assembled = String::new();

    for node in nodes {
        let effective_tier = get_effective_privacy(
            node.node_privacy_tier.as_deref(),
            node.sub_vault_privacy_tier.as_deref(),
            node.vault_privacy_tier.as_deref(),
        );

        let policy = match config.scope.as_str() {
            "cloud" => cloud_llm_context_policy(effective_tier),
            "local" => local_llm_context_policy(effective_tier, config.is_unlocked),
            _ => unrestricted_llm_context_policy(effective_tier),
        };

        let block = match policy {
            LlmContextPolicy::Omit => continue,
            LlmContextPolicy::Stub => generate_pointer_stub(&node.title, &node.id),
            LlmContextPolicy::Full => format!(
                "<document title=\"{}\">\n{}\n\n{}\n</document>",
                escape_xml_attr(&node.title),
                escape_xml_body(&node.summary),
                escape_xml_body(&node.detail)
            ),
        };

        let candidate = if assembled.is_empty() {
            block.clone()
        } else {
            format!("{assembled}\n\n{block}")
        };

        if count_tokens(&candidate) > config.max_tokens {
            if assembled.is_empty() {
                let trimmed = trim_tail_with_attention_sink(&block, config.max_tokens);
                if !trimmed.is_empty() {
                    assembled = trimmed;
                }
            }
            break;
        }

        assembled = candidate;
    }

    Ok(trim_tail_with_attention_sink(&assembled, config.max_tokens))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        build_context, count_tokens, fetch_requested_nodes, trim_tail_with_attention_sink,
        AssemblerConfig, ATTENTION_SINK_TOKENS, SQLITE_IN_CLAUSE_BATCH_SIZE,
    };
    use rusqlite::Connection;

    fn setup_in_memory_db() -> Connection {
        let conn = match Connection::open_in_memory() {
            Ok(db) => db,
            Err(err) => panic!("failed opening in-memory sqlite for assembler test: {err}"),
        };

        if let Err(err) = conn.execute_batch(
            "CREATE TABLE vaults (
                id TEXT PRIMARY KEY,
                privacy_tier TEXT NOT NULL,
                deleted_at TEXT
            );
            CREATE TABLE sub_vaults (
                id TEXT PRIMARY KEY,
                vault_id TEXT NOT NULL,
                privacy_tier TEXT,
                deleted_at TEXT
            );
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY,
                vault_id TEXT NOT NULL,
                sub_vault_id TEXT,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                detail TEXT,
                privacy_tier TEXT,
                priority TEXT NOT NULL,
                is_archived INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT
            );",
        ) {
            panic!("failed creating assembler test schema: {err}");
        }

        if let Err(err) = conn.execute(
            "INSERT INTO vaults (id, privacy_tier, deleted_at) VALUES (?1, ?2, NULL);",
            ["vault_a", "open"],
        ) {
            panic!("failed inserting vault for assembler test: {err}");
        }

        if let Err(err) = conn.execute(
            "INSERT INTO sub_vaults (id, vault_id, privacy_tier, deleted_at) VALUES (?1, ?2, ?3, NULL);",
            ["subvault_redacted", "vault_a", "redacted"],
        ) {
            panic!("failed inserting sub-vault for assembler test: {err}");
        }

        if let Err(err) = conn.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, title, summary, detail, privacy_tier, priority, is_archived, deleted_at
            ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 0, NULL);",
            [
                "node_local_only",
                "vault_a",
                "Local Only Node",
                "local summary",
                "local detail",
                "local_only",
                "{\"access_count_30active\":6}",
            ],
        ) {
            panic!("failed inserting node for assembler test: {err}");
        }

        if let Err(err) = conn.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, title, summary, detail, privacy_tier, priority, is_archived, deleted_at
            ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 0, NULL);",
            [
                "node_locked",
                "vault_a",
                "Locked Node",
                "locked summary",
                "locked detail",
                "locked",
                "{\"access_count_30active\":6}",
            ],
        ) {
            panic!("failed inserting node for assembler test: {err}");
        }

        if let Err(err) = conn.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, title, summary, detail, privacy_tier, priority, is_archived, deleted_at
            ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 0, NULL);",
            [
                "node_redacted",
                "vault_a",
                "Redacted Node",
                "redacted summary",
                "redacted detail",
                "redacted",
                "{\"access_count_30active\":6}",
            ],
        ) {
            panic!("failed inserting node for assembler test: {err}");
        }

        if let Err(err) = conn.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, title, summary, detail, privacy_tier, priority, is_archived, deleted_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, NULL);",
            [
                "node_nested_redacted",
                "vault_a",
                "subvault_redacted",
                "Nested Redacted Node",
                "nested summary",
                "nested detail",
                "open",
                "{\"access_count_30active\":6}",
            ],
        ) {
            panic!("failed inserting nested redacted node for assembler test: {err}");
        }

        conn
    }

    #[test]
    fn test_context_assembler_privacy_tiers() {
        let conn = setup_in_memory_db();
        let node_ids = vec![
            "node_local_only".to_string(),
            "node_locked".to_string(),
            "node_redacted".to_string(),
            "node_nested_redacted".to_string(),
        ];

        // 1. Local scope (locked): local_only included; locked, redacted are stubbed.
        let local_result = match build_context(
            &conn,
            node_ids.clone(),
            AssemblerConfig {
                scope: "local".to_string(),
                max_tokens: 4000,
                is_unlocked: false,
            },
        ) {
            Ok(value) => value,
            Err(err) => panic!("local scope assembler failed: {err}"),
        };

        // local_only is included
        assert!(local_result.contains("<document title=\"Local Only Node\">"));
        assert!(local_result.contains("local detail"));
        // locked is stubbed since we aren't unlocked
        assert!(local_result.contains("[LOCKED NODE STUB] Title: Locked Node"));
        // redacted is fully omitted, even for local models
        assert!(!local_result.contains("Redacted Node"));
        assert!(!local_result.contains("Nested Redacted Node"));

        // 1.5 Local scope (unlocked): locked is fully included.
        let local_unlocked_result = match build_context(
            &conn,
            node_ids.clone(),
            AssemblerConfig {
                scope: "local".to_string(),
                max_tokens: 4000,
                is_unlocked: true,
            },
        ) {
            Ok(value) => value,
            Err(err) => panic!("local scope assembler failed: {err}"),
        };
        assert!(local_unlocked_result.contains("<document title=\"Locked Node\">"));
        assert!(local_unlocked_result.contains("locked detail"));
        assert!(!local_unlocked_result.contains("Redacted Node"));

        // 2. Cloud scope: locked is stubbed; local_only and redacted are completely omitted.
        let cloud_result = match build_context(
            &conn,
            node_ids,
            AssemblerConfig {
                scope: "cloud".to_string(),
                max_tokens: 4000,
                is_unlocked: false,
            },
        ) {
            Ok(value) => value,
            Err(err) => panic!("cloud scope assembler failed: {err}"),
        };

        // locked is stubbed
        assert!(cloud_result.contains("[LOCKED NODE STUB] Title: Locked Node"));
        // local_only is completely omitted
        assert!(!cloud_result.contains("Local Only Node"));
        // redacted is completely omitted
        assert!(!cloud_result.contains("Redacted Node"));
        // nested redacted inheritance is completely omitted
        assert!(!cloud_result.contains("Nested Redacted Node"));
    }

    #[test]
    fn trimming_preserves_prompt_head_attention_sink() {
        let conn = setup_in_memory_db();
        let large_detail = "x".repeat(4000);
        if let Err(err) = conn.execute(
            "UPDATE nodes SET detail = ?2 WHERE id = ?1;",
            ["node_local_only", large_detail.as_str()],
        ) {
            panic!("failed updating large detail for assembler test: {err}");
        }

        let full = match build_context(
            &conn,
            vec!["node_local_only".to_string()],
            AssemblerConfig {
                scope: "local".to_string(),
                max_tokens: 4000,
                is_unlocked: false,
            },
        ) {
            Ok(value) => value,
            Err(err) => panic!("full assembler context failed: {err}"),
        };

        let trimmed = match build_context(
            &conn,
            vec!["node_local_only".to_string()],
            AssemblerConfig {
                scope: "local".to_string(),
                max_tokens: 120,
                is_unlocked: false,
            },
        ) {
            Ok(value) => value,
            Err(err) => panic!("trimmed assembler context failed: {err}"),
        };

        let bpe = match tiktoken_rs::cl100k_base() {
            Ok(value) => value,
            Err(err) => panic!("cl100k in test failed: {err}"),
        };
        let full_ids = bpe.encode_with_special_tokens(&full);
        let sink_n = full_ids.len().min(ATTENTION_SINK_TOKENS);
        let head = match bpe.decode(full_ids[..sink_n].to_vec()) {
            Ok(value) => value,
            Err(err) => panic!("decode sink prefix failed: {err}"),
        };
        assert!(trimmed.starts_with(&head));
        assert!(count_tokens(&trimmed) <= 120);

        let direct = trim_tail_with_attention_sink(&full, 120);
        assert_eq!(direct, trimmed);
    }

    #[test]
    fn fetch_requested_nodes_chunks_large_in_lists() {
        let conn = setup_in_memory_db();

        for index in 0..(SQLITE_IN_CLAUSE_BATCH_SIZE + 25) {
            let node_id = format!("node_extra_{index}");
            let title = format!("Extra Node {index}");
            let summary = format!("Extra summary {index}");
            let detail = format!("Extra detail {index}");
            let priority = format!("{{\"score\":{}}}", index as f64 / 1000.0);
            if let Err(err) = conn.execute(
                "INSERT INTO nodes (
                    id, vault_id, sub_vault_id, title, summary, detail, privacy_tier, priority, is_archived, deleted_at
                ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 0, NULL);",
                [
                    node_id.as_str(),
                    "vault_a",
                    title.as_str(),
                    summary.as_str(),
                    detail.as_str(),
                    "open",
                    priority.as_str(),
                ],
            ) {
                panic!("failed inserting extra node for assembler batching test: {err}");
            }
        }

        let mut node_ids = Vec::new();
        for index in 0..(SQLITE_IN_CLAUSE_BATCH_SIZE + 25) {
            node_ids.push(format!("node_extra_{index}"));
        }

        let nodes = match fetch_requested_nodes(&conn, &node_ids) {
            Ok(value) => value,
            Err(err) => panic!("batched fetch_requested_nodes failed: {err}"),
        };

        assert_eq!(nodes.len(), SQLITE_IN_CLAUSE_BATCH_SIZE + 25);
        assert!(nodes.iter().any(|node| node.id == "node_extra_0"));
        assert!(nodes
            .iter()
            .any(|node| node.id == format!("node_extra_{}", SQLITE_IN_CLAUSE_BATCH_SIZE + 24)));
    }

    #[test]
    fn test_assembler_preserves_relevance_ordering() {
        let conn = setup_in_memory_db();
        // Insert nodes with different priority values to make sure priority sorting
        // is overridden by relevance ordering
        let node_ids = vec!["node_locked".to_string(), "node_local_only".to_string()];

        let nodes = fetch_requested_nodes(&conn, &node_ids).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id, "node_locked");
        assert_eq!(nodes[1].id, "node_local_only");

        let context = build_context(
            &conn,
            node_ids,
            AssemblerConfig {
                scope: "local".to_string(),
                max_tokens: 4000,
                is_unlocked: true,
            },
        )
        .unwrap();

        // The assembled context should contain Locked Node document before Local Only Node document
        let pos_locked = context.find("title=\"Locked Node\"").unwrap();
        let pos_local = context.find("title=\"Local Only Node\"").unwrap();
        assert!(
            pos_locked < pos_local,
            "Locked Node should come before Local Only Node to preserve relevance ordering!"
        );
    }

    #[test]
    fn test_xml_escaping_behavior() {
        let conn = setup_in_memory_db();
        // Insert a node with special characters in title, summary, and detail
        if let Err(err) = conn.execute(
            "INSERT INTO nodes (
                id, vault_id, sub_vault_id, title, summary, detail, privacy_tier, priority, is_archived, deleted_at
            ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 0, NULL);",
            [
                "node_escaping_test",
                "vault_a",
                "Testing \"Quotes\" & <Tags> 'Single'",
                "Summary with & < > \"Quotes\" and 'Single'",
                "Detail with & < > \"Quotes\" and 'Single'",
                "open",
                "{\"score\":0.5}",
            ],
        ) {
            panic!("failed inserting extra node for escaping test: {err}");
        }

        let context = build_context(
            &conn,
            vec!["node_escaping_test".to_string()],
            AssemblerConfig {
                scope: "local".to_string(),
                max_tokens: 4000,
                is_unlocked: false,
            },
        )
        .unwrap();

        // Title is an attribute and must be fully escaped (including quotes)
        assert!(context.contains(
            "title=\"Testing &quot;Quotes&quot; &amp; &lt;Tags&gt; &apos;Single&apos;\""
        ));

        // Summary is in the body: &, <, > must be escaped; quotes must NOT be escaped
        assert!(context.contains("Summary with &amp; &lt; &gt; \"Quotes\" and 'Single'"));

        // Detail is in the body: &, <, > must be escaped; quotes must NOT be escaped
        assert!(context.contains("Detail with &amp; &lt; &gt; \"Quotes\" and 'Single'"));
    }
}
