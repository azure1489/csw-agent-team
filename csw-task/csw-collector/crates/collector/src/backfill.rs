//! 补宽取留痕：老轮次宽取时没逐条落库（09-28 r56：1431 → 128 只有两个数），
//! 按同一发布时间窗口重新取一次贴文列表，用同一套规则复算每条去向，存成 `source=refetch`。
//!
//! **只取元数据**：不下载图、不识别、不判断，不写引擎、不提交。主编退回时工作台在原轮次返工，
//! 交付包里的 `trace/window_ids.jsonl` 就会列出全部宽取条目，汇总里写明是事后复算。
//! 复算得到的窗口内集合与这一轮当时判过的逐键对照，对不上的逐条打出来。

use std::collections::BTreeSet;

use anyhow::{Context, Result};

use csw_collector_core::types::Timestamp;
use csw_collector_core::{Config, Secrets, rounds};
use csw_collector_harvest::collector::CswWindow;
use csw_collector_harvest::pipeline::{self, CollectorDyn, TraceOutcome};

use crate::serve::services::Services;

pub async fn run(cfg: &Config, secrets: &Secrets, round_id: i64) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let r = rounds::get(&conn, round_id)?.ok_or_else(|| anyhow::anyhow!("没有第 {round_id} 轮"))?;
    let live = csw_collector_core::window_trace::of_round(&conn, r.id)?;
    if live.iter().any(|x| x.source == "live") {
        anyhow::bail!("第 {} 轮已有当轮宽取留痕，不用补", r.id);
    }
    let svc = Services::build(cfg, secrets, &conn).await?;
    let from: Timestamp = r.window_start.parse().context("窗口起点")?;
    let to: Timestamp = r.window_end.parse().context("窗口终点")?;
    let window = CswWindow {
        client: svc.csw.clone(),
        lookback_days: csw_collector_harvest::csw::WINDOW_LOOKBACK_DAYS,
    };
    let collectors: Vec<&dyn CollectorDyn> = vec![&window];
    let (_, sweeps, blocked) = pipeline::collect_all(&collectors, from, to, true).await;
    if let Some(why) = blocked {
        anyhow::bail!("重新取数失败：{why}");
    }
    crate::serve::round::save_window_trace(&conn, r.id, &sweeps, "refetch");

    let s = &sweeps[0];
    println!("第 {} 轮 窗口 {} ~ {}", r.id, r.window_start, r.window_end);
    println!(
        "重新宽取：{}；接口返回 {} 条，去重后 {} 条",
        s.query, s.found, s.fetched_unique
    );
    for o in [
        TraceOutcome::InWindow,
        TraceOutcome::DupInSweep,
        TraceOutcome::NotImageOnly,
        TraceOutcome::BeforeWindow,
        TraceOutcome::AfterWindow,
        TraceOutcome::NoTime,
    ] {
        let n = s.trace.iter().filter(|t| t.outcome == o).count();
        println!("  {}：{n}", o.cn());
    }
    // 与这一轮当时判过的逐键对照
    let now_in: BTreeSet<String> = s
        .trace
        .iter()
        .filter(|t| t.outcome == TraceOutcome::InWindow)
        .map(|t| t.candidate_key.clone())
        .collect();
    let mut st = conn.prepare("SELECT candidate_key FROM round_candidates WHERE round_id = ?1")?;
    let then_in: BTreeSet<String> = st
        .query_map([r.id], |x| x.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    let missing: Vec<_> = then_in.difference(&now_in).collect();
    let extra: Vec<_> = now_in.difference(&then_in).collect();
    println!(
        "对照当时判过的 {} 条：复算窗口内 {} 条，一致 {} 条；当时有、复算没有 {} 条；复算有、当时没有 {} 条",
        then_in.len(),
        now_in.len(),
        then_in.intersection(&now_in).count(),
        missing.len(),
        extra.len()
    );
    for k in missing {
        println!("  当时有、复算没有：{k}");
    }
    for k in extra {
        println!("  复算有、当时没有：{k}");
    }
    println!("已存为 source=refetch；不写引擎、不提交");
    Ok(())
}
