use axum::{
    debug_handler,
    extract::{Query, State},
    response::sse::{Event, Sse},
    routing::get,
    routing::post,
    Json, Router,
};

mod auth_routes;
use axum::http::{HeaderMap, Request, Response, StatusCode};
use dotenv::dotenv;
use futures::{Stream, StreamExt};
use kb_auth::{jwt::JwtService, rbac::RbacService, session::SessionService};
use kb_core::{QueryRequest, QueryResponse};
use kb_error::KbError;
use kb_llm::{make_providers, ChatProviderConfig, EmbedProviderConfig};
use kb_rag::{DefaultGraphRagEngine, GraphRagEngine, RagEngine};
use once_cell::sync::Lazy;
use rig::client::CompletionClient;
use rig::completion::Prompt;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::{
    auth::{AsyncAuthorizeRequest, AsyncRequireAuthorizationLayer},
    cors::CorsLayer,
    trace::TraceLayer,
};
use tracing::info;
use uuid::Uuid;

static SESSIONS: Lazy<tokio::sync::RwLock<std::collections::HashMap<Uuid, SessionState>>> =
    Lazy::new(|| tokio::sync::RwLock::new(std::collections::HashMap::new()));

static REDIS_URL: Lazy<Option<String>> = Lazy::new(|| std::env::var("REDIS_URL").ok());
static SESS_TTL_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("SESS_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600)
});
static SETTINGS: Lazy<tokio::sync::RwLock<HashMap<String, String>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

fn is_secret_key(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("KEY")
        || upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASS")
}

#[derive(Clone, Default)]
struct AdminAuthorizer;

impl<B> AsyncAuthorizeRequest<B> for AdminAuthorizer
where
    B: Send + 'static,
{
    type RequestBody = B;
    type ResponseBody = axum::body::Body;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Request<B>, Response<Self::ResponseBody>>>
                + Send,
        >,
    >;

    fn authorize(&mut self, request: Request<B>) -> Self::Future {
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let headers = parts.headers.clone();
            if admin_auth_ok(&headers) {
                Ok(Request::from_parts(parts, body))
            } else {
                Err(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(axum::body::Body::from("unauthorized"))
                    .unwrap())
            }
        })
    }
}

// ===============
// 简易 Job 队列/状态
// ===============
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Job {
    id: Uuid,
    kind: String,               // url | pdf_glob
    payload: serde_json::Value, // { url, document_id } 或 { glob, prefix }
    status: String,             // pending | running | done | error
    message: Option<String>,
    created_at: i64,
    updated_at: i64,
    attempts: u32,
    idempotency_key: Option<String>,
    progress: Option<JobProgress>,
    resume: Option<JobResume>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Default)]
struct JobProgress {
    total: Option<usize>,
    completed: usize,
    current: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
enum JobResume {
    PdfGlob {
        paths: Vec<String>,
        next: usize,
        prefix: String,
        chunk_size: usize,
        overlap: usize,
    },
}

static JOBS: Lazy<tokio::sync::RwLock<HashMap<Uuid, Job>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));
static JOB_QUEUE: Lazy<tokio::sync::RwLock<Vec<Uuid>>> =
    Lazy::new(|| tokio::sync::RwLock::new(Vec::new()));
static IDEMPOTENCY: Lazy<tokio::sync::RwLock<HashMap<String, Uuid>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

#[derive(Clone)]
struct SledStore {
    db: sled::Db,
}

impl SledStore {
    fn new(path: &str) -> Self {
        Self {
            db: sled::open(path).expect("open sled db"),
        }
    }
    fn save_all(&self, jobs: &HashMap<Uuid, Job>, queue: &Vec<Uuid>, idem: &HashMap<String, Uuid>) {
        let jobs_json = serde_json::to_vec(jobs).unwrap_or_default();
        let queue_json = serde_json::to_vec(queue).unwrap_or_default();
        let idem_json = serde_json::to_vec(idem).unwrap_or_default();
        let _ = self.db.insert("jobs", jobs_json);
        let _ = self.db.insert("queue", queue_json);
        let _ = self.db.insert("idem", idem_json);
        let _ = self.db.flush();
    }
    fn load_all(&self) -> (HashMap<Uuid, Job>, Vec<Uuid>, HashMap<String, Uuid>) {
        let jobs: HashMap<Uuid, Job> = self
            .db
            .get("jobs")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let queue: Vec<Uuid> = self
            .db
            .get("queue")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let idem: HashMap<String, Uuid> = self
            .db
            .get("idem")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        (jobs, queue, idem)
    }
}

static JOB_STORE: Lazy<SledStore> = Lazy::new(|| SledStore::new("data/jobs"));

// 记录每个 document_id 的已索引 chunks 数
static INDEX_COUNTS: Lazy<tokio::sync::RwLock<HashMap<String, usize>>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));

async fn inc_index_count(doc_id: &str, by: usize) {
    let mut m = INDEX_COUNTS.write().await;
    *m.entry(doc_id.to_string()).or_insert(0) += by;
}

// ===============
// 可选：Postgres 持久化（特性开关 pg）
// 这些函数在未启用特性时为空操作，启用后会真正写入数据库。
// 迁移脚本见 deployments/migrations/。

#[cfg(feature = "pg")]
mod pg_jobstore {
    use super::*;
    use sqlx::{types::Json, Connection, PgConnection, Row};

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../deployments/migrations");

    async fn get_conn() -> Result<PgConnection, String> {
        let url = std::env::var("DATABASE_URL").map_err(|_| "missing DATABASE_URL".to_string())?;
        PgConnection::connect(&url).await.map_err(|e| e.to_string())
    }

    pub async fn ensure_migrated() -> Result<(), String> {
        let mut conn = get_conn().await?;
        MIGRATOR.run(&mut conn).await.map_err(|e| e.to_string())
    }

    pub async fn load_all() -> Option<(HashMap<Uuid, Job>, Vec<Uuid>, HashMap<String, Uuid>)> {
        let mut conn = get_conn().await.ok()?;
        let mut jobs = HashMap::new();
        let mut queue = Vec::new();
        let mut idem = HashMap::new();
        let rows = sqlx::query("SELECT id, kind, payload, status, message, created_at, updated_at, attempts, idempotency_key, progress, resume FROM jobs")
            .fetch_all(&mut conn).await.ok()?;
        for r in rows {
            let id: Uuid = r.get("id");
            let kind: String = r.get("kind");
            let payload: Json<serde_json::Value> = r.get("payload");
            let status: String = r.get("status");
            let message: Option<String> = r.get("message");
            let created_at: i64 = r.get("created_at");
            let updated_at: i64 = r.get("updated_at");
            let attempts: i32 = r.get("attempts");
            let idempotency_key: Option<String> = r.get("idempotency_key");
            let progress: Option<Json<serde_json::Value>> = r.try_get("progress").ok();
            let resume: Option<Json<serde_json::Value>> = r.try_get("resume").ok();
            let job = Job {
                id,
                kind,
                payload: payload.0,
                status,
                message,
                created_at,
                updated_at,
                attempts: attempts as u32,
                idempotency_key,
                progress: progress.and_then(|j| serde_json::from_value(j.0).ok()),
                resume: resume.and_then(|j| serde_json::from_value(j.0).ok()),
            };
            jobs.insert(id, job);
        }
        let rows = sqlx::query("SELECT job_id FROM job_queue ORDER BY enqueued_at ASC")
            .fetch_all(&mut conn)
            .await
            .ok()?;
        for r in rows {
            let id: Uuid = r.get("job_id");
            queue.push(id);
        }
        let rows = sqlx::query("SELECT key, job_id FROM idempotency")
            .fetch_all(&mut conn)
            .await
            .ok()?;
        for r in rows {
            let k: String = r.get("key");
            let id: Uuid = r.get("job_id");
            idem.insert(k, id);
        }
        Some((jobs, queue, idem))
    }

    pub async fn upsert_job(job: &Job) {
        if let Ok(mut conn) = get_conn().await {
            let _ = sqlx::query(
                "INSERT INTO jobs (id, kind, payload, status, message, created_at, updated_at, attempts, idempotency_key, progress, resume)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                 ON CONFLICT (id) DO UPDATE SET kind=$2, payload=$3, status=$4, message=$5, updated_at=$7, attempts=$8, idempotency_key=$9, progress=$10, resume=$11"
            )
            .bind(job.id)
            .bind(&job.kind)
            .bind(Json(job.payload.clone()))
            .bind(&job.status)
            .bind(&job.message)
            .bind(job.created_at)
            .bind(job.updated_at)
            .bind(job.attempts as i32)
            .bind(&job.idempotency_key)
            .bind(job.progress.as_ref().map(|p| Json(serde_json::to_value(p).unwrap_or_default())))
            .bind(job.resume.as_ref().map(|r| Json(serde_json::to_value(r).unwrap_or_default())))
            .execute(&mut conn).await;
        }
    }

    pub async fn enqueue(id: Uuid) {
        if let Ok(mut conn) = get_conn().await {
            let _ =
                sqlx::query("INSERT INTO job_queue (job_id) VALUES ($1) ON CONFLICT DO NOTHING")
                    .bind(id)
                    .execute(&mut conn)
                    .await;
        }
    }
    pub async fn dequeue(id: Uuid) {
        if let Ok(mut conn) = get_conn().await {
            let _ = sqlx::query("DELETE FROM job_queue WHERE job_id=$1")
                .bind(id)
                .execute(&mut conn)
                .await;
        }
    }
    pub async fn upsert_idem(key: &str, id: Uuid) {
        if let Ok(mut conn) = get_conn().await {
            let _ = sqlx::query("INSERT INTO idempotency (key, job_id) VALUES ($1,$2) ON CONFLICT (key) DO UPDATE SET job_id=$2")
                .bind(key)
                .bind(id)
                .execute(&mut conn).await;
        }
    }
}

#[cfg(not(feature = "pg"))]
mod pg_jobstore {
    use super::*;
    pub async fn load_all() -> Option<(HashMap<Uuid, Job>, Vec<Uuid>, HashMap<String, Uuid>)> {
        None
    }
    pub async fn upsert_job(_job: &Job) {}
    pub async fn enqueue(_id: Uuid) {}
    pub async fn dequeue(_id: Uuid) {}
    pub async fn upsert_idem(_key: &str, _id: Uuid) {}
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SessionState {
    query: String,
    top_k: usize,
    filters: Option<serde_json::Value>,
    chat_history: Vec<rig::message::Message>,
    pending_tool: Option<PendingTool>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct PendingTool {
    id: String,
    call_id: Option<String>,
    name: String,
}

#[derive(Clone)]
struct AppState {
    rag: Arc<dyn RagEngine>,
    graph: Arc<dyn GraphRagEngine>,
    auth_services: auth_routes::AuthServices,
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    server: ServerCfg,
    chat_provider: ChatCfgYaml,
    embedding_provider: EmbedCfgYaml,
    vector_store: VectorStoreCfg,
    generation: Option<GenCfg>,
    extractor: Option<ExtractorCfg>,
}

#[derive(Debug, Deserialize)]
struct ServerCfg {
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
struct ChatCfgYaml {
    kind: String,
    base_url: Option<String>,
    api_key_env: Option<String>,
    api_url: Option<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct EmbedCfgYaml {
    kind: String,
    base_url: Option<String>,
    api_key_env: Option<String>,
    model: String,
    api_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VectorStoreCfg {
    kind: String,
    url: Option<String>,
    collection: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct GenCfg {
    use_rig_agent: Option<bool>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractorCfg {
    url: Option<String>,
    token_env: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    dotenv().ok();

    #[cfg(feature = "pg")]
    match pg_jobstore::ensure_migrated().await {
        Ok(()) => {}
        Err(e) if e.contains("missing DATABASE_URL") => {
            tracing::info!("DATABASE_URL not set; skipping postgres migrations");
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to apply postgres migrations");
        }
    }
    let cfg: AppConfig = load_config()?;

    // 将 extractor 配置注入到环境变量，统一由下游使用
    if let Some(ex) = cfg.extractor.as_ref() {
        if let Some(u) = ex.url.as_ref() {
            if std::env::var("EXTRACT_URL").is_err() {
                std::env::set_var("EXTRACT_URL", u);
            }
        }
        if let Some(token_env) = ex.token_env.as_ref() {
            if let Ok(tok) = std::env::var(token_env) {
                std::env::set_var("EXTRACT_TOKEN", tok);
            }
        }
    }

    // Build providers
    let chat_cfg = match cfg.chat_provider.kind.as_str() {
        "openai_compat" => ChatProviderConfig::OpenAiCompat {
            base_url: cfg
                .chat_provider
                .base_url
                .unwrap_or_else(|| "https://api.openai.com".into()),
            api_key: read_env(
                &cfg.chat_provider
                    .api_key_env
                    .unwrap_or_else(|| "OPENAI_API_KEY".into()),
            )?,
            model: cfg.chat_provider.model,
        },
        "anthropic" => ChatProviderConfig::Anthropic {
            api_url: cfg.chat_provider.api_url,
            api_key: read_env(
                &cfg.chat_provider
                    .api_key_env
                    .unwrap_or_else(|| "ANTHROPIC_API_KEY".into()),
            )?,
            model: cfg.chat_provider.model,
        },
        other => anyhow::bail!("unsupported chat provider kind={}", other),
    };

    let embed_cfg = match cfg.embedding_provider.kind.as_str() {
        "openai_compat" => EmbedProviderConfig::OpenAiCompat {
            base_url: cfg
                .embedding_provider
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".into()),
            api_key: read_env(
                &cfg.embedding_provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "OPENAI_API_KEY".into()),
            )?,
            model: cfg.embedding_provider.model.clone(),
        },
        "qwen" => EmbedProviderConfig::QwenDashScope {
            api_url: cfg.embedding_provider.api_url.clone(),
            api_key: read_env(
                &cfg.embedding_provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "DASHSCOPE_API_KEY".into()),
            )?,
            model: cfg.embedding_provider.model.clone(),
        },
        "deepseek" => EmbedProviderConfig::DeepSeek {
            base_url: cfg.embedding_provider.base_url.clone(),
            api_key: read_env(
                &cfg.embedding_provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "DEEPSEEK_API_KEY".into()),
            )?,
            model: cfg.embedding_provider.model.clone(),
        },
        other => anyhow::bail!("unsupported embedding provider kind={}", other),
    };

    let providers =
        make_providers(chat_cfg, embed_cfg).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // 选择向量检索实现：qdrant -> Rig+Qdrant；memory/rig_mem -> Rig 内存实现；否则为简易多提供商内存实现
    let rag: Arc<dyn RagEngine> = match cfg.vector_store.kind.as_str() {
        "qdrant" => {
            let url = cfg
                .vector_store
                .url
                .unwrap_or_else(|| "http://localhost:6334".into());
            let coll = cfg
                .vector_store
                .collection
                .unwrap_or_else(|| "kb_chunks".into());
            let oai_embed_model = std::env::var("OPENAI_EMBED_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".into());
            let engine = kb_rag::RigQdrantRagEngine::new(
                url,
                coll,
                oai_embed_model,
                Arc::from(providers.chat),
            )
            .await?;
            info!("RigQdrantRagEngine:qdrant_engine");
            Arc::new(engine)
        }
        "memory" | "rig_mem" => {
            let oai_embed_model = std::env::var("OPENAI_EMBED_MODEL")
                .unwrap_or_else(|_| cfg.embedding_provider.model.clone());
            info!("RigInMemoryRagEngine:oai_embed_model={}", oai_embed_model);
            Arc::new(kb_rag::RigInMemoryRagEngine::new(
                oai_embed_model,
                Arc::from(providers.chat),
            ))
        }
        _ => Arc::new(kb_rag::MultiProviderRagEngine::new(
            Arc::from(providers.chat),
            Arc::from(providers.embed),
        )),
    };

    // 初始化认证服务
    let jwt_secret =
        std::env::var("JWT_SECRET").unwrap_or_else(|_| "default_secret_key".to_string());
    let redis_url = std::env::var("REDIS_URL").ok();

    let jwt_service = Arc::new(JwtService::new(&jwt_secret));
    let session_service = Arc::new(SessionService::new(redis_url, Some(24)).unwrap());

    // 简化实现：需要数据库连接池来创建RbacService
    // 这里使用一个虚拟的数据库池
    use sqlx::PgPool;
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgresql://localhost/kb".to_string());
    let db_pool = PgPool::connect(&database_url).await.unwrap_or_else(|_| {
        // 如果连接失败，创建一个虚拟池（仅用于编译）
        panic!("Failed to connect to database");
    });

    let rbac_service = Arc::new(RbacService::new(db_pool));
    let auth_services = auth_routes::AuthServices::new(jwt_service, session_service, rbac_service);

    let state = AppState {
        rag,
        graph: Arc::new(DefaultGraphRagEngine),
        auth_services,
    };

    // Admin routes use AsyncRequireAuthorizationLayer with custom authorizer

    let state_for_router = state.clone();
    let app = Router::new()
        .route("/api/v1/query", post(query))
        .route("/api/v1/query_trace", post(query_trace))
        .route(
            "/api/v1/query/stream",
            post(query_stream).get(query_stream_get),
        )
        .nest(
            "/api/v1/admin",
            Router::new()
                .route("/settings", get(admin_get_settings).put(admin_put_settings))
                .route("/upload", post(admin_upload))
                .route("/jobs", get(admin_jobs_list).post(admin_jobs_create))
                .route("/jobs/:id", get(admin_jobs_get))
                .route("/index/status", get(admin_index_status_all))
                .route("/index/status/:document_id", get(admin_index_status))
                .route("/extract/health", get(admin_extract_health))
                .route("/extract/test", post(admin_extract_test))
                .layer(AsyncRequireAuthorizationLayer::new(
                    AdminAuthorizer::default(),
                )),
        )
        .route("/api/v1/documents/text", post(index_text))
        .route(
            "/api/v1/documents/text_with_meta",
            post(index_text_with_meta),
        )
        .route("/api/v1/documents/pdf_glob", post(index_pdf_glob))
        .route("/api/v1/documents/url", post(index_url))
        .route("/api/v1/session/start", post(session_start))
        .route("/api/v1/session/tool_result", post(session_tool_result))
        .route("/api/v1/session/stream", get(session_stream))
        // .nest(
        //     "/api/v1/auth",
        //     auth_routes::create_auth_routes()
        //         .with_state(state.auth_services.clone()),
        // )
        .route("/api/v1/health", get(health))
        .with_state(state_for_router)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // 后台启动 Job Runner（抽取 + 切分 + 索引）
    {
        let rag_for_jobs = state.rag.clone();
        tokio::spawn(async move { job_runner(rag_for_jobs).await });
    }

    let addr: SocketAddr = format!("{}:{}", cfg.server.host, cfg.server.port)
        .parse()
        .unwrap();
    tracing::info!(%addr, "kb-api listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};
    let fmt_layer = fmt::layer().with_target(false);
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,tower_http=info"))
        .unwrap();
    let subscriber = Registry::default().with(filter).with(fmt_layer);
    tracing::subscriber::set_global_default(subscriber).ok();
}

async fn query(
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Json<QueryResponse> {
    let mode = req.mode.clone().unwrap_or_else(|| "rag".into());
    let mut use_rig_agent = false;
    if mode == "rag" {
        if let Ok(cfg) = load_config() {
            use_rig_agent = cfg
                .generation
                .as_ref()
                .and_then(|g| g.use_rig_agent)
                .unwrap_or(false);
        }
    }

    let resp = if use_rig_agent {
        // Rig Agent 非流式：动态上下文 + 可选工具
        use qdrant_client::Qdrant;
        use rig::{
            client::{CompletionClient, EmbeddingsClient, ProviderClient},
            providers,
        };
        use rig_qdrant::QdrantVectorStore;

        let cfg = load_config().unwrap();
        let url = cfg
            .vector_store
            .url
            .unwrap_or_else(|| "http://localhost:6334".into());
        let coll = cfg
            .vector_store
            .collection
            .unwrap_or_else(|| "kb_chunks".into());
        let chat_model = cfg
            .generation
            .as_ref()
            .and_then(|g| g.model.clone())
            .unwrap_or(cfg.chat_provider.model);
        let embed_model = cfg.embedding_provider.model;
        info!("RigQdrantRagEngine:chat_model={}, embed_model={}", chat_model, embed_model);

        let client = providers::openai::Client::from_env();
        let embed = client.embedding_model(&embed_model);
        let q = Qdrant::from_url(&url).build().unwrap();
        let mut qp = qdrant_client::qdrant::QueryPointsBuilder::new(&coll).with_payload(true);
        if let Some(flt) = build_qdrant_filter(&req.filters) {
            qp = qp.filter(flt);
        }
        let index: QdrantVectorStore<_> = QdrantVectorStore::new(q, embed.clone(), qp.build());

        // 工具示例：TimeNow（返回当前时间）
        #[derive(serde::Deserialize, serde::Serialize)]
        struct TimeNow;
        #[derive(Debug)]
        struct ToolError;
        impl std::fmt::Display for ToolError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "tool error")
            }
        }
        impl std::error::Error for ToolError {}
        impl rig::tool::Tool for TimeNow {
            const NAME: &'static str = "time_now";
            type Error = ToolError;
            type Args = serde_json::Value; // no args
            type Output = String;
            async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
                rig::completion::ToolDefinition {
                    name: "time_now".into(),
                    description: "Return current server time in RFC3339".into(),
                    parameters: json!({"type":"object","properties":{}}),
                }
            }
            async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
                Ok(chrono::Utc::now().to_rfc3339())
            }
        }

        let k = req.top_k.unwrap_or(5) as usize;
        let agent = client
            .agent(&chat_model)
            .preamble("You are a helpful assistant. Answer using only the provided context.")
            .dynamic_context(k, index)
            .tool(TimeNow)
            .build();

        match agent.prompt(&req.query).await {
            Ok(answer) => Ok(QueryResponse {
                answer,
                citations: vec![],
                contexts: vec![],
                mode: mode.clone(),
                latency_ms: 0,
            }),
            Err(e) => Err(KbError::Internal {
                message: e.to_string(),
                details: None,
            }),
        }
    } else {
        match mode.as_str() {
            "graph" => state.graph.query(req).await,
            "hybrid" => state.rag.query(req).await,
            "lexical" | _ => state.rag.query(req).await,
        }
    };
    Json(resp.unwrap_or_else(|e| QueryResponse {
        answer: format!("error: {e}"),
        citations: vec![],
        contexts: vec![],
        mode: mode.to_string(),
        latency_ms: 0,
    }))
}

#[derive(serde::Serialize)]
struct QueryTraceResp {
    answer: String,
    tool_trace: Vec<serde_json::Value>,
    citations: Vec<kb_core::Citation>,
    contexts: Vec<String>,
    mode: String,
    latency_ms: i64,
}

async fn query_trace(
    State(_state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Json<serde_json::Value> {
    use qdrant_client::Qdrant;
    use rig::{
        agent::{MultiTurnStreamItem, PromptHook},
        client::{CompletionClient, EmbeddingsClient, ProviderClient},
        completion::CompletionModel,
        message::Message,
        providers,
        streaming::StreamingPrompt,
    };
    use rig_qdrant::QdrantVectorStore;

    let cfg: AppConfig = load_config().unwrap();
    let url = cfg
        .vector_store
        .url
        .clone()
        .unwrap_or_else(|| "http://localhost:6334".into());
    let coll = cfg
        .vector_store
        .collection
        .clone()
        .unwrap_or_else(|| "kb_chunks".into());
    let chat_model = cfg.chat_provider.model.clone();
    let embed_model = cfg.embedding_provider.model.clone();
    let client = providers::openai::Client::from_env();
    let embed = client.embedding_model(&embed_model);
    let q = Qdrant::from_url(&url).build().unwrap();
    let mut qp = qdrant_client::qdrant::QueryPointsBuilder::new(&coll).with_payload(true);
    if let Some(flt) = build_qdrant_filter(&req.filters) {
        qp = qp.filter(flt);
    }
    let index: QdrantVectorStore<_> = QdrantVectorStore::new(q, embed.clone(), qp.build());
    let agent = client
        .agent(&chat_model)
        .preamble("You are a helpful assistant. Answer using only the provided context.")
        .dynamic_context(req.top_k.unwrap_or(5) as usize, index)
        .build();

    #[derive(Clone)]
    struct TraceHook {
        events: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    }
    impl<M> PromptHook<M> for TraceHook
    where
        M: CompletionModel,
    {
        fn on_tool_call(
            &self,
            tool_name: &str,
            args: &str,
        ) -> impl std::future::Future<Output = ()> + Send {
            let ev = self.events.clone();
            let name = tool_name.to_string();
            let a = args.to_string();
            async move {
                ev.lock()
                    .unwrap()
                    .push(serde_json::json!({"type":"tool_call","name":name,"args":a}));
            }
        }
        fn on_tool_result(
            &self,
            tool_name: &str,
            _args: &str,
            result: &str,
        ) -> impl std::future::Future<Output = ()> + Send {
            let ev = self.events.clone();
            let name = tool_name.to_string();
            let r = result.to_string();
            async move {
                ev.lock()
                    .unwrap()
                    .push(serde_json::json!({"type":"tool_result","name":name,"result":r}));
            }
        }
        fn on_stream_completion_response_finish(
            &self,
            _p: &Message,
            _r: &<M as CompletionModel>::StreamingResponse,
        ) -> impl std::future::Future<Output = ()> + Send {
            async {}
        }
    }
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let hook = TraceHook {
        events: events.clone(),
    };
    let mut stream = agent
        .stream_prompt(&req.query)
        .multi_turn(10)
        .with_hook(hook)
        .await;
    let mut final_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::Text(t)) => {
                final_text.push_str(&t.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(_fr)) => {}
            Err(_) => {}
        }
    }
    let trace = events.lock().unwrap().clone();
    Json(serde_json::json!(QueryTraceResp {
        answer: final_text,
        tool_trace: trace,
        citations: vec![],
        contexts: vec![],
        mode: req.mode.unwrap_or_else(|| "rag".into()),
        latency_ms: 0
    }))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

// Admin Auth 改为使用 RequireAuthorizationLayer::basic（见路由构建处）。

fn load_config() -> anyhow::Result<AppConfig> {
    let s = std::fs::read_to_string("configs/default.yaml")?;
    // info!("load_config: {}", s);
    let cfg: AppConfig = serde_yaml::from_str(&s)?;
    info!("load_config: {:?}", cfg);
    Ok(cfg)
}

fn read_env(key: &str) -> anyhow::Result<String> {
    std::env::var(key).map_err(|_| anyhow::anyhow!("missing env {}", key))
}

#[derive(Deserialize)]
struct IndexTextReq {
    document_id: String,
    text: String,
    page: Option<i32>,
}

async fn index_text(
    State(state): State<AppState>,
    Json(req): Json<IndexTextReq>,
) -> Json<serde_json::Value> {
    let _ = state
        .rag
        .add_document_text(&req.document_id, &req.text, req.page)
        .await;
    inc_index_count(&req.document_id, 1).await;
    Json(serde_json::json!({"status":"ok"}))
}

#[derive(Deserialize)]
struct IndexTextMetaReq {
    document_id: String,
    text: String,
    page: Option<i32>,
    tenant_id: Option<String>,
    source: Option<String>,
    tags: Option<Vec<String>>,
    created_at: Option<i64>,
}

#[debug_handler]
async fn index_text_with_meta(
    State(state): State<AppState>,
    Json(req): Json<IndexTextMetaReq>,
) -> Json<serde_json::Value> {
    let meta = kb_rag::RagMeta {
        tenant_id: req.tenant_id,
        source: req.source,
        tags: req.tags,
        created_at: req.created_at,
        custom_fields: None,
    };
    let _ = state
        .rag
        .add_document_text_with_meta(&req.document_id, &req.text, req.page, Some(meta))
        .await;
    inc_index_count(&req.document_id, 1).await;
    Json(serde_json::json!({"status":"ok"}))
}

#[derive(Deserialize)]
struct IndexPdfGlobReq {
    glob: String,
    prefix: Option<String>,
}

#[debug_handler]
async fn index_pdf_glob(
    State(state): State<AppState>,
    Json(req): Json<IndexPdfGlobReq>,
) -> Json<serde_json::Value> {
    // 若配置了统一抽取服务，则优先走抽取（便于 OCR），否则回退 Rig 的 PDF 解析
    let prefix = req.prefix.unwrap_or_default();
    let mut total_chunks = 0usize;
    if std::env::var("EXTRACT_URL").is_ok() {
        let paths = expand_simple_glob(&req.glob);
        let mut file_idx = 0usize;
        for p in paths {
            if !p.to_ascii_lowercase().ends_with(".pdf") {
                continue;
            }
            let doc_id = format!("{}pdf_{}", prefix, file_idx);
            file_idx += 1;
            match kb_rag::extract_text_via_service(&p).await {
                Ok(text) => {
                    let n = chunk_and_index(state.rag.clone(), &doc_id, &text, 1800, 0).await;
                    inc_index_count(&doc_id, n).await;
                    total_chunks += n;
                }
                Err(_e) => {}
            }
        }
        return Json(serde_json::json!({"status":"ok", "chunks": total_chunks}));
    } else {
        // 回退：使用 Rig PdfFileLoader 加载文本，再本地切分与写入，便于统计计数
        let contents: Vec<String> = match rig::loaders::PdfFileLoader::with_glob(&req.glob) {
            Ok(loader) => loader.read().into_iter().filter_map(|r| r.ok()).collect(),
            Err(_e) => Vec::new(),
        };
        for (i, content) in contents.into_iter().enumerate() {
            let doc_id = format!("{}pdf_{}", prefix, i);
            let n = chunk_and_index(state.rag.clone(), &doc_id, &content, 1800, 0).await;
            inc_index_count(&doc_id, n).await;
            total_chunks += n;
        }
        return Json(serde_json::json!({"status":"ok", "chunks": total_chunks}));
    }
}

// 极简 glob 展开：支持如 /path/*.pdf；不支持递归 **
fn expand_simple_glob(pattern: &str) -> Vec<String> {
    use std::fs;
    use std::path::{Path, PathBuf};
    if !(pattern.contains('*') || pattern.contains('?')) {
        if Path::new(pattern).is_file() {
            return vec![pattern.to_string()];
        }
        return Vec::new();
    }
    let (dir, file_pat) = match pattern.rfind('/') {
        Some(pos) => (&pattern[..pos], &pattern[pos + 1..]),
        None => (".", pattern),
    };
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Ok(ft) = e.file_type() {
                if !ft.is_file() {
                    continue;
                }
            }
            let name = match e.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if wildcard_match(file_pat, &name) {
                let mut p = PathBuf::from(dir);
                p.push(&name);
                if let Some(s) = p.to_str() {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}

// 仅支持 * 通配符（匹配任意长度），其余字面匹配
fn wildcard_match(pat: &str, text: &str) -> bool {
    let mut pi = 0usize;
    let mut ti = 0usize;
    let pb = pat.as_bytes();
    let tb = text.as_bytes();
    let mut star: Option<usize> = None;
    let mut match_i: usize = 0;
    while ti < tb.len() {
        if pi < pb.len() && (pb[pi] == tb[ti]) {
            pi += 1;
            ti += 1;
            continue;
        }
        if pi < pb.len() && pb[pi] == b'*' {
            star = Some(pi);
            match_i = ti;
            pi += 1;
            continue;
        }
        if let Some(si) = star {
            pi = si + 1;
            match_i += 1;
            ti = match_i;
            continue;
        }
        return false;
    }
    while pi < pb.len() && pb[pi] == b'*' {
        pi += 1;
    }
    pi == pb.len()
}

async fn chunk_and_index(
    engine: Arc<dyn RagEngine>,
    document_id: &str,
    text: &str,
    chunk_size: usize,
    overlap: usize,
) -> usize {
    let mut chunks = Vec::<String>::new();
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
    if !current.is_empty() {
        chunks.push(current);
    }
    let mut n = 0usize;
    for c in chunks.into_iter() {
        let _ = engine.add_document_text(document_id, &c, None).await;
        n += 1;
    }
    n
}

#[derive(Deserialize)]
struct IndexUrlReq {
    url: String,
    document_id: String,
}

#[debug_handler]
async fn index_url(
    State(state): State<AppState>,
    Json(req): Json<IndexUrlReq>,
) -> Json<serde_json::Value> {
    let _ = kb_rag::index_web_url(state.rag.clone(), &req.url, &req.document_id).await;
    inc_index_count(&req.document_id, 1).await;
    Json(serde_json::json!({"status":"ok"}))
}

async fn query_stream(
    State(_state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use qdrant_client::Qdrant;
    use rig::{
        agent::{MultiTurnStreamItem, PromptHook},
        client::{CompletionClient, EmbeddingsClient, ProviderClient},
        completion::CompletionModel,
        message::Message,
        providers,
        streaming::StreamingPrompt,
    };
    use rig_qdrant::QdrantVectorStore;

    // Only support Qdrant backed streaming in this iteration
    let k = req.top_k.unwrap_or(5) as u64;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);

    let chat_model = std::env::var("OPENAI_CHAT_MODEL").unwrap_or_else(|_| "gpt-4o".into());
    let embed_model =
        std::env::var("OPENAI_EMBED_MODEL").unwrap_or_else(|_| "text-embedding-3-small".into());

    // Load app config again (cheap) to read vector store details
    let cfg: AppConfig = load_config().expect("config");
    let url = cfg
        .vector_store
        .url
        .unwrap_or_else(|| "http://localhost:6334".into());
    let coll = cfg
        .vector_store
        .collection
        .unwrap_or_else(|| "kb_chunks".into());

    #[derive(Clone)]
    struct SseHook {
        tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    }
    impl<M> PromptHook<M> for SseHook
    where
        M: rig::completion::CompletionModel,
    {
        fn on_tool_call(
            &self,
            tool_name: &str,
            args: &str,
        ) -> impl std::future::Future<Output = ()> + Send {
            let tx = self.tx.clone();
            let data = format!("{} {}", tool_name, args);
            async move {
                let _ = tx
                    .send(Ok(Event::default().event("tool_call").data(data)))
                    .await;
            }
        }
        fn on_tool_result(
            &self,
            tool_name: &str,
            _args: &str,
            result: &str,
        ) -> impl std::future::Future<Output = ()> + Send {
            let tx = self.tx.clone();
            let data = format!("{} {}", tool_name, result);
            async move {
                let _ = tx
                    .send(Ok(Event::default().event("tool_result").data(data)))
                    .await;
            }
        }
        fn on_stream_completion_response_finish(
            &self,
            _prompt: &Message,
            _resp: &<M as CompletionModel>::StreamingResponse,
        ) -> impl std::future::Future<Output = ()> + Send {
            async {}
        }
    }

    tokio::spawn(async move {
        let client = providers::openai::Client::from_env();
        let embed = client.embedding_model(&embed_model);
        let q = Qdrant::from_url(&url).build().unwrap();
        let mut qp = qdrant_client::qdrant::QueryPointsBuilder::new(&coll).with_payload(true);
        if let Some(flt) = build_qdrant_filter(&req.filters) {
            qp = qp.filter(flt);
        }
        let index: QdrantVectorStore<_> = QdrantVectorStore::new(q, embed.clone(), qp.build());

        let agent = client
            .agent(&chat_model)
            .preamble("You are a helpful assistant. Answer using only the provided context.")
            .dynamic_context(k as usize, index)
            .build();

        let hook = SseHook { tx: tx.clone() };
        let mut stream = agent
            .stream_prompt(&req.query)
            .multi_turn(10)
            .with_hook(hook)
            .await;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(MultiTurnStreamItem::Text(t)) => {
                    let _ = tx
                        .send(Ok(Event::default().event("text").data(t.text)))
                        .await;
                }
                Ok(MultiTurnStreamItem::FinalResponse(fr)) => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("final")
                            .data(fr.response().to_string())))
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(e.to_string())))
                        .await;
                    break;
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
}

// 兼容 GET SSE：通过查询参数获取 query/top_k
#[derive(Deserialize)]
struct StreamQuery {
    query: String,
    top_k: Option<u64>,
}

async fn query_stream_get(
    State(state): State<AppState>,
    Query(q): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let req = QueryRequest {
        query: q.query,
        mode: Some("rag".into()),
        top_k: q.top_k.map(|v| v as u16),
        rerank: None,
        filters: None,
        stream: Some(true),
        include_raw_matches: None,
    };
    query_stream(State(state), Json(req)).await
}

#[derive(Deserialize)]
struct SessionStartReq {
    query: String,
    top_k: Option<u16>,
    filters: Option<serde_json::Value>,
}

async fn session_start(Json(req): Json<SessionStartReq>) -> Json<serde_json::Value> {
    let sid = Uuid::new_v4();
    let st = SessionState {
        query: req.query,
        top_k: req.top_k.unwrap_or(5) as usize,
        filters: req.filters,
        chat_history: vec![],
        pending_tool: None,
    };
    save_session(sid, &st).await;
    Json(json!({"session_id": sid}))
}

#[derive(Deserialize)]
struct SessionToolResultReq {
    session_id: Uuid,
    result: String,
}

async fn session_tool_result(Json(req): Json<SessionToolResultReq>) -> Json<serde_json::Value> {
    use rig::message::{Message, ToolResultContent, UserContent};
    use rig::OneOrMany;
    if let Some(mut st) = load_session(req.session_id).await {
        if let Some(p) = st.pending_tool.clone() {
            // 注入 tool_result
            let content = OneOrMany::one(ToolResultContent::text(&req.result));
            let msg = if let Some(call_id) = p.call_id.clone() {
                Message::User {
                    content: OneOrMany::one(UserContent::tool_result_with_call_id(
                        &p.id, call_id, content,
                    )),
                }
            } else {
                Message::User {
                    content: OneOrMany::one(UserContent::tool_result(&p.id, content)),
                }
            };
            st.chat_history.push(msg);
            st.pending_tool = None;
            save_session(req.session_id, &st).await;
            return Json(json!({"status":"ok"}));
        }
        return Json(json!({"status":"no_pending_tool"}));
    }
    Json(json!({"status":"not_found"}))
}

#[derive(Deserialize)]
struct SessionStreamQuery {
    session_id: Uuid,
}

async fn session_stream(
    Query(q): Query<SessionStreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use qdrant_client::Qdrant;
    use rig::{
        client::{EmbeddingsClient, ProviderClient},
        providers,
        streaming::StreamingCompletion,
    };
    use rig_qdrant::QdrantVectorStore;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
    let sid = q.session_id;
    tokio::spawn(async move {
        // 读取会话
        let st_opt = { load_session(sid).await };
        if st_opt.is_none() {
            let _ = tx
                .send(Ok(Event::default().event("error").data("not_found")))
                .await;
            return;
        }
        let st = st_opt.unwrap();

        // 构建 agent + index
        let cfg: AppConfig = match load_config() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(e.to_string())))
                    .await;
                return;
            }
        };
        let url = cfg
            .vector_store
            .url
            .unwrap_or_else(|| "http://localhost:6334".into());
        let coll = cfg
            .vector_store
            .collection
            .unwrap_or_else(|| "kb_chunks".into());
        let chat_model = cfg.chat_provider.model;
        let embed_model = cfg.embedding_provider.model;
        let client = providers::openai::Client::from_env();
        let embed = client.embedding_model(&embed_model);
        let qd = match Qdrant::from_url(&url).build() {
            Ok(x) => x,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(e.to_string())))
                    .await;
                return;
            }
        };
        let mut qp = qdrant_client::qdrant::QueryPointsBuilder::new(&coll).with_payload(true);
        if let Some(f) = build_qdrant_filter(&st.filters) {
            qp = qp.filter(f);
        }
        let index: QdrantVectorStore<_> = QdrantVectorStore::new(qd, embed.clone(), qp.build());
        // 定义一个示例工具供客户端驱动闭环演示（模型可决定是否调用）
        #[derive(serde::Deserialize, serde::Serialize)]
        struct TimeNow;
        #[derive(Debug)]
        struct ToolError;
        impl std::fmt::Display for ToolError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "tool error")
            }
        }
        impl std::error::Error for ToolError {}
        impl rig::tool::Tool for TimeNow {
            const NAME: &'static str = "time_now";
            type Error = ToolError;
            type Args = serde_json::Value; // 无参数
            type Output = String;
            async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
                rig::completion::ToolDefinition {
                    name: "time_now".into(),
                    description: "Return current server time in RFC3339".into(),
                    parameters: json!({"type":"object","properties":{}}),
                }
            }
            async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
                Ok(chrono::Utc::now().to_rfc3339())
            }
        }

        let agent = client.agent(&chat_model)
            .preamble("You are a helpful assistant. Answer using only the provided context. Tools may be available.")
            .dynamic_context(st.top_k, index)
            .tool(TimeNow)
            .build();

        // 低层流式：不自动执行工具；遇到 tool_call 挂起，等待客户端回传 /session/tool_result
        let builder = match agent
            .stream_completion(&st.query, st.chat_history.clone())
            .await
        {
            Ok(b) => b,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(e.to_string())))
                    .await;
                return;
            }
        };
        let mut stream = match builder.stream().await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(e.to_string())))
                    .await;
                return;
            }
        };
        while let Some(chunk) = stream.next().await {
            use rig::streaming::StreamedAssistantContent;
            match chunk {
                Ok(StreamedAssistantContent::Text(t)) => {
                    let _ = tx
                        .send(Ok(Event::default().event("text").data(t.text)))
                        .await;
                }
                Ok(StreamedAssistantContent::Reasoning(r)) => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("reasoning")
                            .data(r.reasoning.into_iter().collect::<String>())))
                        .await;
                }
                Ok(StreamedAssistantContent::ToolCall(tc)) => {
                    // 写入会话：Assistant ToolCall，并挂起等待 tool_result
                    use rig::message::{AssistantContent, Message};
                    use rig::OneOrMany;
                    if let Some(mut s) = load_session(sid).await {
                        s.chat_history.push(Message::Assistant {
                            id: None,
                            content: OneOrMany::one(AssistantContent::ToolCall(tc.clone())),
                        });
                        s.pending_tool = Some(PendingTool {
                            id: tc.id.clone(),
                            call_id: tc.call_id.clone(),
                            name: tc.function.name.clone(),
                        });
                        save_session(sid, &s).await;
                    }
                    let _ = tx
                        .send(Ok(Event::default().event("tool_call").data(format!(
                            "{} {}",
                            tc.function.name, tc.function.arguments
                        ))))
                        .await;
                    break;
                }
                Ok(StreamedAssistantContent::Final(final_msg)) => {
                    let json = serde_json::to_string(&final_msg).unwrap_or_default();
                    let _ = tx
                        .send(Ok(Event::default().event("final").data(json)))
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(e.to_string())))
                        .await;
                    break;
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(rx))
}

fn build_qdrant_filter(
    filters: &Option<serde_json::Value>,
) -> Option<qdrant_client::qdrant::Filter> {
    use qdrant_client::qdrant::r#match::MatchValue;
    use qdrant_client::qdrant::{Condition, Filter, Range};
    let f = filters.as_ref()?;
    let mut must: Vec<Condition> = Vec::new();
    if let Some(doc_id) = f.get("document_id").and_then(|v| v.as_str()) {
        must.push(Condition::matches(
            "document_id",
            MatchValue::from(doc_id.to_string()),
        ));
    }
    if let Some(tenant) = f.get("tenant_id").and_then(|v| v.as_str()) {
        must.push(Condition::matches(
            "tenant_id",
            MatchValue::from(tenant.to_string()),
        ));
    }
    if let Some(source) = f.get("source").and_then(|v| v.as_str()) {
        must.push(Condition::matches(
            "source",
            MatchValue::from(source.to_string()),
        ));
    }
    if let Some(tags) = f.get("tags").and_then(|v| v.as_array()) {
        let tag_strings: Vec<String> = tags
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        if !tag_strings.is_empty() {
            must.push(Condition::matches(
                "tags",
                qdrant_client::qdrant::r#match::MatchValue::from(tag_strings),
            ));
        }
    }
    if let Some(gte) = f.get("start_time").and_then(|v| v.as_f64()) {
        must.push(Condition::range(
            "created_at",
            Range {
                gte: Some(gte),
                ..Default::default()
            },
        ));
    }
    if let Some(lte) = f.get("end_time").and_then(|v| v.as_f64()) {
        must.push(Condition::range(
            "created_at",
            Range {
                lte: Some(lte),
                ..Default::default()
            },
        ));
    }
    if must.is_empty() {
        None
    } else {
        Some(Filter::must(must))
    }
}

async fn load_session(id: Uuid) -> Option<SessionState> {
    if let Some(url) = &*REDIS_URL {
        if let Ok(client) = redis::Client::open(url.as_str()) {
            if let Ok(mut conn) = client.get_async_connection().await {
                let key = format!("session:{}", id);
                if let Ok(val) = redis::Cmd::get(&key)
                    .query_async::<_, Option<String>>(&mut conn)
                    .await
                {
                    if let Some(s) = val {
                        if let Ok(st) = serde_json::from_str::<SessionState>(&s) {
                            return Some(st);
                        }
                    }
                }
            }
        }
    }
    SESSIONS.read().await.get(&id).cloned()
}

async fn save_session(id: Uuid, st: &SessionState) {
    if let Some(url) = &*REDIS_URL {
        if let Ok(client) = redis::Client::open(url.as_str()) {
            if let Ok(mut conn) = client.get_async_connection().await {
                let key = format!("session:{}", id);
                let val = serde_json::to_string(st).unwrap_or_default();
                let _: Result<(), _> = redis::Cmd::set(&key, val).query_async(&mut conn).await;
                let _: Result<(), _> = redis::Cmd::expire(&key, *SESS_TTL_SECS as usize)
                    .query_async(&mut conn)
                    .await;
                return;
            }
        }
    }
    SESSIONS.write().await.insert(id, st.clone());
}

// ===============
// Admin: Settings
// ===============

// use axum::http::HeaderMap; // consolidated import at top

fn admin_auth_ok(headers: &HeaderMap) -> bool {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let user = std::env::var("ADMIN_USER").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("ADMIN_PASS").unwrap_or_else(|_| "admin".into());
    let static_token = std::env::var("ADMIN_BEARER").ok();

    // Basic
    if auth.starts_with("Basic ") {
        let b64 = &auth[6..];
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::STANDARD;
        if let Ok(decoded) = engine.decode(b64) {
            if let Ok(s) = String::from_utf8(decoded) {
                if s == format!("{}:{}", user, pass) {
                    return true;
                }
            }
        }
        return false;
    }

    // Bearer
    if let Some(token) = auth.strip_prefix("Bearer ") {
        // 1) static bearer
        if let Some(st) = static_token.as_ref() {
            if token == st {
                return true;
            }
        }
        // 2) permissive JWT (claims-only) if enabled
        if std::env::var("ADMIN_JWT_ALLOW_UNVERIFIED")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            return jwt_claims_check(token).unwrap_or(false);
        }
        return false;
    }
    false
}

fn jwt_claims_check(token: &str) -> anyhow::Result<bool> {
    // Very light-weight, no-signature JWT validation: decode payload, check exp/iss/aud.
    // Not for production unless behind a verifying proxy. Enable with ADMIN_JWT_ALLOW_UNVERIFIED=true.
    let mut parts = token.split('.');
    let _header_b64 = parts.next().ok_or_else(|| anyhow::anyhow!("jwt header"))?;
    let payload_b64 = parts.next().ok_or_else(|| anyhow::anyhow!("jwt payload"))?;
    let _sig = parts.next();
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload_bytes = engine
        .decode(payload_b64)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let v: serde_json::Value = serde_json::from_slice(&payload_bytes)?;
    if let Some(exp) = v.get("exp").and_then(|x| x.as_i64()) {
        let now = chrono::Utc::now().timestamp();
        if now >= exp {
            return Ok(false);
        }
    }
    if let Ok(iss) = std::env::var("ADMIN_OIDC_ISSUER") {
        if v.get("iss").and_then(|x| x.as_str()) != Some(iss.as_str()) {
            return Ok(false);
        }
    }
    if let Ok(aud) = std::env::var("ADMIN_OIDC_AUDIENCE") {
        // aud can be string or array
        let ok = match v.get("aud") {
            Some(serde_json::Value::String(s)) => s == &aud,
            Some(serde_json::Value::Array(arr)) => {
                arr.iter().any(|e| e.as_str() == Some(aud.as_str()))
            }
            _ => false,
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn admin_get_settings(headers: HeaderMap) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    let map = SETTINGS.read().await.clone();
    // 掩码敏感字段
    let mut masked = serde_json::Map::new();
    for (k, v) in map.iter() {
        let is_secret = k.contains("KEY") || k.contains("TOKEN") || k.contains("PASS");
        if is_secret {
            let s = v.clone();
            let tail = s
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            masked.insert(
                k.clone(),
                serde_json::Value::String(format!("****{}", tail)),
            );
        } else {
            masked.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
    }
    Json(json!({ "settings": masked }))
}

#[derive(Deserialize)]
struct SettingsInput(serde_json::Value);

async fn admin_put_settings(
    headers: HeaderMap,
    Json(SettingsInput(v)): Json<SettingsInput>,
) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    let mut map = SETTINGS.write().await;
    if let Some(obj) = v.as_object() {
        for (k, val) in obj.iter() {
            if let Some(s) = val.as_str() {
                if is_secret_key(k) {
                    // 仅存储掩码值，避免在内存存明文；真实值放入 env
                    let tail: String = s
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    map.insert(k.clone(), format!("****{}", tail));
                } else {
                    map.insert(k.clone(), s.to_string());
                }
                // 同步到进程 env，使后续请求读取最新值（如 LLM/API Keys）
                std::env::set_var(k, s);
            }
        }
    }
    // 返回掩码后的设置
    drop(map);
    admin_get_settings(headers).await
}

// ===============
// Admin: Upload
// ===============

use axum::extract::Multipart;

async fn admin_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    use tokio::io::AsyncWriteExt;
    let mut document_id = String::new();
    let mut chunk_size: usize = 1800;
    let mut overlap: usize = 0;
    let mut saved_path: Option<String> = None;
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        if name == "document_id" {
            document_id = field.text().await.unwrap_or_default();
        } else if name == "chunk_size" {
            chunk_size = field
                .text()
                .await
                .unwrap_or_default()
                .parse()
                .unwrap_or(1800);
        } else if name == "overlap" {
            overlap = field.text().await.unwrap_or_default().parse().unwrap_or(0);
        } else if name == "file" {
            let filename = field
                .file_name()
                .map(|s| s.to_string())
                .unwrap_or("upload.bin".into());
            let data = field.bytes().await.unwrap_or_default();
            let tmp = format!("/tmp/{}", filename);
            let mut f = tokio::fs::File::create(&tmp).await.unwrap();
            let _ = f.write_all(&data).await;
            saved_path = Some(tmp);
        }
    }
    if document_id.is_empty() {
        return Json(json!({"error":"missing document_id"}));
    }
    if let Some(path) = saved_path {
        let lower = path.to_lowercase();
        if lower.ends_with(".pdf") {
            // 优先使用统一抽取服务（支持 OCR），否则回退 PDF 文本解析
            let text = if std::env::var("EXTRACT_URL").is_ok() {
                match kb_rag::extract_text_via_service(&path).await {
                    Ok(t) => t,
                    Err(e) => return Json(json!({"error": format!("extract failed: {}", e)})),
                }
            } else {
                // 回退：Rig PDF 加载器直接读取该文件（取第一份结果）
                match rig::loaders::PdfFileLoader::with_glob(&path) {
                    Ok(loader) => {
                        let mut it = loader.read().into_iter();
                        match it.next() {
                            Some(Ok(s)) => s,
                            Some(Err(_)) => String::new(),
                            None => String::new(),
                        }
                    }
                    Err(e) => return Json(json!({"error": format!("pdf parse failed: {}", e)})),
                }
            };
            // 本地分片并写入引擎
            let mut chunks: Vec<String> = Vec::new();
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
            if !current.is_empty() {
                chunks.push(current);
            }
            let mut n: usize = 0;
            for c in chunks.into_iter() {
                let _ = state.rag.add_document_text(&document_id, &c, None).await;
                n += 1;
            }
            inc_index_count(&document_id, n).await;
            return Json(json!({"status":"ok","chunks": n}));
        } else {
            // 文档解析分支：html/htm -> 转纯文本；md -> 粗略去标记；doc/docx/ppt/pptx/xls/xlsx/rtf/epub/odt -> 调用统一抽取服务（EXTRACT_URL）
            let text = if lower.ends_with(".html") || lower.ends_with(".htm") {
                let content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                html2text::from_read(content.as_bytes(), 80)
            } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
                let content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                markdown_to_text(&content)
            } else if [
                ".doc", ".docx", ".ppt", ".pptx", ".xls", ".xlsx", ".rtf", ".epub", ".odt",
            ]
            .iter()
            .any(|s| lower.ends_with(s))
            {
                match kb_rag::extract_text_via_service(&path).await {
                    Ok(t) => t,
                    Err(e) => return Json(json!({"error": format!("extract failed: {}", e)})),
                }
            } else {
                // 其它未知类型：若配置了 EXTRACT_URL，尝试统一抽取；否则按文本读取
                if std::env::var("EXTRACT_URL").is_ok() {
                    match kb_rag::extract_text_via_service(&path).await {
                        Ok(t) => t,
                        Err(_) => tokio::fs::read_to_string(&path).await.unwrap_or_default(),
                    }
                } else {
                    tokio::fs::read_to_string(&path).await.unwrap_or_default()
                }
            };
            // 本地分片并写入引擎
            let mut chunks: Vec<String> = Vec::new();
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
            if !current.is_empty() {
                chunks.push(current);
            }
            let mut n: usize = 0;
            for c in chunks.into_iter() {
                let _ = state.rag.add_document_text(&document_id, &c, None).await;
                n += 1;
            }
            inc_index_count(&document_id, n).await;
            return Json(json!({"status":"ok","chunks": n}));
        }
    }
    Json(json!({"error":"missing file"}))
}

fn markdown_to_text(input: &str) -> String {
    // 极简 Markdown 去标记：移除 #*`_[]() 等标记，保留文字与换行
    let mut out = String::with_capacity(input.len());
    let mut in_code_block = false;
    for line in input.lines() {
        let mut l = line.to_string();
        if l.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        // 头部 #
        while l.starts_with('#') {
            l.remove(0);
        }
        // 简单替换标记字符
        let mut s = l.replace('`', "").replace('*', "").replace('_', "");
        // 简单处理链接: [text](url) -> text
        let mut tmp = String::new();
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '!' {
                continue;
            }
            if ch == '[' {
                // collect until ']'
                let mut label = String::new();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == ']' {
                        break;
                    }
                    label.push(c);
                }
                // skip optional (url)
                if let Some(&'(') = chars.peek() {
                    // consume '('
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c == ')' {
                            break;
                        }
                    }
                }
                if !label.is_empty() {
                    tmp.push_str(&label);
                }
            } else {
                tmp.push(ch);
            }
        }
        s = tmp;
        out.push_str(s.trim());
        out.push('\n');
    }
    out
}

// ===============
// Admin: Jobs
// ===============

async fn admin_jobs_list(headers: HeaderMap) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    let m = JOBS.read().await;
    let mut list: Vec<Job> = m.values().cloned().collect();
    list.sort_by_key(|j| j.created_at);
    Json(json!({"jobs": list}))
}

#[derive(Deserialize)]
struct JobCreate {
    kind: String,
    payload: serde_json::Value,
    idempotency_key: Option<String>,
}

async fn admin_jobs_create(
    headers: HeaderMap,
    Json(req): Json<JobCreate>,
) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    // 幂等：若携带 idempotency_key 且已存在，直接返回现有 job_id
    if let Some(key) = req.idempotency_key.as_ref() {
        if let Some(existing) = IDEMPOTENCY.read().await.get(key).cloned() {
            return Json(json!({"status":"ok","job_id": existing, "idempotent": true}));
        }
    }
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().timestamp();
    let job = Job {
        id,
        kind: req.kind.clone(),
        payload: req.payload.clone(),
        status: "pending".into(),
        message: None,
        created_at: now,
        updated_at: now,
        attempts: 0,
        idempotency_key: req.idempotency_key.clone(),
        progress: None,
        resume: None,
    };
    {
        let mut j = JOBS.write().await;
        j.insert(id, job);
        let mut q = JOB_QUEUE.write().await;
        q.push(id);
        if let Some(k) = req.idempotency_key {
            IDEMPOTENCY.write().await.insert(k, id);
        }
        JOB_STORE.save_all(&*j, &*q, &*IDEMPOTENCY.read().await);
        // 持久化到 Postgres（如开启特性）
        pg_jobstore::upsert_job(j.get(&id).unwrap()).await;
        pg_jobstore::enqueue(id).await;
        if let Some(key) =
            IDEMPOTENCY
                .read()
                .await
                .iter()
                .find_map(|(k, v)| if *v == id { Some(k.clone()) } else { None })
        {
            pg_jobstore::upsert_idem(&key, id).await;
        }
    }
    Json(json!({"status":"ok","job_id": id}))
}

use axum::extract::Path;
async fn admin_jobs_get(headers: HeaderMap, Path(id): Path<Uuid>) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    if let Some(job) = JOBS.read().await.get(&id).cloned() {
        return Json(json!({"job": job}));
    }
    Json(json!({"error":"not_found"}))
}

async fn admin_index_status(
    headers: HeaderMap,
    Path(doc): Path<String>,
) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    let m = INDEX_COUNTS.read().await;
    let n = m.get(&doc).cloned().unwrap_or(0);
    Json(json!({"document_id": doc, "chunks": n}))
}

async fn admin_index_status_all(headers: HeaderMap) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    let m = INDEX_COUNTS.read().await;
    Json(json!({"status": "ok", "index_counts": &*m }))
}

async fn admin_extract_health(headers: HeaderMap) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    match kb_rag::extract_service_health().await {
        Ok(()) => Json(json!({"status":"ok"})),
        Err(e) => Json(json!({"status":"error","message": e.to_string()})),
    }
}

#[derive(Deserialize)]
struct ExtractTestReq {
    text: Option<String>,
    filename: Option<String>,
}

async fn admin_extract_test(
    headers: HeaderMap,
    Json(req): Json<ExtractTestReq>,
) -> Json<serde_json::Value> {
    if !admin_auth_ok(&headers) {
        return Json(json!({"error":"unauthorized"}));
    }
    let url = match std::env::var("EXTRACT_URL") {
        Ok(u) => u,
        Err(_) => return Json(json!({"error":"EXTRACT_URL not set"})),
    };
    let filename = req.filename.unwrap_or_else(|| "test.txt".into());
    let data = req
        .text
        .unwrap_or_else(|| "hello extractor".into())
        .into_bytes();
    match kb_rag::extract_text_via_service_bytes(&filename, &data, &url).await {
        Ok(text) => {
            Json(json!({"status":"ok","text": text.chars().take(2000).collect::<String>()}))
        }
        Err(e) => Json(json!({"status":"error","message": e.to_string()})),
    }
}

async fn job_runner(rag: Arc<dyn RagEngine>) {
    // 启动时从磁盘载入上次的 JOB 状态
    {
        // 优先从 Postgres 恢复（如启用 pg 特性且配置了 DATABASE_URL），否则回退 sled
        if let Some((jobs, q, idem)) = pg_jobstore::load_all().await {
            if !jobs.is_empty() || !q.is_empty() || !idem.is_empty() {
                let mut jm = JOBS.write().await;
                *jm = jobs;
                let mut qq = JOB_QUEUE.write().await;
                *qq = q;
                let mut im = IDEMPOTENCY.write().await;
                *im = idem;
                tracing::info!(
                    jobs = jm.len(),
                    queue = qq.len(),
                    "restored jobs from postgres"
                );
            }
        } else {
            let (jobs, q, idem) = JOB_STORE.load_all();
            if !jobs.is_empty() || !q.is_empty() || !idem.is_empty() {
                let mut jm = JOBS.write().await;
                *jm = jobs;
                let mut qq = JOB_QUEUE.write().await;
                *qq = q;
                let mut im = IDEMPOTENCY.write().await;
                *im = idem;
                tracing::info!(jobs = jm.len(), queue = qq.len(), "restored jobs from sled");
            }
        }
        // 将所有 running 状态复位为 pending，便于断点续跑
        {
            let mut jm = JOBS.write().await;
            for (_id, j) in jm.iter_mut() {
                if j.status == "running" {
                    j.status = "pending".into();
                }
            }
            JOB_STORE.save_all(&*jm, &*JOB_QUEUE.read().await, &*IDEMPOTENCY.read().await);
        }
    }
    loop {
        let maybe_id = { JOB_QUEUE.write().await.pop() };
        if let Some(id) = maybe_id {
            // 从 PG 队列移除（若启用）
            pg_jobstore::dequeue(id).await;
            tracing::info!(job_id=%id, "job picked from queue");
            if let Some(job) = JOBS.write().await.get_mut(&id) {
                job.status = "running".into();
                job.updated_at = chrono::Utc::now().timestamp();
            }
            {
                JOB_STORE.save_all(
                    &*JOBS.read().await,
                    &*JOB_QUEUE.read().await,
                    &*IDEMPOTENCY.read().await,
                );
                if let Some(j) = JOBS.read().await.get(&id) {
                    pg_jobstore::upsert_job(j).await;
                }
            }
            if let Err(e) = run_job_with_retries(rag.clone(), id).await {
                tracing::error!(job_id=%id, error=%e, "job failed");
                if let Some(job) = JOBS.write().await.get_mut(&id) {
                    job.status = "error".into();
                    job.message = Some(e);
                    job.updated_at = chrono::Utc::now().timestamp();
                }
                JOB_STORE.save_all(
                    &*JOBS.read().await,
                    &*JOB_QUEUE.read().await,
                    &*IDEMPOTENCY.read().await,
                );
                if let Some(j) = JOBS.read().await.get(&id) {
                    pg_jobstore::upsert_job(j).await;
                }
            } else if let Some(job) = JOBS.write().await.get_mut(&id) {
                job.status = "done".into();
                job.updated_at = chrono::Utc::now().timestamp();
                tracing::info!(job_id=%id, "job done");
                JOB_STORE.save_all(
                    &*JOBS.read().await,
                    &*JOB_QUEUE.read().await,
                    &*IDEMPOTENCY.read().await,
                );
                if let Some(j) = JOBS.read().await.get(&id) {
                    pg_jobstore::upsert_job(j).await;
                }
            }
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}

async fn run_job_with_retries(rag: Arc<dyn RagEngine>, id: Uuid) -> Result<(), String> {
    let max_retries: usize = std::env::var("JOB_MAX_RETRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let mut backoff_ms: u64 = std::env::var("JOB_RETRY_BASE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    let mut attempt = 0usize;
    loop {
        attempt += 1;
        if let Some(job) = JOBS.write().await.get_mut(&id) {
            job.attempts = attempt as u32;
            job.message = Some(format!("attempt {}", attempt));
            job.updated_at = chrono::Utc::now().timestamp();
        }
        match run_job_once(rag.clone(), id).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(job_id=%id, attempt, error=%e, "job attempt failed");
                if attempt <= max_retries {
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms.saturating_mul(2);
                    continue;
                }
                return Err(e);
            }
        }
    }
}

async fn run_job_once(rag: Arc<dyn RagEngine>, id: Uuid) -> Result<(), String> {
    let job_opt = { JOBS.read().await.get(&id).cloned() };
    let job = job_opt.ok_or_else(|| "job not found".to_string())?;
    match job.kind.as_str() {
        "url" => {
            let url = job
                .payload
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("missing url")?;
            let doc = job
                .payload
                .get("document_id")
                .and_then(|v| v.as_str())
                .ok_or("missing document_id")?;
            let timeout_ms: u64 = std::env::var("JOB_URL_TIMEOUT_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10000);
            let body = reqwest::Client::new()
                .get(url)
                .timeout(std::time::Duration::from_millis(timeout_ms))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .text()
                .await
                .map_err(|e| e.to_string())?;
            let text = html2text::from_read(body.as_bytes(), 80);
            let n = chunk_and_index(rag.clone(), doc, &text, 1800, 0).await;
            inc_index_count(doc, n).await;
            tracing::info!(job_id=%id, kind="url", url=%url, chunks=n, "indexed from url");
            Ok(())
        }
        "pdf_glob" => {
            let glob = job
                .payload
                .get("glob")
                .and_then(|v| v.as_str())
                .ok_or("missing glob")?;
            let prefix = job
                .payload
                .get("prefix")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let chunk_size = job
                .payload
                .get("chunk_size")
                .and_then(|v| v.as_u64())
                .unwrap_or(1800) as usize;
            let overlap = job
                .payload
                .get("overlap")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let mut total = 0usize;
            if std::env::var("EXTRACT_URL").is_ok() {
                // 断点续跑：若存在 resume，则用其中的 paths/next；否则初始化
                let (paths, mut idx) = {
                    if let Some(JobResume::PdfGlob { paths, next, .. }) = job.resume.clone() {
                        (paths, next)
                    } else {
                        (expand_simple_glob(glob), 0usize)
                    }
                };
                let total_files = paths.len();
                {
                    if let Some(j) = JOBS.write().await.get_mut(&id) {
                        j.progress = Some(JobProgress {
                            total: Some(total_files),
                            completed: idx,
                            current: None,
                        });
                        j.resume = Some(JobResume::PdfGlob {
                            paths: paths.clone(),
                            next: idx,
                            prefix: prefix.to_string(),
                            chunk_size,
                            overlap,
                        });
                        JOB_STORE.save_all(
                            &*JOBS.read().await,
                            &*JOB_QUEUE.read().await,
                            &*IDEMPOTENCY.read().await,
                        );
                    }
                }
                for p in paths.into_iter().skip(idx) {
                    if !p.to_ascii_lowercase().ends_with(".pdf") {
                        continue;
                    }
                    let doc_id = format!("{}pdf_{}", prefix, idx);
                    idx += 1;
                    match kb_rag::extract_text_via_service(&p).await {
                        Ok(text) => {
                            let n =
                                chunk_and_index(rag.clone(), &doc_id, &text, chunk_size, overlap)
                                    .await;
                            inc_index_count(&doc_id, n).await;
                            total += n;
                            if let Some(j) = JOBS.write().await.get_mut(&id) {
                                if let Some(ref mut pr) = j.progress {
                                    pr.completed = idx;
                                    pr.current = Some(doc_id.clone());
                                }
                                j.resume = Some(JobResume::PdfGlob {
                                    paths: vec![],
                                    next: idx,
                                    prefix: prefix.to_string(),
                                    chunk_size,
                                    overlap,
                                });
                                j.updated_at = chrono::Utc::now().timestamp();
                                JOB_STORE.save_all(
                                    &*JOBS.read().await,
                                    &*JOB_QUEUE.read().await,
                                    &*IDEMPOTENCY.read().await,
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(job_id=%id, file=%p, error=%e, "extract failed");
                        }
                    }
                }
            } else {
                let contents: Vec<String> = match rig::loaders::PdfFileLoader::with_glob(glob) {
                    Ok(loader) => loader.read().into_iter().filter_map(|r| r.ok()).collect(),
                    Err(_) => Vec::new(),
                };
                for (i, content) in contents.into_iter().enumerate() {
                    let doc_id = format!("{}pdf_{}", prefix, i);
                    let n =
                        chunk_and_index(rag.clone(), &doc_id, &content, chunk_size, overlap).await;
                    inc_index_count(&doc_id, n).await;
                    total += n;
                }
            }
            tracing::info!(job_id=%id, kind="pdf_glob", total_chunks=total, "indexed from pdf_glob");
            Ok(())
        }
        "file" => {
            use std::path::Path;
            let path = job
                .payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("missing path")?;
            let doc = job
                .payload
                .get("document_id")
                .and_then(|v| v.as_str())
                .ok_or("missing document_id")?;
            let chunk_size = job
                .payload
                .get("chunk_size")
                .and_then(|v| v.as_u64())
                .unwrap_or(1800) as usize;
            let overlap = job
                .payload
                .get("overlap")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let filename = Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("upload.bin");
            let data = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
            let n = index_bytes(rag.clone(), doc, filename, &data, chunk_size, overlap).await?;
            inc_index_count(doc, n).await;
            tracing::info!(job_id=%id, kind="file", path=%path, chunks=n, "indexed from file");
            Ok(())
        }
        "object_url" => {
            let doc = job
                .payload
                .get("document_id")
                .and_then(|v| v.as_str())
                .ok_or("missing document_id")?;
            let (data, filename) = fetch_object_bytes(&job.payload).await?;
            let chunk_size = job
                .payload
                .get("chunk_size")
                .and_then(|v| v.as_u64())
                .unwrap_or(1800) as usize;
            let overlap = job
                .payload
                .get("overlap")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let n = index_bytes(rag.clone(), doc, &filename, &data, chunk_size, overlap).await?;
            inc_index_count(doc, n).await;
            tracing::info!(job_id=%id, kind="object_url", chunks=n, url=?job.payload.get("url"), "indexed from object_url");
            Ok(())
        }
        "s3" | "oss" => {
            let doc = job
                .payload
                .get("document_id")
                .and_then(|v| v.as_str())
                .ok_or("missing document_id")?;
            let (data, filename) = fetch_object_bytes(&job.payload).await?; // 兼容 presigned_url/url/s3_url/oss_url
            let chunk_size = job
                .payload
                .get("chunk_size")
                .and_then(|v| v.as_u64())
                .unwrap_or(1800) as usize;
            let overlap = job
                .payload
                .get("overlap")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let n = index_bytes(rag.clone(), doc, &filename, &data, chunk_size, overlap).await?;
            inc_index_count(doc, n).await;
            tracing::info!(job_id=%id, kind=%job.kind, chunks=n, "indexed from object storage");
            Ok(())
        }
        _ => Err("unsupported job kind".into()),
    }
}

async fn fetch_object_bytes(payload: &serde_json::Value) -> Result<(Vec<u8>, String), String> {
    // 支持字段（择一）：presigned_url | url | s3_url (s3://bucket/key) | oss_url (oss://bucket/key)
    // 可选 headers: { "Authorization": "Bearer ..." }（仅对 url/presigned_url 生效）
    if let Some(u) = payload
        .get("presigned_url")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("url").and_then(|v| v.as_str()))
    {
        let headers = payload.get("headers").cloned();
        let bytes = http_fetch_bytes(u, headers.as_ref()).await?;
        let filename = payload
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| infer_filename_from_url(u));
        return Ok((bytes, filename.to_string()));
    }
    if let Some(s3u) = payload
        .get("s3_url")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("oss_url").and_then(|v| v.as_str()))
    {
        // 若提供了公共基地址（如 MinIO 网关）则拼接访问，否则要求提供 presigned_url
        let base = std::env::var("OBJECT_PUBLIC_BASE_URL").ok();
        if let Some(base) = base {
            if let Some((bucket, key)) = parse_object_url(s3u) {
                let url = format!("{}/{}/{}", base.trim_end_matches('/'), bucket, key);
                let bytes = http_fetch_bytes(&url, None).await?;
                let filename = payload
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| key.split('/').last().unwrap_or("object.bin"));
                return Ok((bytes, filename.to_string()));
            }
        }
        return Err("missing presigned_url or OBJECT_PUBLIC_BASE_URL".into());
    }
    Err("missing url/presigned_url/s3_url/oss_url".into())
}

fn parse_object_url(u: &str) -> Option<(String, String)> {
    // 解析 s3://bucket/key 或 oss://bucket/key
    let pat = ["s3://", "oss://"];
    let mut s = u.to_string();
    for p in pat {
        if let Some(stripped) = u.strip_prefix(p) {
            s = stripped.to_string();
            break;
        }
    }
    let mut parts = s.splitn(2, '/');
    let bucket = parts.next()?.to_string();
    let key = parts.next()?.to_string();
    Some((bucket, key))
}

fn infer_filename_from_url(u: &str) -> &str {
    u.split('?')
        .next()
        .and_then(|p| p.rsplit('/').next())
        .unwrap_or("download.bin")
}

async fn http_fetch_bytes(
    url: &str,
    headers: Option<&serde_json::Value>,
) -> Result<Vec<u8>, String> {
    let timeout_ms: u64 = std::env::var("JOB_FETCH_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15000);
    let retries: usize = std::env::var("JOB_FETCH_RETRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let mut backoff_ms: u64 = std::env::var("JOB_FETCH_RETRY_BASE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    let client = reqwest::Client::new();
    let mut attempt = 0usize;
    loop {
        attempt += 1;
        let mut rb = client
            .get(url)
            .timeout(std::time::Duration::from_millis(timeout_ms));
        if let Some(h) = headers {
            if let Some(obj) = h.as_object() {
                for (k, v) in obj.iter() {
                    if let Some(val) = v.as_str() {
                        rb = rb.header(k, val);
                    }
                }
            }
        }
        match rb.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    return resp
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .map_err(|e| e.to_string());
                }
                let status = resp.status();
                let retryable = status.as_u16() == 429 || status.is_server_error();
                if retryable && attempt <= retries + 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms.saturating_mul(2);
                    continue;
                }
                return Err(format!("http {}", status));
            }
            Err(e) => {
                if attempt <= retries + 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms.saturating_mul(2);
                    continue;
                }
                return Err(e.to_string());
            }
        }
    }
}

async fn index_bytes(
    rag: Arc<dyn RagEngine>,
    document_id: &str,
    filename: &str,
    data: &[u8],
    chunk_size: usize,
    overlap: usize,
) -> Result<usize, String> {
    let lower = filename.to_ascii_lowercase();
    let text = if lower.ends_with(".html") || lower.ends_with(".htm") {
        let content = String::from_utf8_lossy(data).to_string();
        html2text::from_read(content.as_bytes(), 80)
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        let content = String::from_utf8_lossy(data).to_string();
        markdown_to_text(&content)
    } else {
        // 其它类型：优先抽取服务
        match std::env::var("EXTRACT_URL") {
            Ok(url) => match kb_rag::extract_text_via_service_bytes(filename, data, &url).await {
                Ok(t) => t,
                Err(e) => return Err(e.to_string()),
            },
            Err(_) => String::from_utf8(data.to_vec()).unwrap_or_default(),
        }
    };
    let n = chunk_and_index(rag, document_id, &text, chunk_size, overlap).await;
    Ok(n)
}
