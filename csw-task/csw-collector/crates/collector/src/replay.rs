//! `replay`：在回放模式下真跑一轮，不出网、不花钱。
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
//! 夹具就落在 `<data_dir>/fixtures/` 下。之后 `replay --fixtures <目录>` 回放，
//! 窗口从夹具里认出来，不用人记。
//!
//! # 五个出网口子都得挂上
//!
//! 模型、Jev、向量、**贴文库**、**图片下载**。后两个曾经漏了，于是这个模块
//! 说的「不出网」只对判断那半段成立：采集那一段照样在调真接口、照样在拉图。
//!
//! 图片那一跳特别一点：下载是先取回字节才算得出哈希的，blob 缓存挡不住出网，
//! 所以夹具里记的是 URL → 哈希，图本身还在 blob 目录。**夹具与 blob 是一对**，
//! blob 被清理过（图片有 120 天清理）回放就会失败并说清是哪张——
//! 这比静默去拉图强，那样回放就又出网了。

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

    // 窗口必须和录制时一模一样，否则第一条请求就不命中。
    // 与其要人记住上次录的是哪一天，不如从夹具里认出来。
    let (start, end) = window_from(&rec)?;
    println!("窗口取自夹具：{start} ~ {end}");

    let (r, _) = csw_collector_core::rounds::open_round(
        &conn,
        &csw_collector_core::rounds::NewRound {
            kind: csw_collector_core::types::RoundKind::Replay,
            trigger: csw_collector_core::types::RoundTrigger::Manual,
            run_id: None,
            task_id: None,
            stage_code: Some("intake".into()),
            target_version: 0,
            parent_round_id: None,
            window_start: start,
            window_end: end,
            plan_version: 1,
            rubric_version: csw_collector_judge::rubric::RUBRIC_VERSION.into(),
            kb_snapshot: cfg.vector.embed_model.clone(),
            instructions_hash: String::new(),
        },
    )?;
    println!("第 {} 轮开始。**不写引擎，不群播报。**\n", r.id);

    // 跑的是和 `run --manual` 同一段编排（第 2–5 步），只是客户端全挂着回放层
    let out = crate::serve::round::run_manual(&conn, &r, cfg, &svc, Default::default()).await;
    match out {
        Ok(c) => {
            csw_collector_core::rounds::finish_round(&conn, r.id, "done", "")?;
            println!("{c}");
            Ok(())
        }
        Err(e) => {
            let why = format!("{e:#}");
            csw_collector_core::rounds::finish_round(&conn, r.id, "failed", &why)?;
            // 未命中是回放最常见的失败，单独点一句该怎么办
            if why.contains("回放未命中") {
                eprintln!(
                    "\n夹具对不上。请求变了（提示词、schema、窗口、图片尺寸都算）就得重录：\n  \
                     CSW_COLLECTOR_RECORD=record ./csw-collector run --manual"
                );
            }
            Err(e)
        }
    }
}

/// 从 csw 夹具里认出录制时用的窗口。
///
/// 取**最早的 start 与最晚的 end**：一次录制里窗口取数会分页，
/// 每页一条夹具，但 start/end 在同一轮里是一样的；单条与生成贴文那些请求没有这两个参数，
/// 自然被跳过。
fn window_from(rec: &Recorder) -> Result<(String, String)> {
    let mut start: Option<String> = None;
    let mut end: Option<String> = None;
    for req in rec.requests("csw") {
        let Some(path) = req.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        if !path.contains("/posts/window?") {
            continue;
        }
        for kv in path.split(['?', '&']).skip(1) {
            match kv.split_once('=') {
                Some(("start", v)) => {
                    let v = v.to_string();
                    start = Some(match start.take() {
                        Some(s) => s.min(v),
                        None => v,
                    });
                }
                Some(("end", v)) => {
                    let v = v.to_string();
                    end = Some(match end.take() {
                        Some(s) => s.max(v),
                        None => v,
                    });
                }
                _ => {}
            }
        }
    }
    match (start, end) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => anyhow::bail!(
            "夹具里没有窗口取数的请求，认不出该回放哪个窗口。\
             录制时要跑完整的一轮：CSW_COLLECTOR_RECORD=record ./csw-collector run --manual"
        ),
    }
}
