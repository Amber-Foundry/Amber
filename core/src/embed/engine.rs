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
                if !vector.is_empty() {
                    vector[0] = 1.0;
                }
                return Ok(vector);
            }

            for value in &mut vector {
                *value /= norm;
            }
            Ok(vector)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_all_zero_norm_fallback() {
        let input = vec![vec![0.0, 0.0, 0.0]];
        let result = normalize_all(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_normalize_all_standard() {
        let input = vec![vec![3.0, 4.0, 0.0]];
        let result = normalize_all(input).unwrap();
        assert_eq!(result.len(), 1);
        assert!((result[0][0] - 0.6).abs() < 1e-5);
        assert!((result[0][1] - 0.8).abs() < 1e-5);
        assert!((result[0][2] - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_normalize_all_empty_vector() {
        let input = vec![vec![]];
        let result = normalize_all(input).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].is_empty());
    }
}
