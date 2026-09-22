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
//!
//! # 预取轮与正式轮的关系
//!
//! 预取轮（[`run_prefetch`]）只跑第 2–5 步，产物落在本地库：图片描述进
//! `media_descriptions`，判断结论进 `judgements`。正式轮**照样从接单开始、
//! 照样重新取一次窗口**，只是逐条去查「这张图识别过吗」「这条判过吗」——
//! 命中就跳过，没命中就照常跑。
//!
//! 复用做在**条**这一级，不在**步**这一级：两轮的窗口不是同一个
//! （01:40 与 05:30 各取前 24 小时），步级指纹永远对不上，而条级的
//! 绝大多数都能对上——重叠的那二十小时里的候选一条没变。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::Connection;

use csw_collector_core::rounds::{self, Round};
use csw_collector_core::types::{
    Candidate, Judgement, MediaDescription, StepCode, StepStatus, Timestamp,
};
use csw_collector_core::{Config, ledger, media, mirror};
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
#[allow(clippy::too_many_arguments)]
pub async fn harvest(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    csw: Arc<CswClient>,
    downloader: &Downloader,
    model: &csw_collector_core::model::ModelClient,
    vector: &csw_collector_core::vector::VectorClient,
    van_links: &[String],
    caches: Caches,
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

    let cache = DescCache { conn };
    let (prepared, stats) = pipeline::prepare(
        cands,
        &pipeline::Deps {
            downloader,
            model,
            vector,
            image_only: true,
            concurrency: cfg.model.concurrency,
            // 预取轮识别过的直接拿来用。识别是这一步里最大的一块开销。
            cache: caches
                .descriptions
                .then_some(&cache as &dyn pipeline::Descriptions),
        },
    )
    .await;

    // 描述立刻落库：只留在内存里的话，这一趟网关钱下一轮还要再花一遍
    let mut stored = 0;
    for p in &prepared {
        match media::put_prepared(
            conn,
            &p.candidate.candidate_key,
            &p.candidate.media,
            &p.descriptions,
        ) {
            Ok((_, n)) => stored += n,
            // 一条存不下不该让整轮停下：它只是下一轮要重识别一次
            Err(e) => {
                tracing::warn!(候选 = %p.candidate.candidate_key, 原因 = %format!("{e:#}"), "描述没落库")
            }
        }
    }

    let seen = prepared.iter().filter(|p| p.image_seen()).count();
    let reused = prepared.iter().filter(|p| p.reused).count();
    let counts = serde_json::json!({
        "候选": prepared.len(),
        "读到实图": seen,
        "复用识别": reused,
        "新落库描述": stored,
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

/// 这一轮允许用哪些缓存。**重跑一步就是把对应的那个关掉**——
/// 复用做在条这一级，没有「把某一步作废」这回事，关掉缓存才是真正的重算。
#[derive(Debug, Clone, Copy)]
pub struct Caches {
    /// 用已有的图片描述（不重走识别）
    pub descriptions: bool,
    /// 用指纹一致的旧结论（不重问模型）
    pub judgements: bool,
}

impl Default for Caches {
    fn default() -> Self {
        Self {
            descriptions: true,
            judgements: true,
        }
    }
}

impl Caches {
    /// 从第几步起真的重算。`harvest` 连识别一起重来，`judge` 只重判。
    pub fn from_step(step: &str) -> Self {
        match step {
            "harvest" => Self {
                descriptions: false,
                judgements: false,
            },
            _ => Self {
                descriptions: true,
                judgements: false,
            },
        }
    }
}

/// 第 2–5 步：采集 → 合并 → 对照 → 逐条判断，结论落本地库。
///
/// **预取轮与正式轮走的是同一段代码。** 两边算出来的 `inputs_hash` 必须
/// 一模一样，复用才成立；分两处写迟早会分叉，而分叉的表现只是
/// 「复用永远落空、每天慢四十分钟」，不报任何错。
async fn harvest_and_judge(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    standard: &str,
    caches: Caches,
) -> Result<Judged> {
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
        caches,
    )
    .await?;

    // 三～五、合并、对照、逐条判断
    let step = rounds::begin_step(conn, round.id, StepCode::Judge, &round.instructions_hash)?;
    let items = judge_items(conn, &prepared, svc).await?;
    let cache = JudgeCache { conn };
    let outcome = csw_collector_judge::pipeline::run(
        &items,
        &csw_collector_judge::pipeline::Deps {
            model: &svc.model,
            jev: svc.jev.as_ref(),
            work_standard: standard,
            batch_concurrency: cfg.model.concurrency,
            on_batch: None,
            // 预取轮判过、且输入一点没变的，直接拿
            cached: caches
                .judgements
                .then_some(&cache as &dyn csw_collector_judge::pipeline::Cached),
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
            "其中复用": outcome.reused.len(),
            "未判": outcome.unjudged.len(),
            "合并成事件": outcome.groups.len(),
            "被标出的依据": outcome.flags.len(),
        }),
        &outcome.unjudged.join("、"),
    )?;

    Ok(Judged {
        by_key: prepared
            .iter()
            .map(|p| (p.candidate.candidate_key.clone(), p.candidate.clone()))
            .collect(),
        reused_judgements: outcome.reused.len(),
        reused_recognition: prepared.iter().filter(|p| p.reused).count(),
        prepared,
        sweeps,
        judgements: outcome.judgements,
    })
}

/// 第 2–5 步的产物。
struct Judged {
    prepared: Vec<Prepared>,
    sweeps: Vec<SweepCount>,
    judgements: Vec<Judgement>,
    by_key: HashMap<String, Candidate>,
    /// 直接拿了旧结论、没问模型的条数
    reused_judgements: usize,
    /// 直接拿了旧描述、没走网关识别的条数
    reused_recognition: usize,
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
) -> Result<(RoundCounts, Finished)> {
    let standard = work_standard(detail);
    let j = harvest_and_judge(conn, round, cfg, svc, &standard, Caches::default()).await?;
    if j.reused_recognition > 0 || j.reused_judgements > 0 {
        tracing::info!(
            复用识别 = j.reused_recognition,
            复用结论 = j.reused_judgements,
            "预取轮省下来的"
        );
    }

    // 八、登记
    let carried: HashSet<String> = HashSet::new();
    if let Some(run_id) = round.run_id {
        register_and_enqueue(
            conn,
            round,
            run_id,
            &j.judgements,
            &j.sweeps,
            &j.by_key,
            &carried,
        )?;
    } else {
        tracing::info!("手动轮不写引擎，只落本地");
    }

    let mut counts = count_round(conn, round.id, &j.prepared)?;

    // 六、深核首批。失败不抛错——条目照样登记，只在缺口里写明。
    let deep_out = super::finish::deepcheck(
        conn,
        round,
        cfg,
        &j.judgements,
        &j.by_key,
        &j.prepared,
        &standard,
    )
    .await;
    counts.deepchecked = deep_out.iter().filter(|o| o.done()).count();

    Ok((
        counts,
        Finished {
            judgements: j.judgements,
            sweeps: j.sweeps,
            by_key: j.by_key,
            deep_gaps: super::finish::deepcheck_gaps(&deep_out),
        },
    ))
}

/// 预取轮：只跑第 2–5 步，**不写引擎、不群播报、不深核、不交付**。
///
/// # 它是缓存，不是前置条件
///
/// 没跑过、跑失败了、跑到一半断了，正式轮照样能从零跑完——只是慢，
/// 而且**要如实告诉主编会迟到**，不能假装一切正常。所以这里的失败
/// 只收轮、记日志，不往引擎报（引擎根本不知道有这一轮）。
///
/// # 作业标准从上一次派单来
///
/// 01:40 没有派单，判断却要把作业标准算进指纹。拿最近一次 01 的标准来跑：
/// 它与当天派下来的那份一样时（主编没改派工单备注，常态），结论就复用得上；
/// 不一样时描述和向量仍然省下了，只是判断那一段要重来。
pub async fn run_prefetch(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
) -> Result<RoundCounts> {
    run_local(conn, round, cfg, svc, Caches::default(), "预取轮").await
}

/// 手动开的一轮，或重跑。与预取轮一样**只跑第 2–5 步、不写引擎**。
///
/// 重跑时 `caches` 把对应的缓存关掉——那才是真正的重算。**登记与提交不动**：
/// 引擎那边已经收到的台账要改，只能走补件，那是人的决定，不是重跑的副作用。
pub async fn run_manual(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    caches: Caches,
) -> Result<RoundCounts> {
    run_local(conn, round, cfg, svc, caches, "手动轮").await
}

async fn run_local(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    caches: Caches,
    label: &str,
) -> Result<RoundCounts> {
    // 作业标准从镜像取：手动轮与预取轮都没有派单，而标准要进指纹。
    // **重跑一轮带任务号的，要用那一轮当时那份**——换一份标准重判等于换了依据，
    // 而台账上看不出来换过。
    let standard = match round.task_id {
        Some(t) => mirror::work_standard_of(conn, t)?,
        None => None,
    }
    .or(mirror::latest_work_standard(conn, "intake")?)
    .unwrap_or_default();
    if standard.is_empty() {
        tracing::warn!(
            "{label}：还没接过 01 的单，用空作业标准跑——描述与向量能省，判断那一段省不了"
        );
    }
    let j = harvest_and_judge(conn, round, cfg, svc, &standard, caches).await?;
    let counts = count_round(conn, round.id, &j.prepared)?;
    tracing::info!(
        候选 = counts.candidates,
        判了 = counts.judged,
        复用识别 = j.reused_recognition,
        复用结论 = j.reused_judgements,
        "{label}跑完，产物只在本地"
    );
    Ok(counts)
}

/// 把本地库当成识别缓存。
///
/// 取不到当没有：缓存读失败只会让这条重识别一遍，不该让整轮停下。
struct DescCache<'a> {
    conn: &'a Connection,
}

impl pipeline::Descriptions for DescCache<'_> {
    fn get(&self, candidate_key: &str) -> Vec<MediaDescription> {
        media::descriptions_for(
            self.conn,
            candidate_key,
            csw_collector_harvest::recognize::PROMPT_VERSION,
        )
        .unwrap_or_else(|e| {
            tracing::warn!(候选 = %candidate_key, 原因 = %format!("{e:#}"), "读识别缓存失败，重识别一遍");
            Vec::new()
        })
    }
}

/// 把本地库当成判断缓存。
struct JudgeCache<'a> {
    conn: &'a Connection,
}

impl csw_collector_judge::pipeline::Cached for JudgeCache<'_> {
    fn get(&self, candidate_key: &str, inputs_hash: &str) -> Option<Judgement> {
        ledger::judgement_by_hash(self.conn, candidate_key, inputs_hash)
            .unwrap_or_else(|e| {
                tracing::warn!(候选 = %candidate_key, 原因 = %format!("{e:#}"), "读判断缓存失败，重判一遍");
                None
            })
    }
}

/// 一轮跑到登记为止的产物，交给「自查 → 交付物 → 提交」那三步。
pub struct Finished {
    pub judgements: Vec<Judgement>,
    pub sweeps: Vec<SweepCount>,
    pub by_key: HashMap<String, Candidate>,
    /// 深核补出来的缺口，按条目键
    pub deep_gaps: HashMap<String, Vec<String>>,
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
            store: &svc.store,
            brands: &svc.brands,
            tok: &svc.tok,
            reranker: Some(&svc.vector),
        }
        .search(
            conn,
            &Query {
                text: &text,
                vector: p.fused.as_deref(),
                exclude_post_id: Some(p.candidate.source_id.clone()),
                limit: csw_collector_kb::search::FINAL_MAX,
                // 判断受「五类缺一不判」约束，某一类整体缺席会把条目卡成待核
                backfill_kinds: true,
            },
        )
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
