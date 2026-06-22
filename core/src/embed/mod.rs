pub mod bundled;
pub mod chunking;
pub mod config;
pub mod engine;
pub mod job;
pub mod ollama;
pub mod registry;
pub mod search;
pub mod storage;
pub mod tier_resolver;

pub use search::{cosine_similarity, find_top_n_similar};

pub use bundled::{model_artifact_paths, sanitize_model_id, BundledEmbedEngine};
pub use chunking::{chunk_node_text, ChunkSpec};
pub use config::{
    chunking_config_for_settings, default_embedding_settings, get_embedding_settings,
    get_local_model_endpoint, seed_embedding_defaults, set_embedding_backend,
    set_embedding_last_computed_at, set_embedding_model, set_embedding_tier,
    validate_reembed_input, EmbeddingSettings, DEFAULT_LOCAL_MODEL_ENDPOINT, EMBEDDING_BACKEND_KEY,
    EMBEDDING_LAST_COMPUTED_AT_KEY, EMBEDDING_MODEL_KEY, EMBEDDING_TIER_KEY,
    LOCAL_MODEL_ENDPOINT_KEY,
};
pub use engine::{normalize_all, EmbedEngine, EmbedError};
pub use job::{
    embed_all_nodes, embed_node, stored_text_columns_changed, EmbedJobHandle, EmbedJobResult,
};
pub use ollama::OllamaEmbedEngine;
pub use registry::{load_registry, tier_config, OllamaDefaultConfig, Registry, TierConfig};
pub use storage::EmbeddingRow;
pub use tier_resolver::{resolve_embedding_tier, stub_hardware_profile, HardwareProfile, Tier};
