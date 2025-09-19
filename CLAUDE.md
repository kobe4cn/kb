# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概览

基于 Rust + Rig 框架的生产级知识库 RAG 平台，支持多模态检索增强生成（RAG + GraphRAG）。

## 工作空间结构

这是一个 Rust workspace 项目，包含以下核心模块：

- `crates/kb-core` - 核心数据结构和类型定义
- `crates/kb-rag` - RAG 引擎实现
- `crates/kb-graph` - GraphRAG 图数据库集成
- `apps/api` - Axum Web API 服务器
- `apps/worker` - 后台任务处理器

## 常用开发命令

### 构建和运行
```bash
# 构建整个 workspace
cargo build

# 运行 API 服务器
cd apps/api && cargo run

# 运行 Worker
cd apps/worker && cargo run

# 运行测试
cargo test

# 检查代码
cargo check

# 格式化代码
cargo fmt

# Linting
cargo clippy
```

### 开发环境设置
参考 `docs/local_dev_setup.md`，需要启动依赖服务：

```bash
# 启动依赖服务
cd deployments
docker compose up -d postgres qdrant minio redis opensearch neo4j

# 初始化数据库
psql postgresql://kb:kb@localhost:5432/kb -f deployments/migrations/0001_jobs.sql
psql postgresql://kb:kb@localhost:5432/kb -f deployments/migrations/0002_indexes.sql
```

### 环境变量配置
核心环境变量（通常放在 `.env` 文件中）：
- `OPENAI_API_KEY` - OpenAI API 密钥
- `ANTHROPIC_API_KEY` - Anthropic API 密钥（可选）
- `DEEPSEEK_API_KEY` - DeepSeek API 密钥（可选）
- `DASHSCOPE_API_KEY` - 阿里云 DashScope API 密钥（可选）
- `REDIS_URL` - Redis 连接 URL（可选，用于会话持久化）
- `DATABASE_URL` - PostgreSQL 连接 URL

## 架构要点

### 模块化设计
- **kb-core**: 包含 `Tenant`, `Document`, `Chunk`, `IndexJob` 等核心数据模型
- **kb-rag**: 集成 Rig 框架，提供 RAG 和 GraphRAG 引擎
- **API 层**: 基于 Axum，提供 RESTful API 和 SSE 流式接口

### Rig 框架集成
- 使用 `rig-core` 和 `rig-qdrant` 进行向量检索
- 支持多种嵌入模型提供商（OpenAI, DeepSeek, Qwen 等）
- Agent 模式: `context_rag_agent` + `dynamic_context`

### 向量存储
- 生产环境：Qdrant (默认 gRPC 端口 6334)
- 开发环境：可切换到内存模式 (`memory` 或 `rig_mem`)
- 配置文件：`configs/default.yaml`

### 多模型支持
- 聊天模型：OpenAI, Anthropic Claude, DeepSeek, Qwen
- 嵌入模型：OpenAI, DeepSeek, Qwen 兼容端点
- 配置解耦：聊天和嵌入可使用不同提供商

## API 接口要点

### 核心端点
- `POST /api/v1/documents/text` - 简单文本索引
- `POST /api/v1/documents/text_with_meta` - 带元数据的文本索引
- `POST /api/v1/query` - 非流式查询
- `GET /api/v1/query/stream` - SSE 流式查询（服务端闭环）
- 会话式工具闭环：`/api/v1/session/start`, `/api/v1/session/stream`, `/api/v1/session/tool_result`

### 数据格式
- 查询支持模式：`rag`, `graph`, `hybrid`, `lexical`
- 支持过滤：`tenant_id`, `tags`, `start_time`, `end_time`
- 支持重排：`rerank: true` (Cohere API 或本地 CrossEncoder)

## 配置文件

主配置文件 `configs/default.yaml` 包含：
- 服务器设置 (host, port)
- 多提供商配置 (chat_provider, embedding_provider)
- 向量存储设置 (vector_store)
- 数据库和对象存储配置
- 可选的抽取服务配置

## 部署相关

### Docker Compose
开发环境使用 `deployments/docker-compose.yml`

### 数据库迁移
迁移脚本位于 `deployments/migrations/`：
- `0001_jobs.sql` - 创建基础表结构
- `0002_indexes.sql` - 创建索引

## 测试

项目包含单元测试和集成测试：
- 单元测试：`cargo test` 运行所有 crate 的测试
- 集成测试：需要先启动依赖服务

## 重要注意事项

1. **向量维度一致性**: 确保 Qdrant collection 维度与嵌入模型一致
2. **环境变量安全**: API 密钥通过环境变量注入，不要硬编码
3. **多租户设计**: 所有数据模型都包含 `tenant_id` 字段
4. **异步架构**: 基于 Tokio 的异步 I/O，所有阻塞操作需要适当处理