use serde::{Deserialize, Serialize};

const ALLOWED_NODE_TYPES: [&str; 8] = [
    "concept",
    "fact",
    "project",
    "preference",
    "event",
    "instruction",
    "identity",
    "summary",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposedNode {
    pub title: String,
    pub summary: String,
    pub detail: Option<String>,
    pub category: Option<String>,
    pub target_vault_key: Option<String>,
    pub tags: Option<Vec<String>>,
    pub node_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProposalEnvelope {
    proposals: Vec<RawProposedNode>,
}

#[derive(Debug, Deserialize)]
struct RawProposedNode {
    title: String,
    summary: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    target_vault_key: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    node_type: Option<String>,
}

fn normalize_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_required(value: String, field_name: &str, index: usize) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "Proposal {} has empty required field '{}'",
            index + 1,
            field_name
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_node_type(value: Option<String>, index: usize) -> Result<Option<String>, String> {
    let Some(raw_value) = value else {
        return Ok(None);
    };
    let normalized = raw_value.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(None);
    }
    if ALLOWED_NODE_TYPES.contains(&normalized.as_str()) {
        Ok(Some(normalized))
    } else {
        Err(format!(
            "Proposal {} has unsupported node_type '{}'",
            index + 1,
            raw_value
        ))
    }
}

fn normalize_tags(tags: Option<Vec<String>>, index: usize) -> Result<Option<Vec<String>>, String> {
    let Some(values) = tags else {
        return Ok(None);
    };

    let mut normalized = Vec::new();
    for tag in values {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Err(format!("Proposal {} includes an empty tag", index + 1));
        }
        normalized.push(trimmed.to_string());
    }

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

/// Bundled Onboarding Agent: fixed system prompt for one-shot extraction from Q&A JSON.
pub const ONBOARDING_EXTRACTION_SYSTEM_PROMPT: &str = r#"You are Amber's onboarding extractor. The user submitted plain onboarding answers as JSON (not a chat).

Your job: infer concise memory nodes they would want in a personal knowledge base.

Output rules:
- Respond with ONLY valid JSON. No markdown fences, no commentary before or after.
- Shape: { "proposals": [ ... ] }
- Each proposal MUST include: "title" (short), "summary" (one or two sentences).
- Each proposal MUST include EITHER "category" OR "target_vault_key" (never omit both).
- Optional: "detail", "tags" (string array), "node_type".

Allowed "category" values (lowercase): demographics, personal, interests, work, learning, health, finance, credentials.

Allowed "target_vault_key" values (lowercase): demographics, personal, interests, work, learning, health, finance, credentials (same intent as category; use when clearer).

Allowed "node_type" values (lowercase): concept, fact, project, preference, event, instruction, identity, summary.

Split distinct themes into separate proposals. Prefer 3–12 proposals unless the answers are very sparse."#;

/// Ensure `answers_json` is a JSON object (Opaque user payload from the Basics step).
pub fn validate_answers_json(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Onboarding answers JSON is empty".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|err| format!("Invalid answers JSON: {err}"))?;
    if !value.is_object() {
        return Err("Onboarding answers must be a JSON object at the top level".to_string());
    }
    Ok(())
}

pub fn build_onboarding_extraction_user_message(answers_json: &str) -> String {
    format!(
        "Here is the user's onboarding answers as JSON. Extract proposals as specified.\n\n{}",
        answers_json.trim()
    )
}

/// After fence removal, strip a leading `json` language token when it appears on the same line as
/// the payload (e.g. `` ```json {"proposals":...} ``` `` with no newline after `json`).
fn strip_leading_json_fence_language_tag(text: &str) -> String {
    let text = text.trim();
    let Some(head) = text.as_bytes().get(..4) else {
        return text.to_string();
    };
    if !head.eq_ignore_ascii_case(b"json") {
        return text.to_string();
    }
    let Some(rest) = text.get(4..) else {
        return text.to_string();
    };
    let rest = rest.trim_start();
    if rest.starts_with('{') || rest.starts_with('[') {
        rest.trim().to_string()
    } else {
        text.to_string()
    }
}

/// Trim optional ```json ... ``` wrappers from model output before parsing.
pub fn normalize_llm_json_response(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if s.starts_with("```") {
        if let Some(rest) = s.strip_prefix("```") {
            let mut inner = rest.trim_start();
            if let Some(idx) = inner.find('\n') {
                inner = inner[idx + 1..].trim_start();
            }
            if let Some(end) = inner.rfind("```") {
                inner = inner[..end].trim();
            }
            s = inner.to_string();
        }
    }
    strip_leading_json_fence_language_tag(&s)
}

/// Parse proposals JSON after optional markdown fence stripping (what LLMs often emit).
pub fn parse_proposals_from_llm_output(
    raw_model_output: &str,
) -> Result<Vec<ProposedNode>, String> {
    let normalized = normalize_llm_json_response(raw_model_output);
    parse_proposals_json(&normalized)
}

/// Parse strict onboarding proposal JSON for both interview extraction and paste import.
/// The accepted payload shape is:
/// `{ "proposals": [ { "title": "...", "summary": "...", ... } ] }`
pub fn parse_proposals_json(raw_json: &str) -> Result<Vec<ProposedNode>, String> {
    let envelope: ProposalEnvelope = serde_json::from_str(raw_json)
        .map_err(|err| format!("Invalid onboarding proposals JSON: {err}"))?;

    envelope
        .proposals
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let title = normalize_required(raw.title, "title", index)?;
            let summary = normalize_required(raw.summary, "summary", index)?;
            let detail = normalize_non_empty(raw.detail);
            let category = normalize_non_empty(raw.category).map(|v| v.to_lowercase());
            let target_vault_key =
                normalize_non_empty(raw.target_vault_key).map(|v| v.to_lowercase());
            if category.is_none() && target_vault_key.is_none() {
                return Err(format!(
                    "Proposal {} must include 'category' or 'target_vault_key'",
                    index + 1
                ));
            }

            let tags = normalize_tags(raw.tags, index)?;
            let node_type = validate_node_type(raw.node_type, index)?;

            Ok(ProposedNode {
                title,
                summary,
                detail,
                category,
                target_vault_key,
                tags,
                node_type,
            })
        })
        .collect()
}

/// Stable category keys used by onboarding prompts and import parsers.
/// Keep these aligned with `db/migrations/0003_onboarding_default_vaults.sql`.
pub fn vault_id_for_category_key(category_key: &str) -> Option<&'static str> {
    match category_key.trim().to_lowercase().as_str() {
        "demographics" => Some("vault_personal"),
        "interests" | "personal" => Some("vault_personal"),
        "work" => Some("vault_work"),
        "learning" => Some("vault_learning"),
        "health" => Some("vault_health"),
        "finance" => Some("vault_finance"),
        "credentials" => Some("vault_credentials"),
        _ => None,
    }
}

/// Canonical onboarding vault ID → category key for import targeting.
///
/// Not a mechanical inversion of [`vault_id_for_category_key`] (that map is many-to-one:
/// `demographics`, `interests`, and `personal` all resolve to `vault_personal`). Import uses
/// `"personal"` as the canonical key for `vault_personal`; `demographics` remains an
/// onboarding-only alias accepted by [`vault_id_for_category_key`].
///
/// Returns `None` for custom / non-onboarding vault IDs (Commit 10 option a: those imports
/// fall back to `vault_root_graph` at changeset build time).
pub fn category_key_for_vault_id(vault_id: &str) -> Option<&'static str> {
    match vault_id.trim() {
        "vault_personal" => Some("personal"),
        "vault_work" => Some("work"),
        "vault_learning" => Some("learning"),
        "vault_health" => Some("health"),
        "vault_finance" => Some("finance"),
        "vault_credentials" => Some("credentials"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        category_key_for_vault_id, normalize_llm_json_response, parse_proposals_from_llm_output,
        parse_proposals_json, validate_answers_json, vault_id_for_category_key,
    };

    #[test]
    fn parse_valid_proposals_golden() {
        let payload = r#"{
  "proposals": [
    {
      "title": "Primary work focus",
      "summary": "Leading customer onboarding revamp project",
      "detail": "Cross-functional rollout tracked weekly",
      "category": "work",
      "tags": ["project", "priority"],
      "node_type": "project"
    },
    {
      "title": "Prefers short daily workouts",
      "summary": "20-minute routines are easiest to sustain",
      "target_vault_key": "health"
    }
  ]
}"#;

        let parsed = match parse_proposals_json(payload) {
            Ok(value) => value,
            Err(err) => panic!("expected valid proposals payload: {err}"),
        };

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].title, "Primary work focus");
        assert_eq!(parsed[0].category.as_deref(), Some("work"));
        assert_eq!(parsed[0].node_type.as_deref(), Some("project"));
        assert_eq!(parsed[1].target_vault_key.as_deref(), Some("health"));
    }

    #[test]
    fn parse_rejects_trailing_junk() {
        let payload =
            r#"{"proposals":[{"title":"A","summary":"B","category":"personal"}]} trailing"#;
        let err = match parse_proposals_json(payload) {
            Ok(_) => panic!("expected trailing junk payload to fail"),
            Err(value) => value,
        };
        assert!(err.contains("Invalid onboarding proposals JSON"));
    }

    #[test]
    fn parse_rejects_missing_required_field() {
        let payload = r#"{"proposals":[{"title":"A","category":"personal"}]}"#;
        let err = match parse_proposals_json(payload) {
            Ok(_) => panic!("expected missing required field payload to fail"),
            Err(value) => value,
        };
        assert!(err.contains("Invalid onboarding proposals JSON"));
    }

    #[test]
    fn parse_rejects_without_category_or_target() {
        let payload = r#"{"proposals":[{"title":"A","summary":"B"}]}"#;
        let err = match parse_proposals_json(payload) {
            Ok(_) => panic!("expected category/target validation to fail"),
            Err(value) => value,
        };
        assert!(err.contains("must include 'category' or 'target_vault_key'"));
    }

    #[test]
    fn vault_map_resolves_expected_keys() {
        assert_eq!(
            vault_id_for_category_key("personal"),
            Some("vault_personal")
        );
        assert_eq!(
            vault_id_for_category_key("Demographics"),
            Some("vault_personal")
        );
        assert_eq!(vault_id_for_category_key("unknown"), None);
    }

    #[test]
    fn category_key_for_vault_id_maps_onboarding_vaults() {
        assert_eq!(
            category_key_for_vault_id("vault_learning"),
            Some("learning")
        );
        assert_eq!(
            category_key_for_vault_id("vault_personal"),
            Some("personal")
        );
        assert_eq!(category_key_for_vault_id("vault_custom_user"), None);
    }

    #[test]
    fn category_key_for_vault_id_uses_personal_not_demographics() {
        assert_eq!(
            category_key_for_vault_id("vault_personal"),
            Some("personal")
        );
        assert_ne!(
            category_key_for_vault_id("vault_personal"),
            Some("demographics")
        );
    }

    #[test]
    fn validate_answers_json_accepts_object() {
        assert!(validate_answers_json(r#"{"name":"Ada","focus":"work"}"#).is_ok());
    }

    #[test]
    fn validate_answers_json_rejects_array() {
        assert!(validate_answers_json(r#"[1,2]"#).is_err());
    }

    #[test]
    fn normalize_llm_json_response_strips_fence() {
        let raw = "```json\n{\"proposals\":[{\"title\":\"T\",\"summary\":\"S\",\"category\":\"work\"}]}\n```";
        let normalized = normalize_llm_json_response(raw);
        assert!(normalized.starts_with('{') && normalized.ends_with('}'));
        assert!(!normalized.contains("```"));
    }

    #[test]
    fn normalize_llm_json_response_strips_json_prefix_when_same_line_as_payload() {
        let raw = "```json {\"proposals\":[{\"title\":\"A\",\"summary\":\"B\",\"category\":\"personal\"}]} ```";
        let normalized = normalize_llm_json_response(raw);
        assert!(
            normalized.starts_with('{'),
            "expected raw JSON object, got: {normalized:?}"
        );
        assert!(
            !normalized[..normalized.len().min(12)].contains("json"),
            "leading json token should be removed"
        );
        parse_proposals_json(&normalized).unwrap_or_else(|err| {
            panic!("parse proposals after same-line json prefix: {err}");
        });
    }

    #[test]
    fn normalize_llm_json_response_no_panic_when_byte_four_not_char_boundary() {
        // First glyph is 3 UTF-8 bytes; second is 2 bytes — byte index 4 splits the second char,
        // so `text[..4]` would panic; normalization must stay panic-free on odd prefixes.
        let s = "\u{0800}\u{0410}{\"proposals\":[]}";
        let normalized = normalize_llm_json_response(s);
        assert!(
            normalized.contains("proposals"),
            "unexpected normalization: {normalized:?}"
        );
    }

    #[test]
    fn parse_proposals_from_llm_output_fenced_roundtrip() {
        let raw = "```json
{\"proposals\":[{\"title\":\"Dev\",\"summary\":\"Ships features\",\"category\":\"work\",\"node_type\":\"project\"}]}
```";
        let parsed = parse_proposals_from_llm_output(raw).unwrap_or_else(|err| {
            panic!("parse fenced output: {err}");
        });
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Dev");
    }

    #[test]
    fn parse_allows_unknown_fields() {
        let payload = r#"{
  "proposals": [
    {
      "title": "Dev",
      "summary": "Ships features",
      "category": "work",
      "some_unsupported_field": "ignored"
    }
  ],
  "unsupported_envelope_field": "ignored"
}"#;
        let parsed = match parse_proposals_json(payload) {
            Ok(value) => value,
            Err(err) => panic!("Expected unknown fields to be allowed: {err}"),
        };
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Dev");
    }
}
