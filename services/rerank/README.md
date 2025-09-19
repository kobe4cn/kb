# Rerank Service (CrossEncoder)

一个用于“向量检索结果二次重排”的轻量服务，基于 Python + FastAPI + sentence-transformers 实现，默认模型为 `cross-encoder/ms-marco-MiniLM-L-12-v2`。

配合后端（kb-api/kb-rag）：
- 当请求携带 `"rerank": true` 时，后端会优先使用 Cohere Re-rank；
- 若未配置 Cohere 且设置了 `RERANK_URL`，将调用本服务完成重排；
- 否则回退到启发式重排（查询词重叠度）。

## 接口

- 健康检查：
  - `GET /health` → `{ "status": "ok" }`

- 重排：
  - `POST /rerank`
  - 请求：
    ```json
    {
      "query": "your query",
      "candidates": ["text-1", "text-2", "..."]
    }
    ```
  - 响应：
    ```json
    { "scores": [0.83, 0.41, ...] }
    ```
  - 鉴权（可选）：若设置环境变量 `RERANK_TOKEN`，请求头需要携带 `Authorization: Bearer <token>`。

## 运行

### 方式 A：Docker（推荐）

```bash
docker build -t kb-rerank services/rerank
# 可选：设置鉴权 token
docker run -d --name kb-rerank -p 8000:8000 -e RERANK_TOKEN=secret kb-rerank
# 健康检查
curl http://localhost:8000/health
```

### 方式 B：本地 Python

```bash
cd services/rerank
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
# 可选：导出鉴权 token
export RERANK_TOKEN=secret
uvicorn server:app --host 0.0.0.0 --port 8000
```

## 调用示例

- 无鉴权：

```bash
curl -X POST http://localhost:8000/rerank \
  -H 'Content-Type: application/json' \
  -d '{"query":"best db","candidates":["use mysql","try qdrant","hello"]}'
```

- 带鉴权：

```bash
curl -X POST http://localhost:8000/rerank \
  -H 'Authorization: Bearer secret' \
  -H 'Content-Type: application/json' \
  -d '{"query":"best db","candidates":["use mysql","try qdrant","hello"]}'
```

## 环境变量

- `RERANK_TOKEN`：可选，设置后启用 Bearer 认证。

## 与后端对接

- 后端环境：
  - `RERANK_URL=http://localhost:8000/rerank`
  - 可选：`RERANK_TOKEN=secret`（后端会在请求中携带 `Authorization: Bearer <token>`）
- 查询请求：在 `/api/v1/query` 的 body 中加 `"rerank": true` 触发重排。

## 性能与注意事项

- 首次启动会下载模型缓存（较慢），建议长期运行以复用缓存；
- 默认为 CPU 推理，吞吐取决于机器资源与文本长度；
- 文本较长时可考虑启用更强的模型或分片重排策略；
- 生产建议：开启进程数/线程优化、容器健康检查与资源监控。

## Cohere 方案（替代本服务）

- 短期更推荐使用 Cohere Re-rank 云服务（无需本地模型）：
  - 后端设置 `COHERE_API_KEY`（可选 `COHERE_RERANK_MODEL=rerank-multilingual-v3.0`），
  - 查询带 `"rerank": true` 即可触发云端重排；
  - 优先级：Cohere > 本服务（RERANK_URL） > 启发式。

