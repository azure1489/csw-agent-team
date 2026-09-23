//! M3 回测：Van 拍过板的贴文，判断落在哪一档；同一批判两次，档一致不一致。
//!
//! **标准答案只用 Van 自己的决定**：写过 / 批准写（采用）、否决（`rejected`）。
//! 流程淘汰（`dropped`）是上一任收集员与研究员的判断，不是她的口味，默认不出题。
//!
//! 期望：采用的落推荐或备选，否决的落不推荐。待核单列——那是证据不够，不是判错。
//!
//! **防泄题**：判断时藏起每条贴文自己的那条决定（`Query::exclude_urls`）。
//! 同一件事在别的贴文上的决定没藏——正式一轮也看得见，那不算泄题；
//! 但它让回测偏乐观，报告里写明。
//!
//! 两次判断各开一轮手动轮（第二轮关掉判断缓存、挂在第一轮下面），
//! 结果在工作台的判断台账上都看得到。只写本地，不写引擎。
//! **要花模型钱**：采集识别一次、判断两次。原话不送（开关照旧）。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use csw_collector_core::types::{RoundKind, RoundTrigger, Tier};
use csw_collector_core::{Config, Secrets, rounds};

use crate::serve::{round, services::Services};

pub struct Opts {
    /// 只出前几题（按类各取）。0 = 全部
    pub limit: usize,
    /// 也拿流程淘汰出几题（不算验收，单列）
    pub dropped: usize,
    pub out: PathBuf,
}

#[derive(Debug, Clone)]
struct Case {
    url: String,
    brand: String,
    title: String,
    /// adopted / rejected / dropped
    truth: &'static str,
}

pub async fn run(cfg: &Config, secrets: &Secrets, o: Opts) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let svc = Services::build(cfg, secrets, &conn).await?;

    // ── 出题 ──
    let mut cases = cases_of(&conn, "adopted", &["written", "approved_write"], o.limit)?;
    cases.extend(cases_of(&conn, "rejected", &["rejected"], o.limit)?);
    if o.dropped > 0 {
        cases.extend(cases_of(&conn, "dropped", &["dropped"], o.dropped)?);
    }
    // 先各取一次：取不到的（csw 没收）现在就剔掉——Van 补链接那一路缺一条就整轮失败
    let mut ok = Vec::new();
    let mut gone = Vec::new();
    for c in cases {
        let code = c.url.rsplit('/').next().unwrap_or_default().to_string();
        match svc.csw.post(&code).await {
            Ok(_) => ok.push(c),
            Err(_) => gone.push(c),
        }
    }
    println!(
        "出题 {} 条（采用 {}、否决 {}、流程淘汰 {}）；csw 取不到 {} 条不计",
        ok.len(),
        ok.iter().filter(|c| c.truth == "adopted").count(),
        ok.iter().filter(|c| c.truth == "rejected").count(),
        ok.iter().filter(|c| c.truth == "dropped").count(),
        gone.len()
    );
    anyhow::ensure!(!ok.is_empty(), "没有可出的题");
    let links: Vec<String> = ok
        .iter()
        .map(|c| c.url.rsplit('/').next().unwrap_or_default().to_string())
        .collect();

    // ── 两次判断 ──
    let first = open(&conn, cfg, None)?;
    println!("第一次：第 {} 轮", first.id);
    let a = finish(
        &conn,
        first.id,
        round::run_backtest(&conn, &first, cfg, &svc, round::Caches::default(), &links).await,
    )?;
    let second = open(&conn, cfg, Some(first.id))?;
    println!("第二次（不用判断缓存）：第 {} 轮", second.id);
    let b = finish(
        &conn,
        second.id,
        round::run_backtest(
            &conn,
            &second,
            cfg,
            &svc,
            round::Caches {
                descriptions: true,
                judgements: false,
            },
            &links,
        )
        .await,
    )?;

    // ── 对答案 ──
    let url_of = urls_by_key(&conn)?;
    let tier_by_url = |js: &[csw_collector_core::types::Judgement]| -> HashMap<String, Tier> {
        js.iter()
            .filter_map(|j| {
                url_of
                    .get(&j.candidate_key)
                    .map(|u| (csw_collector_kb::docs::norm_url(u), j.tier))
            })
            .collect()
    };
    let ta = tier_by_url(&a);
    let tb = tier_by_url(&b);
    let rows: Vec<Row> = ok
        .iter()
        .map(|c| {
            let k = csw_collector_kb::docs::norm_url(&c.url);
            Row {
                case: c.clone(),
                first: ta.get(&k).copied(),
                second: tb.get(&k).copied(),
            }
        })
        .collect();
    let mut report = render(&rows, first.id, second.id);
    print!("{}", summary(&rows));
    // 口径兜底各触发几次（第一次那一轮）：看新口径在 Van 拍过板的题上改了多少
    let stats = crate::rejudge::rule_stats(&conn, first.id)?;
    let line = if stats.is_empty() {
        "口径兜底：一次都没触发".to_string()
    } else {
        format!(
            "口径兜底：{}",
            stats
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join("、")
        )
    };
    println!("{line}");
    report.push_str(&format!("\n{line}\n"));
    std::fs::write(&o.out, report).with_context(|| format!("写报告 {}", o.out.display()))?;
    println!("\n明细：{}", o.out.display());
    Ok(())
}

/// 按结论码出题。只要 Instagram `/p/` 链接——`/reel/` 是视频，采集只取图文，出了也判不到。
fn cases_of(
    conn: &Connection,
    truth: &'static str,
    codes: &[&str],
    limit: usize,
) -> Result<Vec<Case>> {
    let mut stmt = conn.prepare(
        "SELECT url, brand, title, substr(body, 1, 40) FROM kb_docs
          WHERE kind = 'decision' AND url LIKE '%instagram.com/p/%'
          ORDER BY id DESC",
    )?;
    let rows = stmt
        .query_map(params![], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (url, brand, title, head) in rows {
        let c = head
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("结论："))
            .unwrap_or("")
            .trim()
            .to_string();
        if !codes.contains(&c.as_str()) {
            continue;
        }
        let key = csw_collector_kb::docs::norm_url(&url);
        if !seen.insert(key.clone()) {
            continue;
        }
        out.push(Case {
            url: key,
            brand,
            title,
            truth,
        });
        if limit > 0 && out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn open(conn: &Connection, cfg: &Config, parent: Option<i64>) -> Result<rounds::Round> {
    // 空窗口：候选只来自给定短码。窗口放在很久以前，贴文库那一路取到的是零条
    let (r, _) = rounds::open_round(
        conn,
        &rounds::NewRound {
            kind: RoundKind::Manual,
            trigger: RoundTrigger::Manual,
            run_id: None,
            task_id: None,
            stage_code: Some("intake".into()),
            target_version: 0,
            parent_round_id: parent,
            window_start: "2020-01-01T00:00:00Z".into(),
            window_end: "2020-01-01T00:00:01Z".into(),
            plan_version: 1,
            rubric_version: csw_collector_judge::rubric::RUBRIC_VERSION.into(),
            kb_snapshot: cfg.vector.embed_model.clone(),
            instructions_hash: String::new(),
        },
    )?;
    Ok(r)
}

fn finish(
    conn: &Connection,
    id: i64,
    r: Result<Vec<csw_collector_core::types::Judgement>>,
) -> Result<Vec<csw_collector_core::types::Judgement>> {
    match r {
        Ok(j) => {
            rounds::finish_round(conn, id, "done", "M3 回测")?;
            Ok(j)
        }
        Err(e) => {
            rounds::finish_round(conn, id, "failed", &format!("M3 回测：{e:#}"))?;
            Err(e)
        }
    }
}

fn urls_by_key(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT candidate_key, url FROM candidates")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

struct Row {
    case: Case,
    first: Option<Tier>,
    second: Option<Tier>,
}

fn tier_name(t: Option<Tier>) -> &'static str {
    match t {
        Some(Tier::Recommend) => "推荐",
        Some(Tier::Alternate) => "备选",
        Some(Tier::NotRecommend) => "不推荐",
        Some(Tier::PendingCheck) => "待核",
        None => "没判到",
    }
}

/// 这一档对不对得上答案。待核与没判到不算对也不算错。
fn verdict(truth: &str, t: Option<Tier>) -> Option<bool> {
    match (truth, t) {
        (_, None | Some(Tier::PendingCheck)) => None,
        ("adopted", Some(t)) => Some(matches!(t, Tier::Recommend | Tier::Alternate)),
        ("rejected" | "dropped", Some(t)) => Some(t == Tier::NotRecommend),
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq)]
struct Tally {
    n: usize,
    right: usize,
    wrong: usize,
    undecided: usize,
}

fn tally(rows: &[Row], truth: &str) -> Tally {
    let mut t = Tally::default();
    for r in rows.iter().filter(|r| r.case.truth == truth) {
        t.n += 1;
        match verdict(truth, r.first) {
            Some(true) => t.right += 1,
            Some(false) => t.wrong += 1,
            None => t.undecided += 1,
        }
    }
    t
}

/// 两次都判到了的里面，档一样的占几成
fn consistency(rows: &[Row]) -> (usize, usize) {
    let both: Vec<&Row> = rows
        .iter()
        .filter(|r| r.first.is_some() && r.second.is_some())
        .collect();
    let same = both.iter().filter(|r| r.first == r.second).count();
    (same, both.len())
}

fn summary(rows: &[Row]) -> String {
    let mut s = String::new();
    for (truth, name, want) in [
        ("adopted", "Van 采用", "推荐/备选"),
        ("rejected", "Van 否决", "不推荐"),
        ("dropped", "流程淘汰（不算验收）", "不推荐"),
    ] {
        let t = tally(rows, truth);
        if t.n == 0 {
            continue;
        }
        s.push_str(&format!(
            "{name} {} 条：落{want} {}、落反了 {}、待核或没判到 {}\n",
            t.n, t.right, t.wrong, t.undecided
        ));
    }
    let (same, both) = consistency(rows);
    s.push_str(&format!(
        "两次判断档一致：{same}/{both}{}\n",
        if both > 0 {
            format!("（{:.0}%）", same as f64 * 100.0 / both as f64)
        } else {
            String::new()
        }
    ));
    s
}

fn render(rows: &[Row], first: i64, second: i64) -> String {
    let mut s = format!(
        "# M3 判断回测\n\n生成于 {}。第一次第 {first} 轮，第二次第 {second} 轮（不用判断缓存）。\n\n\
         每条判断时藏起了它自己那条决定；同一件事在别的贴文上的决定没藏，结果偏乐观。\n\n```\n",
        jiff::Timestamp::now().strftime("%Y-%m-%d %H:%M UTC")
    );
    s.push_str(&summary(rows));
    s.push_str(
        "```\n\n| 答案 | 品牌 | 标题 | 第一次 | 第二次 | 对不对 |\n|---|---|---|---|---|---|\n",
    );
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by_key(|r| (r.case.truth, verdict(r.case.truth, r.first)));
    for r in sorted {
        s.push_str(&format!(
            "| {} | {} | [{}]({}) | {} | {} | {} |\n",
            match r.case.truth {
                "adopted" => "采用",
                "rejected" => "否决",
                _ => "流程淘汰",
            },
            r.case.brand.replace('|', "\\|"),
            r.case
                .title
                .replace('|', "\\|")
                .chars()
                .take(40)
                .collect::<String>(),
            r.case.url,
            tier_name(r.first),
            tier_name(r.second),
            match verdict(r.case.truth, r.first) {
                Some(true) => "对",
                Some(false) => "**反**",
                None => "—",
            }
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(truth: &'static str, a: Option<Tier>, b: Option<Tier>) -> Row {
        Row {
            case: Case {
                url: String::new(),
                brand: String::new(),
                title: String::new(),
                truth,
            },
            first: a,
            second: b,
        }
    }

    #[test]
    fn 采用落备选算对_否决落推荐算反_待核不算() {
        use Tier::*;
        let rows = vec![
            row("adopted", Some(Alternate), Some(Alternate)),
            row("adopted", Some(PendingCheck), Some(Recommend)),
            row("rejected", Some(Recommend), Some(NotRecommend)),
            row("rejected", Some(NotRecommend), None),
        ];
        assert_eq!(
            tally(&rows, "adopted"),
            Tally {
                n: 2,
                right: 1,
                wrong: 0,
                undecided: 1
            }
        );
        assert_eq!(
            tally(&rows, "rejected"),
            Tally {
                n: 2,
                right: 1,
                wrong: 1,
                undecided: 0
            }
        );
        // 两次都判到的只有前三条，其中第一条一致
        assert_eq!(consistency(&rows), (1, 3));
    }
}
