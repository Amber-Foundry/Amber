use crate::embed::{
    load_registry, resolve_embedding_tier, stub_hardware_profile, tier_config, Tier, TierConfig,
};
use crate::ipc_types::EmbeddingReembedInput;
use rusqlite::{params, Connection, OptionalExtension};

pub const EMBEDDING_MODEL_KEY: &str = "embedding.model";
pub const EMBEDDING_TIER_KEY: &str = "embedding.tier";
pub const EMBEDDING_BACKEND_KEY: &str = "embedding.backend";
pub const EMBEDDING_LAST_COMPUTED_AT_KEY: &str = "embedding.last_computed_at";
pub const LOCAL_MODEL_ENDPOINT_KEY: &str = "local_model_endpoint";
pub const DEFAULT_LOCAL_MODEL_ENDPOINT: &str = "http://localhost:11434";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSettings {
    pub model: String,
    pub tier: String,
    pub backend: String,
    pub last_computed_at: Option<String>,
}

pub fn default_embedding_settings() -> Result<EmbeddingSettings, String> {
    let registry = load_registry()?;
    let tier = resolve_embedding_tier(&stub_hardware_profile(), 0);
    let (tier_name, tier_config) = match tier {
        Tier::Light => ("light", registry.tiers.light),
        Tier::Standard => ("standard", registry.tiers.standard),
        Tier::Quality => ("quality", registry.tiers.quality),
    };

    Ok(EmbeddingSettings {
        model: tier_config.model_id,
        tier: tier_name.to_string(),
        backend: "onnx".to_string(),
        last_computed_at: None,
    })
}

pub fn get_embedding_settings(conn: &Connection) -> Result<EmbeddingSettings, String> {
    let defaults = default_embedding_settings()?;

    Ok(EmbeddingSettings {
        model: read_string_setting(conn, EMBEDDING_MODEL_KEY)?.unwrap_or(defaults.model),
        tier: read_string_setting(conn, EMBEDDING_TIER_KEY)?.unwrap_or(defaults.tier),
        backend: read_string_setting(conn, EMBEDDING_BACKEND_KEY)?.unwrap_or(defaults.backend),
        last_computed_at: read_string_setting(conn, EMBEDDING_LAST_COMPUTED_AT_KEY)?
            .or(defaults.last_computed_at),
    })
}

pub fn get_local_model_endpoint(conn: &Connection) -> Result<String, String> {
    Ok(read_string_setting(conn, LOCAL_MODEL_ENDPOINT_KEY)?
        .unwrap_or_else(|| DEFAULT_LOCAL_MODEL_ENDPOINT.to_string()))
}

pub fn set_embedding_model(conn: &Connection, model: &str) -> Result<(), String> {
    write_string_setting(conn, EMBEDDING_MODEL_KEY, model)
}

pub fn set_embedding_tier(conn: &Connection, tier: &str) -> Result<(), String> {
    write_string_setting(conn, EMBEDDING_TIER_KEY, tier)
}

pub fn set_embedding_backend(conn: &Connection, backend: &str) -> Result<(), String> {
    write_string_setting(conn, EMBEDDING_BACKEND_KEY, backend)
}

pub fn set_embedding_last_computed_at(conn: &Connection, timestamp: &str) -> Result<(), String> {
    write_string_setting(conn, EMBEDDING_LAST_COMPUTED_AT_KEY, timestamp)
}

pub fn seed_embedding_defaults(conn: &Connection) -> Result<(), String> {
    let defaults = default_embedding_settings()?;
    insert_string_setting_if_missing(conn, EMBEDDING_MODEL_KEY, &defaults.model)?;
    insert_string_setting_if_missing(conn, EMBEDDING_TIER_KEY, &defaults.tier)?;
    insert_string_setting_if_missing(conn, EMBEDDING_BACKEND_KEY, &defaults.backend)?;
    Ok(())
}

pub fn chunking_config_for_settings(settings: &EmbeddingSettings) -> Result<TierConfig, String> {
    match settings.backend.to_ascii_lowercase().as_str() {
        "onnx" => {
            let config = tier_config(&settings.tier)
                .ok_or_else(|| format!("unknown embedding tier: {}", settings.tier))?;
            if settings.model != config.model_id {
                return tier_config_for_model(&settings.model).ok_or_else(|| {
                    format!(
                        "embedding.model '{}' does not match {} tier model '{}' and is not a registered ONNX model",
                        settings.model, settings.tier, config.model_id
                    )
                });
            }
            Ok(config)
        }
        "ollama" => {
            let registry = load_registry()?;
            let ollama = registry.ollama_default;
            if settings.model != ollama.model_id {
                return Err(format!(
                    "embedding.model '{}' does not match Ollama default model '{}'",
                    settings.model, ollama.model_id
                ));
            }
            Ok(TierConfig {
                model_id: settings.model.clone(),
                params_m: 0,
                dims: ollama.dims,
                max_tokens: ollama.max_tokens,
                onnx_size_mb: 0,
                chunk_target_tokens: ollama.chunk_target_tokens,
                chunk_overlap_tokens: vec![ollama.chunk_overlap_tokens],
                rules: serde_json::json!({}),
                fallback_model_id: None,
            })
        }
        other => Err(format!("unsupported embedding backend: {other}")),
    }
}

pub fn validate_reembed_input(input: &EmbeddingReembedInput) -> Result<(), String> {
    let model = input.model.trim();
    let tier = input.tier.trim().to_ascii_lowercase();
    let backend = input.backend.trim().to_ascii_lowercase();

    if model.is_empty() {
        return Err("embedding model must not be empty".to_string());
    }
    if tier.is_empty() {
        return Err("embedding tier must not be empty".to_string());
    }
    if backend.is_empty() {
        return Err("embedding backend must not be empty".to_string());
    }

    let registry = load_registry()?;
    match backend.as_str() {
        "onnx" => {
            let config = match tier.as_str() {
                "light" => &registry.tiers.light,
                "standard" => &registry.tiers.standard,
                "quality" => &registry.tiers.quality,
                _ => return Err(format!("unsupported embedding tier for ONNX: {tier}")),
            };
            if model != config.model_id {
                return Err(format!(
                    "embedding model '{}' is not valid for ONNX tier '{}'; expected '{}'",
                    model, tier, config.model_id
                ));
            }
        }
        "ollama" => {
            if model != registry.ollama_default.model_id {
                return Err(format!(
                    "embedding model '{}' is not valid for Ollama; expected '{}'",
                    model, registry.ollama_default.model_id
                ));
            }
        }
        _ => return Err(format!("unsupported embedding backend: {backend}")),
    }

    Ok(())
}

fn tier_config_for_model(model: &str) -> Option<TierConfig> {
    let registry = load_registry().ok()?;
    let tiers = [
        registry.tiers.light,
        registry.tiers.standard,
        registry.tiers.quality,
    ];
    tiers.into_iter().find(|config| config.model_id == model)
}

fn read_string_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    let raw = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| format!("Failed reading setting {key}: {err}"))?;

    Ok(raw.map(|value| parse_setting_string(&value)))
}

fn parse_setting_string(value: &str) -> String {
    match serde_json::from_str::<String>(value) {
        Ok(parsed) => parsed,
        Err(_) => value.to_string(),
    }
}

fn write_string_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    let encoded = serde_json::to_string(value)
        .map_err(|err| format!("Failed serializing setting {key}: {err}"))?;
    conn.execute(
        "INSERT INTO settings (key, value, scope, updated_at)
         VALUES (?1, ?2, 'global', datetime('now'))
         ON CONFLICT(key) DO UPDATE
         SET value = excluded.value,
             scope = excluded.scope,
             updated_at = datetime('now');",
        params![key, encoded],
    )
    .map_err(|err| format!("Failed writing setting {key}: {err}"))?;
    Ok(())
}

fn insert_string_setting_if_missing(
    conn: &Connection,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let encoded = serde_json::to_string(value)
        .map_err(|err| format!("Failed serializing default setting {key}: {err}"))?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, scope)
         VALUES (?1, ?2, 'global');",
        params![key, encoded],
    )
    .map_err(|err| format!("Failed inserting default setting {key}: {err}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_settings_db() -> Result<Connection, Box<dyn std::error::Error>> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;
        Ok(conn)
    }

    #[test]
    fn test_default_embedding_settings() -> Result<(), Box<dyn std::error::Error>> {
        let settings = default_embedding_settings()?;
        assert_eq!(settings.model, "avsolatorio/GIST-small-Embedding-v0");
        assert_eq!(settings.tier, "light");
        assert_eq!(settings.backend, "onnx");
        assert_eq!(settings.last_computed_at, None);
        Ok(())
    }

    #[test]
    fn test_embedding_settings_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_settings_db()?;
        set_embedding_model(&conn, "avsolatorio/GIST-Embedding-v0")?;
        set_embedding_tier(&conn, "standard")?;
        set_embedding_backend(&conn, "onnx")?;
        set_embedding_last_computed_at(&conn, "2026-06-21T17:00:00.000Z")?;

        let settings = get_embedding_settings(&conn)?;
        assert_eq!(settings.model, "avsolatorio/GIST-Embedding-v0");
        assert_eq!(settings.tier, "standard");
        assert_eq!(settings.backend, "onnx");
        assert_eq!(
            settings.last_computed_at,
            Some("2026-06-21T17:00:00.000Z".to_string())
        );

        let scope: String = conn.query_row(
            "SELECT scope FROM settings WHERE key = ?1;",
            [EMBEDDING_MODEL_KEY],
            |row| row.get(0),
        )?;
        assert_eq!(scope, "global");
        Ok(())
    }

    #[test]
    fn test_missing_embedding_settings_fall_back_to_defaults(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_settings_db()?;
        let settings = get_embedding_settings(&conn)?;
        assert_eq!(settings.model, "avsolatorio/GIST-small-Embedding-v0");
        assert_eq!(settings.tier, "light");
        assert_eq!(settings.backend, "onnx");
        Ok(())
    }

    #[test]
    fn test_json_and_raw_string_reads() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup_settings_db()?;
        conn.execute(
            "INSERT INTO settings (key, value, scope) VALUES (?1, ?2, 'global');",
            params![LOCAL_MODEL_ENDPOINT_KEY, "\"http://localhost:11434\""],
        )?;
        assert_eq!(
            get_local_model_endpoint(&conn)?,
            "http://localhost:11434".to_string()
        );

        conn.execute(
            "UPDATE settings SET value = ?2 WHERE key = ?1;",
            params![LOCAL_MODEL_ENDPOINT_KEY, "http://127.0.0.1:11434"],
        )?;
        assert_eq!(
            get_local_model_endpoint(&conn)?,
            "http://127.0.0.1:11434".to_string()
        );
        Ok(())
    }

    #[test]
    fn test_chunking_config_for_onnx_and_ollama() -> Result<(), Box<dyn std::error::Error>> {
        let onnx = EmbeddingSettings {
            model: "avsolatorio/GIST-small-Embedding-v0".to_string(),
            tier: "light".to_string(),
            backend: "onnx".to_string(),
            last_computed_at: None,
        };
        let onnx_config = chunking_config_for_settings(&onnx)?;
        assert_eq!(onnx_config.dims, 384);

        let ollama = EmbeddingSettings {
            model: "nomic-embed-text".to_string(),
            tier: "light".to_string(),
            backend: "ollama".to_string(),
            last_computed_at: None,
        };
        let ollama_config = chunking_config_for_settings(&ollama)?;
        assert_eq!(ollama_config.dims, 768);
        assert_eq!(ollama_config.chunk_overlap_tokens, vec![64]);
        Ok(())
    }
}
