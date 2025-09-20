use async_trait::async_trait;
use kb_core::{Citation, QueryRequest, QueryResponse};
use kb_error::KbError;
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
};
use rig_qdrant::QdrantVectorStore;
use tracing::{info, warn};

use anyhow::Result as AnyResult;
use uuid::Uuid;

use crate::{engine::RagDocumentChunk, RagEngine, RagMeta};

/// 知识块文档结构，符合 Rig 最佳实践
type KnowledgeChunk = RagDocumentChunk;

pub struct QdrantRagEngine {
    client: Qdrant,
    collection_name: String,
    embed_model: rig::providers::openai::EmbeddingModel,
    vector_store: QdrantVectorStore<rig::providers::openai::EmbeddingModel>,
}

impl QdrantRagEngine {
    pub async fn new(qdrant_url: String, collection_name: String, key: String) -> AnyResult<Self> {
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
    ) -> AnyResult<Vec<(f64, String, KnowledgeChunk)>> {
        // 生成查询向量

        let result = self
            .vector_store
            .top_n::<KnowledgeChunk>(req.clone())
            .await?;
        Ok(result)
    }

    /// 获取文档计数
    pub async fn document_count(&self) -> AnyResult<usize> {
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
    pub async fn delete_collection(&self) -> AnyResult<()> {
        let delete_request = DeleteCollectionBuilder::new(&self.collection_name).build();

        self.client.delete_collection(delete_request).await?;

        info!("Deleted Qdrant collection: {}", self.collection_name);
        Ok(())
    }

    /// 添加文档
    pub async fn add_document(&self, knowledge_chunks: Vec<KnowledgeChunk>) -> AnyResult<()> {
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
        let start_time = std::time::Instant::now();
        let filters = req.filters.clone().unwrap_or_default();
        let samples = req.top_k.unwrap_or(5) as u64;

        let vector_req = VectorSearchRequest::builder()
            .query(&req.query)
            .samples(samples)
            .additional_params(filters)
            .map_err(|e| KbError::VectorStore {
                operation: "build_search_request".to_string(),
                message: format!("Failed to build search request: {}", e),
            })?
            .build()
            .map_err(|e| KbError::VectorStore {
                operation: "build_search_request".to_string(),
                message: format!("Failed to build search request: {}", e),
            })?;

        let results = self
            .vector_search_with_request(&vector_req)
            .await
            .map_err(|e| KbError::VectorStore {
                operation: "vector_search".to_string(),
                message: format!("Failed to execute vector search: {}", e),
            })?;

        let mut citations = Vec::new();
        let mut contexts = Vec::new();

        for (score, _point_id, chunk) in &results {
            let snippet = if chunk.text.len() > 240 {
                let mut snippet = chunk.text.chars().take(240).collect::<String>();
                snippet.push_str("...");
                snippet
            } else {
                chunk.text.clone()
            };

            citations.push(Citation {
                document_id: chunk.document_id.clone(),
                chunk_id: chunk.chunk_id.clone(),
                page: chunk.page,
                score: *score as f32,
                snippet,
            });

            contexts.push(chunk.text.clone());
        }

        let latency_ms = start_time.elapsed().as_millis() as i64;

        Ok(QueryResponse {
            answer: String::new(),
            citations,
            contexts,
            mode: req.mode.unwrap_or_else(|| "qdrant".to_string()),
            latency_ms,
        })
    }

    /// 添加带元数据的文档文本
    async fn add_document_text_with_meta(
        &self,
        document_id: &str,
        text: &str,
        page: Option<i32>,
        meta: Option<RagMeta>,
    ) -> kb_error::Result<()> {
        let chunk = RagDocumentChunk::from_text(
            document_id.to_string(),
            format!("{}#{}", document_id, Uuid::new_v4()),
            text.to_string(),
            page,
            meta,
        );

        self.add_document(vec![chunk])
            .await
            .map_err(|e| KbError::VectorStore {
                operation: "insert_documents".to_string(),
                message: format!("Failed to insert documents into Qdrant: {}", e),
            })?;
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
