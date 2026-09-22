//! 深核：codex app-server 的薄 stdio 客户端 + 深核编排。
//!
//! 自写客户端，不依赖 `codex-app-server-protocol`（会拖进 500+ 依赖，版本 `0.0.0` 锁不住）。
//! NDJSON over stdio，只为用到的六七个类型写 serde 结构，其余 `serde_json::Value` 兜底。
//!
//! 协议要点（已对源码与 0.155.1 实测核过）：
//! - `initialize` 之后**必须**发 `initialized` 通知，否则后续请求不被处理。
//! - `threadId` 在 `result.thread.id`，不是 `result.threadId`。
//! - `turn/start` **是异步的**：响应只回 `inProgress`，完成信号是 `turn/completed` 通知。
//! - 同一线程上一回合没结束时，起带不同 `outputSchema` 的新回合会被拒
//!   （`ActiveTurnOutputSchemaMismatch`）→ **一条线程一个回合**。
//! - `turn/interrupt` 要 `threadId` **和** `turnId`，两个都必填。
//! - 五个审批类服务端请求必须应答，拒绝值是 `{"decision":"decline"}`（不是 `denied`）。

pub mod codex;
pub mod run;
