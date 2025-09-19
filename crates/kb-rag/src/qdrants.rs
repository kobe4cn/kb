use async_trait::async_trait;
use kb_core::{Error, QueryRequest, QueryResponse};
use qdrant_client::{
    qdrant::{
        CreateCollectionBuilder, DeleteCollectionBuilder, Distance, QueryPointsBuilder,
        ScrollPoints, VectorParamsBuilder,
    },
    Qdrant,
};

use rig::{
    client::EmbeddingsClient,
    embeddings::{EmbeddingModel, EmbeddingsBuilder},
    providers::openai::{Client, TEXT_EMBEDDING_ADA_002},
    vector_store::{InsertDocuments, VectorSearchRequest, VectorStoreIndex},
    Embed,
};
use rig_qdrant::QdrantVectorStore;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument, warn};

use anyhow::Result;

use crate::{RagEngine, RagMeta};

/// 知识块文档结构，符合 Rig 最佳实践
#[derive(Serialize, Deserialize, Debug, Embed)]
pub struct KnowledgeChunk {
    /// 文档ID
    document_id: String,
    /// 块ID
    chunk_id: String,
    /// 页码（可选）
    page: Option<i32>,
    /// 租户ID（可选）
    tenant_id: Option<String>,
    /// 标签列表（可选）
    tags: Option<Vec<String>>,
    /// 来源信息（可选）
    source: Option<String>,
    /// 创建时间戳
    created_at: i64,
    /// 自定义字段（可选）
    custom_fields: Option<serde_json::Value>,
    /// 文本内容（用于嵌入）
    #[embed]
    description: String,
}

pub struct QdrantRagEngine {
    client: Qdrant,
    collection_name: String,
    embed_model: rig::providers::openai::EmbeddingModel,
    vector_store: QdrantVectorStore<rig::providers::openai::EmbeddingModel>,
}

impl QdrantRagEngine {
    pub async fn new(qdrant_url: String, collection_name: String, key: String) -> Result<Self> {
        let client = Qdrant::from_url(&qdrant_url).build()?;
        let openai_client = Client::new(&key);
        let embed_model = openai_client.embedding_model(TEXT_EMBEDDING_ADA_002);

        let vector_size = embed_model.ndims();

        if !client.collection_exists(&collection_name).await? {
            client
                .create_collection(
                    CreateCollectionBuilder::new(&collection_name).vectors_config(
                        VectorParamsBuilder::new(vector_size as u64, Distance::Cosine).build(),
                    ),
                )
                .await?;
        }
        let query_params = QueryPointsBuilder::new(collection_name.clone()).with_payload(true);
        let vector_store =
            QdrantVectorStore::new(client.clone(), embed_model.clone(), query_params.build());

        let engine = Self {
            client,
            collection_name: collection_name.clone(),
            embed_model,
            vector_store,
        };

        Ok(engine)
    }

    /// 向量搜索（使用 VectorSearchRequest 接口）
    pub async fn vector_search_with_request(
        &self,
        req: &VectorSearchRequest,
    ) -> Result<Vec<(f64, String, KnowledgeChunk)>> {
        // 生成查询向量

        let result = self
            .vector_store
            .top_n::<KnowledgeChunk>(req.clone())
            .await?;
        Ok(result)
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

        self.client.delete_collection(delete_request).await?;

        info!("Deleted Qdrant collection: {}", self.collection_name);
        Ok(())
    }

    /// 添加文档
    pub async fn add_document(&self, knowledge_chunks: Vec<KnowledgeChunk>) -> Result<()> {
        let documents = EmbeddingsBuilder::new(self.embed_model.clone())
            .documents(knowledge_chunks)?
            .build()
            .await?;

        self.vector_store.insert_documents(documents).await?;

        Ok(())
    }
}

#[async_trait]
impl RagEngine for QdrantRagEngine {
    /// 执行查询
    async fn query(&self, req: QueryRequest) -> kb_error::Result<QueryResponse> {
        let vector_req = VectorSearchRequest::builder()
            .query(&req.query)
            .samples(req.top_k.unwrap_or(1) as u64)
            //创建额外的搜索参数
            // req.filters 中的写法如下:
            // let mut additional_params = serde_json::Map::new();
            // additional_params.insert(
            //     "tenant_id".to_string(),
            //     serde_json::Value::String("2".to_string()),
            // );
            // additional_params.insert(
            //     "page".to_string(),
            //     serde_json::Value::Number(serde_json::Number::from(1)),
            // );
            // let additional_params = serde_json::Value::Object(additional_params);
            .additional_params(req.filters.unwrap_or_default())
            .map_err(|e| kb_error::KbError::VectorStore {
                operation: "build_search_request".to_string(),
                message: format!("Failed to build search request: {}", e),
            })?
            .build()
            .map_err(|e| kb_error::KbError::VectorStore {
                operation: "build_search_request".to_string(),
                message: format!("Failed to build search request: {}", e),
            })?;
        let results = self.vector_search_with_request(&vector_req).await?;

        Ok(QueryResponse {
            answer: "".to_string(),
            citations: vec![],
            contexts: vec![],
            mode: "qdrant".to_string(),
            latency_ms: 0,
        })
    }

    /// 添加带元数据的文档文本
    async fn add_document_text_with_meta(
        &self,
        document_id: String,
        /// 块ID
        chunk_id: String,
        /// 页码（可选）
        page: Option<i32>,
        /// 租户ID（可选）
        tenant_id: Option<String>,
        /// 标签列表（可选）
        tags: Option<Vec<String>>,
        /// 来源信息（可选）
        source: Option<String>,
        /// 创建时间戳
        created_at: i64,
        /// 自定义字段（可选）
        custom_fields: Option<serde_json::Value>,
        /// 文本内容（用于嵌入）
        #[embed]
        description: String,
    ) -> kb_error::Result<()> {
        Ok(())
    }
}

// #[async_trait]
// impl RagEngine for QdrantRagEngine {
//     #[instrument(skip(self, req))]
//     async fn query(&self, req: QueryRequest) -> Result<QueryResponse> {

//     }

//     #[instrument(skip(self, text))]
//     async fn add_document_text_with_meta(
//         &self,
//         document_id: &str,
//         text: &str,
//         page: Option<i32>,
//         meta: Option<RagMeta>,
//     ) -> Result<()> {
//         Ok(())
//     }
// }
