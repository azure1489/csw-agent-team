//! 任务流转引擎（csw-task-svc 运行面）的客户端。
//!
//! 覆盖：`GET /me/tasks`、任务详情、`ack`（兼作心跳）、`PUT items`、`sweeps`、
//! `intake-judgements`、`intake-check`、`intake-trace`、`deliverables`、`fail`、`ledger`、`memory`。
//!
//! 三条必须记住的引擎事实（勘察实测，见实施计划「硬事实」）：
//! 1. 任务在触发时就钉到 `ActiveAgentByRole` 返回的那一行 agent——新建 agent 接不到单，
//!    采集服务必须用**同一 agent 行再签一枚 token**。
//! 2. 无独立心跳接口，重复 `POST /tasks/:id/ack` 就是心跳。
//! 3. `Idempotency-Key` **只对 POST 生效**；请求中途崩溃则同键永久 409。
//!    所以提交前先把 zip 与幂等键落盘，重试只发已落盘的那一份，绝不换键。
//!
//! 另有 [`admin`]：管理后台的鉴权转发，给工作台的登录用。

pub mod admin;
