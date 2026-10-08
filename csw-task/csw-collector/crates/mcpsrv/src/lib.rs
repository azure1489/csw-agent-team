//! 本地 MCP 服务：把知识库与选题记忆暴露给 codex 深核线程和（可选）Hermes。
//!
//! 默认工具：`kb_search`、`kb_similar_selected`、`memory_lookup`——全部只读。
//! `fetch_page` 与 `xhs_search` 实现但**不进深核白名单**，给 Hermes 的挂载默认关闭。

pub mod magazine;
pub mod server;
pub mod tools;
