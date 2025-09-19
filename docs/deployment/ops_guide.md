# 部署与运维（部署/运维）

本指南覆盖开发、预发与生产的环境初始化、部署、数据初始化与日常运维。

## 组件与默认选型
- API/Worker：Rust + Axum + Tokio；RAG/Embeddings 通过 Rig 调用；
- 元数据库：PostgreSQL 14+；
- 向量库：Qdrant（生产推荐）或 SurrealDB/LanceDB/In-Memory（开发）；
- 图数据库：Neo4j（可选，GraphRAG）；
- 对象存储：MinIO/S3；
- 原文检索：OpenSearch 或 PostgreSQL FTS；
- 缓存与队列：Redis；
- 可观测：Prometheus、Grafana、Loki/ELK、OpenTelemetry。

## 环境初始化
1) 准备基础设施（任选）
- Docker Compose（开发）：安装 Docker Desktop；
- Kubernetes（生产）：K8s 1.25+，可用的 Ingress/CSI/监控组件；
- 云托管：RDS/Qdrant Cloud/Neo4j Aura/托管对象存储。

2) 凭证与密钥
- `OPENAI_API_KEY` 或本地推理端点；
- 数据库/存储/图/搜索服务凭证；
- 以 `.env` 或 K8s Secret 管理；生产建议使用 External Secrets。

3) 数据库初始化
- 执行 `scripts/migrations/0001_init.sql` 初始化表结构；
- 若启用 FTS，创建相应 index（OpenSearch 或 PG FTS GIN/GIST）。

## 部署方式
### Docker Compose（开发）
- 使用 `deployments/docker-compose.yaml` 启动 Postgres、Qdrant、MinIO、Redis、OpenSearch、Neo4j（可选）；
- `apps/api`、`apps/worker` 挂载源码，热重载（可选）。

### Kubernetes（生产）
- 推荐 Helm（提供 `deployments/helm/` 样例）：
  - `values.yaml` 配置镜像、资源、环境变量、探针、HPA；
  - 以 StatefulSet 部署存储组件（或使用云托管资源）。
- 网络：启用 Ingress + TLS；API 网关限流与 WAF；
- 存储：开启备份策略与快照（Postgres/Qdrant/Neo4j/MinIO）。

## 数据初始化与索引
- 上传样例文档（管理后台或 API）；
- 触发 Ingestion Job（向量/图/原文）；
- 监控任务队列与日志，确保索引成功；
- 执行评测样例以建立基线（RAGAS）。

## 运行与维护
- 监控：QPS、延迟、失败率、队列长度、任务时延、资源使用；
- 告警：阈值（P95/P99、失败率、积压、可用性），与 On-Call 结合；
- 日志：请求/任务/错误/审计字段齐全，可检索；
- 备份：定期备份元数据与向量库/图库，校验可用性；
- 变更：灰度发布、回滚策略、依赖升级评估。

## 操作手册（常见任务）
- 扩容：调整副本数与 HPA；热点分片与缓存策略；
- 故障排查：按 TraceID 追踪；检查任务死信与补偿；
- 数据回滚：使用文档版本快照恢复索引与图；
- 合规导出：按租户导出查询与审计日志；
- 秘钥轮换：滚动更新 Secret，验证存活探针。

## 安全与合规
- OIDC/OAuth2 单点登录，JWT 鉴权；
- 多租户与 RBAC；
- 数据脱敏策略与审计报告导出。

