# 本地开发环境设置指南

本文档提供完整的本地开发环境设置指南，帮助开发者快速启动知识库 RAG 平台并进行调试。

## 📋 目录

- [前置准备](#1-前置准备)
- [项目结构](#2-项目结构)
- [环境配置](#3-环境配置)
- [依赖服务启动](#4-依赖服务启动)
- [后端服务启动](#5-后端服务启动)
- [前端服务启动](#6-前端服务启动)
- [API 测试](#7-api-测试)
- [开发工具](#8-开发工具)
- [故障排除](#9-故障排除)

## 1. 前置准备

### 必需工具

- **Rust** (stable toolchain)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  source ~/.cargo/env
  rustup default stable
  ```

- **Node.js** (>= 18)
  ```bash
  # 使用 nvm 安装
  curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
  nvm install 18
  nvm use 18
  ```

- **Docker** (可选，用于依赖服务)
  ```bash
  # macOS
  brew install --cask docker
  
  # Ubuntu
  sudo apt-get update
  sudo apt-get install docker.io
  ```

### 可选工具

- **PostgreSQL** (本地安装，替代 Docker)
- **Redis** (本地安装，替代 Docker)
- **Qdrant** (本地安装，替代 Docker)

## 2. 项目结构

```
kb/
├── apps/
│   ├── api/                 # Axum Web API 服务器
│   ├── worker/             # 后台任务处理器
│   ├── web/                # 静态 HTML 演示
│   └── web-next/           # Next.js 前端应用
├── crates/
│   ├── kb-core/            # 核心数据结构和类型
│   ├── kb-error/           # 统一错误处理
│   ├── kb-rag/             # RAG 引擎实现
│   ├── kb-graph/           # GraphRAG 图数据库集成
│   └── kb-llm/             # LLM 提供商集成
├── services/
│   └── rerank/             # CrossEncoder 重排服务
├── deployments/
│   ├── docker-compose.yaml # 依赖服务编排
│   └── migrations/         # 数据库迁移脚本
├── configs/
│   └── default.yaml        # 主配置文件
└── docs/                   # 项目文档
```

## 3. 环境配置

### 3.1 创建环境变量文件

在项目根目录创建 `.env` 文件：

```bash
# 必需：OpenAI API 密钥
OPENAI_API_KEY=sk-your-openai-api-key

# 可选：模型配置
OPENAI_EMBED_MODEL=text-embedding-3-small
OPENAI_CHAT_MODEL=gpt-4o

# 可选：会话持久化
REDIS_URL=redis://127.0.0.1:6379
SESS_TTL_SECS=3600

# 可选：重排服务
RERANK_URL=http://localhost:8000/rerank
RERANK_TOKEN=secret

# 可选：数据库
DATABASE_URL=postgresql://kb:kb@localhost:5432/kb

# 可选：其他 LLM 提供商
ANTHROPIC_API_KEY=your-anthropic-key
DEEPSEEK_API_KEY=your-deepseek-key
DASHSCOPE_API_KEY=your-dashscope-key
```

### 3.2 多提供商配置示例

#### OpenAI 兼容服务
```bash
OPENAI_API_KEY=sk-your-key
OPENAI_EMBED_MODEL=text-embedding-3-small
OPENAI_CHAT_MODEL=gpt-4o
```

#### DeepSeek
```bash
DEEPSEEK_API_KEY=your-deepseek-key
# 在 configs/default.yaml 中配置：
# chat_provider:
#   kind: openai_compat
#   base_url: https://api.deepseek.com
#   api_key_env: DEEPSEEK_API_KEY
#   model: deepseek-chat
```

#### 阿里云 DashScope (Qwen)
```bash
DASHSCOPE_API_KEY=your-dashscope-key
# 在 configs/default.yaml 中配置：
# embedding_provider:
#   kind: qwen
#   api_url: https://dashscope.aliyuncs.com/api/v1/embeddings
#   api_key_env: DASHSCOPE_API_KEY
#   model: text-embedding-v2
```

## 4. 依赖服务启动

### 4.1 使用 Docker Compose (推荐)

```bash
# 启动所有依赖服务
cd deployments
docker compose up -d

# 检查服务状态
docker compose ps

# 查看日志
docker compose logs -f
```

服务端口映射：
- PostgreSQL: `localhost:5432`
- Qdrant: `localhost:6333` (REST), `localhost:6334` (gRPC)
- Redis: `localhost:6379`
- MinIO: `localhost:9000` (API), `localhost:9001` (Console)
- OpenSearch: `localhost:9200`
- Neo4j: `localhost:7474` (Web), `localhost:7687` (Bolt)

### 4.2 数据库初始化

```bash
# 应用数据库迁移
psql postgresql://kb:kb@localhost:5432/kb -f deployments/migrations/0001_jobs.sql
psql postgresql://kb:kb@localhost:5432/kb -f deployments/migrations/0002_indexes.sql
```

### 4.3 单独启动服务

#### Qdrant (向量数据库)
```bash
docker run -d --name qdrant \
  -p 6333:6333 \
  -p 6334:6334 \
  qdrant/qdrant
```

#### Redis (会话存储)
```bash
docker run -d --name redis \
  -p 6379:6379 \
  redis:7
```

#### PostgreSQL (元数据存储)
```bash
docker run -d --name postgres \
  -e POSTGRES_USER=kb \
  -e POSTGRES_PASSWORD=kb \
  -e POSTGRES_DB=kb \
  -p 5432:5432 \
  postgres:15
```

### 4.4 CrossEncoder 重排服务 (可选)

```bash
# 构建重排服务
cd services/rerank
docker build -t kb-rerank .

# 启动重排服务
docker run -d --name kb-rerank \
  -p 8000:8000 \
  -e RERANK_TOKEN=secret \
  kb-rerank

# 验证服务
curl http://localhost:8000/health
# 预期输出: {"status":"ok"}
```

## 5. 后端服务启动

### 5.1 API 服务器

```bash
# 从项目根目录启动
cd apps/api
cargo run

# 或者使用环境变量
OPENAI_API_KEY=sk-your-key cargo run

# 启用 PostgreSQL 特性
cargo run --features pg
```

默认监听地址：`http://localhost:8080`

### 5.2 Worker 服务

```bash
cd apps/worker
cargo run
```

### 5.3 开发模式

```bash
# 监听文件变化自动重启
cargo install cargo-watch
cargo watch -x run

# 启用调试日志
RUST_LOG=debug cargo run

# 检查代码
cargo check
cargo clippy
cargo fmt
```

## 6. 前端服务启动

### 6.1 Next.js 应用

```bash
cd apps/web-next

# 安装依赖
npm install

# 启动开发服务器
npm run dev
```

默认访问地址：`http://localhost:3000`

### 6.2 静态 HTML 演示

```bash
# 直接在浏览器中打开
open apps/web/public/demo.html
```

### 6.3 前端代理配置

Next.js 已配置代理，将 `/api/*` 请求转发到后端：

```javascript
// apps/web-next/next.config.js
async rewrites() {
  const apiBase = process.env.NEXT_PUBLIC_API_BASE || 'http://localhost:8080';
  return [
    { source: '/api/:path*', destination: `${apiBase}/api/:path*` },
  ]
}
```

## 7. API 测试

### 7.1 健康检查

```bash
curl http://localhost:8080/api/v1/health
# 预期输出: {"status":"ok"}
```

### 7.2 添加测试数据

#### 简单文本
```bash
curl -X POST http://localhost:8080/api/v1/documents/text \
  -H 'Content-Type: application/json' \
  -d '{
    "document_id": "doc-1",
    "text": "这是一个示例文档，描述知识库平台的功能和特性。",
    "page": 1
  }'
```

#### 带元数据文本
```bash
curl -X POST http://localhost:8080/api/v1/documents/text_with_meta \
  -H 'Content-Type: application/json' \
  -d '{
    "document_id": "doc-2",
    "text": "平台支持 RAG、GraphRAG、词汇检索等多种检索模式。",
    "page": 2,
    "tenant_id": "tenant-001",
    "source": "manual",
    "tags": ["platform", "features"],
    "created_at": 1726420000
  }'
```

#### PDF 批量导入
```bash
curl -X POST http://localhost:8080/api/v1/documents/pdf_glob \
  -H 'Content-Type: application/json' \
  -d '{
    "glob": "/path/to/documents/*.pdf",
    "prefix": "pdf-"
  }'
```

#### 网页导入
```bash
curl -X POST http://localhost:8080/api/v1/documents/url \
  -H 'Content-Type: application/json' \
  -d '{
    "url": "https://example.com",
    "document_id": "url-1"
  }'
```

### 7.3 查询测试

#### 基本查询
```bash
curl -X POST http://localhost:8080/api/v1/query \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "平台有什么功能？",
    "top_k": 3,
    "mode": "rag"
  }'
```

#### 带过滤和重排
```bash
curl -X POST http://localhost:8080/api/v1/query \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "平台能力？",
    "top_k": 3,
    "mode": "rag",
    "rerank": true,
    "filters": {
      "tenant_id": "tenant-001",
      "tags": ["platform", "features"]
    }
  }'
```

#### 工具调用测试
```bash
curl -X POST http://localhost:8080/api/v1/query_trace \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "现在的时间是什么？",
    "top_k": 2
  }'
```

### 7.4 流式查询测试

#### SSE 流式查询
```bash
# GET 方式
curl -N "http://localhost:8080/api/v1/query/stream?query=平台功能&top_k=3"

# POST 方式
curl -X POST http://localhost:8080/api/v1/query/stream \
  -H 'Content-Type: application/json' \
  -d '{"query": "平台功能", "top_k": 3}' \
  -N
```

#### 会话式工具调用
```bash
# 1. 启动会话
SESSION_ID=$(curl -s -X POST http://localhost:8080/api/v1/session/start \
  -H 'Content-Type: application/json' \
  -d '{"query": "现在的时间？", "top_k": 3}' | jq -r '.session_id')

# 2. 拉取流
curl -N "http://localhost:8080/api/v1/session/stream?session_id=$SESSION_ID"

# 3. 提交工具结果（如果需要）
curl -X POST http://localhost:8080/api/v1/session/tool_result \
  -H 'Content-Type: application/json' \
  -d "{\"session_id\": \"$SESSION_ID\", \"result\": \"2024-01-01T00:00:00Z\"}"
```

## 8. 开发工具

### 8.1 Rust 开发工具

```bash
# 代码格式化
cargo fmt

# 代码检查
cargo clippy

# 运行测试
cargo test

# 运行特定测试
cargo test lexical::tests::test_tokenize

# 生成文档
cargo doc --open

# 依赖更新
cargo update
```

### 8.2 前端开发工具

```bash
# 代码检查
npm run lint

# 类型检查
npm run type-check

# 构建生产版本
npm run build

# 启动生产服务器
npm run start
```

### 8.3 数据库工具

```bash
# PostgreSQL 连接
psql postgresql://kb:kb@localhost:5432/kb

# Redis 连接
redis-cli -h localhost -p 6379

# Qdrant 管理界面
open http://localhost:6333/dashboard
```

### 8.4 监控和调试

```bash
# 查看 API 日志
RUST_LOG=debug cargo run

# 查看 Docker 服务日志
docker compose logs -f api

# 查看特定服务日志
docker logs -f qdrant
docker logs -f redis
```

## 9. 故障排除

### 9.1 常见问题

#### API 服务无法启动
```bash
# 检查端口占用
lsof -i :8080

# 检查环境变量
echo $OPENAI_API_KEY

# 检查依赖服务
curl http://localhost:6334/health  # Qdrant
curl http://localhost:6379         # Redis
```

#### 向量数据库连接失败
```bash
# 检查 Qdrant 状态
docker ps | grep qdrant

# 重启 Qdrant
docker restart qdrant

# 检查配置
cat configs/default.yaml | grep -A 5 vector_store
```

#### 前端代理问题
```bash
# 检查 Next.js 配置
cat apps/web-next/next.config.js

# 检查环境变量
echo $NEXT_PUBLIC_API_BASE

# 重启前端服务
cd apps/web-next && npm run dev
```

#### 重排服务问题
```bash
# 检查服务状态
curl http://localhost:8000/health

# 查看日志
docker logs kb-rerank

# 重启服务
docker restart kb-rerank
```

### 9.2 性能优化

#### 内存优化
```bash
# 限制 Docker 内存使用
docker run --memory=2g --memory-swap=2g qdrant/qdrant

# 调整 Rust 编译优化
export RUSTFLAGS="-C target-cpu=native"
cargo build --release
```

#### 网络优化
```bash
# 使用本地网络
docker network create kb-network
docker run --network kb-network qdrant/qdrant
```

### 9.3 调试技巧

#### 启用详细日志
```bash
# Rust 应用
RUST_LOG=trace cargo run

# Docker 服务
docker compose logs -f --tail=100
```

#### 性能分析
```bash
# 安装性能分析工具
cargo install flamegraph

# 生成火焰图
cargo flamegraph --bin kb-api
```

#### 内存分析
```bash
# 安装内存分析工具
cargo install cargo-valgrind

# 运行内存检查
cargo valgrind run
```

## 10. 清理和重置

### 10.1 停止服务

```bash
# 停止所有 Docker 服务
docker compose down

# 停止特定服务
docker stop qdrant redis postgres

# 清理容器
docker rm qdrant redis postgres
```

### 10.2 清理数据

```bash
# 清理 Docker 卷
docker volume prune

# 清理特定卷
docker volume rm deployments_qdrant_data
docker volume rm deployments_redis_data
```

### 10.3 重置开发环境

```bash
# 清理 Rust 构建缓存
cargo clean

# 清理 Node.js 缓存
cd apps/web-next && rm -rf node_modules package-lock.json

# 重新安装依赖
npm install
```

## 11. 生产部署准备

### 11.1 环境变量检查

确保生产环境包含所有必需的环境变量：

```bash
# 必需
OPENAI_API_KEY=sk-your-production-key
DATABASE_URL=postgresql://user:pass@prod-host:5432/kb

# 推荐
REDIS_URL=redis://prod-redis:6379
RERANK_URL=http://prod-rerank:8000/rerank
RERANK_TOKEN=production-secret
```

### 11.2 构建生产版本

```bash
# Rust 应用
cargo build --release

# Next.js 应用
cd apps/web-next
npm run build
```

### 11.3 健康检查

```bash
# API 健康检查
curl http://localhost:8080/api/v1/health

# 数据库连接检查
psql $DATABASE_URL -c "SELECT 1"

# Redis 连接检查
redis-cli -u $REDIS_URL ping
```

---

## 📚 相关文档

- [架构设计](architecture.md)
- [API 文档](api/openapi.yaml)
- [产品规格](product_spec.md)
- [部署指南](deployment/ops_guide.md)
- [测试计划](testing/test_plan.md)

## 🤝 贡献指南

1. Fork 项目
2. 创建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

## 📞 支持

如果遇到问题，请：

1. 查看本文档的故障排除部分
2. 检查项目的 [Issues](https://github.com/your-org/kb/issues)
3. 创建新的 Issue 并提供详细的错误信息

---

*最后更新：2024年1月*