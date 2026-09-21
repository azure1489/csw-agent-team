//! 共享类型、配置、本地状态库、录制回放层。
//!
//! 这一层不认识 HTTP，也不认识引擎——上面每个 crate 都依赖它，它不依赖任何业务 crate。
//!
//! 职责边界：
//! - `types`：候选、采集方案、判断输出、对照材料、事件。跨 crate 传的值只用这里的类型。
//! - `config`：env > 配置文件 > 内置默认（与 csw-task-svc 同口径）。密钥只从 env 取，不落配置文件。
//! - `store`：本地 SQLite。**结构化数据全在这里，LanceDB 只放向量。**
//! - `record`：模型 / Jev / 向量 / 重排四个客户端的 record｜replay｜passthrough 开关。
//!
//! 时区口径：**内部一律 UTC**，只在界面与引擎交互的边界换算北京时间。

pub mod config;
pub mod record;
pub mod store;
pub mod types;

pub use config::{Config, Secrets};
pub use record::{Mode as RecordMode, Recorder};
pub use store::SCHEMA_VERSION as LOCAL_SCHEMA_VERSION;
