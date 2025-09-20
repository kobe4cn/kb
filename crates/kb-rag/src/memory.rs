use crate::engine::{BaseRagEngine, RagDocumentChunk, RagEngine, RagEngineConfig, RagMeta};
use async_trait::async_trait;
use kb_core::{Citation, QueryRequest, QueryResponse};
use kb_error::{KbError, Result};
use kb_llm::{ChatModel, EmbedModel};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::instrument;

/// 内存中的文档块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    pub id: String,
    pub document_id: String,
    pub text: String,
    pub embedding: Vec<f32>,
    pub page: Option<i32>,
    pub meta: Option<RagMeta>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 基于内存的 RAG 引擎
pub struct MemoryRagEngine {
    base: BaseRagEngine,
    chunks: Arc<RwLock<Vec<MemoryChunk>>>,
}

impl MemoryRagEngine {
    fn new_internal(
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Self {
        Self {
            base: BaseRagEngine::new(chat_model, embed_model, config.unwrap_or_default()),
            chunks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    // 兼容性构造函数 - 为旧API提供支持
    pub async fn new(
        _url: String,
        _collection: String,
        _embed_model: String,
        chat_model: Arc<dyn kb_llm::ChatModel>,
    ) -> Result<Self> {
        // 兼容 RigQdrantRagEngine::new
        let embed_model = Arc::new(MockEmbedModel);
        Ok(Self::new_internal(chat_model, embed_model, None))
    }

    pub fn new_memory(_embed_model: String, chat_model: Arc<dyn kb_llm::ChatModel>) -> Self {
        let embed_model = Arc::new(MockEmbedModel);
        Self::new_internal(chat_model, embed_model, None)
    }

    pub fn new_multi_provider(
        chat_model: Arc<dyn kb_llm::ChatModel>,
        embed_model: Arc<dyn kb_llm::EmbedModel>,
    ) -> Self {
        Self::new_internal(chat_model, embed_model, None)
    }

    // 新的构造函数
    pub fn from_models(
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Self {
        Self::new_internal(chat_model, embed_model, config)
    }
}

/// Mock 嵌入模型用于兼容性
pub struct MockEmbedModel;

#[async_trait]
impl EmbedModel for MockEmbedModel {
    async fn embed(&self, texts: &[String]) -> kb_llm::Result<Vec<Vec<f32>>> {
        // 返回固定维度的随机向量用于测试
        Ok(texts
            .iter()
            .map(|_| vec![0.1, 0.2, 0.3, 0.4, 0.5])
            .collect())
    }
}

impl MemoryRagEngine {
    /// 获取当前索引的文档数量
    pub async fn document_count(&self) -> usize {
        let chunks = self.chunks.read().await;
        chunks.len()
    }

    /// 清空所有文档
    pub async fn clear(&self) -> Result<()> {
        let mut chunks = self.chunks.write().await;
        chunks.clear();
        Ok(())
    }

    /// 根据文档ID删除所有相关块
    pub async fn remove_document(&self, document_id: &str) -> Result<usize> {
        let mut chunks = self.chunks.write().await;
        let original_len = chunks.len();
        chunks.retain(|chunk| chunk.document_id != document_id);
        let removed_count = original_len - chunks.len();

        tracing::info!(
            "Removed {} chunks for document {}",
            removed_count,
            document_id
        );
        Ok(removed_count)
    }

    /// 向量相似度搜索
    #[instrument(skip(self, query_embedding))]
    async fn vector_search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        filters: Option<&serde_json::Value>,
    ) -> Result<Vec<(f32, MemoryChunk)>> {
        let chunks = self.chunks.read().await;

        let mut scored_chunks: Vec<(f32, &MemoryChunk)> = chunks
            .iter()
            .map(|chunk| {
                let similarity =
                    BaseRagEngine::cosine_similarity(query_embedding, &chunk.embedding);
                (similarity, chunk)
            })
            .collect();

        // 应用过滤器
        if let Some(filters) = filters {
            scored_chunks.retain(|(_, chunk)| self.apply_filters(chunk, filters));
        }

        // 按相似度排序并取 top-k
        scored_chunks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<(f32, MemoryChunk)> = scored_chunks
            .into_iter()
            .take(top_k)
            .map(|(score, chunk)| (score, chunk.clone()))
            .collect();

        Ok(results)
    }

    /// 应用查询过滤器
    fn apply_filters(&self, chunk: &MemoryChunk, filters: &serde_json::Value) -> bool {
        // 文档ID过滤
        if let Some(document_id) = filters.get("document_id").and_then(|v| v.as_str()) {
            if chunk.document_id != document_id {
                return false;
            }
        }

        // 租户ID过滤
        if let Some(tenant_id) = filters.get("tenant_id").and_then(|v| v.as_str()) {
            if let Some(ref meta) = chunk.meta {
                if meta.tenant_id.as_deref() != Some(tenant_id) {
                    return false;
                }
            } else {
                return false;
            }
        }

        // 标签过滤
        if let Some(tags_filter) = filters.get("tags").and_then(|v| v.as_array()) {
            if let Some(ref meta) = chunk.meta {
                if let Some(ref chunk_tags) = meta.tags {
                    let filter_tags: Vec<String> = tags_filter
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect();

                    // 检查是否有交集
                    let has_intersection = filter_tags.iter().any(|tag| chunk_tags.contains(tag));
                    if !has_intersection {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        // 时间范围过滤
        if let Some(start_time) = filters.get("start_time").and_then(|v| v.as_i64()) {
            if chunk.created_at.timestamp() < start_time {
                return false;
            }
        }

        if let Some(end_time) = filters.get("end_time").and_then(|v| v.as_i64()) {
            if chunk.created_at.timestamp() > end_time {
                return false;
            }
        }

        true
    }
}

#[async_trait]
impl RagEngine for MemoryRagEngine {
    #[instrument(skip(self, req))]
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        let start_time = std::time::Instant::now();

        // 生成查询向量
        let query_embedding = self
            .base
            .embed_model
            .embed(&[req.query.clone()])
            .await
            .map_err(|e| KbError::EmbeddingService {
                provider: "memory".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            })?
            .into_iter()
            .next()
            .unwrap_or_default();

        // 执行向量搜索
        let top_k = req.top_k.unwrap_or(self.base.config.default_top_k) as usize;
        let search_results = self
            .vector_search(&query_embedding, top_k, req.filters.as_ref())
            .await?;

        // 过滤低相似度结果
        let filtered_results: Vec<(f32, MemoryChunk)> = search_results
            .into_iter()
            .filter(|(score, _)| *score >= self.base.config.similarity_threshold)
            .collect();

        if filtered_results.is_empty() {
            return Ok(QueryResponse {
                answer: "抱歉，我在知识库中没有找到相关的信息来回答您的问题。".to_string(),
                citations: vec![],
                contexts: vec![],
                mode: req.mode.unwrap_or_else(|| "memory".to_string()),
                latency_ms: start_time.elapsed().as_millis() as i64,
            });
        }

        // 构建引用和上下文
        let mut citations = Vec::new();
        let mut contexts = Vec::new();

        for (score, chunk) in &filtered_results {
            citations.push(Citation {
                document_id: chunk.document_id.clone(),
                chunk_id: chunk.id.clone(),
                page: chunk.page,
                score: *score,
                snippet: if chunk.text.len() > 240 {
                    chunk.text.chars().take(240).collect::<String>() + "..."
                } else {
                    chunk.text.clone()
                },
            });
            contexts.push(chunk.text.clone());
        }

        // 格式化上下文并生成回答
        let formatted_context = self.base.format_context(&citations);
        let answer = self
            .base
            .generate_answer(&formatted_context, &req.query)
            .await?;

        let latency_ms = start_time.elapsed().as_millis() as i64;

        tracing::info!(
            query = %req.query,
            results_count = filtered_results.len(),
            latency_ms = latency_ms,
            "Memory RAG query completed"
        );

        Ok(QueryResponse {
            answer,
            citations,
            contexts,
            mode: req.mode.unwrap_or_else(|| "memory".to_string()),
            latency_ms,
        })
    }

    #[instrument(skip(self, text))]
    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        // 分块处理文本并生成统一结构
        let chunk_records = self
            .base
            .chunk_document(document_id, text, page, meta.clone());

        if chunk_records.is_empty() {
            tracing::warn!("No chunks created for document {}", document_id);
            return Ok(());
        }

        // 为所有块生成嵌入
        let embed_inputs: Vec<String> = chunk_records.iter().map(|c| c.text.clone()).collect();
        let embeddings = self
            .base
            .embed_model
            .embed(&embed_inputs)
            .await
            .map_err(|e| KbError::EmbeddingService {
                provider: "memory".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            })?;

        // 创建块对象
        let mut new_chunks = Vec::new();
        for (chunk, embedding) in chunk_records.into_iter().zip(embeddings) {
            let RagDocumentChunk {
                document_id,
                chunk_id,
                page,
                tenant_id,
                tags,
                source,
                created_at,
                custom_fields,
                text,
            } = chunk;

            let created_at_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(created_at, 0)
                .unwrap_or_else(|| chrono::Utc::now());

            let meta = RagMeta {
                tenant_id,
                source,
                tags,
                created_at: Some(created_at),
                custom_fields,
            };

            let memory_chunk = MemoryChunk {
                id: chunk_id,
                document_id,
                text,
                embedding,
                page,
                meta: Some(meta),
                created_at: created_at_dt,
            };
            new_chunks.push(memory_chunk);
        }

        // 添加到索引
        let mut chunks = self.chunks.write().await;
        chunks.extend(new_chunks.clone());

        tracing::info!(
            document_id = %document_id,
            chunks_added = new_chunks.len(),
            total_chunks = chunks.len(),
            "Added document chunks to memory index"
        );

        Ok(())
    }
}
