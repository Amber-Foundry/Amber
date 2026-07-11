use serde::de::DeserializeOwned;

/// Parses a model registry JSON string into the target configuration type.
pub fn parse_registry_json<T: DeserializeOwned>(json_str: &str) -> Result<T, String> {
    serde_json::from_str(json_str).map_err(|err| format!("Failed to parse registry JSON: {}", err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct DummyRegistry {
        name: String,
        version: u32,
    }

    #[test]
    fn test_parse_registry_json_success() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{"name": "test", "version": 1}"#;
        let registry: DummyRegistry =
            parse_registry_json(json).map_err(Box::<dyn std::error::Error>::from)?;
        assert_eq!(
            registry,
            DummyRegistry {
                name: "test".to_string(),
                version: 1
            }
        );
        Ok(())
    }

    #[test]
    fn test_parse_registry_json_failure() {
        let json = r#"{"invalid": true}"#;
        let res: Result<DummyRegistry, String> = parse_registry_json(json);
        assert!(res.is_err());
    }
}
