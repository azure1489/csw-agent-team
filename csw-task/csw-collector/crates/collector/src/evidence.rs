//! 定向补证：某一轮里**待核**的条目，重新深核（深核能联网：去找官网、商品页、全文）
//! 并带着条目卡重判，结果写回原轮次。
//!
//! 09-28 r56：19 条待核里 12 条卡在「关键内容在官网 / 商品页 / 全文，取不到」——深核当时不能上网。
//! 打开深核的网络搜索后用它补一遍。**不采集、不写引擎、不提交**：改的是本地台账，
//! 主编退回时工作台在原轮次上返工，会把补到的结论带进下一版。
//!
//! 要花模型钱（深核与重判）。输出只有键、档位、缺口这类结论字段。

use anyhow::{Context, Result};

use csw_collector_core::types::Tier;
use csw_collector_core::{Config, Secrets, mirror, rounds};

use crate::serve::round;
use crate::serve::services::Services;

pub struct Opts {
    pub round: i64,
    /// 只补这几条（条目键）。空 = 这一轮全部待核
    pub keys: Vec<String>,
}

pub async fn run(cfg: &Config, secrets: &Secrets, o: Opts) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let svc = Services::build(cfg, secrets, &conn).await?;
    let r = rounds::get(&conn, o.round)?.ok_or_else(|| anyhow::anyhow!("没有第 {} 轮", o.round))?;
    // 作业标准取这一轮任务最近一次派单时的那份（含主编退回意见）
    let standard = match r.task_id {
        Some(t) => mirror::work_standard_of(&conn, t)?,
        None => None,
    }
    .or(mirror::latest_work_standard(&conn, "intake")?)
    .unwrap_or_default();

    let (mut judgements, mut topics, by_key) = round::load_round_state(&conn, r.id)?;
    let focus: Vec<String> = judgements
        .iter()
        .filter(|j| {
            if o.keys.is_empty() {
                j.tier == Tier::PendingCheck
            } else {
                o.keys.contains(&j.candidate_key)
            }
        })
        .map(|j| j.candidate_key.clone())
        .collect();
    let before: Vec<(String, Tier)> = judgements
        .iter()
        .filter(|j| focus.contains(&j.candidate_key))
        .map(|j| (j.candidate_key.clone(), j.tier))
        .collect();
    println!("第 {} 轮：定向补证 {} 条", r.id, focus.len());
    if focus.is_empty() {
        return Ok(());
    }

    let (_, outcomes) = round::evidence_pass(
        &conn,
        &r,
        cfg,
        &svc,
        &standard,
        &focus,
        &mut judgements,
        &by_key,
        "【补证】联网深核后重判",
    )
    .await?;
    round::retier_topics(&mut topics, &judgements);
    csw_collector_core::topics::put_topics(&conn, r.id, &topics)?;

    let done = outcomes.iter().filter(|o| o.done()).count();
    println!("深核做成 {done}/{}", outcomes.len());
    let mut moved = 0;
    for (k, old) in &before {
        let Some(j) = judgements.iter().find(|j| &j.candidate_key == k) else {
            continue;
        };
        if j.tier != *old {
            moved += 1;
        }
        let gap = j
            .gaps
            .iter()
            .find(|g| g.level == csw_collector_core::types::GapLevel::Decision)
            .map(|g| g.what.chars().take(60).collect::<String>())
            .unwrap_or_default();
        println!("- {k}：{old:?} → {:?}　{gap}", j.tier);
    }
    println!(
        "档位变了 {moved} 条；台账已写回第 {} 轮（不写引擎、不提交）",
        r.id
    );
    Ok(())
}
