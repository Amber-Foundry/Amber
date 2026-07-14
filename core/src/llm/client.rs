use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

#[async_trait]
pub trait LlmClient {
    async fn list_models(&self) -> Result<Vec<String>, crate::AppError>;
    async fn complete(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
    ) -> Result<String, crate::AppError>;
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    Ollama,
    LmStudio,
    Anthropic,
    OpenAi,
    Google,
    XAi,
}

pub struct UniversalClient {
    pub provider: LlmProvider,
    pub endpoint: String,
    pub model: String,
}

impl UniversalClient {
    pub fn new(provider: LlmProvider, endpoint: String, model: String) -> Self {
        Self {
            provider,
            endpoint,
            model,
        }
    }

    fn normalized_endpoint(&self) -> &str {
        self.endpoint.trim_end_matches('/')
    }
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatApiMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaChatApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaChatResponseMessage,
}

#[derive(Deserialize)]
struct OllamaChatResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct LmStudioModelsResponse {
    data: Vec<LmStudioModel>,
}

#[derive(Deserialize)]
struct LmStudioModel {
    id: String,
}

#[derive(Serialize)]
struct LmStudioChatRequest {
    model: String,
    messages: Vec<LmStudioPayloadMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct LmStudioPayloadMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct LmStudioChatResponse {
    choices: Vec<LmStudioChoice>,
}

#[derive(Deserialize)]
struct LmStudioChoice {
    message: LmStudioChoiceMessage,
}

#[derive(Deserialize)]
struct LmStudioChoiceMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
}

#[async_trait]
impl LlmClient for UniversalClient {
    async fn list_models(&self) -> Result<Vec<String>, crate::AppError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        match self.provider {
            LlmProvider::Ollama => {
                let url = format!("{}/api/tags", self.normalized_endpoint());
                let response = http
                    .get(url)
                    .send()
                    .await
                    .map_err(|err| format!("Failed calling Ollama tags endpoint: {err}"))?;

                let status = response.status();
                if !status.is_success() {
                    let body = response
                        .text()
                        .await
                        .map_err(|err| format!("Failed reading Ollama tags error body: {err}"))?;
                    return Err(format!("Ollama tags request failed ({status}): {body}"));
                }

                let parsed: OllamaTagsResponse = response
                    .json()
                    .await
                    .map_err(|err| format!("Failed parsing Ollama tags response: {err}"))?;
                Ok(parsed.models.into_iter().map(|model| model.name).collect())
            }
            LlmProvider::LmStudio => {
                let url = format!("{}/v1/models", self.normalized_endpoint());
                let response =
                    http.get(url).send().await.map_err(|err| {
                        format!("Failed calling LM Studio models endpoint: {err}")
                    })?;

                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.map_err(|err| {
                        format!("Failed reading LM Studio models error body: {err}")
                    })?;
                    return Err(format!(
                        "LM Studio models request failed ({status}): {body}"
                    ));
                }

                let parsed: LmStudioModelsResponse = response
                    .json()
                    .await
                    .map_err(|err| format!("Failed parsing LM Studio models response: {err}"))?;
                Ok(parsed.data.into_iter().map(|model| model.id).collect())
            }
            LlmProvider::Anthropic => Ok(vec![
                "claude-3-5-sonnet-20241022".to_string(),
                "claude-3-5-haiku-20241022".to_string(),
                "claude-3-opus-20240229".to_string(),
            ]),
            LlmProvider::OpenAi => Ok(vec![
                "gpt-4o".to_string(),
                "gpt-4o-mini".to_string(),
                "o1-mini".to_string(),
                "o1-preview".to_string(),
            ]),
            LlmProvider::Google => Ok(vec![
                "gemini-1.5-pro".to_string(),
                "gemini-1.5-flash".to_string(),
                "gemini-2.0-flash-exp".to_string(),
            ]),
            LlmProvider::XAi => Ok(vec!["grok-2-1212".to_string(), "grok-beta".to_string()]),
        }
    }

    async fn complete(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
    ) -> Result<String, crate::AppError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        match self.provider {
            LlmProvider::Ollama => {
                let url = format!("{}/api/chat", self.normalized_endpoint());
                let mut ollama_messages = Vec::with_capacity(messages.len().saturating_add(1));
                ollama_messages.push(OllamaChatApiMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                });
                for msg in messages {
                    ollama_messages.push(OllamaChatApiMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
                let payload = OllamaChatRequest {
                    model: self.model.clone(),
                    messages: ollama_messages,
                    stream: false,
                };

                let response = http
                    .post(url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| format!("Failed calling Ollama chat endpoint: {err}"))?;

                let status = response.status();
                if !status.is_success() {
                    let body = response
                        .text()
                        .await
                        .map_err(|err| format!("Failed reading Ollama chat error body: {err}"))?;
                    return Err(format!("Ollama chat request failed ({status}): {body}"));
                }

                let parsed: OllamaChatResponse = response
                    .json()
                    .await
                    .map_err(|err| format!("Failed parsing Ollama chat response: {err}"))?;
                Ok(parsed.message.content)
            }
            LlmProvider::LmStudio => {
                let url = format!("{}/v1/chat/completions", self.normalized_endpoint());
                let mut openai_messages = Vec::with_capacity(messages.len().saturating_add(1));
                openai_messages.push(LmStudioPayloadMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                });
                for msg in messages {
                    openai_messages.push(LmStudioPayloadMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
                let payload = LmStudioChatRequest {
                    model: self.model.clone(),
                    messages: openai_messages,
                    stream: false,
                };

                let response = http
                    .post(url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| format!("Failed calling LM Studio chat endpoint: {err}"))?;

                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.map_err(|err| {
                        format!("Failed reading LM Studio chat error body: {err}")
                    })?;
                    return Err(format!("LM Studio chat request failed ({status}): {body}"));
                }

                let parsed: LmStudioChatResponse = response
                    .json()
                    .await
                    .map_err(|err| format!("Failed parsing LM Studio chat response: {err}"))?;
                let first = parsed
                    .choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| "LM Studio returned no chat choices".to_string())?;
                let text = first
                    .message
                    .content
                    .filter(|s| !s.is_empty())
                    .or_else(|| first.message.reasoning_content.filter(|s| !s.is_empty()))
                    .unwrap_or_default();
                Ok(text)
            }
            LlmProvider::Anthropic => {
                if self.endpoint.trim().is_empty() {
                    return Err(
                        "Anthropic API Key is required. Please set it in LLM Settings.".to_string(),
                    );
                }
                let url = "https://api.anthropic.com/v1/messages";
                let mut anthropic_messages = Vec::with_capacity(messages.len());
                for msg in messages {
                    anthropic_messages.push(AnthropicMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
                let payload = AnthropicChatRequest {
                    model: self.model.clone(),
                    max_tokens: 4000,
                    system: system_prompt.to_string(),
                    messages: anthropic_messages,
                };

                let response = http
                    .post(url)
                    .header("x-api-key", self.endpoint.trim())
                    .header("anthropic-version", "2023-06-01")
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| format!("Failed calling Anthropic messages endpoint: {err}"))?;

                let status = response.status();
                if !status.is_success() {
                    let body = response
                        .text()
                        .await
                        .map_err(|err| format!("Failed reading Anthropic error body: {err}"))?;
                    return Err(format!("Anthropic request failed ({status}): {body}"));
                }

                let parsed: AnthropicChatResponse = response
                    .json()
                    .await
                    .map_err(|err| format!("Failed parsing Anthropic response: {err}"))?;
                let text = parsed
                    .content
                    .into_iter()
                    .filter(|c| c.content_type == "text")
                    .map(|c| c.text)
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(text)
            }
            LlmProvider::OpenAi | LlmProvider::Google | LlmProvider::XAi => {
                let provider_name = match self.provider {
                    LlmProvider::OpenAi => "OpenAI",
                    LlmProvider::Google => "Google Gemini",
                    LlmProvider::XAi => "xAI Grok",
                    _ => unreachable!(),
                };
                if self.endpoint.trim().is_empty() {
                    return Err(format!(
                        "{provider_name} API Key is required. Please set it in LLM Settings."
                    ));
                }
                let url = match self.provider {
                    LlmProvider::OpenAi => "https://api.openai.com/v1/chat/completions",
                    LlmProvider::Google => {
                        "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
                    }
                    LlmProvider::XAi => "https://api.x.ai/v1/chat/completions",
                    _ => unreachable!(),
                };
                let mut openai_messages = Vec::with_capacity(messages.len().saturating_add(1));
                openai_messages.push(LmStudioPayloadMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                });
                for msg in messages {
                    openai_messages.push(LmStudioPayloadMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
                let payload = LmStudioChatRequest {
                    model: self.model.clone(),
                    messages: openai_messages,
                    stream: false,
                };

                let response = http
                    .post(url)
                    .header("Authorization", format!("Bearer {}", self.endpoint.trim()))
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| {
                        format!("Failed calling {provider_name} chat endpoint: {err}")
                    })?;

                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.map_err(|err| {
                        format!("Failed reading {provider_name} chat error body: {err}")
                    })?;
                    return Err(format!(
                        "{provider_name} chat request failed ({status}): {body}"
                    ));
                }

                let parsed: LmStudioChatResponse = response.json().await.map_err(|err| {
                    format!("Failed parsing {provider_name} chat response: {err}")
                })?;
                let first = parsed
                    .choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| format!("{provider_name} returned no chat choices"))?;
                let text = first
                    .message
                    .content
                    .filter(|s| !s.is_empty())
                    .or_else(|| first.message.reasoning_content.filter(|s| !s.is_empty()))
                    .unwrap_or_default();
                Ok(text)
            }
        }
    }
}

// Anthropic Request/Response Structs
#[derive(Serialize)]
struct AnthropicChatRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicChatResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
    #[serde(rename = "type")]
    content_type: String,
}
