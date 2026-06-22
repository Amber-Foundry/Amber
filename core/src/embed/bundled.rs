use crate::embed::{normalize_all, EmbedEngine, EmbedError, TierConfig};
use ort::execution_providers::CPUExecutionProvider;
#[cfg(target_os = "windows")]
use ort::execution_providers::DirectMLExecutionProvider;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::env;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

pub const DEFAULT_BUNDLED_MODEL_ID: &str = "avsolatorio/GIST-small-Embedding-v0";

#[derive(Debug, Clone)]
pub struct ModelArtifactPaths {
    pub onnx: PathBuf,
    pub tokenizer: PathBuf,
}

#[derive(Debug)]
struct TokenizedBatch {
    input_ids: Vec<i64>,
    attention_mask: Vec<i64>,
    token_type_ids: Vec<i64>,
    batch_size: usize,
    seq_len: usize,
}

pub struct BundledEmbedEngine {
    model_id: String,
    dims: usize,
    tokenizer: Tokenizer,
    session: Session,
    input_names: Vec<String>,
    output_names: Vec<String>,
}

impl BundledEmbedEngine {
    pub fn new(model_id: impl Into<String>, dims: usize) -> Result<Self, EmbedError> {
        let model_id = model_id.into();
        let paths = model_artifact_paths(&model_id)?;
        Self::from_paths(model_id, dims, paths)
    }

    pub fn from_tier(config: &TierConfig) -> Result<Self, EmbedError> {
        Self::new(config.model_id.clone(), config.dims)
    }

    pub fn from_paths(
        model_id: impl Into<String>,
        dims: usize,
        paths: ModelArtifactPaths,
    ) -> Result<Self, EmbedError> {
        let model_id = model_id.into();
        ensure_file_exists(&paths.onnx, "ONNX model")?;
        ensure_file_exists(&paths.tokenizer, "tokenizer")?;

        let tokenizer = Tokenizer::from_file(&paths.tokenizer).map_err(|err| {
            EmbedError::InferenceFailed(format!(
                "failed to load tokenizer {}: {}",
                paths.tokenizer.display(),
                err
            ))
        })?;

        let session = build_session(&paths.onnx)?;
        let input_names = session
            .inputs
            .iter()
            .map(|input| input.name.clone())
            .collect();
        let output_names = session
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .collect();

        Ok(Self {
            model_id,
            dims,
            tokenizer,
            session,
            input_names,
            output_names,
        })
    }

    fn tokenize(&self, texts: &[String]) -> Result<TokenizedBatch, EmbedError> {
        let input_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let encodings = self
            .tokenizer
            .encode_batch(input_refs, true)
            .map_err(|err| EmbedError::InferenceFailed(format!("tokenization failed: {}", err)))?;

        let batch_size = encodings.len();
        let seq_len = encodings
            .iter()
            .map(|encoding| encoding.get_ids().len())
            .max()
            .unwrap_or(0);
        let pad_id = pad_token_id(&self.tokenizer);

        let mut input_ids = Vec::with_capacity(batch_size * seq_len);
        let mut attention_mask = Vec::with_capacity(batch_size * seq_len);
        let mut token_type_ids = Vec::with_capacity(batch_size * seq_len);

        for encoding in encodings {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            let type_ids = encoding.get_type_ids();

            for index in 0..seq_len {
                input_ids.push(ids.get(index).copied().map(i64::from).unwrap_or(pad_id));
                attention_mask.push(mask.get(index).copied().map(i64::from).unwrap_or(0));
                token_type_ids.push(type_ids.get(index).copied().map(i64::from).unwrap_or(0));
            }
        }

        Ok(TokenizedBatch {
            input_ids,
            attention_mask,
            token_type_ids,
            batch_size,
            seq_len,
        })
    }

    fn run_batch(&self, batch: TokenizedBatch) -> Result<Vec<Vec<f32>>, EmbedError> {
        let shape = vec![batch.batch_size as i64, batch.seq_len as i64];
        let wants_token_type_ids = self.input_names.iter().any(|name| name == "token_type_ids");

        let mut inputs = ort::inputs! {
            "input_ids" => Tensor::from_array((shape.clone(), batch.input_ids))?,
            "attention_mask" => Tensor::from_array((shape.clone(), batch.attention_mask.clone()))?
        }
        .map_err(|err| {
            EmbedError::InferenceFailed(format!("failed to create ONNX inputs: {}", err))
        })?;

        if wants_token_type_ids {
            inputs.push((
                "token_type_ids".into(),
                Tensor::from_array((shape, batch.token_type_ids))
                    .map_err(|err| {
                        EmbedError::InferenceFailed(format!(
                            "failed to create token_type_ids tensor: {}",
                            err
                        ))
                    })?
                    .into(),
            ));
        }

        let outputs = self.session.run(inputs).map_err(|err| {
            EmbedError::InferenceFailed(format!("ONNX inference failed: {}", err))
        })?;

        if let Some(output_name) = self.direct_embedding_output_name() {
            let output = outputs.get(&output_name).ok_or_else(|| {
                EmbedError::InferenceFailed(format!(
                    "expected embedding output '{}' was not returned",
                    output_name
                ))
            })?;
            let tensor = output.try_extract_tensor::<f32>().map_err(|err| {
                EmbedError::InferenceFailed(format!(
                    "failed to read embedding output '{}': {}",
                    output_name, err
                ))
            })?;
            let shape = tensor.shape().to_vec();
            let data = tensor.as_slice_memory_order().ok_or_else(|| {
                EmbedError::InferenceFailed(format!(
                    "embedding output '{}' was not contiguous",
                    output_name
                ))
            })?;
            let vectors =
                normalize_all(extract_direct_embeddings(&shape, data, batch.batch_size)?)?;
            return self.validate_dims(vectors);
        }

        let output_name = self
            .last_hidden_state_output_name()
            .or_else(|| self.output_names.first().cloned())
            .ok_or_else(|| {
                EmbedError::InferenceFailed("ONNX session has no outputs".to_string())
            })?;
        let output = outputs.get(&output_name).ok_or_else(|| {
            EmbedError::InferenceFailed(format!(
                "expected hidden-state output '{}' was not returned",
                output_name
            ))
        })?;
        let tensor = output.try_extract_tensor::<f32>().map_err(|err| {
            EmbedError::InferenceFailed(format!(
                "failed to read hidden-state output '{}': {}",
                output_name, err
            ))
        })?;
        let shape = tensor.shape().to_vec();
        let data = tensor.as_slice_memory_order().ok_or_else(|| {
            EmbedError::InferenceFailed(format!(
                "hidden-state output '{}' was not contiguous",
                output_name
            ))
        })?;

        let vectors = normalize_all(mean_pool(
            &shape,
            data,
            &batch.attention_mask,
            batch.batch_size,
        )?)?;
        self.validate_dims(vectors)
    }

    fn direct_embedding_output_name(&self) -> Option<String> {
        self.output_names
            .iter()
            .find(|name| {
                name.eq_ignore_ascii_case("sentence_embedding")
                    || name.eq_ignore_ascii_case("embeddings")
            })
            .cloned()
    }

    fn last_hidden_state_output_name(&self) -> Option<String> {
        self.output_names
            .iter()
            .find(|name| name.eq_ignore_ascii_case("last_hidden_state"))
            .cloned()
    }

    fn validate_dims(&self, vectors: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>, EmbedError> {
        for vector in &vectors {
            if vector.len() != self.dims {
                return Err(EmbedError::InferenceFailed(format!(
                    "embedding dimension mismatch for {}: expected {}, got {}",
                    self.model_id,
                    self.dims,
                    vector.len()
                )));
            }
        }

        Ok(vectors)
    }
}

impl EmbedEngine for BundledEmbedEngine {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let batch = self.tokenize(texts)?;
        self.run_batch(batch)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        self.dims
    }
}

pub fn sanitize_model_id(model_id: &str) -> String {
    model_id.replace('/', "_")
}

pub fn models_dir() -> Result<PathBuf, EmbedError> {
    let home = amber_home_dir()?;
    Ok(home.join("models").join("embed"))
}

pub fn model_artifact_paths(model_id: &str) -> Result<ModelArtifactPaths, EmbedError> {
    let base_name = sanitize_model_id(model_id);
    let dir = models_dir()?;
    Ok(ModelArtifactPaths {
        onnx: dir.join(format!("{}.onnx", base_name)),
        tokenizer: dir.join(format!("{}_tokenizer.json", base_name)),
    })
}

fn amber_home_dir() -> Result<PathBuf, EmbedError> {
    let home = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| {
            EmbedError::ModelNotFound(
                "could not resolve home directory for ~/.amber/models/embed".to_string(),
            )
        })?;
    Ok(home.join(".amber"))
}

fn pad_token_id(tokenizer: &Tokenizer) -> i64 {
    if let Some(padding) = tokenizer.get_padding() {
        return padding.pad_id as i64;
    }

    tokenizer.token_to_id("").unwrap_or(0) as i64
}

fn ensure_file_exists(path: &Path, label: &str) -> Result<(), EmbedError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(EmbedError::ModelNotFound(format!(
            "{} artifact missing at {}",
            label,
            path.display()
        )))
    }
}

fn build_session(model_path: &Path) -> Result<Session, EmbedError> {
    #[cfg(target_os = "windows")]
    {
        let builder = Session::builder()
            .and_then(|builder| builder.with_optimization_level(GraphOptimizationLevel::Level3))
            .map_err(|err| {
                EmbedError::InferenceFailed(format!(
                    "failed to create ONNX session builder: {}",
                    err
                ))
            })?;
        match builder
            .with_execution_providers([DirectMLExecutionProvider::default().build()])
            .and_then(|builder| builder.commit_from_file(model_path))
        {
            Ok(session) => return Ok(session),
            Err(err) => {
                eprintln!(
                    "Warning: DirectML embedding session failed, falling back to CPU: {}",
                    err
                );
            }
        }
    }

    let builder = Session::builder()
        .and_then(|builder| builder.with_optimization_level(GraphOptimizationLevel::Level3))
        .map_err(|err| {
            EmbedError::InferenceFailed(format!("failed to create ONNX session builder: {}", err))
        })?;
    builder
        .with_execution_providers([CPUExecutionProvider::default().build()])
        .and_then(|builder| builder.commit_from_file(model_path))
        .map_err(|err| {
            EmbedError::InferenceFailed(format!(
                "failed to load ONNX model {}: {}",
                model_path.display(),
                err
            ))
        })
}

fn extract_direct_embeddings(
    shape: &[usize],
    data: &[f32],
    batch_size: usize,
) -> Result<Vec<Vec<f32>>, EmbedError> {
    match shape {
        [batch, dims] if *batch == batch_size => Ok(data
            .chunks_exact(*dims)
            .map(|chunk| chunk.to_vec())
            .collect()),
        [batch, one, dims] if *batch == batch_size && *one == 1 => {
            let mut vectors = Vec::with_capacity(batch_size);
            for batch_index in 0..batch_size {
                let start = batch_index * dims;
                let end = start + dims;
                vectors.push(data[start..end].to_vec());
            }
            Ok(vectors)
        }
        [dims] if batch_size == 1 => Ok(vec![data[..*dims].to_vec()]),
        _ => Err(EmbedError::InferenceFailed(format!(
            "unsupported embedding output shape {:?}",
            shape
        ))),
    }
}

fn mean_pool(
    shape: &[usize],
    data: &[f32],
    attention_mask: &[i64],
    batch_size: usize,
) -> Result<Vec<Vec<f32>>, EmbedError> {
    let [batch, seq_len, dims] = shape else {
        return Err(EmbedError::InferenceFailed(format!(
            "expected last_hidden_state shape [batch, seq, dim], got {:?}",
            shape
        )));
    };

    if *batch != batch_size {
        return Err(EmbedError::InferenceFailed(format!(
            "hidden-state batch size mismatch: expected {}, got {}",
            batch_size, batch
        )));
    }

    let expected_mask_len = batch_size * seq_len;
    if attention_mask.len() != expected_mask_len {
        return Err(EmbedError::InferenceFailed(format!(
            "attention mask length mismatch: expected {}, got {}",
            expected_mask_len,
            attention_mask.len()
        )));
    }

    let mut vectors = Vec::with_capacity(batch_size);
    for batch_index in 0..batch_size {
        let mut pooled = vec![0.0f32; *dims];
        let mut token_count = 0.0f32;

        for token_index in 0..*seq_len {
            let mask_index = batch_index * seq_len + token_index;
            if attention_mask[mask_index] == 0 {
                continue;
            }

            token_count += 1.0;
            let token_offset = (batch_index * seq_len + token_index) * dims;
            for dim_index in 0..*dims {
                pooled[dim_index] += data[token_offset + dim_index];
            }
        }

        if token_count > 0.0 {
            for value in &mut pooled {
                *value /= token_count;
            }
        }
        vectors.push(pooled);
    }

    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_model_id() {
        assert_eq!(
            sanitize_model_id("avsolatorio/GIST-small-Embedding-v0"),
            "avsolatorio_GIST-small-Embedding-v0"
        );
    }

    #[test]
    fn test_bundled_embed_engine_setup() -> Result<(), Box<dyn std::error::Error>> {
        let paths = model_artifact_paths(DEFAULT_BUNDLED_MODEL_ID)?;
        if !paths.onnx.is_file() || !paths.tokenizer.is_file() {
            eprintln!(
                "Model not found; skipping test. Expected {} and {}",
                paths.onnx.display(),
                paths.tokenizer.display()
            );
            return Ok(());
        }

        let engine = BundledEmbedEngine::new(DEFAULT_BUNDLED_MODEL_ID, 384)?;
        let embeddings = engine.embed(&["Amber local embedding smoke test".to_string()])?;

        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].len(), 384);
        Ok(())
    }

    #[test]
    #[ignore = "requires local GIST-small ONNX and tokenizer artifacts in ~/.amber/models/embed"]
    fn test_bundled_embed_engine_gist_small() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BundledEmbedEngine::new(DEFAULT_BUNDLED_MODEL_ID, 384)?;
        let embeddings = engine.embed(&[
            "Amber stores local memories.".to_string(),
            "Embeddings support semantic retrieval.".to_string(),
        ])?;

        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 384);
        assert_eq!(embeddings[1].len(), 384);
        Ok(())
    }
}
