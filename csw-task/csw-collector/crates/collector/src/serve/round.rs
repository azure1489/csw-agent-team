//! 一轮十步，从接单到提交。
//!
//! # 每一步都进 `round_steps`，且都带输入指纹
//!
//! 指纹是复用与续跑的唯一依据。预取轮先跑过第 2–5 步，正式轮拿同样的指纹
//! 一比就知道能不能省下这四十分钟；进程崩了重启，拿指纹一比就知道哪几步
//! 还得重来。**没有指纹的步等于每次都要重跑**，所以每一步都要给。
//!
//! # 失败的粒度
//!
//! - **必扫采集器失败** → 整轮停在第 2 步告警，**不用旧接口兜底**。
//!   兜底会让「这一轮到底扫没扫全」变成一个没人能回答的问题。
//! - **一批判断失败** → 那一批标「未判」，其余照走。台账会如实少几条，
//!   `intake-check` 会因此报红——这是对的，不该被抹平。
//!   - **深核失败** → 条目照样登记，只在缺口里写明。
//! - **提交冲突** → 停下交给人，不换幂等键重试。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::Connection;

use csw_collector_core::rounds::{self, Round};
use csw_collector_core::types::{Candidate, Judgement, StepCode, StepStatus, Timestamp};
use csw_collector_core::{Config, ledger};
use csw_collector_engineapi::types::TaskDetail;
use csw_collector_harvest::collector::{CswWindow, VanLinks};
use csw_collector_harvest::csw::CswClient;
use csw_collector_harvest::download::Downloader;
use csw_collector_harvest::pipeline::{self, CollectorDyn, Prepared, SweepCount};
use csw_collector_judge::rubric::RUBRIC_VERSION;

use super::register;

/// 一轮跑完的账。写进 `round_steps.counts_json`，也给总览页。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RoundCounts {
    pub candidates: usize,
    pub photos: usize,
    pub image_seen: usize,
    pub judged: usize,
    pub unjudged: usize,
    pub recommend: usize,
    pub alternate: usize,
    pub pending_check: usize,
    pub not_recommend: usize,
    pub deepchecked: usize,
}

/// 第 2 步「采集媒体信息」的输入指纹。
///
/// 覆盖窗口、采集方案版本、识别提示词版本。**不覆盖时间**——
/// 覆盖了的话同一个窗口每次算出来的指纹都不同，复用就永远命中不了。
pub fn harvest_hash(window: (&str, &str), plan_version: i64, recognize_version: &str) -> String {
    let mut h = blake3::Hasher::new();
    for s in [window.0, window.1, recognize_version] {
        h.update(s.as_bytes());
        h.update(b"\x1f");
    }
    h.update(&plan_version.to_le_bytes());
    h.finalize().to_hex().to_string()
}

/// 第 2 步：取候选 → 下载 → 识别 → 向量化。四段连着做完才算这步完成。
pub async fn harvest(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    csw: Arc<CswClient>,
    downloader: &Downloader,
    model: &csw_collector_core::model::ModelClient,
    vector: &csw_collector_core::vector::VectorClient,
    van_links: &[String],
) -> Result<(Vec<Prepared>, Vec<SweepCount>)> {
    let hash = harvest_hash(
        (&round.window_start, &round.window_end),
        1,
        csw_collector_harvest::recognize::PROMPT_VERSION,
    );
    let step = rounds::begin_step(conn, round.id, StepCode::Harvest, &hash)?;

    let from: Timestamp = round.window_start.parse().context("窗口起点")?;
    let to: Timestamp = round.window_end.parse().context("窗口终点")?;
    let window = CswWindow {
        client: csw.clone(),
        lookback_days: csw_collector_harvest::csw::WINDOW_LOOKBACK_DAYS,
    };
    let van = VanLinks {
        client: csw.clone(),
        short_codes: van_links.to_vec(),
    };
    let collectors: Vec<&dyn CollectorDyn> = vec![&window, &van];
    // 只取图文是总方案定死的，不是开关
    let (cands, sweeps, blocked) = pipeline::collect_all(&collectors, from, to, true).await;

    if let Some(why) = blocked {
        // 必扫失败不兜底：兜底会让「这一轮到底扫没扫全」没人能回答
        rounds::end_step(
            conn,
            step.id,
            StepStatus::Failed,
            &serde_json::json!({"候选": cands.len()}),
            &why,
        )?;
        anyhow::bail!("{why}");
    }

    for c in &cands {
        ledger::upsert_candidate(conn, c)?;
        ledger::attach_candidate(conn, round.id, &c.candidate_key, &c.collector, false)?;
    }

    let (prepared, stats) = pipeline::prepare(
        cands,
        &pipeline::Deps {
            downloader,
            model,
            vector,
            image_only: true,
            concurrency: cfg.model.concurrency,
        },
    )
    .await;

    let seen = prepared.iter().filter(|p| p.image_seen()).count();
    let counts = serde_json::json!({
        "候选": prepared.len(),
        "读到实图": seen,
        "下载毫秒": stats.download_ms,
        "识别向量墙钟毫秒": stats.wall_ms,
        "网关占用毫秒": stats.recognize_ms,
        "GPU占用毫秒": stats.embed_ms,
    });
    // 有图没处理成功的算 partial：它们会落待核，不阻塞下游
    let any_failed = prepared.iter().any(|p| !p.failed_media.is_empty());
    rounds::end_step(
        conn,
        step.id,
        if any_failed {
            StepStatus::Partial
        } else {
            StepStatus::Succeeded
        },
        &counts,
        "",
    )?;
    Ok((prepared, sweeps))
}

/// 第 8–10 步：登记 → 自查 → 提交。
///
/// 自查**先看结果再决定要不要提交**：`intake-check` 报红时照样提交，
/// 等于把自查变成摆设。但也不能就此不交——主编要看到东西才能处置。
/// 所以：红灯时**照交，并把红灯写进交付物的缺口**，让人一眼看见。
pub fn register_and_enqueue(
    conn: &Connection,
    round: &Round,
    run_id: i64,
    judgements: &[Judgement],
    sweeps: &[SweepCount],
    by_key: &HashMap<String, Candidate>,
    carried: &HashSet<String>,
) -> Result<i64> {
    let step = rounds::begin_step(conn, round.id, StepCode::Register, &round.instructions_hash)?;
    let unjudged = ledger::unjudged_keys(conn, round.id)?.len();
    let lookup = |k: &str| by_key.get(k).cloned();

    let items = register::item_inputs(judgements, lookup);
    let sweep_inputs = register::sweep_inputs(
        sweeps,
        (&round.window_start, &round.window_end),
        judgements.len(),
        unjudged,
    );
    let jis = register::judgement_inputs(judgements, lookup, carried, RUBRIC_VERSION)?;
    let last = register::enqueue_registration(conn, round.id, run_id, &items, &sweep_inputs, &jis)?;

    rounds::end_step(
        conn,
        step.id,
        StepStatus::Succeeded,
        &serde_json::json!({
            "条目": items.len(),
            "采集轮": sweep_inputs.len(),
            "判断": jis.len(),
            "未判": unjudged,
        }),
        "",
    )?;
    Ok(last)
}

/// 统计这一轮的账。
pub fn count_round(conn: &Connection, round_id: i64, prepared: &[Prepared]) -> Result<RoundCounts> {
    let tiers: HashMap<String, usize> = ledger::tier_counts(conn, round_id)?.into_iter().collect();
    let (total, _carried) = ledger::round_candidate_count(conn, round_id)?;
    Ok(RoundCounts {
        candidates: total,
        photos: prepared
            .iter()
            .map(|p| {
                p.candidate
                    .media
                    .iter()
                    .filter(|m| m.kind == csw_collector_core::types::MediaKind::Photo)
                    .count()
            })
            .sum(),
        image_seen: prepared.iter().filter(|p| p.image_seen()).count(),
        judged: ledger::judged_keys(conn, round_id)?.len(),
        unjudged: ledger::unjudged_keys(conn, round_id)?.len(),
        recommend: *tiers.get("recommend").unwrap_or(&0),
        alternate: *tiers.get("alternate").unwrap_or(&0),
        pending_check: *tiers.get("pending_check").unwrap_or(&0),
        not_recommend: *tiers.get("not_recommend").unwrap_or(&0),
        deepchecked: 0,
    })
}

/// 任务下发的三段作业标准，原样拼给模型。
pub fn work_standard(t: &TaskDetail) -> String {
    [
        ("作业内容", t.instructions.as_str()),
        ("自检", t.self_check_criteria.as_str()),
        ("验收", t.acceptance.as_str()),
        ("派工单备注", t.editor_note.as_str()),
    ]
    .iter()
    .filter(|(_, v)| !v.trim().is_empty())
    .map(|(k, v)| format!("【{k}】\n{}", v.trim()))
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// 下载器按配置建一个。
pub fn downloader(cfg: &Config) -> Result<Downloader> {
    Downloader::new(csw_collector_harvest::download::DownloadConfig {
        dir: cfg.blob_dir(),
        width: cfg.csw.thumb_width,
        concurrency: cfg.limits.download_concurrency,
        timeout: Duration::from_secs(60),
        max_attempts: 3,
    })
}

/// 跑一轮 01「情报逐条」，从已经接了的单开始。
///
/// 这里只到**登记**为止。深核、交付物与提交挂在登记确认之后
/// （`intake-check` 要拿台账与采集轮对账，判断没登记就查，查出来的是假红灯）。
pub async fn run_intake(
    conn: &Connection,
    round: &Round,
    detail: &TaskDetail,
    cfg: &Config,
    svc: &super::services::Services,
) -> Result<RoundCounts> {
    let standard = work_standard(detail);

    // 二、采集媒体信息
    let (prepared, sweeps) = harvest(
        conn,
        round,
        cfg,
        svc.csw.clone(),
        &svc.downloader,
        &svc.model,
        &svc.vector,
        &[],
    )
    .await?;

    // 三～五、合并、对照、逐条判断
    let step = rounds::begin_step(conn, round.id, StepCode::Judge, &round.instructions_hash)?;
    let items = judge_items(conn, &prepared, svc).await?;
    let outcome = csw_collector_judge::pipeline::run(
        &items,
        &csw_collector_judge::pipeline::Deps {
            model: &svc.model,
            jev: svc.jev.as_ref(),
            work_standard: &standard,
            batch_concurrency: cfg.model.concurrency,
            on_batch: None,
        },
    )
    .await;

    for j in &outcome.judgements {
        let flags: Vec<String> = outcome
            .flags
            .iter()
            .filter(|f| f.candidate_key == j.candidate_key)
            .map(|f| f.note())
            .collect();
        // 一条写不进去不该让整轮停下——它会留在「没判」里被自查抓到
        if let Err(e) =
            ledger::put_judgement(conn, round.id, j, &flags, &cfg.model.model, RUBRIC_VERSION)
        {
            tracing::warn!(候选 = %j.candidate_key, 原因 = %format!("{e:#}"), "这条判断没落库");
        }
    }
    rounds::end_step(
        conn,
        step.id,
        if outcome.unjudged.is_empty() {
            StepStatus::Succeeded
        } else {
            StepStatus::Partial
        },
        &serde_json::json!({
            "判了": outcome.judgements.len(),
            "未判": outcome.unjudged.len(),
            "合并成事件": outcome.groups.len(),
            "被标出的依据": outcome.flags.len(),
        }),
        &outcome.unjudged.join("、"),
    )?;

    // 八、登记
    let by_key: HashMap<String, Candidate> = prepared
        .iter()
        .map(|p| (p.candidate.candidate_key.clone(), p.candidate.clone()))
        .collect();
    let carried: HashSet<String> = HashSet::new();
    if let Some(run_id) = round.run_id {
        register_and_enqueue(
            conn,
            round,
            run_id,
            &outcome.judgements,
            &sweeps,
            &by_key,
            &carried,
        )?;
    } else {
        tracing::info!("手动轮不写引擎，只落本地");
    }

    count_round(conn, round.id, &prepared)
}

/// 把采集产物与检索结果拼成判断那一步要的输入。
async fn judge_items<'a>(
    conn: &Connection,
    prepared: &'a [Prepared],
    svc: &'a super::services::Services,
) -> Result<Vec<csw_collector_judge::pipeline::Item<'a>>> {
    use csw_collector_kb::search::{Query, Retriever};

    let mut out = Vec::with_capacity(prepared.len());
    for p in prepared {
        let text = judge_text(p);
        let retrieved = Retriever {
            conn,
            store: &svc.store,
            brands: &svc.brands,
            tok: &svc.tok,
            reranker: Some(&svc.vector),
        }
        .search(&Query {
            text: &text,
            vector: p.fused.as_deref(),
            exclude_post_id: Some(p.candidate.source_id.clone()),
            limit: csw_collector_kb::search::FINAL_MAX,
            // 判断受「五类缺一不判」约束，某一类整体缺席会把条目卡成待核
            backfill_kinds: true,
        })
        .await
        .unwrap_or_default();

        let materials = csw_collector_judge::materials::assemble(&retrieved, &[]);
        let images_b64 = csw_collector_judge::verdict::pick_images(&p.descriptions, |_| None);
        out.push(csw_collector_judge::pipeline::Item {
            candidate: &p.candidate,
            descriptions: &p.descriptions,
            images_b64,
            materials,
            fused: p.fused.as_deref(),
            image_seen: p.image_seen(),
            image_gap: p.failed_media.join("；"),
            heat_note: csw_collector_judge::order::heat_note(&p.candidate, None),
        });
    }
    Ok(out)
}

/// 检索用的查询文本：正文 + 译文 + 每张图的描述。
///
/// 只用正文的话，很多贴文只有一句话加一串话题，检索找不着它——
/// 真正的信息在图里。
fn judge_text(p: &Prepared) -> String {
    let mut s = p.candidate.text.clone();
    if !p.candidate.translated.trim().is_empty() {
        s.push('\n');
        s.push_str(&p.candidate.translated);
    }
    for d in &p.descriptions {
        s.push('\n');
        s.push_str(&d.content);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 采集指纹不含时间() {
        let a = harvest_hash(("A", "B"), 1, "recognize/v1");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        // 含时间的话同一个窗口每次算出来都不同，复用永远命中不了
        assert_eq!(a, harvest_hash(("A", "B"), 1, "recognize/v1"));
        // 窗口、方案版本、识别提示词版本任一变了就要变
        assert_ne!(a, harvest_hash(("A", "C"), 1, "recognize/v1"));
        assert_ne!(a, harvest_hash(("A", "B"), 2, "recognize/v1"));
        assert_ne!(a, harvest_hash(("A", "B"), 1, "recognize/v2"));
    }

    #[test]
    fn 作业标准原样拼不改写() {
        let t = TaskDetail {
            task: csw_collector_engineapi::types::Task {
                id: 1,
                run_id: 48,
                stage_code: "intake".into(),
                stage_name: String::new(),
                role_code: "collector".into(),
                status: csw_collector_engineapi::types::TaskStatus::Dispatched,
                item_key: String::new(),
                action_class: String::new(),
                due_at: String::new(),
                dispatched_at: String::new(),
                cur_version: 1,
                sla_minutes: 30,
                rework_pending: false,
            },
            instructions: "每条都判，不设 top K".into(),
            self_check_criteria: "窗口内条数与库内一致".into(),
            acceptance: "".into(),
            editor_note: "本期 Van 补了两条链接".into(),
            upstreams: vec![],
            latest_review: None,
            item: None,
        };
        let s = work_standard(&t);
        assert!(s.contains("【作业内容】\n每条都判，不设 top K"));
        assert!(s.contains("【自检】"));
        assert!(s.contains("【派工单备注】"));
        // 空的那段不留一个空标题
        assert!(!s.contains("【验收】"), "{s}");
    }
}
