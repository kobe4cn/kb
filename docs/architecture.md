# 架构设计（架构师）

## 总体架构
- 技术栈：Rust + Axum/Tokio（API 与 Worker），Rig（RAG/嵌入/向量库集成），PostgreSQL（元数据），Qdrant（向量库，生产推荐），Neo4j（图数据库，可选），MinIO/S3（原始文档与中间产物），Redis（缓存/队列），OpenSearch/PG FTS（原文检索），NATS/Kafka（任务编排，可选）。
- 部署形态：Docker Compose（开发），Kubernetes + Helm（生产）。
- 可观测：Prometheus + Grafana（指标），ELK/ Loki（日志），OpenTelemetry（Trace）。

## 关键模块
- API Gateway（Axum）：统一鉴权、限流、路由、SSE/WS 流式返回；
- Ingestion Service（Worker）：数据采集、清洗、切分、嵌入与写入向量库/FTS/对象存储；
- RAG Service（Rig）：
  - Embeddings：OpenAI/本地模型（Rig providers）；
  - Vector Stores：Qdrant/SurrealDB/LanceDB/InMemory（Rig 集成）；
  - Agents：`context_rag_agent` + `dynamic_context(k, index)`；
- Graph Service：实体关系抽取（LLM/规则），入库 Neo4j，图查询检索与混合 rerank；
- Search Service：原文匹配（FTS/布尔/正则）与高亮；
- Job Orchestrator：任务队列、重试/幂等、速率控制，支持批/增量索引；
- Metadata Store（Postgres）：文档/切片/元数据/ACL/任务/审计。

## 数据流（简）
1) 采集→清洗→切分→嵌入→写入（向量库/FTS/对象存储），回写元数据与任务状态；
2) 查询（RAG）：向量 Top-K→过滤→重排序→上下文组装→生成→引用与高亮；
3) 查询（GraphRAG）：图检索（邻域扩展/路径约束）→证据拼接→生成→引用；
4) 混合：向量/图/原文匹配结果融合，提升可追溯性与准确率。

## 逻辑视图
- 多租户与 RBAC：所有实体含租户标识与 ACL；
- 限流与缓存：按租户与用户维度；
- 可用性：无状态 API 水平扩容；存储具备副本与备份；
- 审计：查询与生成留痕，支持检索还原证据链。

## 数据模型（核心）
- Source：数据源配置（类型、凭证、调度）；
- Document：原始文档（存储 URI、版本、哈希）；
- Chunk：切片（文本、位置、页码、向量 ID、图节点 IDs）；
- IndexJob：任务（状态机：待处理/处理中/成功/失败/重试次数）；
- Graph Node/Edge：实体与关系（类型、置信度、来源 chunk）；
- QueryLog：查询与响应（参数、耗时、证据、模型、租户、用户）。

## 性能与扩展
- Rust + Tokio 无阻塞 IO，Rig 向量检索与 RAG Agent 封装低开销；
- Qdrant/PGVector 支持 HNSW/IVF，高维向量 ANN 查询；
- 预热缓存与 L2 缓存；热点问题维护 FAQ/Cache 命中；
- 任务分片与并行：切分/嵌入/写入批量并发；
- 模型推理：可外接 OpenAI/Azure/OpenRouter 或自托管 NIM/TGI。

## 可靠性
- 每步任务幂等（基于内容哈希/版本），失败重试与死信队列；
- 存储备份：Postgres/WAL、Qdrant/快照、Neo4j/备份、MinIO/版本；
- 回滚策略：按版本快照恢复索引与图。

## 安全
- OIDC/OAuth2/JWT 鉴权；细粒度权限（租户/空间/标签/文档）；
- 机密管理：K8s Secrets/External Secrets；
- 数据脱敏：上传/查询级别的 PII 选项。

## 与 Rig 的集成要点
- Embeddings 与 VectorStore：
  - InMemory（开发）、Qdrant/SurrealDB/LanceDB（生产可选）。
  - 通过 `EmbeddingsBuilder` 统一生成文档嵌入；
  - 使用 `vector_store.index(model)` 构建检索索引；
- RAG Agent：
  - `openai_client.context_rag_agent(model)` + `.dynamic_context(k, index)` 实时拼装上下文；
- 示例（伪代码，参见 docs 中 Rig Snippet）：
  ```rust
  let openai = openai::Client::from_env();
  let embed = openai.embedding_model("text-embedding-3-small");
  let mut store = InMemoryVectorStore::default();
  let docs = EmbeddingsBuilder::new(embed.clone())
      .simple_document("doc1", "...")
      .build()
      .await?;
  store.add_documents(docs).await?;
  let rag = openai.context_rag_agent("gpt-4o")
      .preamble("...")
      .dynamic_context(3, store.index(embed))
      .build();
  ```

