use std::fmt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EmbedError {
    ModelNotFound(String),
    InferenceFailed(String),
    Cancelled,
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedError::ModelNotFound(msg) => write!(f, "Model not found: {}", msg),
            EmbedError::InferenceFailed(msg) => write!(f, "Inference failed: {}", msg),
            EmbedError::Cancelled => write!(f, "Cancelled"),
        }
    }
}

impl std::error::Error for EmbedError {}

pub trait EmbedEngine: Send + Sync {
    /// Embed one or more text strings. Returns one vector per input, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn model_id(&self) -> &str;
    fn dims(&self) -> usize;
}

pub fn normalize_all(vectors: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>, EmbedError> {
    vectors
        .into_iter()
        .map(|mut vector| {
            let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
            if norm == 0.0 {
                return Err(EmbedError::InferenceFailed(
                    "embedding output had zero norm".to_string(),
                ));
            }

            for value in &mut vector {
                *value /= norm;
            }
            Ok(vector)
        })
        .collect()
}
