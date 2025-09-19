# 快速启动指南

> 这是知识库 RAG 平台的快速启动指南，适合首次使用的开发者。

## 🚀 5分钟快速启动

### 1. 环境准备

```bash
# 确保已安装 Rust、Node.js 和 Docker
rustc --version  # 需要 1.70+
node --version   # 需要 18+
docker --version # 需要 20+
```

### 2. 克隆项目

```bash
git clone <repository-url>
cd kb
```

### 3. 设置环境变量

```bash
# 创建 .env 文件
cat > .env << EOF
OPENAI_API_KEY=sk-your-openai-api-key
REDIS_URL=redis://127.0.0.1:6379
EOF
```

### 4. 启动依赖服务

```bash
# 启动所有依赖服务
cd deployments
docker compose up -d

# 等待服务启动（约30秒）
sleep 30
```

### 5. 启动后端

```bash
# 回到项目根目录
cd ..

# 启动 API 服务器
cd apps/api
cargo run
```

### 6. 启动前端

```bash
# 新终端窗口
cd apps/web-next
npm install
npm run dev
```

### 7. 测试系统

访问 `http://localhost:3000` 开始使用！

## 📝 添加测试数据

```bash
# 添加示例文档
curl -X POST http://localhost:8080/api/v1/documents/text \
  -H 'Content-Type: application/json' \
  -d '{
    "document_id": "demo-1",
    "text": "这是一个知识库平台，支持 RAG 检索和智能问答。",
    "page": 1
  }'
```

## 🔍 测试查询

在前端界面输入问题："这个平台有什么功能？"

## 🛠️ 开发模式

### 热重载开发

```bash
# 后端热重载
cargo install cargo-watch
cargo watch -x run

# 前端热重载（已自动启用）
npm run dev
```

### 调试模式

```bash
# 启用详细日志
RUST_LOG=debug cargo run
```

## 🐛 常见问题

### 端口冲突
```bash
# 检查端口占用
lsof -i :8080  # API 端口
lsof -i :3000  # 前端端口
```

### 服务连接失败
```bash
# 检查 Docker 服务
docker compose ps
docker compose logs
```

### API 密钥问题
```bash
# 检查环境变量
echo $OPENAI_API_KEY
```

## 📚 更多信息

- 完整文档：[本地开发环境设置](local_dev_setup.md)
- API 文档：[OpenAPI 规范](api/openapi.yaml)
- 架构设计：[架构文档](architecture.md)

---

**需要帮助？** 查看 [故障排除指南](local_dev_setup.md#9-故障排除) 或创建 Issue。