use rig::embeddings::{EmbedError, EmbeddingError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, warn};

#[cfg(feature = "axum")]
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json},
};

/// 系统统一错误类型
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum KbError {
    // === 业务错误 ===
    #[error("资源未找到: {resource}")]
    NotFound { resource: String },

    #[error("请求无效: {reason}")]
    InvalidRequest { reason: String },

    #[error("权限不足: {operation}")]
    Unauthorized { operation: String },

    #[error("认证失败: {message}")]
    Authentication { message: String },

    #[error("验证失败: {message}")]
    Validation { message: String },

    #[error("资源冲突: {details}")]
    Conflict { details: String },

    #[error("配额超限: {resource} 已达到 {limit}")]
    QuotaExceeded { resource: String, limit: String },

    // === 技术错误 ===
    #[error("数据库错误")]
    Database {
        message: String,
        #[serde(skip)]
        context: Option<DatabaseContext>,
    },

    #[error("向量存储错误: {operation} 失败")]
    VectorStore { operation: String, message: String },

    #[error("搜索引擎错误: {engine}")]
    SearchEngine { engine: String, message: String },

    #[error("LLM 服务错误 ({provider})")]
    LlmService {
        provider: String,
        message: String,
        #[serde(skip)]
        retry_after: Option<std::time::Duration>,
    },

    #[error("嵌入服务错误 ({provider})")]
    EmbeddingService {
        provider: String,
        message: String,
        #[serde(skip)]
        retry_after: Option<std::time::Duration>,
    },

    #[error("外部服务不可用: {service}")]
    ServiceUnavailable {
        service: String,
        #[serde(skip)]
        retry_after: Option<std::time::Duration>,
    },

    // === 系统错误 ===
    #[error("内部系统错误: {message}")]
    Internal {
        message: String,
        details: Option<String>,
    },

    #[error("配置错误: {key} - {reason}")]
    Configuration { key: String, reason: String },

    #[error("序列化错误: {format}")]
    Serialization { format: String, message: String },

    #[error("网络错误: {operation}")]
    Network { operation: String, message: String },

    #[error("超时错误: {operation} 超过 {timeout_ms}ms")]
    Timeout { operation: String, timeout_ms: u64 },

    #[error("并发错误: {operation}")]
    Concurrency { operation: String, message: String },

    #[error("Qdrant错误: {operation}")]
    QdrantError { operation: String, message: String },

    #[error("Rig错误: {operation}")]
    RigError { operation: String, message: String },

    #[error("Embed错误: {operation}")]
    EmbedError { operation: String, message: String },

    #[error("VectorStore错误: {operation}")]
    VectorStoreError { operation: String, message: String },

    #[error("Anyhow错误: {operation}")]
    AnyhowError { operation: String, message: String },
}

/// 数据库上下文信息
#[derive(Debug, Clone)]
pub struct DatabaseContext {
    pub query: Option<String>,
    pub table: Option<String>,
    pub connection_id: Option<String>,
}

/// 错误严重级别
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ErrorSeverity {
    Low,      // 可预期的业务错误
    Medium,   // 技术错误但不影响核心功能
    High,     // 影响核心功能的错误
    Critical, // 系统级严重错误
}

/// 错误元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMetadata {
    pub error_id: String,
    pub severity: ErrorSeverity,
    pub component: String,
    pub operation: Option<String>,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub request_id: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub context: std::collections::HashMap<String, String>,
}

impl KbError {
    /// 获取错误的严重级别
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            KbError::NotFound { .. } | KbError::InvalidRequest { .. } => ErrorSeverity::Low,
            KbError::Unauthorized { .. }
            | KbError::Authentication { .. }
            | KbError::Validation { .. }
            | KbError::Conflict { .. }
            | KbError::QuotaExceeded { .. } => ErrorSeverity::Medium,
            KbError::Database { .. }
            | KbError::VectorStore { .. }
            | KbError::SearchEngine { .. } => ErrorSeverity::High,
            KbError::LlmService { .. } | KbError::EmbeddingService { .. } => ErrorSeverity::Medium,
            KbError::ServiceUnavailable { .. }
            | KbError::Network { .. }
            | KbError::Timeout { .. } => ErrorSeverity::Medium,
            KbError::Internal { .. } | KbError::Configuration { .. } => ErrorSeverity::Critical,
            KbError::Serialization { .. } | KbError::Concurrency { .. } => ErrorSeverity::High,
            KbError::QdrantError { .. } => ErrorSeverity::High,
            KbError::RigError { .. } => ErrorSeverity::High,
            KbError::EmbedError { .. } => ErrorSeverity::High,
            KbError::VectorStoreError { .. } => ErrorSeverity::High,
            KbError::AnyhowError { .. } => ErrorSeverity::High,
        }
    }

    /// 是否为可重试错误
    pub fn is_retryable(&self) -> bool {
        match self {
            KbError::ServiceUnavailable { retry_after, .. } => retry_after.is_some(),
            KbError::Network { .. } | KbError::Timeout { .. } => true,
            KbError::LlmService { retry_after, .. }
            | KbError::EmbeddingService { retry_after, .. } => retry_after.is_some(),
            KbError::Concurrency { .. } => true,
            _ => false,
        }
    }

    /// 获取重试延迟时间
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            KbError::ServiceUnavailable { retry_after, .. }
            | KbError::LlmService { retry_after, .. }
            | KbError::EmbeddingService { retry_after, .. } => *retry_after,
            KbError::Network { .. } => Some(std::time::Duration::from_millis(500)),
            KbError::Timeout { .. } => Some(std::time::Duration::from_millis(1000)),
            KbError::Concurrency { .. } => Some(std::time::Duration::from_millis(100)),
            _ => None,
        }
    }

    /// 记录错误日志
    pub fn log(&self, metadata: &ErrorMetadata) {
        match metadata.severity {
            ErrorSeverity::Low => {
                warn!(
                    error_id = %metadata.error_id,
                    component = %metadata.component,
                    operation = ?metadata.operation,
                    user_id = ?metadata.user_id,
                    tenant_id = ?metadata.tenant_id,
                    request_id = ?metadata.request_id,
                    error = %self,
                    "业务错误"
                );
            }
            ErrorSeverity::Medium => {
                warn!(
                    error_id = %metadata.error_id,
                    component = %metadata.component,
                    operation = ?metadata.operation,
                    user_id = ?metadata.user_id,
                    tenant_id = ?metadata.tenant_id,
                    request_id = ?metadata.request_id,
                    error = %self,
                    context = ?metadata.context,
                    "技术错误"
                );
            }
            ErrorSeverity::High | ErrorSeverity::Critical => {
                error!(
                    error_id = %metadata.error_id,
                    component = %metadata.component,
                    operation = ?metadata.operation,
                    user_id = ?metadata.user_id,
                    tenant_id = ?metadata.tenant_id,
                    request_id = ?metadata.request_id,
                    error = %self,
                    context = ?metadata.context,
                    severity = ?metadata.severity,
                    "严重错误"
                );
            }
        }
    }

    /// 转换为 HTTP 状态码
    pub fn to_http_status(&self) -> u16 {
        match self {
            KbError::NotFound { .. } => 404,
            KbError::InvalidRequest { .. } => 400,
            KbError::Unauthorized { .. } => 401,
            KbError::Authentication { .. } => 401,
            KbError::Validation { .. } => 400,
            KbError::Conflict { .. } => 409,
            KbError::QuotaExceeded { .. } => 429,
            KbError::ServiceUnavailable { .. } => 503,
            KbError::Timeout { .. } => 408,
            KbError::Configuration { .. } => 500,
            _ => 500,
        }
    }

    /// 获取用户友好的错误消息
    pub fn user_message(&self) -> String {
        match self {
            KbError::NotFound { .. } => "请求的资源不存在".to_string(),
            KbError::InvalidRequest { .. } => "请求参数有误，请检查后重试".to_string(),
            KbError::Unauthorized { .. } => "没有权限执行此操作".to_string(),
            KbError::Authentication { .. } => "认证失败，请重新登录".to_string(),
            KbError::Validation { .. } => "输入数据验证失败，请检查格式".to_string(),
            KbError::Conflict { .. } => "操作冲突，请稍后重试".to_string(),
            KbError::QuotaExceeded { .. } => "使用配额已超限，请稍后重试".to_string(),
            KbError::ServiceUnavailable { .. } => "服务暂时不可用，请稍后重试".to_string(),
            KbError::Timeout { .. } => "请求超时，请重试".to_string(),
            _ => "系统内部错误，请联系管理员".to_string(),
        }
    }
}

/// 创建错误元数据的便捷构造器
pub struct ErrorMetadataBuilder {
    metadata: ErrorMetadata,
}

impl ErrorMetadataBuilder {
    pub fn new(component: &str) -> Self {
        Self {
            metadata: ErrorMetadata {
                error_id: uuid::Uuid::new_v4().to_string(),
                severity: ErrorSeverity::Medium,
                component: component.to_string(),
                operation: None,
                user_id: None,
                tenant_id: None,
                request_id: None,
                timestamp: chrono::Utc::now(),
                context: std::collections::HashMap::new(),
            },
        }
    }

    pub fn operation(mut self, operation: &str) -> Self {
        self.metadata.operation = Some(operation.to_string());
        self
    }

    pub fn user_id(mut self, user_id: &str) -> Self {
        self.metadata.user_id = Some(user_id.to_string());
        self
    }

    pub fn tenant_id(mut self, tenant_id: &str) -> Self {
        self.metadata.tenant_id = Some(tenant_id.to_string());
        self
    }

    pub fn request_id(mut self, request_id: &str) -> Self {
        self.metadata.request_id = Some(request_id.to_string());
        self
    }

    pub fn context(mut self, key: &str, value: &str) -> Self {
        self.metadata
            .context
            .insert(key.to_string(), value.to_string());
        self
    }

    pub fn build(mut self, error: &KbError) -> ErrorMetadata {
        self.metadata.severity = error.severity();
        self.metadata
    }
}

pub type Result<T> = std::result::Result<T, KbError>;

// === 转换实现 ===

impl From<serde_json::Error> for KbError {
    fn from(err: serde_json::Error) -> Self {
        KbError::Serialization {
            format: "json".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<reqwest::Error> for KbError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            KbError::Timeout {
                operation: "http_request".to_string(),
                timeout_ms: 30000, // 默认超时时间
            }
        } else if err.is_connect() {
            KbError::Network {
                operation: "connect".to_string(),
                message: err.to_string(),
            }
        } else {
            KbError::Network {
                operation: "http_request".to_string(),
                message: err.to_string(),
            }
        }
    }
}

impl From<uuid::Error> for KbError {
    fn from(err: uuid::Error) -> Self {
        KbError::Serialization {
            format: "uuid".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<tokio::task::JoinError> for KbError {
    fn from(err: tokio::task::JoinError) -> Self {
        KbError::Concurrency {
            operation: "task_join".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<qdrant_client::QdrantError> for KbError {
    fn from(err: qdrant_client::QdrantError) -> Self {
        KbError::QdrantError {
            operation: "qdrant_client".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<EmbeddingError> for KbError {
    fn from(err: EmbeddingError) -> Self {
        KbError::EmbedError {
            operation: "embed".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<rig::vector_store::VectorStoreError> for KbError {
    fn from(err: rig::vector_store::VectorStoreError) -> Self {
        KbError::VectorStore {
            operation: "vector_store".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<rig::embeddings::embed::EmbedError> for KbError {
    fn from(err: rig::embeddings::embed::EmbedError) -> Self {
        KbError::EmbedError {
            operation: "embed".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<anyhow::Error> for KbError {
    fn from(err: anyhow::Error) -> Self {
        KbError::AnyhowError {
            operation: "anyhow".to_string(),
            message: err.to_string(),
        }
    }
}
// Axum integration
#[cfg(feature = "axum")]
impl IntoResponse for KbError {
    fn into_response(self) -> axum::response::Response {
        let status_code = match self {
            KbError::Authentication { .. } => StatusCode::UNAUTHORIZED,
            KbError::Validation { .. } => StatusCode::BAD_REQUEST,
            KbError::NotFound { .. } => StatusCode::NOT_FOUND,
            KbError::Unauthorized { .. } => StatusCode::FORBIDDEN,
            KbError::Conflict { .. } => StatusCode::CONFLICT,
            KbError::QuotaExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
            KbError::ServiceUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            KbError::Timeout { .. } => StatusCode::REQUEST_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = serde_json::json!({
            "error": self.to_string(),
            "message": self.user_message()
        });

        (status_code, Json(body)).into_response()
    }
}
