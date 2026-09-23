//! 重判一轮：拿某一轮的候选，按**现在的**口径再判一遍，对照新旧结论。
//!
//! 给 09-22 判断台账反馈的验收用：第 3 轮点名的几条（SML 磁吸包、norbit 夹克、Hyperlite
//! 四款帐篷、Dapple Born 桌板、SOUTH2 WEST8 × BEN MILLER 夹克）按新口径会判成什么样。
//!
//! 只写本地一个新轮，**不写引擎、不群播报**。要花模型钱（识别复用，判断重来）。
//! 报告里只有标题、档位、查重结论、缺口这类结论字段，不含 Van 原话。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use csw_collector_core::rounds;
use csw_collector_core::types::{ComparisonVerdict, GapLevel, Judgement, RoundKind, RoundTrigger};
use csw_collector_core::{Config, Secrets};

use crate::serve::round;
use crate::serve::services::Services;

pub struct Opts {
    pub round: i64,
    /// 关注的条目：条目键或账号里含这些字样的（逗号分隔，不区分大小写）
    pub focus: Vec<String>,
    pub out: PathBuf,
}

pub async fn run(cfg: &Config, secrets: &Secrets, o: Opts) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let svc = Services::build(cfg, secrets, &conn).await?;

    let old = old_tiers(&conn, o.round)?;
    anyhow::ensure!(!old.is_empty(), "第 {} 轮没有判断", o.round);
    let links: Vec<String> = old.values().map(|(code, _, _)| code.clone()).collect();
    println!("第 {} 轮 {} 条候选，按现在的口径重判", o.round, links.len());

    let (r, _) = rounds::open_round(
        &conn,
        &rounds::NewRound {
            kind: RoundKind::Backtest,
            trigger: RoundTrigger::Manual,
            run_id: None,
            task_id: None,
            stage_code: Some("intake".into()),
            target_version: 0,
            parent_round_id: Some(o.round),
            // 空窗口：候选只来自给定短码
            window_start: "2020-01-01T00:00:00Z".into(),
            window_end: "2020-01-01T00:00:01Z".into(),
            plan_version: 1,
            rubric_version: csw_collector_judge::rubric::RUBRIC_VERSION.into(),
            kb_snapshot: cfg.vector.embed_model.clone(),
            instructions_hash: String::new(),
        },
    )?;
    println!("重判写在第 {} 轮（手动轮，只在本地）", r.id);
    let js = match round::run_rejudge(&conn, &r, cfg, &svc, &links).await {
        Ok(j) => {
            rounds::finish_round(&conn, r.id, "done", &format!("重判第 {} 轮", o.round))?;
            j
        }
        Err(e) => {
            rounds::finish_round(&conn, r.id, "failed", &format!("重判：{e:#}"))?;
            return Err(e);
        }
    };

    let report = render(&conn, o.round, r.id, &old, &js, &o.focus)?;
    print!("{}", report.lines().take(40).collect::<Vec<_>>().join("\n"));
    println!();
    std::fs::write(&o.out, &report).with_context(|| format!("写报告 {}", o.out.display()))?;
    println!("\n明细：{}", o.out.display());
    Ok(())
}

/// 旧轮每条：键 → (短码, 有效档, 账号)
fn old_tiers(conn: &Connection, round: i64) -> Result<HashMap<String, (String, String, String)>> {
    let sql = format!(
        "SELECT j.candidate_key, c.source_id, {}, c.account FROM judgements j
         JOIN candidates c ON c.candidate_key = j.candidate_key
         WHERE j.round_id = ?1 AND c.platform = 'instagram'",
        csw_collector_core::ledger::EFFECTIVE_TIER_SQL
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([round], |r| {
        Ok((
            r.get::<_, String>(0)?,
            (
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ),
        ))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 这一轮口径兜底各触发了几次（按留痕的第一句归类）。
pub fn rule_stats(conn: &Connection, round: i64) -> Result<Vec<(String, usize)>> {
    let mut st = conn.prepare("SELECT check_flags_json FROM judgements WHERE round_id = ?1")?;
    let mut m: HashMap<String, usize> = HashMap::new();
    for flags in st.query_map([round], |r| r.get::<_, String>(0))? {
        let flags: Vec<String> = serde_json::from_str(&flags?).unwrap_or_default();
        for f in flags {
            let kind = if f.starts_with(csw_collector_core::ledger::REJUDGED_FLAG) {
                "退回重判一次".to_string()
            } else if let Some(rest) = f.strip_prefix(csw_collector_judge::rules::NOTE) {
                rest.split(['，', '（', '「'])
                    .next()
                    .unwrap_or(rest)
                    .to_string()
            } else {
                continue;
            };
            *m.entry(kind).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(v)
}

fn tier_cn(t: &str) -> &str {
    match t {
        "recommend" => "推荐",
        "alternate" => "备选",
        "pending_check" => "待核",
        "not_recommend" => "不推荐",
        other => other,
    }
}

fn dedup_cn(v: ComparisonVerdict) -> &'static str {
    match v {
        ComparisonVerdict::Unrelated => "无重复",
        ComparisonVerdict::SameBrandWithGain => "同品牌有增量",
        ComparisonVerdict::SameFactNoGain => "同事实无增量",
        ComparisonVerdict::Unconfirmed => "查重未确认",
    }
}

fn render(
    conn: &Connection,
    old_round: i64,
    new_round: i64,
    old: &HashMap<String, (String, String, String)>,
    js: &[Judgement],
    focus: &[String],
) -> Result<String> {
    use std::fmt::Write as _;
    let mut s = String::new();
    let count = |xs: &mut dyn Iterator<Item = String>| {
        let mut m: HashMap<String, usize> = HashMap::new();
        for x in xs {
            *m.entry(x).or_default() += 1;
        }
        m
    };
    let o = count(&mut old.values().map(|(_, t, _)| t.clone()));
    let n = count(&mut js.iter().map(|j| {
        serde_json::to_value(j.tier)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }));
    let _ = writeln!(
        s,
        "# 重判第 {old_round} 轮（新口径写在第 {new_round} 轮）\n"
    );
    let _ = writeln!(s, "| 档 | 原来 | 新口径 |\n|---|---|---|");
    for t in ["recommend", "alternate", "pending_check", "not_recommend"] {
        let _ = writeln!(
            s,
            "| {} | {} | {} |",
            tier_cn(t),
            o.get(t).copied().unwrap_or(0),
            n.get(t).copied().unwrap_or(0)
        );
    }
    let topics = csw_collector_core::topics::topics(conn, new_round)?;
    let tc = csw_collector_judge::topic::counts(&topics, js);
    let _ = writeln!(
        s,
        "\n选题：推荐 {} 题 / {} 帖，备选 {} 题 / {} 帖，共 {} 个选题。\n",
        tc["推荐选题"], tc["推荐贴文"], tc["备选选题"], tc["备选贴文"], tc["选题总数"]
    );
    let _ = writeln!(s, "## 口径兜底触发\n");
    let stats = rule_stats(conn, new_round)?;
    if stats.is_empty() {
        let _ = writeln!(s, "一次都没触发。");
    }
    for (k, v) in stats {
        let _ = writeln!(s, "- {k}：{v}");
    }
    let refetched: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT candidate_key) FROM refetches WHERE round_id = ?1",
        params![new_round],
        |r| r.get(0),
    )?;
    let fetched_ok: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT candidate_key) FROM refetches WHERE round_id = ?1 AND status = 'ok'",
        params![new_round],
        |r| r.get(0),
    )?;
    let _ = writeln!(s, "- 补读：{refetched} 条，取到 {fetched_ok} 条");

    let _ = writeln!(s, "\n## 关注的条目\n");
    let focus: Vec<String> = focus.iter().map(|f| f.to_lowercase()).collect();
    for j in js {
        let (_, old_tier, account) = old.get(&j.candidate_key).cloned().unwrap_or_default();
        let hay = format!("{} {}", j.candidate_key, account).to_lowercase();
        if !focus.is_empty() && !focus.iter().any(|f| hay.contains(f)) {
            continue;
        }
        let topic = topics
            .iter()
            .find(|t| t.members.contains(&j.candidate_key))
            .filter(|t| t.members.len() > 1);
        let _ = writeln!(
            s,
            "### {}（{}）\n\n- 原来：{} → 新口径：**{}**\n- 标题：{}\n- 是什么：{}\n- 看点类型：{:?}{}\n- 查重：{}（命中 {} 条{}）",
            j.candidate_key,
            account,
            tier_cn(&old_tier),
            tier_cn(
                serde_json::to_value(j.tier)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .as_deref()
                    .unwrap_or("")
            ),
            j.headline,
            j.three_sentences.what,
            j.novelty.kind,
            if j.novelty.prior_evidence.is_empty() {
                String::new()
            } else {
                format!("（旧款依据：{}）", j.novelty.prior_evidence)
            },
            dedup_cn(j.comparison.verdict),
            j.comparison.hits.len(),
            if j.comparison.hits.iter().any(|h| !h.body_available) {
                "，含正文缺失的"
            } else {
                ""
            }
        );
        for g in j.gaps.iter().filter(|g| g.level == GapLevel::Decision) {
            let _ = writeln!(
                s,
                "- 影响判断的缺口：{}（{:?}；已尝试：{}；下一步：{}）",
                g.what, g.owner, g.tried, g.next
            );
        }
        if let Some(t) = topic {
            let _ = writeln!(
                s,
                "- 选题：{}（{} 帖：{}）",
                t.headline,
                t.members.len(),
                t.members.join("、")
            );
        }
        s.push('\n');
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{Candidate, Platform, Tier};

    #[test]
    fn 口径触发按第一句归类() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = rounds::open_round(
            &c,
            &rounds::NewRound {
                kind: RoundKind::Backtest,
                trigger: RoundTrigger::Manual,
                run_id: None,
                task_id: None,
                stage_code: None,
                target_version: 0,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v".into(),
                kb_snapshot: "s".into(),
                instructions_hash: String::new(),
            },
        )
        .unwrap();
        for k in ["a", "b"] {
            csw_collector_core::ledger::upsert_candidate(
                &c,
                &Candidate {
                    candidate_key: k.into(),
                    platform: Platform::Instagram,
                    source_id: k.into(),
                    collector: "c".into(),
                    account: "acc".into(),
                    url: format!("https://x/{k}"),
                    text: String::new(),
                    translated: String::new(),
                    posted_at: None,
                    ingested_at: None,
                    likes: None,
                    comments: None,
                    followers: None,
                    heat_ratio: None,
                    content_type: "Image".into(),
                    media: vec![],
                    tags: vec![],
                    hashtags: vec![],
                },
            )
            .unwrap();
        }
        let flags = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        csw_collector_core::ledger::put_judgement(
            &c,
            r.id,
            &Judgement::fixture("a", Tier::Alternate),
            &flags(&[
                "【口径·已重判】上次输出违反口径，已退回重判一次",
                "【口径】核心维度「值得推荐的价值」未成立，推荐改备选（原判推荐）",
            ]),
            "m",
            "v",
        )
        .unwrap();
        csw_collector_core::ledger::put_judgement(
            &c,
            r.id,
            &Judgement::fixture("b", Tier::Alternate),
            &flags(&[
                "【口径】核心维度「值得推荐的价值」未成立，推荐改备选（原判推荐）",
                "别的标记",
            ]),
            "m",
            "v",
        )
        .unwrap();
        let s = rule_stats(&c, r.id).unwrap();
        assert_eq!(s[0], ("核心维度".to_string(), 2));
        assert!(s.contains(&("退回重判一次".to_string(), 1)));
        assert_eq!(s.len(), 2, "不是口径留痕的不算");
    }
}
