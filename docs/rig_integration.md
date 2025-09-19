# Rig 集成指引

本文档摘录并适配 Rig 官方示例，帮助将本平台的 RAG 引擎替换为 Rig 实现。

## 向量检索 + RAG Agent（内存向量库）

```rust
use rig::providers::openai;
use rig::embeddings::EmbeddingsBuilder;
use rig::vector_store::{in_memory_store::InMemoryVectorStore, VectorStore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let openai_client = openai::Client::from_env();
    let embedding_model = openai_client.embedding_model("text-embedding-3-small");

    let mut vector_store = InMemoryVectorStore::default();
    let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
        .simple_document("doc1", "...")
        .simple_document("doc2", "...")
        .build()
        .await?;
    vector_store.add_documents(embeddings).await?;

    let index = vector_store.index(embedding_model);
    let rag_agent = openai_client.context_rag_agent("gpt-4o")
        .preamble("You are an assistant...")
        .dynamic_context(3, index)
        .build();

    let response = rag_agent.prompt("your question").await?;
    println!("{}", response);
    Ok(())
}
```

## Qdrant 向量库（生产推荐）

```rust
// 略：连接 Qdrant，创建 collection，构建 VectorStore，执行 top_n/query
// 参考 Rig 文档：/0xplaygrounds/rig-docs -> integrations/vector_stores/qdrant
```

## SurrealDB/LanceDB 等
- SurrealDB：`rig_surrealdb::SurrealVectorStore`；
- LanceDB：本地 ANN，见 Rig 文档。

## 在本平台中的落地方式
- 在 `crates/kb-rag` 中实现 `RagEngine`：
  - 初始化 Embedding Model；
  - 选择 Vector Store（Qdrant/SurrealDB/InMemory）；
  - 使用 `EmbeddingsBuilder` 将 `Chunk` 构造成向量写入；
  - 查询时用 `index.top_n_from_query(query, k)` 获取上下文，构建 `context_rag_agent` 生成答案；
- 结合过滤与重排：按租户/标签/来源过滤；必要时在应用层排序融合。

```text
注意：出于可编译性考虑，仓库未默认添加 rig 依赖，请在集成时在
kb-rag/Cargo.toml 中添加 rig 及所需向量库插件依赖，并实现替换 DefaultRagEngine。

## 已落地的实现
- `RigInMemoryRagEngine`：基于 Rig 的 `InMemoryVectorStore`，用于本地内存检索；
- `RigQdrantRagEngine`：基于 Rig 的 `QdrantVectorStore`，用于生产向量检索；

在 `configs/default.yaml` 中将 `vector_store.kind` 设置为：
- `memory` 或 `rig_mem`：启用内存引擎；
- `qdrant`：启用 Qdrant 引擎（默认）。
```
