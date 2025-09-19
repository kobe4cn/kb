use async_trait::async_trait;
use kb_core::{QueryRequest, QueryResponse, Citation};
use kb_error::Result;
use kb_llm::{ChatModel, EmbedModel};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::instrument;

/// RAG 引擎的统一抽象接口
#[async_trait]
pub trait RagEngine: Send + Sync {
    /// 执行查询
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse>;

    /// 添加文档文本
    async fn add_document_text(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
    ) -> Result<()> {
        self.add_document_text_with_meta(document_id, text, page, None).await
    }

    /// 添加带元数据的文档文本
    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()>;

    /// 健康检查
    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::Healthy)
    }

    /// 获取引擎统计信息
    async fn stats(&self) -> Result<EngineStats> {
        Ok(EngineStats::default())
    }
}

/// GraphRAG 引擎抽象
#[async_trait]
pub trait GraphRagEngine: Send + Sync {
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse>;
    async fn build_graph(&self, documents: &[String]) -> Result<()>;
    async fn get_entity_neighbors(&self, entity: &str, hops: u8) -> Result<Vec<String>>;
}

/// RAG 文档元数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RagMeta {
    pub tenant_id: Option<String>,
    pub source: Option<String>,
    pub tags: Option<Vec<String>>,
    pub created_at: Option<i64>,
    pub custom_fields: Option<serde_json::Value>,
}

/// 引擎健康状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { error: String },
}

/// 引擎统计信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineStats {
    pub total_documents: u64,
    pub total_chunks: u64,
    pub index_size_bytes: u64,
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    pub query_count: u64,
    pub average_query_latency_ms: f64,
}

/// 通用 RAG 引擎基类
pub struct BaseRagEngine {
    pub chat_model: Arc<dyn ChatModel>,
    pub embed_model: Arc<dyn EmbedModel>,
    pub config: RagEngineConfig,
}

/// RAG 引擎配置
#[derive(Debug, Clone)]
pub struct RagEngineConfig {
    pub max_context_length: usize,
    pub default_top_k: u16,
    pub similarity_threshold: f32,
    pub enable_reranking: bool,
    pub chunk_size: usize,
    pub chunk_overlap: usize,
}

impl Default for RagEngineConfig {
    fn default() -> Self {
        Self {
            max_context_length: 8000,
            default_top_k: 5,
            similarity_threshold: 0.7,
            enable_reranking: false,
            chunk_size: 1000,
            chunk_overlap: 200,
        }
    }
}

impl BaseRagEngine {
    pub fn new(
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: RagEngineConfig,
    ) -> Self {
        Self {
            chat_model,
            embed_model,
            config,
        }
    }

    /// 通用的文本分块逻辑
    pub fn chunk_text(&self, text: &str) -> Vec<String> {
        let mut chunks = Vec::new();
        let words: Vec<&str> = text.split_whitespace().collect();

        if words.is_empty() {
            return chunks;
        }

        let mut current_chunk = String::new();
        let mut word_count = 0;
        let target_words = self.config.chunk_size / 5; // 估算每个词约5个字符
        let overlap_words = self.config.chunk_overlap / 5;

        for word in words {
            if word_count >= target_words && !current_chunk.is_empty() {
                chunks.push(current_chunk.trim().to_string());

                // 保留重叠部分
                if overlap_words > 0 && word_count > overlap_words {
                    let chunk_words: Vec<&str> = current_chunk.split_whitespace().collect();
                    let overlap_start = chunk_words.len().saturating_sub(overlap_words);
                    current_chunk = chunk_words[overlap_start..].join(" ");
                    current_chunk.push(' ');
                    word_count = overlap_words;
                } else {
                    current_chunk.clear();
                    word_count = 0;
                }
            }

            if !current_chunk.is_empty() {
                current_chunk.push(' ');
            }
            current_chunk.push_str(word);
            word_count += 1;
        }

        if !current_chunk.trim().is_empty() {
            chunks.push(current_chunk.trim().to_string());
        }

        chunks
    }

    /// 通用的上下文格式化逻辑
    pub fn format_context(&self, citations: &[Citation]) -> String {
        citations
            .iter()
            .enumerate()
            .map(|(i, citation)| {
                format!(
                    "[{}] (doc={} page={:?} score={:.3})\n{}",
                    i + 1,
                    citation.document_id,
                    citation.page,
                    citation.score,
                    citation.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// 通用的 LLM 查询逻辑
    #[instrument(skip(self, context, query))]
    pub async fn generate_answer(&self, context: &str, query: &str) -> Result<String> {
        let system = "You are a helpful assistant. Answer the user's question based on the provided context. If the context doesn't contain enough information to answer the question, say so clearly. Always cite your sources using [1], [2], etc. when referencing information from the context.";

        // 检查上下文长度
        let context_length = context.len();
        let truncated_context = if context_length > self.config.max_context_length {
            tracing::warn!(
                "Context length {} exceeds maximum {}, truncating",
                context_length,
                self.config.max_context_length
            );
            &context[..self.config.max_context_length]
        } else {
            context
        };

        self.chat_model
            .chat(system, truncated_context, query)
            .await
            .map_err(|e| kb_error::KbError::LlmService {
                provider: "chat".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            })
    }

    /// 计算余弦相似度
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let mut dot_product = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;

        let len = a.len().min(b.len());
        for i in 0..len {
            dot_product += a[i] * b[i];
            norm_a += a[i] * a[i];
            norm_b += b[i] * b[i];
        }

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot_product / (norm_a.sqrt() * norm_b.sqrt())
    }
}

/// 占位 RAG 引擎实现（用于测试）
pub struct NoopRagEngine;

#[async_trait]
impl RagEngine for NoopRagEngine {
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        Ok(QueryResponse {
            answer: format!("[noop] Query: {}", req.query),
            citations: vec![],
            contexts: vec![],
            mode: req.mode.unwrap_or_else(|| "noop".to_string()),
            latency_ms: 0,
        })
    }

    async fn add_document_text_with_meta(
        &self,
        _document_id: &str,
        _text: &str,
        _page: Option<i32>,
        _meta: Option<RagMeta>,
    ) -> Result<()> {
        Ok(())
    }
}