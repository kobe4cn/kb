# KB Platform (RAG + GraphRAG) with Rig

一个面向生产环境的知识库管理与问答平台，后端基于 Rust 与 Rig 框架，支持：

- RAG（检索增强生成）与 GraphRAG（图谱增强检索）
- 原始文档匹配（FTS/布尔/正则/精确匹配）
- 多数据源采集、清洗、切分、向量化与索引
- 查询 API 与前端界面（Chat/Search/Graph 浏览）
- 多租户、RBAC、安全与审计，观测与评测

## 🚀 快速开始

### 5分钟快速启动

```bash
# 1. 克隆项目
git clone <repository-url>
cd kb

# 2. 设置环境变量
echo "OPENAI_API_KEY=sk-your-key" > .env

# 3. 启动依赖服务
cd deployments && docker compose up -d && cd ..

# 4. 启动后端
cd apps/api && cargo run &

# 5. 启动前端
cd apps/web-next && npm install && npm run dev
```

访问 `http://localhost:3000` 开始使用！

详细步骤请参考：[快速启动指南](docs/quickstart.md)

## 📚 文档

### 开发文档
- [快速启动指南](docs/quickstart.md) - 5分钟快速上手
- [本地开发环境设置](docs/local_dev_setup.md) - 完整的开发环境配置
- [环境变量配置说明](docs/env_config.md) - 所有环境变量详解

### 架构文档
- [架构设计](docs/architecture.md) - 系统架构概览
- [产品规格](docs/product_spec.md) - 产品功能规划
- [API 文档](docs/api/openapi.yaml) - OpenAPI 规范

### 部署文档
- [部署与运维指南](docs/deployment/ops_guide.md) - 生产环境部署
- [测试计划](docs/testing/test_plan.md) - 测试策略和计划

## 🏗️ 项目结构

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

## 🔧 多模型集成

支持多种 LLM 提供商：

- **OpenAI**: GPT-4, GPT-3.5, text-embedding-3-small
- **Anthropic**: Claude-3.5-Sonnet
- **DeepSeek**: DeepSeek-Chat, DeepSeek-Embedding
- **阿里云 DashScope**: Qwen 系列模型

配置示例：
```yaml
# configs/default.yaml
chat_provider:
  kind: openai_compat
  base_url: https://api.openai.com
  api_key_env: OPENAI_API_KEY
  model: gpt-4o

embedding_provider:
  kind: openai_compat
  base_url: https://api.openai.com
  api_key_env: OPENAI_API_KEY
  model: text-embedding-3-small
```

## 🚀 核心功能

### 文档处理
- 支持多种格式：PDF, DOCX, PPTX, XLSX, HTML, Markdown
- 智能分块和向量化
- 元数据提取和索引

### 检索模式
- **RAG**: 语义检索 + 上下文生成
- **GraphRAG**: 知识图谱增强检索
- **词汇检索**: 关键词和 TF-IDF 匹配
- **混合检索**: 多种检索方式融合

### 查询接口
- RESTful API
- Server-Sent Events (SSE) 流式响应
- 工具调用和会话管理
- 过滤和重排序

### 管理功能
- 多租户支持
- 权限控制
- 任务队列管理
- 监控和审计

## 🛠️ 开发工具

### Rust 开发
```bash
# 代码格式化
cargo fmt

# 代码检查
cargo clippy

# 运行测试
cargo test

# 生成文档
cargo doc --open
```

### 前端开发
```bash
# 安装依赖
npm install

# 开发模式
npm run dev

# 构建生产版本
npm run build
```

## 🔍 API 测试

### 添加文档
```bash
curl -X POST http://localhost:8080/api/v1/documents/text \
  -H 'Content-Type: application/json' \
  -d '{
    "document_id": "demo-1",
    "text": "这是一个知识库平台，支持 RAG 检索和智能问答。",
    "page": 1
  }'
```

### 查询测试
```bash
curl -X POST http://localhost:8080/api/v1/query \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "这个平台有什么功能？",
    "top_k": 3,
    "mode": "rag"
  }'
```

## 🐛 故障排除

### 常见问题
- **端口冲突**: 检查 8080 (API) 和 3000 (前端) 端口
- **服务连接失败**: 确认 Docker 服务正常运行
- **API 密钥问题**: 检查环境变量配置

详细故障排除请参考：[本地开发环境设置 - 故障排除](docs/local_dev_setup.md#9-故障排除)

## 🤝 贡献指南

1. Fork 项目
2. 创建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

## 📄 许可证

本项目采用 MIT 许可证。详见 [LICENSE](LICENSE) 文件。

## 📞 支持

如果遇到问题，请：

1. 查看本文档的故障排除部分
2. 检查项目的 [Issues](https://github.com/your-org/kb/issues)
3. 创建新的 Issue 并提供详细的错误信息

---

*最后更新：2024年1月*
