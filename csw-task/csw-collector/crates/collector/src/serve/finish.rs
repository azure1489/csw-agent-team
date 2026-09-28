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
    deepcheck_these(
        conn, round, cfg, step.id, picked, by_key, prepared, standard, true,
    )
    .await
}

/// 深核指定的这几条。`reuse` = 36 小时内核过的直接用那张条目卡；返工时传 `false`——
/// 主编退回点名要核的事，旧条目卡里没有（09-28 r56：「不能机械拼接旧深核结果」）。
#[allow(clippy::too_many_arguments)]
pub async fn deepcheck_these(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    step_id: i64,
    picked: Vec<&Judgement>,
    by_key: &HashMap<String, Candidate>,
    prepared: &[Prepared],
    standard: &str,
    reuse: bool,
) -> Vec<deep::Outcome> {
    #[derive(Clone, Copy)]
    struct Step {
        id: i64,
    }
    let step = Step { id: step_id };
    // 36 小时内核过、做成了的直接用那张条目卡：预取轮 01:40 核过的，05:30 正式轮不再花时间，
    // 条目卡一样、重判的指纹也一样，结论也就直接复用（09-27：深核后重判占了 17 分钟）
    let mut reused: Vec<deep::Outcome> = Vec::new();
    let picked: Vec<&Judgement> = picked
        .into_iter()
        .filter(
            |j| match recent_card(conn, &j.candidate_key, 36).filter(|_| reuse) {
                Some((from, card)) => {
                    reused.push(deep::Outcome {
                        candidate_key: j.candidate_key.clone(),
                        status: "done".into(),
                        card: Some(card),
                        items: vec![],
                        note: format!("复用第 {from} 轮的深核"),
                        attempts: 0,
                    });
                    false
                }
                None => true,
            },
        )
        .collect();
    for o in &reused {
        save_deepcheck(conn, round.id, o);
    }
    if picked.is_empty() && !reused.is_empty() {
        let _ = rounds::end_step(
            conn,
            step.id,
            StepStatus::Succeeded,
            &serde_json::json!({"深核": reused.len(), "做成": reused.len(), "复用": reused.len()}),
            "",
        );
        return reused;
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

    let mut outcomes =
        deep::check_batch(&codex, &targets, standard, cfg.codex.parallel.max(1)).await;
    let _ = codex.shutdown().await;
    for o in &outcomes {
        save_deepcheck(conn, round.id, o);
    }
    let fresh = outcomes.len();
    outcomes.extend(reused);

    let done = outcomes.iter().filter(|o| o.done()).count();
    let _ = rounds::end_step(
        conn,
        step.id,
        if done == outcomes.len() {
            StepStatus::Succeeded
        } else {
            StepStatus::Partial
        },
        &serde_json::json!({"深核": outcomes.len(), "做成": done, "复用": outcomes.len() - fresh}),
        "",
    );
    outcomes
}

/// 一条候选落过库的全部图，按图序，文件名 `候选键_N`（N 从 1 起，与「第 N 张」对上）。
fn all_previews(conn: &Connection, cfg: &Config, key: &str) -> Vec<intake::Preview> {
    let Ok(mut st) = conn.prepare(
        "SELECT blake3, ordinal FROM media WHERE candidate_key = ?1 AND failed = 0 ORDER BY ordinal",
    ) else {
        return Vec::new();
    };
    let rows: Vec<(String, i64)> = st
        .query_map(params![key], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|(hash, ordinal)| {
            let path = csw_collector_harvest::download::blob_path(&cfg.blob_dir(), &hash);
            let bytes = std::fs::read(&path).ok()?;
            Some(intake::Preview {
                candidate_key: format!("{key}_{}", ordinal + 1),
                bytes,
                ext: "jpg".into(),
            })
        })
        .collect()
}

/// 一条候选的封面缩略：能当配图的第一张，没有就第一张。取不到就不放（不让一张图拦住交付）。
fn cover_preview(conn: &Connection, cfg: &Config, key: &str) -> Option<intake::Preview> {
    let hash: String = conn
        .query_row(
            "SELECT m.blake3 FROM media m
             LEFT JOIN media_descriptions d ON d.blake3 = m.blake3
             WHERE m.candidate_key = ?1 AND m.failed = 0
             ORDER BY COALESCE(d.usable_as_figure, 0) DESC, m.ordinal LIMIT 1",
            params![key],
            |r| r.get(0),
        )
        .ok()?;
    let path = csw_collector_harvest::download::blob_path(&cfg.blob_dir(), &hash);
    let bytes = std::fs::read(&path).ok()?;
    Some(intake::Preview {
        candidate_key: key.to_string(),
        bytes,
        ext: "jpg".into(),
    })
}

/// 深核结果落库：页面的条目详情读它，下一轮复用也读它。写不进去不影响这一轮。
/// 同一轮同一条再核一次（补证、返工）就整行覆盖，`started_at` 也换成这一次的——
/// 只换 `ended_at` 的话，按开始时间查会以为新结果没落库（09-28 第 33 轮补证）。
fn save_deepcheck(conn: &Connection, round_id: i64, o: &deep::Outcome) {
    let now = jiff::Timestamp::now().to_string();
    let status = if o.done() {
        "completed"
    } else if o.status == "timeout" {
        "interrupted"
    } else {
        "failed"
    };
    let result = serde_json::json!({
        "card": o.card,
        "note": o.note,
        "attempts": o.attempts,
    });
    if let Err(e) = conn.execute(
        "INSERT INTO deepchecks(round_id, candidate_key, status, result_json, started_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(round_id, candidate_key) DO UPDATE SET
           status = excluded.status, result_json = excluded.result_json,
           started_at = excluded.started_at, ended_at = excluded.ended_at",
        params![round_id, o.candidate_key, status, result.to_string(), now],
    ) {
        tracing::warn!(候选 = %o.candidate_key, "深核结果没落库：{e:#}");
    }
}

/// 这条最近 `hours` 小时内做成了的深核：(哪一轮, 条目卡)。
pub fn recent_card(conn: &Connection, key: &str, hours: i64) -> Option<(i64, deep::Card)> {
    let since = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_hours(hours))
        .ok()?
        .to_string();
    let (round, json): (i64, String) = conn
        .query_row(
            "SELECT round_id, result_json FROM deepchecks
             WHERE candidate_key = ?1 AND status = 'completed' AND ended_at >= ?2
             ORDER BY ended_at DESC LIMIT 1",
            params![key, since],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let card: deep::Card = serde_json::from_value(v.get("card")?.clone()).ok()?;
    Some((round, card))
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

    // 预览图：推荐与备选各一张（能当配图的第一张，没有就第一张）。以前这里传的是空的，
    // 交付包里从来没有图（09-28 r56 退回：「缺 images……逐图证据不能直接复核」）
    //
    // 推荐与深核过的待核**带全部原图**：主编要按「第几张图」复核论据，只放封面等于没给
    //（09-28 r56 #615：SATISFY 引用的第 5 张图没交）。其余备选只放封面。
    let mut previews: Vec<intake::Preview> = Vec::new();
    let mut image_index: Vec<(String, Vec<String>)> = Vec::new();
    for j in judgements {
        let deep = j.gaps.iter().any(|g| g.tried.contains("深核"));
        let all = j.tier == Tier::Recommend || (j.tier == Tier::PendingCheck && deep);
        if all {
            let ps = all_previews(conn, cfg, &j.candidate_key);
            if !ps.is_empty() {
                image_index.push((
                    j.candidate_key.clone(),
                    ps.iter()
                        .map(|p| format!("images/{}.jpg", p.candidate_key))
                        .collect(),
                ));
                previews.extend(ps);
            }
        } else if j.tier == Tier::Alternate
            && let Some(p) = cover_preview(conn, cfg, &j.candidate_key)
        {
            image_index.push((
                j.candidate_key.clone(),
                vec![format!("images/{}.jpg", p.candidate_key)],
            ));
            previews.push(p);
        }
    }
    if !image_index.is_empty() {
        body.push_str("\n## 随包图片（按条目，`_N` 是原帖第 N 张）\n\n");
        for (k, files) in &image_index {
            body.push_str(&format!("- `{k}`：{}\n", files.join("、")));
        }
    }
    let version = round.target_version.max(1);
    let mut entries = intake::assemble(
        Meta {
            task: format!(
                "主编 · r{} 任务#{task_id} 派工单",
                round.run_id.unwrap_or(0)
            ),
            kind: "产出".into(),
            agent: "情报收集员".into(),
            stage: "01-情报逐条".into(),
            version: format!("v{version}"),
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
        &previews,
        trace::to_jsonl(&sweep_lines)?,
        trace::items_jsonl(judgements, lookup)?,
    );
    // 逐条判断全文：六维依据、三句话、查重命中、分级缺口都在这里，主编要能直接复核
    let mut jl = String::new();
    for j in judgements {
        jl.push_str(&serde_json::to_string(j)?);
        jl.push('\n');
    }
    entries.push(pack::Entry::text("trace/judgements.jsonl", jl));
    // 宽取的每一条落到哪儿、窗口内的每条判成什么登记成什么：主编要能从宽取的全部 ID
    // 按 first_seen_at 逐条复算到窗口内的那些（09-28 r56 退回：1431 → 128 只有两个数）
    let (ids, summary) = window_ids(conn, round.id, judgements, topics, sweeps, by_key)?;
    entries.push(pack::Entry::text("trace/window_ids.jsonl", ids));
    entries.push(pack::Entry::text(
        "trace/window_summary.json",
        serde_json::to_string_pretty(&summary)?,
    ));

    let name = format!(
        "情报逐条_情报收集员_r{}_v{version}",
        round.run_id.unwrap_or(0)
    );
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

/// `trace/window_ids.jsonl` 与 `trace/window_summary.json`。
///
/// 有宽取留痕的：宽取的每一条一行（接口返回顺序），带去向与原因；窗口内的再带档位与登记。
/// 判过、却不在留痕的「窗口内」里的（上期结转等）另起一行说明来路。
/// 没有留痕的老轮次：只列判过的，汇总里如实写「宽取未逐条落库」。
fn window_ids(
    conn: &Connection,
    round_id: i64,
    judgements: &[Judgement],
    topics: &[csw_collector_core::types::Topic],
    sweeps: &[SweepCount],
    by_key: &HashMap<String, Candidate>,
) -> Result<(String, serde_json::Value)> {
    use csw_collector_harvest::pipeline::TraceOutcome;
    let item_of = crate::serve::register::registered_item_of(topics);
    let judged: HashMap<&str, &Judgement> = judgements
        .iter()
        .map(|j| (j.candidate_key.as_str(), j))
        .collect();
    let item = |key: &str| -> serde_json::Value {
        let Some(j) = judged.get(key) else {
            return serde_json::json!({"tier": null, "item_key": null, "item_status": "未判（不应出现，出现即是缺陷）"});
        };
        let item_key = item_of.get(key).cloned();
        let item_status = match (&item_key, j.tier) {
            (None, _) => "不登记条目（只进判断台账）",
            (Some(_), Tier::PendingCheck) => "pending_check",
            (Some(_), _) if j.has_decision_gap() => "pending_check",
            (Some(_), _) => "shortlisted",
        };
        serde_json::json!({"tier": j.tier, "item_key": item_key, "item_status": item_status})
    };

    let rows = csw_collector_core::window_trace::of_round(conn, round_id).unwrap_or_default();
    let mut out = String::new();
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut in_window_keys = std::collections::HashSet::new();
    for r in &rows {
        let o = TraceOutcome::parse(&r.outcome);
        let mut line = serde_json::json!({
            "sweep_key": r.sweep_key,
            "source_id": r.source_id,
            "candidate_key": r.candidate_key,
            "account": r.account,
            "url": r.url,
            "first_seen_at": r.ingested_at,
            "posted_at": r.posted_at,
            "media": r.media,
            "outcome": r.outcome,
            "reason": o.map(|o| o.cn()).unwrap_or("未知去向"),
            "dup_of": r.dup_of,
            "trace_source": r.source,
        });
        if o == Some(TraceOutcome::InWindow) {
            in_window_keys.insert(r.candidate_key.clone());
            if let (Some(m), Some(extra)) =
                (line.as_object_mut(), item(&r.candidate_key).as_object())
            {
                m.extend(extra.clone());
            }
        }
        *counts
            .entry(o.map(|o| o.as_str()).unwrap_or("unknown"))
            .or_default() += 1;
        out.push_str(&serde_json::to_string(&line)?);
        out.push('\n');
    }
    // 判过、却不在留痕的窗口内里的：没有留痕的老轮次是全部判过的；有留痕的是结转等
    let mut extra = 0;
    for j in judgements {
        if in_window_keys.contains(&j.candidate_key) {
            continue;
        }
        extra += 1;
        let c = by_key.get(&j.candidate_key);
        let mut line = serde_json::json!({
            "sweep_key": c.map(|c| crate::serve::register::sweep_key_of(&c.collector)).unwrap_or_default(),
            "source_id": c.map(|c| c.source_id.clone()),
            "candidate_key": j.candidate_key,
            "url": c.map(|c| c.url.clone()).unwrap_or_default(),
            "first_seen_at": c.and_then(|c| c.ingested_at).map(|t| t.to_string()),
            "posted_at": c.and_then(|c| c.posted_at).map(|t| t.to_string()),
            "outcome": "judged_not_in_trace",
            "reason": if rows.is_empty() { "判过（这一轮宽取未逐条落库）" } else { "判过，但不在宽取留痕的窗口内（点名链接或上期结转）" },
        });
        if let (Some(m), Some(x)) = (line.as_object_mut(), item(&j.candidate_key).as_object()) {
            m.extend(x.clone());
        }
        out.push_str(&serde_json::to_string(&line)?);
        out.push('\n');
    }
    let source = rows.first().map(|r| r.source.as_str()).unwrap_or("none");
    let summary = serde_json::json!({
        "宽取留痕": match source {
            "live" => "当轮宽取时逐条记下",
            "refetch" => "原宽取未逐条落库；事后按同一发布时间窗口重新取数、同一规则复算（只取元数据，未重判）",
            _ => "这一轮宽取未逐条落库，只列判过的",
        },
        "trace_source": source,
        "留痕条数": rows.len(),
        "按去向": counts,
        "窗口内": in_window_keys.len(),
        "判过": judgements.len(),
        "判过但不在留痕窗口内": extra,
        "采集轮自报": sweeps.iter().map(|s| serde_json::json!({
            "sweep_key": s.sweep_key, "found": s.found, "fetched_unique": s.fetched_unique, "in_window": s.in_window,
        })).collect::<Vec<_>>(),
        "去向说明": {
            "in_window": TraceOutcome::InWindow.cn(),
            "dup_in_sweep": TraceOutcome::DupInSweep.cn(),
            "dup_across": TraceOutcome::DupAcross.cn(),
            "not_image_only": TraceOutcome::NotImageOnly.cn(),
            "before_window": TraceOutcome::BeforeWindow.cn(),
            "after_window": TraceOutcome::AfterWindow.cn(),
            "no_time": TraceOutcome::NoTime.cn(),
            "判定次序": "先去重，再筛图文，再按首次入库时间 [起, 止) 左闭右开；没有入库时间的退回按发布时间",
        },
    });
    Ok((out, summary))
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
    fn 深核结果落库且三十六小时内可复用() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        c.execute(
            "INSERT INTO rounds(id, kind, trigger, window_start, window_end, plan_version,
                                rubric_version, kb_snapshot, status, created_at)
             VALUES (7, 'prefetch', 'manual', 'a', 'b', 1, 'v', 's', 'done', 'x')",
            [],
        )
        .unwrap();
        let ok = deep::Outcome {
            candidate_key: "k1".into(),
            status: "done".into(),
            card: Some(deep::Card {
                original_source: "官网".into(),
                ..Default::default()
            }),
            items: vec![],
            note: String::new(),
            attempts: 1,
        };
        save_deepcheck(&c, 7, &ok);
        let (from, card) = recent_card(&c, "k1", 36).expect("刚核过的要拿得到");
        assert_eq!(from, 7);
        assert_eq!(card.original_source, "官网");
        // 没做成的不复用
        let bad = deep::Outcome {
            candidate_key: "k2".into(),
            status: "failed".into(),
            card: None,
            ..ok.clone()
        };
        save_deepcheck(&c, 7, &bad);
        assert!(recent_card(&c, "k2", 36).is_none());
        assert!(recent_card(&c, "没核过", 36).is_none());
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
