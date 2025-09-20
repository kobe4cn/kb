use crate::engine::{BaseRagEngine, RagDocumentChunk, RagEngine, RagEngineConfig, RagMeta};
use async_trait::async_trait;
use kb_core::{Citation, QueryRequest, QueryResponse};
use kb_error::{KbError, Result};
use kb_llm::{ChatModel, EmbedModel};
use qdrant_client::{
    qdrant::{
        vectors_config::Config, with_payload_selector::SelectorOptions, Condition,
        CreateCollection, DeleteCollectionBuilder, DeletePoints, Distance, FieldCondition, Filter,
        Match, PointStruct, PointsSelector, ScrollPoints, SearchPoints, UpsertPoints, Value,
        VectorParams, VectorsConfig, WithPayloadSelector,
    },
    Qdrant,
};
use rig::vector_store::request::VectorSearchRequest;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

/// 与向量数据库交互的标准分块结构
type KnowledgeChunk = RagDocumentChunk;

/// 基于Qdrant的RAG引擎
pub struct QdrantRagEngine {
    base: BaseRagEngine,
    client: Qdrant,
    collection_name: String,
    vector_size: usize,
}

impl QdrantRagEngine {
    pub async fn new(
        qdrant_url: String,
        collection_name: String,
        chat_model: Arc<dyn ChatModel>,
        embed_model: Arc<dyn EmbedModel>,
        config: Option<RagEngineConfig>,
    ) -> Result<Self> {
        let client = Qdrant::from_url(&qdrant_url)
            .build()
            .map_err(|e| KbError::VectorStore {
                operation: "connect".to_string(),
                message: format!("Failed to connect to Qdrant: {}", e),
            })?;

        let config = config.unwrap_or_default();

        // 获取嵌入模型的向量维度
        let vector_size = embed_model
            .embed(&["test".to_string()])
            .await?
            .first()
            .map(|v| v.len())
            .unwrap_or(1536);
        info!("Using embedding dimension: {} ", vector_size);

        let engine = Self {
            base: BaseRagEngine::new(chat_model, embed_model, config),
            client,
            collection_name: collection_name.clone(),
            vector_size,
        };

        // 确保collection存在
        engine.ensure_collection().await?;

        Ok(engine)
    }

    /// 确保collection存在
    async fn ensure_collection(&self) -> Result<()> {
        // 检查collection是否存在
        match self.client.collection_exists(&self.collection_name).await {
            Ok(exists) => {
                if !exists {
                    self.create_collection().await?;
                }
            }
            Err(e) => {
                warn!("Failed to check collection existence: {}", e);
                // 尝试创建collection
                self.create_collection().await?;
            }
        }
        Ok(())
    }

    /// 创建collection
    async fn create_collection(&self) -> Result<()> {
        let vectors_config = VectorsConfig {
            config: Some(Config::Params(VectorParams {
                size: self.vector_size as u64,
                distance: Distance::Cosine.into(),
                ..Default::default()
            })),
        };

        let create_collection = CreateCollection {
            collection_name: self.collection_name.clone(),
            vectors_config: Some(vectors_config),
            ..Default::default()
        };

        self.client
            .create_collection(create_collection)
            .await
            .map_err(|e| KbError::VectorStore {
                operation: "create_collection".to_string(),
                message: format!(
                    "Failed to create collection {}: {}",
                    self.collection_name, e
                ),
            })?;

        info!("Created Qdrant collection: {}", self.collection_name);
        Ok(())
    }

    /// 向量搜索（使用 VectorSearchRequest 接口）
    #[instrument(skip(self, req))]
    async fn vector_search_with_request(
        &self,
        req: &VectorSearchRequest,
        filters: Option<&serde_json::Value>,
    ) -> Result<Vec<(f32, KnowledgeChunk)>> {
        // 生成查询向量
        let query_embedding = self
            .base
            .embed_model
            .embed(&[req.query().to_string()])
            .await
            .map_err(|e| KbError::EmbeddingService {
                provider: "qdrant".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            })?
            .into_iter()
            .next()
            .unwrap_or_default();

        self.vector_search(query_embedding, req.samples() as usize, filters)
            .await
    }

    /// 向量搜索（底层实现）
    #[instrument(skip(self, query_embedding))]
    async fn vector_search(
        &self,
        query_embedding: Vec<f32>,
        top_k: usize,
        filters: Option<&serde_json::Value>,
    ) -> Result<Vec<(f32, KnowledgeChunk)>> {
        let mut search_points = SearchPoints {
            collection_name: self.collection_name.clone(),
            vector: query_embedding,
            limit: top_k as u64,
            score_threshold: Some(self.base.config.similarity_threshold),
            with_payload: Some(WithPayloadSelector {
                selector_options: Some(SelectorOptions::Enable(true)),
            }),
            ..Default::default()
        };

        // 构建过滤器
        if let Some(filters) = filters {
            search_points.filter = Some(self.build_qdrant_filter(filters)?);
        }

        let search_result = self
            .client
            .search_points(search_points)
            .await
            .map_err(|e| KbError::VectorStore {
                operation: "search".to_string(),
                message: format!("Failed to search points: {}", e),
            })?;

        let mut results = Vec::new();
        for scored_point in search_result.result {
            let payload = scored_point.payload;

            // 转换payload为JSON并反序列化为KnowledgeChunk
            let mut json_payload = serde_json::Map::new();
            for (k, v) in payload {
                if let Some(kind) = v.kind {
                    let json_value = match kind {
                        qdrant_client::qdrant::value::Kind::StringValue(s) => {
                            serde_json::Value::String(s)
                        }
                        qdrant_client::qdrant::value::Kind::IntegerValue(i) => {
                            serde_json::Value::Number(serde_json::Number::from(i))
                        }
                        qdrant_client::qdrant::value::Kind::DoubleValue(f) => {
                            if let Some(num) = serde_json::Number::from_f64(f) {
                                serde_json::Value::Number(num)
                            } else {
                                serde_json::Value::String(f.to_string())
                            }
                        }
                        qdrant_client::qdrant::value::Kind::BoolValue(b) => {
                            serde_json::Value::Bool(b)
                        }
                        qdrant_client::qdrant::value::Kind::ListValue(list) => {
                            let arr: Vec<serde_json::Value> = list
                                .values
                                .into_iter()
                                .map(|item| {
                                    if let Some(kind) = item.kind {
                                        match kind {
                                            qdrant_client::qdrant::value::Kind::StringValue(s) => {
                                                serde_json::Value::String(s)
                                            }
                                            _ => serde_json::Value::String("".to_string()),
                                        }
                                    } else {
                                        serde_json::Value::String("".to_string())
                                    }
                                })
                                .collect();
                            serde_json::Value::Array(arr)
                        }
                        _ => serde_json::Value::String("".to_string()),
                    };
                    json_payload.insert(k, json_value);
                }
            }

            // 尝试反序列化为KnowledgeChunk
            match serde_json::from_value::<KnowledgeChunk>(serde_json::Value::Object(json_payload))
            {
                Ok(chunk) => results.push((scored_point.score, chunk)),
                Err(e) => {
                    warn!("Failed to deserialize chunk: {}. Skipping.", e);
                    continue;
                }
            }
        }

        Ok(results)
    }

    /// 构建Qdrant过滤器
    fn build_qdrant_filter(&self, filters: &serde_json::Value) -> Result<Filter> {
        let mut conditions = Vec::new();

        // 文档ID过滤
        if let Some(document_id) = filters.get("document_id").and_then(|v| v.as_str()) {
            conditions.push(Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "document_id".to_string(),
                        r#match: Some(Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                document_id.to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                )),
            });
        }

        // 租户ID过滤
        if let Some(tenant_id) = filters.get("tenant_id").and_then(|v| v.as_str()) {
            conditions.push(Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "tenant_id".to_string(),
                        r#match: Some(Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                tenant_id.to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                )),
            });
        }

        // 标签过滤
        if let Some(tags_filter) = filters.get("tags").and_then(|v| v.as_array()) {
            for tag_value in tags_filter {
                if let Some(tag) = tag_value.as_str() {
                    conditions.push(Condition {
                        condition_one_of: Some(
                            qdrant_client::qdrant::condition::ConditionOneOf::Field(
                                FieldCondition {
                                    key: "tags".to_string(),
                                    r#match: Some(Match {
                                        match_value: Some(
                                            qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                                tag.to_string(),
                                            ),
                                        ),
                                    }),
                                    ..Default::default()
                                },
                            ),
                        ),
                    });
                }
            }
        }

        // 时间范围过滤
        if let Some(start_time) = filters.get("start_time").and_then(|v| v.as_i64()) {
            conditions.push(Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "created_at".to_string(),
                        range: Some(qdrant_client::qdrant::Range {
                            gte: Some(start_time as f64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
            });
        }

        if let Some(end_time) = filters.get("end_time").and_then(|v| v.as_i64()) {
            conditions.push(Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "created_at".to_string(),
                        range: Some(qdrant_client::qdrant::Range {
                            lte: Some(end_time as f64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
            });
        }

        Ok(Filter {
            must: conditions,
            ..Default::default()
        })
    }

    /// 获取文档计数
    pub async fn document_count(&self) -> Result<usize> {
        let scroll_points = ScrollPoints {
            collection_name: self.collection_name.clone(),
            limit: Some(0), // 只计数，不返回数据
            ..Default::default()
        };

        match self.client.scroll(scroll_points).await {
            Ok(result) => Ok(result.result.len()),
            Err(e) => {
                warn!("Failed to get document count: {}", e);
                Ok(0)
            }
        }
    }

    /// 删除整个 collection
    pub async fn delete_collection(&self) -> Result<()> {
        let delete_request = DeleteCollectionBuilder::new(&self.collection_name).build();

        self.client
            .delete_collection(delete_request)
            .await
            .map_err(|e| KbError::VectorStore {
                operation: "delete_collection".to_string(),
                message: format!(
                    "Failed to delete collection {}: {}",
                    self.collection_name, e
                ),
            })?;

        info!("Deleted Qdrant collection: {}", self.collection_name);
        Ok(())
    }

    /// 删除文档
    pub async fn remove_document(&self, document_id: &str) -> Result<usize> {
        let filter = Filter {
            must: vec![Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "document_id".to_string(),
                        r#match: Some(Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                document_id.to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                )),
            }],
            ..Default::default()
        };

        let points_selector = PointsSelector {
            points_selector_one_of: Some(
                qdrant_client::qdrant::points_selector::PointsSelectorOneOf::Filter(filter),
            ),
        };

        let delete_request = DeletePoints {
            collection_name: self.collection_name.clone(),
            points: Some(points_selector),
            ..Default::default()
        };

        match self.client.delete_points(delete_request).await {
            Ok(response) => {
                let deleted_count = response.result.map(|r| r.status.into()).unwrap_or(0);
                info!(
                    "Deleted {} points for document {}",
                    deleted_count, document_id
                );
                Ok(deleted_count as usize)
            }
            Err(e) => {
                error!("Failed to delete document {}: {}", document_id, e);
                Err(KbError::VectorStore {
                    operation: "delete_document".to_string(),
                    message: format!("Failed to delete document {}: {}", document_id, e),
                })
            }
        }
    }
}

#[async_trait]
impl RagEngine for QdrantRagEngine {
    #[instrument(skip(self, req))]
    async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        let start_time = std::time::Instant::now();

        // 使用 VectorSearchRequest 构建查询
        let top_k = req.top_k.unwrap_or(self.base.config.default_top_k);
        let vector_req = VectorSearchRequest::builder()
            .query(&req.query)
            .samples(top_k as u64)
            .threshold(self.base.config.similarity_threshold as f64)
            .build()
            .map_err(|e| KbError::VectorStore {
                operation: "build_search_request".to_string(),
                message: format!("Failed to build search request: {}", e),
            })?;

        // 执行向量搜索
        let search_results = self
            .vector_search_with_request(&vector_req, req.filters.as_ref())
            .await?;

        if search_results.is_empty() {
            return Ok(QueryResponse {
                answer: "抱歉，我在知识库中没有找到相关的信息来回答您的问题。".to_string(),
                citations: vec![],
                contexts: vec![],
                mode: req.mode.unwrap_or_else(|| "qdrant".to_string()),
                latency_ms: start_time.elapsed().as_millis() as i64,
            });
        }

        // 构建引用和上下文
        let mut citations = Vec::new();
        let mut contexts = Vec::new();

        for (score, chunk) in search_results {
            citations.push(Citation {
                document_id: chunk.document_id.clone(),
                chunk_id: chunk.chunk_id.clone(),
                page: chunk.page,
                score,
                snippet: if chunk.text.len() > 240 {
                    chunk.text.chars().take(240).collect::<String>() + "..."
                } else {
                    chunk.text.clone()
                },
            });
            contexts.push(chunk.text);
        }

        // 格式化上下文并生成回答
        let formatted_context = self.base.format_context(&citations);
        let answer = self
            .base
            .generate_answer(&formatted_context, &req.query)
            .await?;

        let latency_ms = start_time.elapsed().as_millis() as i64;

        info!(
            query = %req.query,
            results_count = citations.len(),
            latency_ms = latency_ms,
            "Qdrant RAG query completed"
        );

        Ok(QueryResponse {
            answer,
            citations,
            contexts,
            mode: req.mode.unwrap_or_else(|| "qdrant".to_string()),
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
        let knowledge_chunks = self
            .base
            .chunk_document(document_id, text, page, meta.clone());

        let chunks_count = knowledge_chunks.len();

        if knowledge_chunks.is_empty() {
            warn!("No chunks created for document {}", document_id);
            return Ok(());
        }

        // 为所有块生成嵌入（仍使用现有的嵌入模型，因为与 Rig 嵌入模型不兼容）
        let embeddings = self
            .base
            .embed_model
            .embed(
                &knowledge_chunks
                    .iter()
                    .map(|c| c.text.clone())
                    .collect::<Vec<_>>(),
            )
            .await
            .map_err(|e| KbError::EmbeddingService {
                provider: "qdrant".to_string(),
                message: e.to_string(),
                retry_after: e.retry_after(),
            })?;

        // 创建向量点
        let mut points = Vec::new();
        for (chunk, embedding) in knowledge_chunks.into_iter().zip(embeddings) {
            let point_id = Uuid::new_v4().to_string();

            // 直接序列化 KnowledgeChunk 为 JSON
            let json_chunk = serde_json::to_value(&chunk).map_err(|e| KbError::VectorStore {
                operation: "serialize_chunk".to_string(),
                message: format!("Failed to serialize chunk: {}", e),
            })?;

            // 转换 JSON 为 Qdrant 格式
            let mut qdrant_payload = std::collections::HashMap::new();
            for (k, v) in json_chunk.as_object().unwrap() {
                let qdrant_value = match v {
                    serde_json::Value::String(s) => Value {
                        kind: Some(qdrant_client::qdrant::value::Kind::StringValue(s.clone())),
                    },
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value {
                                kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)),
                            }
                        } else if let Some(f) = n.as_f64() {
                            Value {
                                kind: Some(qdrant_client::qdrant::value::Kind::DoubleValue(f)),
                            }
                        } else {
                            Value {
                                kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                                    n.to_string(),
                                )),
                            }
                        }
                    }
                    serde_json::Value::Array(arr) => {
                        let list_value = qdrant_client::qdrant::ListValue {
                            values: arr
                                .iter()
                                .map(|item| Value {
                                    kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                                        item.to_string(),
                                    )),
                                })
                                .collect(),
                        };
                        Value {
                            kind: Some(qdrant_client::qdrant::value::Kind::ListValue(list_value)),
                        }
                    }
                    _ => Value {
                        kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                            v.to_string(),
                        )),
                    },
                };
                qdrant_payload.insert(k.clone(), qdrant_value);
            }

            let point = PointStruct {
                id: Some(point_id.into()),
                vectors: Some(embedding.into()),
                payload: qdrant_payload,
            };

            points.push(point);
        }

        // 批量插入向量
        let upsert_request = UpsertPoints {
            collection_name: self.collection_name.clone(),
            points,
            ..Default::default()
        };

        self.client
            .upsert_points(upsert_request)
            .await
            .map_err(|e| KbError::VectorStore {
                operation: "upsert_points".to_string(),
                message: format!("Failed to upsert points: {}", e),
            })?;

        info!(
            document_id = %document_id,
            chunks_added = chunks_count,
            "Added document chunks to Qdrant"
        );

        Ok(())
    }
}
