# 前端（简易演示 + 建议）

- 演示页面：`public/demo.html`，提供最小化的 RAG 查询与 SSE 流式展示，并渲染 `contexts` 与 `citations`。
- 建议生产方案使用 Next.js/React：
  - Chat：SSE/WS 流式；
  - Search：原文高亮（使用 `contexts`/`citations` 返回字段）；
  - Graph：知识图谱可视化（D3/vis-network）。

API 参见 `../../docs/api/openapi.yaml`。
