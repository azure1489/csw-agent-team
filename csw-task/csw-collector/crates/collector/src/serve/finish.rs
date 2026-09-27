//! 登记之后的四步：深核 → 交付物 → 自查 → 提交。
//!
//! # 为什么这四步要等登记确认
//!
//! `intake-check` 拿**引擎那边的**台账与采集轮对账。判断还没发过去就查，
//! 查出来的是假红灯——不是「我们漏了」，而是「引擎还没收到」。
//! 所以顺序是：登记发完 → 自查 → 带着自查结果做交付物 → 提交。
//!
//! # 自查报红也要交
//!
//! 红灯时不交，主编就什么都看不到，也就无从处置。所以**照交，
//! 并把红灯原样写进交付物的缺口**，让人一眼看见哪一条没过。
//! 把红灯藏起来才是真的失职。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::{Connection, params};

use csw_collector_core::Config;
use csw_collector_core::rounds::{self, Round};
use csw_collector_core::types::{
    Candidate, Gap, GapLevel, GapOwner, Judgement, StepCode, StepStatus, Tier,
};
use csw_collector_deepcheck::codex::{Codex, CodexConfig, DEFAULT_ENV_PASSTHROUGH};
use csw_collector_deepcheck::run::{self as deep, Target};
use csw_collector_deliver::index::Meta;
use csw_collector_deliver::{intake, pack, trace};
use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::{IntakeCheckResult, SubmitInput};
use csw_collector_harvest::pipeline::{Prepared, SweepCount};

/// 深核挑哪几条：**主编点名的** → **待核的**（至多 `n_pending`）→ **推荐的**（至多 `n_recommend`）。
///
/// 待核的排在推荐前面：深核过的不再留在待核（09-25 用户），推荐一多、待核就轮不上的老挑法
/// 让待核一直挂着。点名的不看档，但总数受两个上限之和约束——深核一条要几分钟。
/// 不推荐、备选的不进：已经判过取舍了，再核一遍没有意义。
pub fn first_batch<'a>(
    judgements: &'a [Judgement],
    pinned: &[String],
    n_recommend: usize,
    n_pending: usize,
) -> Vec<&'a Judgement> {
    let cap = n_recommend + n_pending;
    let mut picked: Vec<&Judgement> = judgements
        .iter()
        .filter(|j| pinned.contains(&j.candidate_key))
        .take(cap)
        .collect();
    let taken = |picked: &Vec<&Judgement>, j: &Judgement| {
        picked.iter().any(|p| p.candidate_key == j.candidate_key)
    };
    for (tier, n) in [
        (Tier::PendingCheck, n_pending),
        (Tier::Recommend, n_recommend),
    ] {
        let mut added = 0;
        for j in judgements.iter().filter(|j| j.tier == tier) {
            if added >= n || picked.len() >= cap {
                break;
            }
            if !taken(&picked, j) {
                picked.push(j);
                added += 1;
            }
        }
    }
    picked
}

/// 深核条目卡拼成一段，给重判用。
pub fn card_text(c: &deep::Card) -> String {
    let mut s = String::new();
    let mut line = |k: &str, v: &str| {
        if !v.trim().is_empty() {
            s.push_str(&format!("{k}：{}\n", v.trim()));
        }
    };
    line("原始披露时间", &c.disclosed_at);
    line("原始来源", &c.original_source);
    line("对照", &c.comparison_note);
    line("完整图", &c.figure_notes);
    for (k, xs) in [
        ("核到的事实", &c.facts),
        ("证据", &c.evidence),
        ("仍缺", &c.gaps),
    ] {
        if !xs.is_empty() {
            s.push_str(&format!("{k}：\n"));
            for x in xs {
                s.push_str(&format!("- {}\n", x.trim()));
            }
        }
    }
    s
}

/// 第 6 步：深核首批。**失败不抛错**——条目照样登记，只在缺口里写明。
pub async fn deepcheck(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    judgements: &[Judgement],
    by_key: &HashMap<String, Candidate>,
    prepared: &[Prepared],
    standard: &str,
) -> Vec<deep::Outcome> {
    let step = match rounds::begin_step(
        conn,
        round.id,
        StepCode::Deepcheck,
        &round.instructions_hash,
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(原因 = %format!("{e:#}"), "深核这一步没记上");
            return vec![];
        }
    };
    // 主编在工作台上指定过就按他指定的来；没指定就按档自动挑
    let pinned = csw_collector_core::workbench::first_batch(conn, round.id).unwrap_or_else(|e| {
        tracing::warn!(原因 = %format!("{e:#}"), "读指定首批失败，按自动挑法来");
        Vec::new()
    });
    let picked = first_batch(
        judgements,
        &pinned,
        cfg.codex.first_batch,
        cfg.codex.pending_cap,
    );
    if !pinned.is_empty() {
        tracing::info!(
            指定 = pinned.len(),
            实际 = picked.len(),
            "首批按主编指定的挑"
        );
    }
    if picked.is_empty() {
        let _ = rounds::end_step(
            conn,
            step.id,
            StepStatus::Skipped,
            &serde_json::json!({}),
            "没有要深核的",
        );
        return vec![];
    }

    let codex = match Codex::start(CodexConfig {
        bin: cfg.codex.bin.clone(),
        home: cfg.codex.home.clone(),
        cwd: cfg.blob_dir(),
        model: cfg.codex.model.clone(),
        turn_budget: std::time::Duration::from_secs(cfg.codex.budget_secs.max(60)),
        env_passthrough: DEFAULT_ENV_PASSTHROUGH
            .iter()
            .map(|s| s.to_string())
            .collect(),
    })
    .await
    {
        Ok(c) => c,
        Err(e) => {
            // codex 拉不起来不该让这一轮交不出东西
            let _ = rounds::end_step(
                conn,
                step.id,
                StepStatus::Failed,
                &serde_json::json!({}),
                &format!("{e:#}"),
            );
            tracing::warn!(原因 = %format!("{e:#}"), "codex 没拉起来，这一轮不深核");
            return vec![];
        }
    };

    let targets: Vec<Target> = picked
        .iter()
        .map(|j| {
            let c = by_key.get(&j.candidate_key);
            Target {
                candidate_key: j.candidate_key.clone(),
                title: if j.headline.trim().is_empty() {
                    j.three_sentences.what.clone()
                } else {
                    j.headline.clone()
                },
                url: c.map(|c| c.url.clone()).unwrap_or_default(),
                text: c.map(|c| c.text.clone()).unwrap_or_default(),
                images: local_images(prepared, &j.candidate_key, &cfg.blob_dir()),
                verdict_summary: summarize(j),
                // 只给「缺什么」：带级别前缀的话，深核照抄回来就对不上、会被当成新缺口再加一遍
                gaps: j.gaps.iter().map(|g| g.what.clone()).collect(),
            }
        })
        .collect();

    let outcomes = deep::check_batch(&codex, &targets, standard, cfg.codex.parallel.max(1)).await;
    let _ = codex.shutdown().await;

    let done = outcomes.iter().filter(|o| o.done()).count();
    let _ = rounds::end_step(
        conn,
        step.id,
        if done == outcomes.len() {
            StepStatus::Succeeded
        } else {
            StepStatus::Partial
        },
        &serde_json::json!({"深核": outcomes.len(), "做成": done}),
        "",
    );
    outcomes
}

/// 这条候选的图在本地哪儿。**app-server 不接受远程地址**，只能给绝对路径。
///
/// 路径按内容哈希算（图是按哈希存的），所以跨账号转载的同一张图只有一份。
fn local_images(prepared: &[Prepared], key: &str, blob_dir: &std::path::Path) -> Vec<PathBuf> {
    prepared
        .iter()
        .find(|p| p.candidate.candidate_key == key)
        .map(|p| {
            p.candidate
                .media
                .iter()
                .filter_map(|m| m.blake3.as_deref())
                .map(|h| csw_collector_harvest::download::blob_path(blob_dir, h))
                // 下载失败的那几张没有哈希，也就不在这里；本来也没什么可看的
                .filter(|path| path.exists())
                .collect()
        })
        .unwrap_or_default()
}

/// 把结论与依据拼成一段，注入深核线程。
fn summarize(j: &Judgement) -> String {
    let dims = j
        .dims
        .iter()
        .map(|(d, dj)| format!("{d:?}={:?}（{}）", dj.verdict, dj.basis))
        .collect::<Vec<_>>()
        .join("；");
    format!(
        "结论：{:?}\n标题：{}\n六维：{dims}\n三句话（是什么 / 为什么值得看 / 依据）：{} / {} / {}",
        j.tier,
        j.headline,
        j.three_sentences.what,
        j.three_sentences.why_worth,
        j.three_sentences.grounds
    )
}

/// 第 9 步：自查。**报红也要往下走**，把红灯写进交付物的缺口。
pub async fn self_check(
    conn: &Connection,
    round: &Round,
    engine: &EngineClient,
    run_id: i64,
) -> Option<IntakeCheckResult> {
    let step = rounds::begin_step(
        conn,
        round.id,
        StepCode::SelfCheck,
        &round.instructions_hash,
    )
    .ok()?;
    match engine.intake_check(run_id).await {
        Ok(r) => {
            let _ = rounds::end_step(
                conn,
                step.id,
                if r.ok {
                    StepStatus::Succeeded
                } else {
                    StepStatus::Partial
                },
                &serde_json::json!({"判据": r.checks.len(), "红": r.failed, "黄": r.warned}),
                "",
            );
            Some(r)
        }
        Err(e) => {
            // 查不到不等于没问题，也不等于有问题。如实记一笔，交付物里说明「自查没跑成」
            let _ = rounds::end_step(
                conn,
                step.id,
                StepStatus::Failed,
                &serde_json::json!({}),
                &format!("{e:#}"),
            );
            tracing::warn!(原因 = %format!("{e:#}"), "自查没跑成");
            None
        }
    }
}

/// 自查结果里没过的那几条，写进交付物的缺口。
pub fn check_gaps(r: Option<&IntakeCheckResult>) -> Vec<String> {
    match r {
        None => vec!["自查没跑成（引擎不可达或接口未部署），这一轮的对账没有做".into()],
        Some(r) if r.ok => vec![],
        Some(r) => r
            .checks
            .iter()
            .filter(|c| !c.ok && !c.skipped)
            .map(|c| {
                format!(
                    "自查未过：{}{}",
                    c.name,
                    if c.detail.is_empty() {
                        String::new()
                    } else {
                        format!("——{}", c.detail)
                    }
                )
            })
            .collect(),
    }
}

/// 第 7 步：做交付物。**zip 只构建一次**，字节先落盘再进 outbox。
#[allow(clippy::too_many_arguments)]
pub fn build_deliverable(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    task_id: i64,
    judgements: &[Judgement],
    topics: &[csw_collector_core::types::Topic],
    sweeps: &[SweepCount],
    by_key: &HashMap<String, Candidate>,
    extra_gaps: &[String],
) -> Result<pack::Built> {
    let step = rounds::begin_step(conn, round.id, StepCode::Build, &round.instructions_hash)?;
    let lookup = |k: &str| by_key.get(k).cloned();

    let mut body =
        intake::ledger_body((&round.window_start, &round.window_end), judgements, lookup);
    // 选题一览放在逐帖台账前面：推荐位按选题算，同一个对象不让主编读好几遍
    let section = intake::topics_section(topics, judgements, lookup);
    if !section.is_empty() {
        body = match body.find("\n## ") {
            Some(i) => format!("{}\n{section}{}", &body[..i], &body[i..]),
            None => format!("{body}\n{section}"),
        };
    }
    if !extra_gaps.is_empty() {
        // 红灯写在最前面：藏在末尾等于没写
        body = format!(
            "> **这一轮的自查有没过的判据**\n>\n{}\n\n{body}",
            extra_gaps
                .iter()
                .map(|g| format!("> - {g}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let sweep_lines: Vec<trace::SweepLine> = sweeps
        .iter()
        .map(|s| trace::SweepLine {
            sweep_key: s.sweep_key.clone(),
            query: s.query.clone(),
            found: s.found as usize,
            fetched_unique: s.fetched_unique as usize,
            in_window: s.in_window as usize,
            result: s.result.clone(),
            error: s.error.clone(),
        })
        .collect();

    let entries = intake::assemble(
        Meta {
            task: format!(
                "主编 · r{} 任务#{task_id} 派工单",
                round.run_id.unwrap_or(0)
            ),
            kind: "产出".into(),
            agent: "情报收集员".into(),
            stage: "01-情报逐条".into(),
            version: "v1".into(),
            // **不用当前时间**：窗口终点是确定的，用它才能让 zip 两次构建一致
            at: round.window_end.clone(),
            upstreams: vec![format!(
                "主编 · r{} 任务#{task_id} 派工单",
                round.run_id.unwrap_or(0)
            )],
            status: "待审".into(),
            checks: self_checks(judgements, extra_gaps),
            item_key: String::new(),
        },
        body,
        &[],
        trace::to_jsonl(&sweep_lines)?,
        trace::items_jsonl(judgements, lookup)?,
    );

    let name = format!("情报逐条_情报收集员_r{}_v1", round.run_id.unwrap_or(0));
    let out = cfg
        .data_dir
        .join("deliverables")
        .join(format!("{name}.zip"));
    let built = pack::build(&name, &entries, &out)?;

    conn.execute(
        "INSERT INTO deliverables_local(round_id, deliv_kind, item_key, zip_path, zip_sha256,
                                        zip_bytes, idem_key, created_at)
         VALUES (?1,'产出','',?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING",
        params![
            round.id,
            built.path.to_string_lossy(),
            built.sha256,
            built.bytes as i64,
            built.idem_key(task_id),
            jiff::Timestamp::now().to_string(),
        ],
    )?;
    rounds::end_step(
        conn,
        step.id,
        StepStatus::Succeeded,
        &serde_json::json!({"字节": built.bytes, "sha": built.sha256}),
        "",
    )?;
    Ok(built)
}

/// 交付物头里的自检逐条。**如实写**——写成「全部通过」而实际没有，
/// 主编会按它来判断要不要细看。
fn self_checks(judgements: &[Judgement], extra_gaps: &[String]) -> Vec<String> {
    let n = |t: Tier| judgements.iter().filter(|j| j.tier == t).count();
    let unseen = judgements.iter().filter(|j| !j.image_seen).count();
    let mut v = vec![
        format!("窗口内 {} 条全部判过，无抽样、无 top K", judgements.len()),
        format!(
            "四档：推荐 {}、备选 {}、待核 {}、不推荐 {}",
            n(Tier::Recommend),
            n(Tier::Alternate),
            n(Tier::PendingCheck),
            n(Tier::NotRecommend)
        ),
        format!("未读到实图 {unseen} 条，已落待核（不是淘汰）"),
    ];
    v.extend(extra_gaps.iter().cloned());
    v
}

/// 第 10 步：提交。**只发已落盘的那一份 zip**。
pub async fn submit(
    conn: &Connection,
    round: &Round,
    engine: &EngineClient,
    task_id: i64,
    built: &pack::Built,
) -> Result<()> {
    let step = rounds::begin_step(conn, round.id, StepCode::Submit, &built.sha256)?;
    let input = SubmitInput {
        task_id,
        kind: csw_collector_engineapi::types::DeliverableKind::Output,
        zip_path: built.path.clone(),
        file_name: built
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "deliverable.zip".into()),
        idem_key: built.idem_key(task_id),
        note: String::new(),
        affects_deliverable_id: None,
        item_key: String::new(),
    };
    match engine.submit(&input).await {
        Ok(resp) => {
            let id = resp.get("id").and_then(serde_json::Value::as_i64);
            conn.execute(
                "UPDATE deliverables_local SET engine_id=?2 WHERE round_id=?1 AND zip_sha256=?3",
                params![round.id, id, built.sha256],
            )?;
            rounds::end_step(
                conn,
                step.id,
                StepStatus::Succeeded,
                &serde_json::json!({"交付物": id}),
                "",
            )?;
            Ok(())
        }
        Err(e) => {
            let why = format!("{e:#}");
            rounds::end_step(
                conn,
                step.id,
                StepStatus::Failed,
                &serde_json::json!({}),
                &why,
            )?;
            // 幂等键不确定时**绝不换键重发**，上层要去查任务状态对账
            anyhow::bail!("提交失败：{why}")
        }
    }
}

/// 深核结果里的缺口，合进条目的缺口。
pub fn deepcheck_gaps(outcomes: &[deep::Outcome]) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for o in outcomes {
        let mut gaps = Vec::new();
        if let Some(note) = o.gap_note() {
            gaps.push(note);
        }
        if let Some(card) = &o.card {
            gaps.extend(card.gaps.iter().cloned());
        }
        if !gaps.is_empty() {
            m.insert(o.candidate_key.clone(), gaps);
        }
    }
    m
}

/// 把深核补出来的缺口并进条目的缺口。
///
/// **深核的缺口必须出现在台账里**：那是「这条还差什么」的最新一版，
/// 只留在深核的线程日志里等于没人看得见。深核不推翻结论，
/// 它补出来的都算「影响成稿」，由收集员接着补。
pub fn merge_deepcheck_gaps(
    judgements: &mut [Judgement],
    deep_gaps: &HashMap<String, Vec<String>>,
) -> usize {
    let mut n = 0;
    for j in judgements.iter_mut() {
        if let Some(gs) = deep_gaps.get(&j.candidate_key) {
            for g in gs {
                // 深核可能把上一步已经写过的缺口再说一遍，不重复写
                if !j.gaps.iter().any(|x| x.what == *g) {
                    j.gaps.push(Gap {
                        level: GapLevel::Production,
                        what: g.clone(),
                        owner: GapOwner::Collector,
                        tried: "深核".into(),
                        next: "成稿前补齐".into(),
                    });
                    n += 1;
                }
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(key: &str, tier: Tier) -> Judgement {
        let mut j = Judgement::fixture(key, tier);
        j.three_sentences.what = "换了背板".into();
        j.image_seen = tier != Tier::PendingCheck;
        j.inputs_hash = "ih".into();
        j
    }

    #[test]
    fn 主编指定的首批排在自动挑法前面() {
        let js = [
            j("n1", Tier::NotRecommend),
            j("r1", Tier::Recommend),
            j("r2", Tier::Recommend),
        ];
        // 他看完台账说要核不推荐那条，就核那条——不看档
        let picked: Vec<&str> = first_batch(&js, &["n1".into()], 1, 1)
            .iter()
            .map(|x| x.candidate_key.as_str())
            .collect();
        assert_eq!(picked, ["n1", "r1"]);

        // 指定的已经在推荐里，不该出现两遍
        let picked: Vec<&str> = first_batch(&js, &["r2".into()], 2, 1)
            .iter()
            .map(|x| x.candidate_key.as_str())
            .collect();
        assert_eq!(picked, ["r2", "r1",]);

        // 指定得比上限还多时，上限说了算：深核一条要几分钟
        let picked = first_batch(&js, &["n1".into(), "r1".into(), "r2".into()], 1, 1);
        assert_eq!(picked.len(), 2);

        // 指定了一条这一轮没有的，忽略它，别把自动挑法也带崩
        let picked: Vec<&str> = first_batch(&js, &["翻篇了".into()], 1, 0)
            .iter()
            .map(|x| x.candidate_key.as_str())
            .collect();
        assert_eq!(picked, ["r1"]);
    }

    #[test]
    fn 待核先核_推荐另有名额_不要不推荐和备选() {
        let js = [
            j("n1", Tier::NotRecommend),
            j("r1", Tier::Recommend),
            j("p1", Tier::PendingCheck),
            j("r2", Tier::Recommend),
            j("a1", Tier::Alternate),
            j("p2", Tier::PendingCheck),
        ];
        let picked: Vec<&str> = first_batch(&js, &[], 1, 5)
            .iter()
            .map(|x| x.candidate_key.as_str())
            .collect();
        // 待核的全进（深核过的不再待核），推荐按名额；不推荐、备选不进
        assert_eq!(picked, ["p1", "p2", "r1"]);
        // 推荐再多也不挤掉待核
        let js2 = [
            j("r1", Tier::Recommend),
            j("r2", Tier::Recommend),
            j("p1", Tier::PendingCheck),
        ];
        let picked: Vec<&str> = first_batch(&js2, &[], 2, 1)
            .iter()
            .map(|x| x.candidate_key.as_str())
            .collect();
        assert_eq!(picked, ["p1", "r1", "r2"]);
    }

    #[test]
    fn 条目卡拼成一段() {
        let c = deep::Card {
            original_source: "品牌官网".into(),
            facts: vec!["重量 995 克".into()],
            gaps: vec!["内部尺寸未公开".into()],
            ..Default::default()
        };
        let t = card_text(&c);
        assert!(t.contains("原始来源：品牌官网"));
        assert!(t.contains("- 重量 995 克"));
        assert!(t.contains("仍缺"));
        assert!(!t.contains("原始披露时间"), "空的不写");
    }

    #[test]
    fn 自查没跑成与没过是两回事() {
        // 查不到不等于没问题，也不等于有问题
        let none = check_gaps(None);
        assert_eq!(none.len(), 1);
        assert!(none[0].contains("自查没跑成"));

        let ok = IntakeCheckResult {
            checks: vec![],
            failed: 0,
            warned: 0,
            ok: true,
        };
        assert!(check_gaps(Some(&ok)).is_empty());

        let bad = IntakeCheckResult {
            checks: vec![
                csw_collector_engineapi::types::IntakeCheck {
                    name: "每条都判".into(),
                    detail: "3 条未判".into(),
                    ok: false,
                    warn: false,
                    skipped: false,
                },
                csw_collector_engineapi::types::IntakeCheck {
                    name: "跳过的".into(),
                    detail: String::new(),
                    ok: false,
                    warn: false,
                    skipped: true,
                },
            ],
            failed: 1,
            warned: 0,
            ok: false,
        };
        let g = check_gaps(Some(&bad));
        assert_eq!(g.len(), 1, "跳过的不算没过");
        assert!(g[0].contains("每条都判——3 条未判"), "{}", g[0]);
    }

    #[test]
    fn 深核的缺口要并进台账() {
        let mut js = [j("k1", Tier::Recommend), j("k2", Tier::Recommend)];
        js[0].gaps.push(Gap::decision("缺发售日期"));
        let mut deep = HashMap::new();
        deep.insert(
            "k1".to_string(),
            vec!["缺发售日期".to_string(), "找不到原始来源".to_string()],
        );
        deep.insert("k3".to_string(), vec!["这条不在这一批里".to_string()]);

        // 只留在深核的线程日志里等于没人看得见
        assert_eq!(merge_deepcheck_gaps(&mut js, &deep), 1);
        let whats: Vec<&str> = js[0].gaps.iter().map(|g| g.what.as_str()).collect();
        assert_eq!(whats, ["缺发售日期", "找不到原始来源"]);
        assert_eq!(js[0].gaps[1].level, GapLevel::Production);
        assert_eq!(js[0].gaps[1].tried, "深核");
        assert!(js[1].gaps.is_empty());
    }

    #[test]
    fn 自检逐条如实写() {
        let js = [
            j("r1", Tier::Recommend),
            j("p1", Tier::PendingCheck),
            j("n1", Tier::NotRecommend),
        ];
        let c = self_checks(&js, &["自查未过：每条都判".into()]);
        assert!(c[0].contains("3 条全部判过"));
        assert!(c[1].contains("推荐 1、备选 0、待核 1、不推荐 1"));
        assert!(c[2].contains("未读到实图 1 条"));
        // 红灯也进自检那一栏，写成「全部通过」而实际没有会让主编误判
        assert!(c[3].contains("自查未过"));
    }
}
