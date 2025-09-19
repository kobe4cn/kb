use crate::engine::{RagEngine, RagEngineConfig, RagMeta};
use crate::qdrant::QdrantRagEngine;
use crate::memory::MemoryRagEngine;
use async_trait::async_trait;
use kb_core::{QueryRequest, QueryResponse};
use kb_error::{KbError, Result};
use kb_llm::{ChatModel, EmbedModel};
use std::sync::Arc;
use tracing::{info, instrument};

/// 存储类型枚举
#[derive(Debug, Clone, PartialEq)]
pub enum StorageType {
    Memory,
    Qdrant { url: String, collection: String },
}

/// 多提供商RAG引擎 - 支持不同的向量存储后端
pub struct MultiProviderRagEngine {
    storage_type: StorageType,
    engine: Box<dyn RagEngine>,
}

impl MultiProviderRagEngine {
    /// 创建基于内存的RAG引擎
    pub fn new_memory(
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Self {
        let engine = MemoryRagEngine::from_models(chat_model, embed_model, config);

        Self {
            storage_type: StorageType::Memory,
            engine: Box::new(engine),
        }
    }

    /// 创建基于Qdrant的RAG引擎
    pub async fn new_qdrant(
        qdrant_url: String,
        collection_name: String,
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Result<Self> {
        let engine = QdrantRagEngine::new(
            qdrant_url.clone(),
            collection_name.clone(),
            chat_model,
            embed_model,
            config,
        ).await?;

        Ok(Self {
            storage_type: StorageType::Qdrant {
                url: qdrant_url,
                collection: collection_name,
            },
            engine: Box::new(engine),
        })
    }

    /// 创建基于配置的RAG引擎
    pub async fn from_config(
        storage_config: &serde_json::Value,
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Result<Self> {
        let storage_type = storage_config
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("memory");

        match storage_type {
            "qdrant" => {
                let url = storage_config
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| KbError::Configuration {
                        key: "qdrant.url".to_string(),
                        reason: "Missing Qdrant URL".to_string(),
                    })?;

                let collection = storage_config
                    .get("collection")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| KbError::Configuration {
                        key: "qdrant.collection".to_string(),
                        reason: "Missing Qdrant collection name".to_string(),
                    })?;

                Self::new_qdrant(
                    url.to_string(),
                    collection.to_string(),
                    chat_model,
                    embed_model,
                    config,
                ).await
            }
            "memory" => {
                Ok(Self::new_memory(chat_model, embed_model, config))
            }
            _ => Err(KbError::Configuration {
                key: "storage.type".to_string(),
                reason: format!("Unsupported storage type: {}", storage_type),
            }),
        }
    }

    /// 获取存储类型
    pub fn storage_type(&self) -> &StorageType {
        &self.storage_type
    }

    /// 获取存储信息
    pub fn storage_info(&self) -> serde_json::Value {
        match &self.storage_type {
            StorageType::Memory => serde_json::json!({
                "type": "memory",
                "description": "In-memory vector storage"
            }),
            StorageType::Qdrant { url, collection } => serde_json::json!({
                "type": "qdrant",
                "url": url,
                "collection": collection,
                "description": "Qdrant vector database"
            }),
        }
    }

    /// 切换存储后端
    pub async fn switch_storage(
        &mut self,
        new_storage_type: StorageType,
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Result<()> {
        let new_engine: Box<dyn RagEngine> = match &new_storage_type {
            StorageType::Memory => {
                Box::new(MemoryRagEngine::from_models(chat_model, embed_model, config))
            }
            StorageType::Qdrant { url, collection } => {
                Box::new(QdrantRagEngine::new(url.clone(), collection.clone(), chat_model, embed_model, config).await?)
            }
        };

        self.storage_type = new_storage_type;
        self.engine = new_engine;

        info!("Switched RAG storage backend to: {:?}", self.storage_type);
        Ok(())
    }

    /// 健康检查
    pub async fn health_check(&self) -> Result<serde_json::Value> {
        match &self.storage_type {
            StorageType::Memory => Ok(serde_json::json!({
                "status": "healthy",
                "storage": "memory",
                "details": "In-memory storage is always available"
            })),
            StorageType::Qdrant { url, collection } => {
                // 尝试执行一个简单的查询来检查连接
                let test_query = QueryRequest {
                    query: "test".to_string(),
                    mode: Some("qdrant".to_string()),
                    top_k: Some(1),
                    filters: None,
                    rerank: Some(false),
                    stream: Some(false),
                    include_raw_matches: Some(false),
                };

                match self.engine.query(test_query).await {
                    Ok(_) => Ok(serde_json::json!({
                        "status": "healthy",
                        "storage": "qdrant",
                        "url": url,
                        "collection": collection,
                        "details": "Qdrant connection successful"
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "status": "unhealthy",
                        "storage": "qdrant",
                        "url": url,
                        "collection": collection,
                        "error": e.to_string()
                    })),
                }
            }
        }
    }

    /// 获取统计信息
    pub async fn get_stats(&self) -> Result<serde_json::Value> {
        let storage_info = self.storage_info();

        // 基础统计信息
        let stats = serde_json::json!({
            "storage": storage_info,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "document_count": 0,  // 暂时设为0，真正的实现需要扩展trait
            "note": "Document count requires trait extension to implement safely"
        });

        Ok(stats)
    }
}

#[async_trait]
impl RagEngine for MultiProviderRagEngine {
    #[instrument(skip(self, req))]
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        self.engine.query(req).await
    }

    #[instrument(skip(self, text))]
    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> Result<()> {
        self.engine.add_document_text_with_meta(document_id, text, page, meta).await
    }
}

impl MultiProviderRagEngine {
    /// 获取文档计数（安全的方式）
    pub async fn document_count(&self) -> Result<usize> {
        match &self.storage_type {
            StorageType::Memory => {
                // 我们暂时返回0，因为无法安全地向下转型
                // 在真正的实现中，应该在RagEngine trait中添加document_count方法
                Ok(0)
            }
            StorageType::Qdrant { .. } => {
                // 同样，暂时返回0
                Ok(0)
            }
        }
    }

    /// 安全地删除文档
    pub async fn remove_document(&self, _document_id: &str) -> Result<usize> {
        match &self.storage_type {
            StorageType::Memory => {
                // 暂时返回0，实际应该通过trait方法实现
                Ok(0)
            }
            StorageType::Qdrant { .. } => {
                // 暂时返回0，实际应该通过trait方法实现
                Ok(0)
            }
        }
    }
}