# 环境变量配置说明

本文档详细说明知识库 RAG 平台的所有环境变量配置选项。

## 📋 配置分类

- [必需配置](#必需配置)
- [模型配置](#模型配置)
- [服务配置](#服务配置)
- [重排服务配置](#重排服务配置)
- [抽取服务配置](#抽取服务配置)
- [任务队列配置](#任务队列配置)
- [对象存储配置](#对象存储配置)
- [管理配置](#管理配置)
- [日志配置](#日志配置)
- [前端配置](#前端配置)
- [开发配置](#开发配置)

## 必需配置

### OPENAI_API_KEY
- **描述**: OpenAI API 密钥
- **类型**: 字符串
- **必需**: 是
- **示例**: `sk-your-openai-api-key-here`
- **说明**: 用于调用 OpenAI 的聊天和嵌入模型

## 模型配置

### OpenAI 模型配置

#### OPENAI_EMBED_MODEL
- **描述**: OpenAI 嵌入模型名称
- **类型**: 字符串
- **默认值**: `text-embedding-3-small`
- **可选值**: 
  - `text-embedding-3-small`
  - `text-embedding-3-large`
  - `text-embedding-ada-002`

#### OPENAI_CHAT_MODEL
- **描述**: OpenAI 聊天模型名称
- **类型**: 字符串
- **默认值**: `gpt-4o`
- **可选值**:
  - `gpt-4o`
  - `gpt-4o-mini`
  - `gpt-4-turbo`
  - `gpt-3.5-turbo`

### 其他 LLM 提供商

#### ANTHROPIC_API_KEY
- **描述**: Anthropic API 密钥
- **类型**: 字符串
- **必需**: 否
- **说明**: 用于使用 Claude 模型

#### DEEPSEEK_API_KEY
- **描述**: DeepSeek API 密钥
- **类型**: 字符串
- **必需**: 否
- **说明**: 用于使用 DeepSeek 模型

#### DASHSCOPE_API_KEY
- **描述**: 阿里云 DashScope API 密钥
- **类型**: 字符串
- **必需**: 否
- **说明**: 用于使用 Qwen 模型

## 服务配置

### Redis 配置

#### REDIS_URL
- **描述**: Redis 连接 URL
- **类型**: 字符串
- **默认值**: `redis://127.0.0.1:6379`
- **说明**: 用于会话持久化

#### SESS_TTL_SECS
- **描述**: 会话 TTL 秒数
- **类型**: 整数
- **默认值**: `3600`
- **说明**: 会话在 Redis 中的过期时间

### PostgreSQL 配置

#### DATABASE_URL
- **描述**: PostgreSQL 连接 URL
- **类型**: 字符串
- **示例**: `postgresql://kb:kb@localhost:5432/kb`
- **说明**: 用于元数据存储（需要启用 pg 特性）

## 重排服务配置

### CrossEncoder 重排服务

#### RERANK_URL
- **描述**: CrossEncoder 重排服务 URL
- **类型**: 字符串
- **示例**: `http://localhost:8000/rerank`
- **说明**: 本地重排服务端点

#### RERANK_TOKEN
- **描述**: 重排服务认证令牌
- **类型**: 字符串
- **示例**: `secret`
- **说明**: 用于重排服务的 Bearer 认证

### Cohere 重排服务

#### COHERE_API_KEY
- **描述**: Cohere API 密钥
- **类型**: 字符串
- **说明**: 用于云端重排服务

#### COHERE_RERANK_MODEL
- **描述**: Cohere 重排模型
- **类型**: 字符串
- **默认值**: `rerank-multilingual-v3.0`
- **可选值**:
  - `rerank-multilingual-v3.0`
  - `rerank-english-v3.0`

## 抽取服务配置

#### EXTRACT_URL
- **描述**: 统一文档抽取服务 URL
- **类型**: 字符串
- **示例**: `http://localhost:9000/extract`
- **说明**: 用于解析各种文档格式

#### EXTRACT_TOKEN
- **描述**: 抽取服务认证令牌
- **类型**: 字符串
- **说明**: 用于抽取服务的 Bearer 认证

#### EXTRACT_TIMEOUT_MS
- **描述**: 抽取请求超时时间（毫秒）
- **类型**: 整数
- **默认值**: `15000`

#### EXTRACT_RETRIES
- **描述**: 抽取请求重试次数
- **类型**: 整数
- **默认值**: `2`

#### EXTRACT_RETRY_BASE_MS
- **描述**: 抽取请求重试基础延迟（毫秒）
- **类型**: 整数
- **默认值**: `250`

#### EXTRACT_CONCURRENCY
- **描述**: 抽取服务并发数
- **类型**: 整数
- **默认值**: `4`

## 任务队列配置

#### JOB_MAX_RETRIES
- **描述**: 任务最大重试次数
- **类型**: 整数
- **默认值**: `2`

#### JOB_RETRY_BASE_MS
- **描述**: 任务重试基础延迟（毫秒）
- **类型**: 整数
- **默认值**: `500`

#### JOB_URL_TIMEOUT_MS
- **描述**: URL 任务超时时间（毫秒）
- **类型**: 整数
- **默认值**: `10000`

#### JOB_FETCH_TIMEOUT_MS
- **描述**: 文件获取超时时间（毫秒）
- **类型**: 整数
- **默认值**: `15000`

#### JOB_FETCH_RETRIES
- **描述**: 文件获取重试次数
- **类型**: 整数
- **默认值**: `2`

#### JOB_FETCH_RETRY_BASE_MS
- **描述**: 文件获取重试基础延迟（毫秒）
- **类型**: 整数
- **默认值**: `250`

## 对象存储配置

#### OBJECT_PUBLIC_BASE_URL
- **描述**: 对象存储公共访问基础 URL
- **类型**: 字符串
- **示例**: `http://localhost:9000`
- **说明**: 用于访问 MinIO/S3 中的文件

## 管理配置

### 管理员认证

#### ADMIN_USER
- **描述**: 管理员用户名
- **类型**: 字符串
- **默认值**: `admin`

#### ADMIN_PASS
- **描述**: 管理员密码
- **类型**: 字符串
- **默认值**: `admin`

#### ADMIN_BEARER
- **描述**: 管理员 Bearer 令牌
- **类型**: 字符串
- **说明**: 用于 API 认证

### JWT 配置

#### ADMIN_JWT_ALLOW_UNVERIFIED
- **描述**: 是否允许未验证的 JWT
- **类型**: 布尔值
- **默认值**: `false`
- **说明**: 仅用于开发环境

#### ADMIN_OIDC_ISSUER
- **描述**: OIDC 发行者 URL
- **类型**: 字符串
- **示例**: `https://your-issuer.com`

#### ADMIN_OIDC_AUDIENCE
- **描述**: OIDC 受众
- **类型**: 字符串
- **示例**: `your-audience`

## 日志配置

#### RUST_LOG
- **描述**: Rust 日志级别
- **类型**: 字符串
- **默认值**: `info`
- **可选值**:
  - `error`
  - `warn`
  - `info`
  - `debug`
  - `trace`

#### 特定模块日志级别
- **示例**: `RUST_LOG=tower_http=info,kb_rag=debug`
- **说明**: 可以为不同模块设置不同的日志级别

## 前端配置

#### NEXT_PUBLIC_API_BASE
- **描述**: Next.js 前端 API 基础 URL
- **类型**: 字符串
- **默认值**: `http://localhost:8080`
- **说明**: 前端代理转发的目标地址

## 开发配置

#### DEV_MODE
- **描述**: 开发模式标志
- **类型**: 布尔值
- **默认值**: `true`

#### DEBUG_MODE
- **描述**: 调试模式标志
- **类型**: 布尔值
- **默认值**: `false`

#### TRACE_MODE
- **描述**: 跟踪模式标志
- **类型**: 布尔值
- **默认值**: `false`

## 📝 配置示例

### 开发环境配置

```bash
# 开发环境 .env 文件
OPENAI_API_KEY=sk-your-dev-key
REDIS_URL=redis://127.0.0.1:6379
RERANK_URL=http://localhost:8000/rerank
RERANK_TOKEN=dev-secret
RUST_LOG=debug
DEV_MODE=true
```

### 生产环境配置

```bash
# 生产环境 .env 文件
OPENAI_API_KEY=sk-prod-your-key
REDIS_URL=redis://prod-redis:6379
DATABASE_URL=postgresql://prod-user:prod-pass@prod-db:5432/kb
RERANK_URL=https://prod-rerank.example.com/rerank
RERANK_TOKEN=prod-secret-token
ADMIN_BEARER=prod-admin-token
RUST_LOG=warn
DEV_MODE=false
```

### 多提供商配置

```bash
# 使用多个 LLM 提供商
OPENAI_API_KEY=sk-openai-key
ANTHROPIC_API_KEY=ant-anthropic-key
DEEPSEEK_API_KEY=ds-deepseek-key
DASHSCOPE_API_KEY=ds-dashscope-key

# 使用 Cohere 重排
COHERE_API_KEY=cohere-key
COHERE_RERANK_MODEL=rerank-multilingual-v3.0
```

## 🔧 配置验证

### 检查环境变量

```bash
# 检查必需的环境变量
echo "OpenAI API Key: ${OPENAI_API_KEY:0:10}..."
echo "Redis URL: $REDIS_URL"
echo "Rerank URL: $RERANK_URL"
```

### 测试服务连接

```bash
# 测试 Redis 连接
redis-cli -u $REDIS_URL ping

# 测试重排服务
curl -H "Authorization: Bearer $RERANK_TOKEN" $RERANK_URL/health

# 测试数据库连接
psql $DATABASE_URL -c "SELECT 1"
```

## 🚨 安全注意事项

1. **API 密钥安全**
   - 不要在代码中硬编码 API 密钥
   - 使用环境变量或密钥管理服务
   - 定期轮换 API 密钥

2. **生产环境配置**
   - 使用强密码和令牌
   - 启用 HTTPS
   - 配置防火墙规则

3. **日志安全**
   - 避免在生产环境中使用 `debug` 或 `trace` 级别
   - 不要在日志中记录敏感信息

## 📚 相关文档

- [本地开发环境设置](local_dev_setup.md)
- [快速启动指南](quickstart.md)
- [部署指南](deployment/ops_guide.md)
- [架构设计](architecture.md)

---

*最后更新：2024年1月*
