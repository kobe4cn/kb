use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

pub use kb_error::{KbError, Result};

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn chat(&self, system: &str, context: &str, user: &str) -> Result<String>;
}

#[async_trait]
pub trait EmbedModel: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

// ========== OpenAI-compatible (covers OpenAI, DeepSeek, some Qwen proxies) ==========

#[derive(Clone)]
pub struct OpenAiCompatConfig {
    pub base_url: String,                // e.g. https://api.openai.com
    pub api_key: String,                 // Bearer token
    pub chat_model: String,              // e.g. gpt-4o, deepseek-chat
    pub embedding_model: Option<String>, // e.g. text-embedding-3-small
}

#[derive(Clone)]
pub struct OpenAiCompatClient {
    http: Client,
    cfg: OpenAiCompatConfig,
}

impl OpenAiCompatClient {
    pub fn new(cfg: OpenAiCompatConfig) -> Self {
        Self {
            http: Client::new(),
            cfg,
        }
    }
}

#[derive(Serialize)]
struct OaiChatReqMsg {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OaiChatReq {
    model: String,
    messages: Vec<OaiChatReqMsg>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct OaiChatRespChoiceMsg {
    content: String,
}

#[derive(Deserialize)]
struct OaiChatRespChoice {
    message: OaiChatRespChoiceMsg,
}

#[derive(Deserialize)]
struct OaiChatResp {
    choices: Vec<OaiChatRespChoice>,
}

#[async_trait]
impl ChatModel for OpenAiCompatClient {
    #[instrument(skip(self, system, context, user))]
    async fn chat(&self, system: &str, context: &str, user: &str) -> Result<String> {
        let url = format!(
            "{}/v1/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let body = OaiChatReq {
            model: self.cfg.chat_model.clone(),
            messages: vec![
                OaiChatReqMsg {
                    role: "system".into(),
                    content: system.to_string(),
                },
                OaiChatReqMsg {
                    role: "user".into(),
                    content: format!("{}\n\nContext:\n{}", user, context),
                },
            ],
            temperature: Some(0.2),
        };

        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| KbError::Network {
                operation: "http_request".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(KbError::LlmService {
                provider: "openai_compat".to_string(),
                message: format!("status={} body={}", status, txt),
                retry_after: None,
            });
        }

        let data: OaiChatResp = resp.json().await.map_err(|e| KbError::Network {
            operation: "http_request".to_string(),
            message: e.to_string(),
        })?;
        let content = data
            .choices
            .get(0)
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        Ok(content)
    }
}

#[derive(Serialize)]
struct OaiEmbedReq {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct OaiEmbedData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct OaiEmbedResp {
    data: Vec<OaiEmbedData>,
}

#[async_trait]
impl EmbedModel for OpenAiCompatClient {
    #[instrument(skip(self, texts))]
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let model = self
            .cfg
            .embedding_model
            .clone()
            .ok_or_else(|| KbError::Configuration {
                key: "embedding_model".to_string(),
                reason: "not configured".to_string(),
            })?;
        let url = format!("{}/v1/embeddings", self.cfg.base_url.trim_end_matches('/'));
        let body = OaiEmbedReq {
            model,
            input: texts.to_vec(),
        };

        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| KbError::Network {
                operation: "http_request".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(KbError::LlmService {
                provider: "openai_compat".to_string(),
                message: format!("status={} body={}", status, txt),
                retry_after: None,
            });
        }

        let data: OaiEmbedResp = resp.json().await.map_err(|e| KbError::Network {
            operation: "http_request".to_string(),
            message: e.to_string(),
        })?;
        Ok(data.data.into_iter().map(|d| d.embedding).collect())
    }
}

// ========== Anthropic (Claude) ==========

#[derive(Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,   // e.g. claude-3-5-sonnet-latest
    pub api_url: String, // default https://api.anthropic.com
}

#[derive(Clone)]
pub struct AnthropicClient {
    http: Client,
    cfg: AnthropicConfig,
}

impl AnthropicClient {
    pub fn new(cfg: AnthropicConfig) -> Self {
        Self {
            http: Client::new(),
            cfg,
        }
    }
}

#[derive(Serialize)]
struct AnthMessageContent {
    r#type: &'static str,
    text: String,
}

#[derive(Serialize)]
struct AnthMessageReqMsg {
    role: &'static str,
    content: Vec<AnthMessageContent>,
}

#[derive(Serialize)]
struct AnthMessageReq {
    model: String,
    messages: Vec<AnthMessageReqMsg>,
    max_tokens: u32,
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct AnthMessageRespContent {
    #[allow(dead_code)]
    r#type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthMessageResp {
    content: Vec<AnthMessageRespContent>,
}

#[async_trait]
impl ChatModel for AnthropicClient {
    #[instrument(skip(self, system, context, user))]
    async fn chat(&self, system: &str, context: &str, user: &str) -> Result<String> {
        let url = format!("{}/v1/messages", self.cfg.api_url.trim_end_matches('/'));
        let body = AnthMessageReq {
            model: self.cfg.model.clone(),
            messages: vec![AnthMessageReqMsg {
                role: "user",
                content: vec![AnthMessageContent {
                    r#type: "text",
                    text: format!("{}\n\nSystem:\n{}\n\nContext:\n{}", user, system, context),
                }],
            }],
            max_tokens: 2048,
            temperature: Some(0.2),
        };

        let resp = self
            .http
            .post(url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| KbError::Network {
                operation: "http_request".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(KbError::LlmService {
                provider: "openai_compat".to_string(),
                message: format!("status={} body={}", status, txt),
                retry_after: None,
            });
        }

        let data: AnthMessageResp = resp.json().await.map_err(|e| KbError::Network {
            operation: "http_request".to_string(),
            message: e.to_string(),
        })?;
        let mut out = String::new();
        for c in data.content.into_iter() {
            if let Some(t) = c.text {
                out.push_str(&t);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl EmbedModel for AnthropicClient {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Err(KbError::Configuration {
            key: "embedding_provider".to_string(),
            reason: "Anthropic does not provide embeddings; configure another embedding provider"
                .to_string(),
        })
    }
}

// ========== Qwen (DashScope) Embeddings ==========

#[derive(Clone)]
pub struct QwenDashScopeConfig {
    pub api_key: String,
    pub model: String,   // e.g. text-embedding-v2 / v3
    pub api_url: String, // default https://dashscope.aliyuncs.com/api/v1/embeddings
}

#[derive(Clone)]
pub struct QwenDashScopeClient {
    http: Client,
    cfg: QwenDashScopeConfig,
}

impl QwenDashScopeClient {
    pub fn new(cfg: QwenDashScopeConfig) -> Self {
        Self {
            http: Client::new(),
            cfg,
        }
    }
}

#[derive(Serialize)]
struct DashScopeEmbedReq {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct DashScopeEmbedVec {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct DashScopeEmbedResp {
    data: Vec<DashScopeEmbedVec>,
}

#[async_trait]
impl EmbedModel for QwenDashScopeClient {
    #[instrument(skip(self, texts))]
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = self.cfg.api_url.clone();
        let body = DashScopeEmbedReq {
            model: self.cfg.model.clone(),
            input: texts.to_vec(),
        };
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", self.cfg.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| KbError::Network {
                operation: "http_request".to_string(),
                message: e.to_string(),
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(KbError::LlmService {
                provider: "openai_compat".to_string(),
                message: format!("status={} body={}", status, txt),
                retry_after: None,
            });
        }
        let data: DashScopeEmbedResp = resp.json().await.map_err(|e| KbError::Network {
            operation: "http_request".to_string(),
            message: e.to_string(),
        })?;
        Ok(data.data.into_iter().map(|d| d.embedding).collect())
    }
}

// ========== DeepSeek (OpenAI-compatible) Embeddings ==========

#[derive(Clone)]
pub struct DeepSeekConfig {
    pub api_key: String,
    pub base_url: String, // https://api.deepseek.com
    pub model: String,    // e.g. deepseek-embedding
}

#[derive(Clone)]
pub struct DeepSeekClient(OpenAiCompatClient);

impl DeepSeekClient {
    pub fn new(cfg: DeepSeekConfig) -> Self {
        Self(OpenAiCompatClient::new(OpenAiCompatConfig {
            base_url: cfg.base_url,
            api_key: cfg.api_key,
            chat_model: "".into(),
            embedding_model: Some(cfg.model),
        }))
    }
}

#[async_trait]
impl EmbedModel for DeepSeekClient {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.0.embed(texts).await
    }
}

// ========== Provider Factory & Config ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ChatProviderConfig {
    #[serde(rename = "openai_compat")]
    OpenAiCompat {
        base_url: String,
        api_key: String,
        model: String,
    },
    #[serde(rename = "anthropic")]
    Anthropic {
        api_url: Option<String>,
        api_key: String,
        model: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum EmbedProviderConfig {
    #[serde(rename = "openai_compat")]
    OpenAiCompat {
        base_url: String,
        api_key: String,
        model: String,
    },
    #[serde(rename = "qwen")]
    QwenDashScope {
        api_url: Option<String>,
        api_key: String,
        model: String,
    },
    #[serde(rename = "deepseek")]
    DeepSeek {
        base_url: Option<String>,
        api_key: String,
        model: String,
    },
}

pub struct Providers {
    pub chat: Box<dyn ChatModel>,
    pub embed: Box<dyn EmbedModel>,
}

pub fn make_providers(chat: ChatProviderConfig, embed: EmbedProviderConfig) -> Result<Providers> {
    let chat_box: Box<dyn ChatModel> = match chat {
        ChatProviderConfig::OpenAiCompat {
            base_url,
            api_key,
            model,
        } => Box::new(OpenAiCompatClient::new(OpenAiCompatConfig {
            base_url,
            api_key,
            chat_model: model,
            embedding_model: None,
        })),
        ChatProviderConfig::Anthropic {
            api_url,
            api_key,
            model,
        } => Box::new(AnthropicClient::new(AnthropicConfig {
            api_url: api_url.unwrap_or_else(|| "https://api.anthropic.com".into()),
            api_key,
            model,
        })),
    };

    let embed_box: Box<dyn EmbedModel> = match embed {
        EmbedProviderConfig::OpenAiCompat {
            base_url,
            api_key,
            model,
        } => Box::new(OpenAiCompatClient::new(OpenAiCompatConfig {
            base_url,
            api_key,
            chat_model: "".into(),
            embedding_model: Some(model),
        })),
        EmbedProviderConfig::QwenDashScope {
            api_url,
            api_key,
            model,
        } => Box::new(QwenDashScopeClient::new(QwenDashScopeConfig {
            api_key,
            model,
            api_url: api_url
                .unwrap_or_else(|| "https://dashscope.aliyuncs.com/api/v1/embeddings".into()),
        })),
        EmbedProviderConfig::DeepSeek {
            base_url,
            api_key,
            model,
        } => Box::new(DeepSeekClient::new(DeepSeekConfig {
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.deepseek.com".into()),
        })),
    };

    Ok(Providers {
        chat: chat_box,
        embed: embed_box,
    })
}
