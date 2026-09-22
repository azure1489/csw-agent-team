//! `replay`：在回放模式下跑一轮，不出网、不花钱。
//!
//! # 它解决的问题
//!
//! 一轮真跑要四十分钟网关时间、几百万 token（0.4 / 0.5 实测）。
//! 回归测试不能每次都付这个钱，也不能每次都等这么久。
//! 录一次 9/17–9/18 的窗口，之后所有回归都在回放下跑。
//!
//! # 未命中即失败，绝不静默穿透
//!
//! 悄悄去调真接口的回放层等于没有回放层——测试会时快时慢、时对时错，
//! 而且会偷偷花钱。所以回放模式下 [`Recorder::wrap`] 找不到夹具就报错，
//! 报错里带着键，好去对照是哪一条请求变了。
//!
//! # 录制怎么做
//!
//! `CSW_COLLECTOR_RECORD=record` 跑一次真的（`run --manual`），
//! 夹具就落在 `<data_dir>/fixtures/` 下。之后 `replay --fixtures <目录>` 回放。

use std::sync::Arc;

use anyhow::{Context, Result};

use csw_collector_core::record::{Mode, Recorder};
use csw_collector_core::{Config, Secrets};

pub async fn run(cfg: &Config, secrets: &Secrets, fixtures: &str) -> Result<()> {
    let dir = std::path::Path::new(fixtures);
    anyhow::ensure!(dir.exists(), "夹具目录不在：{}", dir.display());

    let rec = Arc::new(Recorder::new(Mode::Replay, dir));
    let inv = rec.inventory();
    anyhow::ensure!(
        !inv.is_empty(),
        "夹具目录 {} 是空的。先用 CSW_COLLECTOR_RECORD=record 跑一次真的。",
        dir.display()
    );
    println!("夹具 {}", dir.display());
    for (client, n) in &inv {
        println!("  {client:<8}{n} 条");
    }

    // 回放下这些密钥用不上，但客户端构造要它们非空——给个占位，
    // **不要去读真密钥**：回放本来就不该有出网的可能
    let mut s = secrets.clone();
    for k in [&mut s.sub2api_key, &mut s.typesafe_key, &mut s.csw_api_key] {
        if k.trim().is_empty() {
            *k = "replay-placeholder".into();
        }
    }

    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let svc = crate::serve::services::Services::build_with_recorder(cfg, &s, &conn, Some(rec)).await?;
    println!("\n客户端已挂上回放层。**这一轮不会出网。**");

    // 真正跑一轮要引擎派单；回放目前覆盖的是出网那几段，
    // 编排那一段由 `run --manual` 走。这里先把夹具与装配验通。
    let healthy = svc.vector.healthy().await;
    println!("向量客户端 healthy()：{healthy}（回放下这一项走的也是夹具）");
    Ok(())
}
