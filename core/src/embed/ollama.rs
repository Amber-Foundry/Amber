use crate::embed::{normalize_all, EmbedEngine, EmbedError, OllamaDefaultConfig};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub struct OllamaEmbedEngine {
    endpoint: String,
    model_id: String,
    dims: usize,
    client: reqwest::blocking::Client,
}

impl OllamaEmbedEngine {
    pub fn new(endpoint: impl Into<String>, model_id: impl Into<String>, dims: usize) -> Self {
        Self {
            endpoint: endpoint.into(),
            model_id: model_id.into(),
            dims,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn from_registry_defaults(
        endpoint: impl Into<String>,
        config: &OllamaDefaultConfig,
    ) -> Self {
        Self::new(endpoint, config.model_id.clone(), config.dims)
    }
}

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl EmbedEngine for OllamaEmbedEngine {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let payload = OllamaEmbedRequest {
            model: &self.model_id,
            input: texts,
        };

        let url = format!("{}/api/embed", self.endpoint.trim_end_matches('/'));

        let response = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(60))
            .json(&payload)
            .send()
            .map_err(|err| {
                EmbedError::InferenceFailed(format!("failed to connect to Ollama: {}", err))
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().unwrap_or_default();
            return Err(EmbedError::InferenceFailed(format!(
                "Ollama returned error status ({}): {}",
                status, error_body
            )));
        }

        let parsed: OllamaEmbedResponse = response.json().map_err(|err| {
            EmbedError::InferenceFailed(format!("failed to parse Ollama response: {}", err))
        })?;

        let embeddings = parsed.embeddings;

        if embeddings.len() != texts.len() {
            return Err(EmbedError::InferenceFailed(format!(
                "Ollama response size mismatch: expected {} vectors, got {}",
                texts.len(),
                embeddings.len()
            )));
        }

        for vector in &embeddings {
            if vector.len() != self.dims {
                return Err(EmbedError::InferenceFailed(format!(
                    "embedding dimension mismatch: expected {}, got {}",
                    self.dims,
                    vector.len()
                )));
            }
        }

        normalize_all(embeddings)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        self.dims
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn spawn_mock_server(
        response_body: &'static str,
        expected_path: &'static str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();

        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                let request = String::from_utf8_lossy(&buffer);

                if request.contains(&format!("POST {}", expected_path)) {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: application/json\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\r\n\
                         {}",
                        response_body.len(),
                        response_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                } else {
                    let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            }
        });

        Ok(format!("http://127.0.0.1:{}", port))
    }

    fn spawn_mock_server_error(
        status_line: &'static str,
        response_body: &'static str,
        expected_path: &'static str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();

        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                let request = String::from_utf8_lossy(&buffer);

                if request.contains(&format!("POST {}", expected_path)) {
                    let response = format!(
                        "HTTP/1.1 {}\r\n\
                         Content-Type: text/plain\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\r\n\
                         {}",
                        status_line,
                        response_body.len(),
                        response_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            }
        });

        Ok(format!("http://127.0.0.1:{}", port))
    }

    #[test]
    fn test_ollama_embed_success() -> Result<(), Box<dyn std::error::Error>> {
        let response_body = r#"{"embeddings":[[3.0,4.0]]}"#;
        let mock_url = spawn_mock_server(response_body, "/api/embed")?;

        let engine = OllamaEmbedEngine::new(mock_url, "nomic-embed-text", 2);
        let result = engine.embed(&["hello".to_string()])?;

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 2);
        // Norm should be exactly 1.0 (vector is normalized [3/5, 4/5] -> [0.6, 0.8])
        assert!((result[0][0] - 0.6).abs() < 1e-5);
        assert!((result[0][1] - 0.8).abs() < 1e-5);
        Ok(())
    }

    #[test]
    fn test_ollama_embed_size_mismatch() -> Result<(), Box<dyn std::error::Error>> {
        // Return 2 embeddings when only 1 input is sent
        let response_body = r#"{"embeddings":[[1.0, 0.0], [0.0, 1.0]]}"#;
        let mock_url = spawn_mock_server(response_body, "/api/embed")?;

        let engine = OllamaEmbedEngine::new(mock_url, "nomic-embed-text", 2);
        let result = engine.embed(&["hello".to_string()]);

        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(err_msg.contains("Ollama response size mismatch"));
            }
            Ok(_) => return Err("expected size mismatch error, but got success".into()),
        }
        Ok(())
    }

    #[test]
    fn test_ollama_embed_dim_mismatch() -> Result<(), Box<dyn std::error::Error>> {
        // Return 3D embedding when 2D is expected
        let response_body = r#"{"embeddings":[[1.0, 0.0, 0.0]]}"#;
        let mock_url = spawn_mock_server(response_body, "/api/embed")?;

        let engine = OllamaEmbedEngine::new(mock_url, "nomic-embed-text", 2);
        let result = engine.embed(&["hello".to_string()]);

        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(err_msg.contains("embedding dimension mismatch"));
            }
            Ok(_) => return Err("expected dimension mismatch error, but got success".into()),
        }
        Ok(())
    }

    #[test]
    fn test_ollama_embed_error_body_captured() -> Result<(), Box<dyn std::error::Error>> {
        let mock_url = spawn_mock_server_error(
            "404 Not Found",
            "model 'some-invalid-model' not found",
            "/api/embed",
        )?;

        let engine = OllamaEmbedEngine::new(mock_url, "some-invalid-model", 2);
        let result = engine.embed(&["hello".to_string()]);

        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(err_msg.contains("Ollama returned error status (404 Not Found)"));
                assert!(err_msg.contains("model 'some-invalid-model' not found"));
            }
            Ok(_) => return Err("expected HTTP 404 error, but got success".into()),
        }
        Ok(())
    }

    #[test]
    fn test_ollama_unreachable() -> Result<(), Box<dyn std::error::Error>> {
        // Use an invalid port/address
        let engine = OllamaEmbedEngine::new("http://127.0.0.1:9999", "nomic-embed-text", 768);
        let result = engine.embed(&["hello".to_string()]);

        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(err_msg.contains("failed to connect to Ollama"));
            }
            Ok(_) => return Err("expected unreachable error, but got success".into()),
        }
        Ok(())
    }

    #[test]
    fn test_ollama_embed_empty() -> Result<(), Box<dyn std::error::Error>> {
        let engine = OllamaEmbedEngine::new("http://127.0.0.1:9999", "nomic-embed-text", 768);
        let result = engine.embed(&[])?;
        assert!(result.is_empty());
        Ok(())
    }

    #[test]
    #[ignore = "requires live local Ollama running 'nomic-embed-text' model at http://localhost:11434"]
    fn manual_ollama_live_test() -> Result<(), Box<dyn std::error::Error>> {
        let engine = OllamaEmbedEngine::new("http://localhost:11434", "nomic-embed-text", 768);
        let result = engine.embed(&[
            "Amber local RAG verification using Ollama".to_string(),
            "This test hits a live server".to_string(),
        ])?;

        println!("==================================================");
        println!("BATCH SIZE: {}", result.len());
        println!("DIMENSIONS: {}", result[0].len());
        let norm = result[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        println!("L2 NORM: {}", norm);
        println!("==================================================");

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), 768);
        assert!((norm - 1.0).abs() < 1e-4);
        Ok(())
    }
}
