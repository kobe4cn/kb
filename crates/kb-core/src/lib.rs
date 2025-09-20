use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub kind: String,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub source_id: Option<Uuid>,
    pub title: Option<String>,
    pub uri: String,
    pub version: String,
    pub sha256: String,
    pub mime_type: Option<String>,
    pub tags: Vec<String>,
    pub visibility: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub ord: i32,
    pub page: Option<i32>,
    pub start_offset: Option<i32>,
    pub end_offset: Option<i32>,
    pub text: String,
    pub vector_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexJob {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub document_ids: Vec<Uuid>,
    pub run_graph: bool,
    pub run_vector: bool,
    pub run_lexical: bool,
    pub status: String,
    pub attempts: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub query: String,
    pub mode: Option<String>, // rag | graph | hybrid | lexical
    pub top_k: Option<u16>,
    pub rerank: Option<bool>,
    pub filters: Option<serde_json::Value>,
    pub stream: Option<bool>,
    pub include_raw_matches: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub document_id: String,
    pub chunk_id: String,
    pub page: Option<i32>,
    pub score: f32,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub answer: String,
    pub citations: Vec<Citation>,
    pub contexts: Vec<String>,
    pub mode: String,
    pub latency_ms: i64,
}

pub use kb_error::{KbError as Error, Result};
