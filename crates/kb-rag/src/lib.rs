pub mod engine;
pub mod memory;
pub mod qdrant;
pub mod multi_provider;
pub mod rerank;
pub mod lexical;
pub mod hybrid;
pub mod qdrants;

// 重新导出新的模块化架构
pub use engine::{
    RagEngine, GraphRagEngine, RagMeta, HealthStatus, EngineStats,
    BaseRagEngine, RagEngineConfig, NoopRagEngine
};
pub use memory::MemoryRagEngine;
pub use qdrant::QdrantRagEngine;
pub use multi_provider::{MultiProviderRagEngine as RealMultiProviderRagEngine, StorageType};
pub use rerank::{Reranker, RerankerFactory};
pub use lexical::{LexicalRagEngine, LexicalConfig, LexicalIndexStats};
pub use hybrid::{HybridRagEngine, HybridConfig, HybridStats, ScoreNormalization, FusionStrategy};

// 重新导出核心类型
pub use kb_core::{QueryRequest, QueryResponse, Citation};
pub use kb_error::{KbError, Result};

// 兼容性别名和占位实现
use async_trait::async_trait;
use std::sync::Arc;

pub use engine::NoopRagEngine as DefaultRagEngine;

// 兼容性包装结构 - 为API层提供向后兼容
pub struct RigQdrantRagEngine(QdrantRagEngine);
pub struct RigInMemoryRagEngine(MemoryRagEngine);
pub struct MultiProviderRagEngine(RealMultiProviderRagEngine);

impl RigQdrantRagEngine {
    pub async fn new(
        url: String,
        collection: String,
        _embed_model: String,
        chat_model: Arc<dyn kb_llm::ChatModel>,
    ) -> Result<Self> {
        // 为了兼容性，我们创建一个默认的嵌入模型
        // 在实际使用中，应该从配置或环境变量中获取真正的嵌入模型
        let embed_model = Arc::new(memory::MockEmbedModel);
        let engine = QdrantRagEngine::new(url, collection, chat_model, embed_model, None).await?;
        Ok(Self(engine))
    }
}

impl RigInMemoryRagEngine {
    pub fn new(
        _embed_model: String,
        chat_model: Arc<dyn kb_llm::ChatModel>,
    ) -> Self {
        // 为了兼容性，我们创建一个默认的嵌入模型
        let embed_model = Arc::new(memory::MockEmbedModel);
        let engine = MemoryRagEngine::from_models(chat_model, embed_model, None);
        Self(engine)
    }
}

impl MultiProviderRagEngine {
    pub fn new(
        chat_model: Arc<dyn kb_llm::ChatModel>,
        embed_model: Arc<dyn kb_llm::EmbedModel>,
    ) -> Self {
        let engine = RealMultiProviderRagEngine::new_memory(chat_model, embed_model, None);
        Self(engine)
    }
}

// 为包装类型实现 RagEngine trait
#[async_trait]
impl RagEngine for RigQdrantRagEngine {
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        self.0.query(req).await
    }

    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        self.0.add_document_text_with_meta(document_id, text, page, meta).await
    }
}

#[async_trait]
impl RagEngine for RigInMemoryRagEngine {
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        self.0.query(req).await
    }

    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        self.0.add_document_text_with_meta(document_id, text, page, meta).await
    }
}

#[async_trait]
impl RagEngine for MultiProviderRagEngine {
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        self.0.query(req).await
    }

    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        self.0.add_document_text_with_meta(document_id, text, page, meta).await
    }
}


// 占位 GraphRAG 实现（向后兼容）
pub struct DefaultGraphRagEngine;

#[async_trait]
impl GraphRagEngine for DefaultGraphRagEngine {
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        Ok(QueryResponse {
            answer: format!("[stub-graph] mode={:?} query={}", req.mode, req.query),
            citations: vec![],
            contexts: vec![],
            mode: req.mode.unwrap_or_else(|| "graph".into()),
            latency_ms: 0,
        })
    }

    async fn build_graph(&self, _documents: &[String]) -> Result<()> {
        Ok(())
    }

    async fn get_entity_neighbors(&self, _entity: &str, _hops: u8) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

// 工具函数
pub use once_cell::sync::Lazy;
pub use std::time::Duration;
pub use tokio::sync::Semaphore;

// 抽取服务相关函数
pub async fn extract_text_via_service(path: &str) -> Result<String> {
    let url = std::env::var("EXTRACT_URL")
        .map_err(|_| KbError::Configuration {
            key: "EXTRACT_URL".to_string(),
            reason: "environment variable not set".to_string(),
        })?;

    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload.bin");

    let data = tokio::fs::read(path)
        .await
        .map_err(|e| KbError::Internal {
            message: format!("Failed to read file: {}", e),
            details: Some(path.to_string()),
        })?;

    extract_text_via_service_bytes(filename, &data, &url).await
}

pub async fn extract_text_via_service_bytes(
    filename: &str,
    data: &[u8],
    url: &str,
) -> Result<String> {
    static EXTRACT_SEM: Lazy<Semaphore> = Lazy::new(|| {
        let permits = std::env::var("EXTRACT_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4);
        Semaphore::new(if permits == 0 { 1 } else { permits })
    });

    let _permit = EXTRACT_SEM
        .acquire()
        .await
        .map_err(|e| KbError::Concurrency {
            operation: "extract_semaphore".to_string(),
            message: e.to_string(),
        })?;

    let timeout_ms: u64 = std::env::var("EXTRACT_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15000);

    let retries: usize = std::env::var("EXTRACT_RETRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    let mut backoff_ms: u64 = std::env::var("EXTRACT_RETRY_BASE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);

    let client = reqwest::Client::new();
    let mut attempt = 0usize;

    loop {
        attempt += 1;
        let mut rb = client
            .post(url)
            .header("Content-Type", "application/octet-stream")
            .header("X-Filename", filename)
            .timeout(Duration::from_millis(timeout_ms))
            .body(data.to_vec());

        if let Ok(token) = std::env::var("EXTRACT_TOKEN") {
            rb = rb.bearer_auth(token);
        }

        if let Some(ext) = filename.split('.').last() {
            rb = rb.header("X-File-Ext", ext);
        }

        let result = rb.send().await;
        match result {
            Ok(resp) => {
                if resp.status().is_success() {
                    let text = resp
                        .text()
                        .await
                        .map_err(|e| KbError::Network {
                            operation: "extract_response_read".to_string(),
                            message: e.to_string(),
                        })?;
                    return Ok(text);
                } else {
                    let status = resp.status();
                    let _body = resp.text().await.unwrap_or_default();
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if retryable && attempt <= retries + 1 {
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                        backoff_ms = backoff_ms.saturating_mul(2);
                        continue;
                    }
                    return Err(KbError::ServiceUnavailable {
                        service: format!("extract_service ({})", status),
                        retry_after: if retryable { Some(Duration::from_secs(30)) } else { None },
                    });
                }
            }
            Err(e) => {
                if attempt <= retries + 1 {
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms.saturating_mul(2);
                    continue;
                }
                return Err(KbError::Network {
                    operation: "extract_request".to_string(),
                    message: e.to_string(),
                });
            }
        }
    }
}

pub async fn extract_service_health() -> Result<()> {
    let url = std::env::var("EXTRACT_URL")
        .map_err(|_| KbError::Configuration {
            key: "EXTRACT_URL".to_string(),
            reason: "environment variable not set".to_string(),
        })?;

    let client = reqwest::Client::new();

    // 先尝试 HEAD，不支持则回退 GET
    let mut rb = client.head(&url);
    if let Ok(token) = std::env::var("EXTRACT_TOKEN") {
        rb = rb.bearer_auth(token);
    }

    let resp = rb.send().await;
    if let Ok(r) = resp {
        if r.status().is_success() {
            return Ok(());
        }
    }

    let mut rb = client.get(&url);
    if let Ok(token) = std::env::var("EXTRACT_TOKEN") {
        rb = rb.bearer_auth(token);
    }

    let r = rb.send().await.map_err(|e| KbError::Network {
        operation: "extract_health_check".to_string(),
        message: e.to_string(),
    })?;
    if r.status().is_success() {
        Ok(())
    } else {
        Err(KbError::ServiceUnavailable {
            service: format!("extract_service ({})", r.status()),
            retry_after: Some(Duration::from_secs(60)),
        })
    }
}

/// 简易文本分块处理
pub async fn index_text_with_chunking(
    engine: std::sync::Arc<dyn RagEngine>,
    document_id: &str,
    text: &str,
    chunk_size: usize,
    overlap: usize,
) -> Result<usize> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if current.len() + word.len() + 1 > chunk_size && !current.is_empty() {
            chunks.push(current.clone());
            if overlap > 0 {
                let keep: String = current
                    .chars()
                    .rev()
                    .take(overlap)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                current = keep;
            } else {
                current.clear();
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }

    if !current.trim().is_empty() {
        chunks.push(current);
    }

    let mut n = 0usize;
    for c in chunks {
        engine.add_document_text(document_id, &c, None).await?;
        n += 1;
    }

    Ok(n)
}

/// 简易网页加载
pub async fn index_web_url(
    engine: std::sync::Arc<dyn RagEngine>,
    url: &str,
    document_id: &str,
) -> Result<()> {
    let body = reqwest::get(url)
        .await
        .map_err(|e| KbError::Network {
            operation: "web_fetch".to_string(),
            message: e.to_string(),
        })?
        .text()
        .await
        .map_err(|e| KbError::Network {
            operation: "web_fetch_text".to_string(),
            message: e.to_string(),
        })?;
    let text = html2text::from_read(body.as_bytes(), 80);
    engine.add_document_text(document_id, &text, None).await
}