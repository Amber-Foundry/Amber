use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub tiers: Tiers,
    #[serde(rename = "ollama_default", alias = "ollamaDefault")]
    pub ollama_default: OllamaDefaultConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tiers {
    pub light: TierConfig,
    pub standard: TierConfig,
    pub quality: TierConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierConfig {
    #[serde(rename = "model_id")]
    pub model_id: String,
    #[serde(rename = "params_m")]
    pub params_m: u32,
    pub dims: usize,
    pub max_tokens: usize,
    #[serde(rename = "onnx_size_mb")]
    pub onnx_size_mb: u32,
    pub chunk_target_tokens: Vec<usize>,
    pub chunk_overlap_tokens: Vec<usize>,
    pub rules: serde_json::Value,
    #[serde(rename = "fallback_model_id")]
    pub fallback_model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaDefaultConfig {
    #[serde(rename = "model_id")]
    pub model_id: String,
    pub dims: usize,
    pub max_tokens: usize,
    pub chunk_target_tokens: Vec<usize>,
    pub chunk_overlap_tokens: usize,
}

pub fn load_registry() -> Result<Registry, String> {
    let json_str = include_str!("../../../embedding_registry.json");
    serde_json::from_str(json_str)
        .map_err(|err| format!("Failed to parse embedding_registry.json: {}", err))
}

pub fn tier_config(tier: &str) -> Option<TierConfig> {
    let registry = load_registry().ok()?;
    match tier.to_lowercase().as_str() {
        "light" => Some(registry.tiers.light),
        "standard" => Some(registry.tiers.standard),
        "quality" => Some(registry.tiers.quality),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_registry() -> Result<(), Box<dyn std::error::Error>> {
        let registry = load_registry().map_err(Box::<dyn std::error::Error>::from)?;

        // Check light tier fields
        assert_eq!(
            registry.tiers.light.model_id,
            "avsolatorio/GIST-small-Embedding-v0"
        );
        assert_eq!(registry.tiers.light.dims, 384);
        assert_eq!(registry.tiers.light.max_tokens, 512);
        assert_eq!(registry.tiers.light.onnx_size_mb, 65);
        assert_eq!(registry.tiers.light.chunk_target_tokens, vec![250, 350]);
        assert_eq!(registry.tiers.light.chunk_overlap_tokens, vec![50, 64]);

        // Check standard tier fields
        assert_eq!(
            registry.tiers.standard.model_id,
            "avsolatorio/GIST-Embedding-v0"
        );
        assert_eq!(registry.tiers.standard.dims, 768);
        assert_eq!(registry.tiers.standard.max_tokens, 512);
        assert_eq!(registry.tiers.standard.onnx_size_mb, 220);
        assert_eq!(registry.tiers.standard.chunk_target_tokens, vec![250, 350]);
        assert_eq!(registry.tiers.standard.chunk_overlap_tokens, vec![50, 64]);

        // Check quality tier fields
        assert_eq!(
            registry.tiers.quality.model_id,
            "microsoft/harrier-oss-v1-270m"
        );
        assert_eq!(registry.tiers.quality.dims, 640);
        assert_eq!(registry.tiers.quality.max_tokens, 32768);
        assert_eq!(registry.tiers.quality.onnx_size_mb, 500);
        assert_eq!(registry.tiers.quality.chunk_target_tokens, vec![500, 800]);
        assert_eq!(registry.tiers.quality.chunk_overlap_tokens, vec![64, 80]);
        assert_eq!(
            registry.tiers.quality.fallback_model_id,
            Some("avsolatorio/GIST-Embedding-v0".to_string())
        );

        // Check ollama default config
        assert_eq!(registry.ollama_default.model_id, "nomic-embed-text");
        assert_eq!(registry.ollama_default.dims, 768);
        assert_eq!(registry.ollama_default.max_tokens, 8192);
        assert_eq!(registry.ollama_default.chunk_target_tokens, vec![300, 400]);
        assert_eq!(registry.ollama_default.chunk_overlap_tokens, 64);

        Ok(())
    }

    #[test]
    fn test_tier_config() -> Result<(), Box<dyn std::error::Error>> {
        let light = tier_config("light").ok_or("Light tier config missing")?;
        assert_eq!(light.dims, 384);

        let standard = tier_config("standard").ok_or("Standard tier config missing")?;
        assert_eq!(standard.dims, 768);

        let quality = tier_config("quality").ok_or("Quality tier config missing")?;
        assert_eq!(quality.dims, 640);

        let invalid = tier_config("invalid");
        assert!(invalid.is_none());

        Ok(())
    }
}
