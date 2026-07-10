use crate::onboarding::normalize_llm_json_response;
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

const ALLOWED_VAULT_KEYS: [&str; 8] = [
    "demographics",
    "personal",
    "interests",
    "work",
    "learning",
    "health",
    "finance",
    "credentials",
];

/// The requested action classification of a parsed memory candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CandidateAction {
    /// Add the candidate as a brand new node.
    #[default]
    Add,
    /// Update an existing node with the candidate's content.
    Update,
    /// Delete an existing node matching the candidate.
    Delete,
}

/// A parsed candidate memory extracted from LLM session completions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateNode {
    /// The short, descriptive title of the candidate node.
    pub title: String,
    /// The concise summary describing the candidate memory.
    pub summary: String,
    /// The detailed markdown content/notes of the candidate, if any.
    pub detail: Option<String>,
    /// The specific category type (e.g. concept, fact, project).
    pub node_type: Option<String>,
    /// The category vault key (e.g. learning, health, personal) targeting a specific vault.
    pub target_vault_key: Option<String>,
    /// Associated tags describing this candidate memory.
    pub tags: Option<Vec<String>>,
    /// Confidence score assigned by the LLM extractor (clamped to 0.0 - 1.0).
    pub confidence: f64,
    /// The action to perform (Add, Update, or Delete).
    pub action: CandidateAction,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

impl Default for CandidateNode {
    fn default() -> Self {
        Self {
            title: String::new(),
            summary: String::new(),
            detail: None,
            node_type: None,
            target_vault_key: None,
            tags: None,
            confidence: 1.0,
            action: CandidateAction::Add,
            source: None,
            source_type: None,
            meta: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CandidateEnvelope {
    candidates: Vec<RawCandidateNode>,
}

fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct RawCandidateNode {
    title: String,
    summary: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    node_type: Option<String>,
    #[serde(default)]
    target_vault_key: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default)]
    action: CandidateAction,
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
            "Candidate {} has empty required field '{}'",
            index + 1,
            field_name
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_node_type(value: Option<String>, index: usize) -> Option<String> {
    let raw_value = value?;
    let normalized = raw_value.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if ALLOWED_NODE_TYPES.contains(&normalized.as_str()) {
        Some(normalized)
    } else {
        eprintln!(
            "Warning: Candidate {} has unsupported node_type '{}'; falling back to None.",
            index + 1,
            raw_value
        );
        None
    }
}

fn validate_target_vault_key(
    value: Option<String>,
    allowed_keys: Option<&[String]>,
    index: usize,
) -> Option<String> {
    let raw_value = value?;
    let normalized = raw_value.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let is_valid = if let Some(keys) = allowed_keys {
        keys.iter().any(|k| k.to_lowercase() == normalized)
    } else {
        ALLOWED_VAULT_KEYS.contains(&normalized.as_str())
    };

    if is_valid {
        Some(normalized)
    } else {
        eprintln!(
            "Warning: Candidate {} has unsupported target_vault_key '{}'; falling back to None.",
            index + 1,
            raw_value
        );
        None
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
            return Err(format!("Candidate {} includes an empty tag", index + 1));
        }
        normalized.push(trimmed.to_string());
    }

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

/// Parses and validates a raw strict JSON string representing memory extraction candidates.
///
/// Enforces correct structures, node types, target vault keys, and normalizes tags and confidence scores.
pub fn parse_candidates_json(
    raw_json: &str,
    allowed_vault_keys: Option<&[String]>,
) -> Result<Vec<CandidateNode>, String> {
    let envelope: CandidateEnvelope =
        serde_json::from_str(raw_json).map_err(|err| format!("Invalid candidates JSON: {err}"))?;

    let parsed: Vec<CandidateNode> = envelope
        .candidates
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let title = normalize_required(raw.title, "title", index)?;
            let summary = normalize_required(raw.summary, "summary", index)?;
            let detail = normalize_non_empty(raw.detail);
            let node_type = validate_node_type(raw.node_type, index);
            let target_vault_key =
                validate_target_vault_key(raw.target_vault_key, allowed_vault_keys, index);
            let tags = normalize_tags(raw.tags, index)?;

            // Clamp confidence score to 0.0 - 1.0
            let confidence = raw.confidence.clamp(0.0, 1.0);

            Ok(CandidateNode {
                title,
                summary,
                detail,
                node_type,
                target_vault_key,
                tags,
                confidence,
                action: raw.action,
                source: None,
                source_type: None,
                meta: None,
            })
        })
        .collect::<Result<Vec<CandidateNode>, String>>()?;

    let filtered = parsed
        .into_iter()
        .filter(|node| node.confidence >= 0.3)
        .collect();

    Ok(filtered)
}

/// Normalizes raw LLM output (removing markdown code blocks/fences) and parses candidate nodes.
pub fn parse_candidates_from_llm_output(
    raw_model_output: &str,
    allowed_vault_keys: Option<&[String]>,
) -> Result<Vec<CandidateNode>, String> {
    let normalized = normalize_llm_json_response(raw_model_output);
    parse_candidates_json(&normalized, allowed_vault_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_candidates_golden() {
        let payload = r#"{
  "candidates": [
    {
      "action": "add",
      "title": "Loves hiking",
      "summary": "Enjoys outdoor hiking in summer.",
      "detail": "Usually goes on weekends",
      "node_type": "preference",
      "target_vault_key": "personal",
      "tags": ["outdoors", "hobbies"],
      "confidence": 0.95
    },
    {
      "action": "update",
      "title": "Primary project name",
      "summary": "Project is now named MindVault.",
      "confidence": 1.0
    },
    {
      "action": "delete",
      "title": "Old address",
      "summary": "Moved out of San Francisco.",
      "confidence": 0.8
    }
  ]
}"#;

        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => panic!("Expected valid candidates: {err}"),
        };
        assert_eq!(parsed.len(), 3);

        assert_eq!(parsed[0].action, CandidateAction::Add);
        assert_eq!(parsed[0].title, "Loves hiking");
        assert_eq!(parsed[0].summary, "Enjoys outdoor hiking in summer.");
        assert_eq!(
            parsed[0].detail.as_deref(),
            Some("Usually goes on weekends")
        );
        assert_eq!(parsed[0].node_type.as_deref(), Some("preference"));
        assert_eq!(parsed[0].target_vault_key.as_deref(), Some("personal"));
        assert_eq!(
            parsed[0].tags,
            Some(vec!["outdoors".to_string(), "hobbies".to_string()])
        );
        assert_eq!(parsed[0].confidence, 0.95);

        assert_eq!(parsed[1].action, CandidateAction::Update);
        assert_eq!(parsed[1].title, "Primary project name");
        assert_eq!(parsed[1].summary, "Project is now named MindVault.");
        assert_eq!(parsed[1].confidence, 1.0);

        assert_eq!(parsed[2].action, CandidateAction::Delete);
        assert_eq!(parsed[2].title, "Old address");
        assert_eq!(parsed[2].summary, "Moved out of San Francisco.");
        assert_eq!(parsed[2].confidence, 0.8);
    }

    #[test]
    fn parse_empty_candidates() {
        let payload = r#"{"candidates":[]}"#;
        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => panic!("Expected empty candidates to parse: {err}"),
        };
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_missing_required_title() {
        let payload = r#"{
  "candidates": [
    {
      "summary": "Enjoys outdoor hiking in summer.",
      "confidence": 0.9
    }
  ]
}"#;
        let err = match parse_candidates_json(payload, None) {
            Ok(_) => panic!("expected missing required field payload to fail"),
            Err(e) => e,
        };
        assert!(err.contains("Invalid candidates JSON") || err.contains("missing field `title`"));
    }

    #[test]
    fn parse_allows_unknown_fields() {
        let payload = r#"{
  "candidates": [
    {
      "title": "Hiking",
      "summary": "Enjoys outdoor hiking.",
      "confidence": 0.9,
      "unsupported_extra_field": "oops"
    }
  ]
}"#;
        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => panic!("Expected unknown field payload to parse successfully: {err}"),
        };
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Hiking");
    }

    #[test]
    fn parse_omitted_confidence_defaults_to_one() {
        let payload = r#"{
  "candidates": [
    {
      "title": "Hiking",
      "summary": "Enjoys outdoor hiking."
    }
  ]
}"#;
        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => panic!("Expected omitted confidence payload to parse successfully: {err}"),
        };
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Hiking");
        assert_eq!(parsed[0].confidence, 1.0);
    }

    #[test]
    fn parse_invalid_node_type() {
        let payload = r#"{
  "candidates": [
    {
      "title": "Hiking",
      "summary": "Enjoys outdoor hiking.",
      "node_type": "super_fancy_type",
      "confidence": 0.9
    }
  ]
}"#;
        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => panic!("expected invalid node type to fall back to None, not fail: {err}"),
        };
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].node_type, None);
    }

    #[test]
    fn parse_invalid_node_type_falls_back_to_none() {
        let payload = r#"{
    "candidates": [
        {
            "action": "add",
            "title": "Hiking",
            "summary": "Enjoys outdoor hiking.",
            "node_type": "preferences",
            "confidence": 0.9
        },
        {
            "action": "add",
            "title": "Valid preference",
            "summary": "Prefers quiet coffee shops.",
            "node_type": "preference",
            "confidence": 0.95
        }
    ]
}"#;

        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => {
                panic!("Expected invalid node_type to be downgraded, not fail parsing: {err}")
            }
        };

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].node_type, None);
        assert_eq!(parsed[1].node_type.as_deref(), Some("preference"));
    }

    #[test]
    fn parse_confidence_clamping() {
        let payload = r#"{
  "candidates": [
    {
      "title": "High confidence",
      "summary": "This is a fact.",
      "confidence": 2.5
    },
    {
      "title": "Too low confidence",
      "summary": "This is a rumor.",
      "confidence": 0.25
    },
    {
      "title": "Valid confidence",
      "summary": "This is a plausible idea.",
      "confidence": 0.35
    }
  ]
}"#;
        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => panic!("Expected confidence clamping candidates to parse: {err}"),
        };
        // Expect 2 kept candidates: High confidence (clamped to 1.0) and Valid confidence (0.35)
        // Too low confidence (0.25) is filtered out (< 0.3)
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].title, "High confidence");
        assert_eq!(parsed[0].confidence, 1.0);
        assert_eq!(parsed[1].title, "Valid confidence");
        assert_eq!(parsed[1].confidence, 0.35);
    }

    #[test]
    fn parse_invalid_target_vault_key() {
        let payload = r#"{
  "candidates": [
    {
      "title": "Hiking",
      "summary": "Enjoys outdoor hiking.",
      "target_vault_key": "super_fancy_vault",
      "confidence": 0.9
    }
  ]
}"#;
        let parsed = match parse_candidates_json(payload, None) {
            Ok(val) => val,
            Err(err) => {
                panic!("expected invalid target vault key to fall back to None, not fail: {err}")
            }
        };
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].target_vault_key, None);
    }

    #[test]
    fn parse_custom_target_vault_key_with_allowed_keys() {
        let payload = r#"{
  "candidates": [
    {
      "title": "Cooking",
      "summary": "Likes to cook Italian food.",
      "target_vault_key": "recipes",
      "confidence": 0.95
    },
    {
      "title": "Tax records",
      "summary": "Wants to keep tax files.",
      "target_vault_key": "finance",
      "confidence": 0.90
    }
  ]
}"#;
        let allowed = vec!["recipes".to_string(), "taxes".to_string()];
        let parsed = match parse_candidates_json(payload, Some(&allowed)) {
            Ok(v) => v,
            Err(e) => panic!("Expected parsing to succeed: {e}"),
        };
        assert_eq!(parsed.len(), 2);
        // "recipes" should be preserved because it's in the allowed list
        assert_eq!(parsed[0].target_vault_key.as_deref(), Some("recipes"));
        // "finance" should be dropped to None because it's not in the allowed list
        assert_eq!(parsed[1].target_vault_key, None);
    }
}
