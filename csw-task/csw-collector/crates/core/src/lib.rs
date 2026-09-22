//! 共享类型、配置、本地状态库、录制回放层。
//!
//! 这一层不认识 HTTP，也不认识引擎——上面每个 crate 都依赖它，它不依赖任何业务 crate。
//!
//! 职责边界：
//! - `types`：候选、采集方案、判断输出、对照材料、事件。跨 crate 传的值只用这里的类型。
//! - `config`：env > 配置文件 > 内置默认（与 csw-task-svc 同口径）。密钥只从 env 取，不落配置文件。
//! - `store`：本地 SQLite。**结构化数据全在这里，LanceDB 只放向量。**
//! - `record`：模型 / Jev / 向量 / 重排四个客户端的 record｜replay｜passthrough 开关。
//! - `vector`：向量与重排服务的客户端。**放这一层是因为单卡要求全进程只有一条队列**——
//!   `harvest` 要向量化、`kb` 要重排，各持一个客户端就等于又并发了（实测并发会让两边都慢 4 倍）。
//! - `model`：生成模型网关的客户端。同理——识别与判断走同一个网关，
//!   并发上限（实测 8）必须是全进程的，各持一个就等于把并发翻倍、全换成 429。
//!
//! 时区口径：**内部一律 UTC**，只在界面与引擎交互的边界换算北京时间。

pub mod config;
pub mod jev;
pub mod model;
pub mod record;
pub mod rounds;
pub mod store;
pub mod types;
pub mod vector;

pub use config::{Config, Secrets};
pub use record::{Mode as RecordMode, Recorder};
pub use store::SCHEMA_VERSION as LOCAL_SCHEMA_VERSION;

/// 装上 rustls 的加密提供者。
///
/// reqwest 以 `rustls-no-provider` 接入（为了避开 aws-lc-rs 的 cmake / nasm，
/// 交叉编译到 x86_64 linux 过不去），代价是**建 Client 之前必须先装一个提供者**，
/// 否则直接 panic。
///
/// 放在这里而不是只在 `main` 里调：每个测试、每个子命令、每个后台任务都可能第一个
/// 建出 Client，靠各处自觉记得是靠不住的。用 `Once` 保证只装一次，重复调无害。
pub fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // 已经装过就算了——比如宿主进程自己先装了一个
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
