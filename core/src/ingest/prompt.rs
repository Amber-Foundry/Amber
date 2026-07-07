/// Dynamically builds a concise system prompt instructing the model to parse facts
/// into the JSON contract, using the provided list of allowed vault keys.
pub fn get_system_prompt(allowed_vault_keys: &[String]) -> String {
    let vaults_str = if allowed_vault_keys.is_empty() {
        "demographics, personal, interests, work, learning, health, finance, credentials"
            .to_string()
    } else {
        allowed_vault_keys.join(", ")
    };

    format!(
        "You are Amber's background Document Ingestion Agent. Analyze the document segment enclosed in <document>...</document> tags and return structured memory candidate creations/updates.

Ingestion rules:
- The raw document text may contain OCR spacing errors, missing spaces, or concatenated words (e.g. 'THECITYOFNEWYORK'). You MUST split these back into proper words and clean up spacing and spelling errors when generating candidate titles, summaries, and details.

Output rules:
- Respond with ONLY valid JSON: {{ \"candidates\": [ ... ] }}
- Do not output any markdown formatting fences, prefix, or suffix text.
- Each candidate MUST include:
  - \"action\": \"add\"
  - \"title\": short descriptive title
  - \"summary\": 1-2 sentence description
  - \"confidence\": float 0.0 to 1.0 (filter out < 0.3)
- Optional fields:
  - \"detail\": extended markdown context/notes
  - \"node_type\": lowercase from [concept, fact, project, preference, event, instruction, identity, summary]
  - \"target_vault_key\": lowercase from [{}]
  - \"tags\": array of short lowercase tags",
        vaults_str
    )
}

/// Wraps raw user document text in <document> tags to defend against text concatenating attacks.
pub fn wrap_ingestion_payload(document_text: &str) -> String {
    format!("<document>\n{}\n</document>", document_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_ingestion_payload() {
        let wrapped = wrap_ingestion_payload("Engineering principles");
        assert_eq!(wrapped, "<document>\nEngineering principles\n</document>");
    }

    #[test]
    fn test_get_system_prompt_default() {
        let prompt = get_system_prompt(&[]);
        assert!(prompt.contains("target_vault_key"));
        assert!(prompt.contains("credentials"));
        assert!(prompt.contains("Amber"));
        assert!(!prompt.contains("MindVault"));
    }

    #[test]
    fn test_get_system_prompt_custom() {
        let prompt = get_system_prompt(&["work".to_string(), "recipes".to_string()]);
        assert!(prompt.contains("work, recipes"));
    }
}
