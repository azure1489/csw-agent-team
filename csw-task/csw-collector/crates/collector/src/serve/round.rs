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
    let (mut cands, sweeps, blocked) = pipeline::collect_all(&collectors, from, to, true).await;
    // 宽取逐条留痕：失败也记，交付包里全部列出，主编能从宽取的全部 ID 复算到窗口内的
    save_window_trace(conn, round.id, &sweeps, "live");
    // 上游断料按必扫失败处理：交 0 条的空台账会让人以为「这期没有料」（09-29 r57）
    // 窗口太短（返工补采缺的那一小段）时 0 条很正常，不据此判断料
    let long_enough = (to - from).get_seconds() >= 6 * 3600;
    let blocked = blocked.or_else(|| {
        sweeps
            .iter()
            .filter(|_| long_enough)
            .find_map(|s| pipeline::source_stalled(s, from, to))
    });

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

    for c in &mut cands {
        ledger::upsert_candidate(conn, c)?;
        ledger::attach_candidate(conn, round.id, &c.candidate_key, &c.collector, false)?;
        // 按链接取的单条（Van 点名、重判）接口不给入库时间：用库里窗口轮记下的，
        // 否则判断会说「缺 first_seen_at，无法确认窗口」落待核（09-24 重判 r53 有 5 条）
        if (c.ingested_at.is_none() || c.posted_at.is_none())
            && let Some(known) = ledger::get_candidate(conn, &c.candidate_key)?
        {
            c.ingested_at = c.ingested_at.or(known.ingested_at);
            c.posted_at = c.posted_at.or(known.posted_at);
        }
    }

    let cache = DescCache { conn };
    let report = |done: usize, total: usize| {
        // 进度写不进去不影响这一轮，只是页面上看不到走到哪了
        let _ = rounds::set_progress(conn, step.id, done, total);
    };
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
            image_vectors: cfg.features.image_vectors,
            progress: Some(&report),
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
    topics: &[csw_collector_core::types::Topic],
    sweeps: &[SweepCount],
    by_key: &HashMap<String, Candidate>,
    carried: &HashSet<String>,
) -> Result<i64> {
    let step = rounds::begin_step(conn, round.id, StepCode::Register, &round.instructions_hash)?;
    let unjudged_keys: HashSet<String> =
        ledger::unjudged_keys(conn, round.id)?.into_iter().collect();
    let judged_keys: HashSet<&str> = judgements
        .iter()
        .map(|j| j.candidate_key.as_str())
        .collect();
    let lookup = |k: &str| by_key.get(k).cloned();

    // 按选题登记：同产品、同事件的多帖只占一个条目
    let mut items = register::item_inputs_by_topic(judgements, topics, lookup);
    // 返工时上次登记过、这次不列的撤下来（第一次登记时没有上一份，这一步是空的）
    items.extend(register::dropped_since(conn, round.id, &items));
    let item_of = register::item_of_with_history(conn, round.id, topics);
    // 每一路各算各的：这一路来的候选里，判了几条、没判几条
    let per = |sweep_key: &str| {
        let col = register::collector_of(sweep_key);
        by_key
            .iter()
            .filter(|(_, c)| c.collector == col)
            .fold((0, 0), |(j, u), (k, _)| {
                if judged_keys.contains(k.as_str()) {
                    (j + 1, u)
                } else if unjudged_keys.contains(k) {
                    (j, u + 1)
                } else {
                    (j, u)
                }
            })
    };
    // 每一路登记了几个条目：按条目主帖的采集来源数（以前从没填过，一直是 0——09-30 r58 v2）
    let mut sweeps = sweeps.to_vec();
    for sw in &mut sweeps {
        let col = register::collector_of(&sw.sweep_key);
        sw.registered = items
            .iter()
            .filter(|i| i.status != "dropped")
            .filter(|i| by_key.get(&i.item_key).is_some_and(|c| c.collector == col))
            .count() as i64;
    }
    let sweep_inputs =
        register::sweep_inputs(&sweeps, (&round.window_start, &round.window_end), per);
    let jis =
        register::judgement_inputs_with_items(judgements, lookup, carried, RUBRIC_VERSION, |k| {
            item_of.get(k).cloned()
        })?;
    let last = register::enqueue_registration(conn, round.id, run_id, &items, &sweep_inputs, &jis)?;

    rounds::end_step(
        conn,
        step.id,
        StepStatus::Succeeded,
        &serde_json::json!({
            "条目": items.len(),
            "采集轮": sweep_inputs.len(),
            "判断": jis.len(),
            "未判": unjudged_keys.len(),
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

/// 任务下发的三段作业标准，原样拼给模型。被退回过的，**主编的退回意见**也原样附上——
/// 返工要改的就是它（09-28 r56：退回意见点名要补的图、要核的来源，返工时判断与深核都得看见）。
pub fn work_standard(t: &TaskDetail) -> String {
    let review = t
        .latest_review
        .as_ref()
        .filter(|r| r.get("verdict").and_then(|v| v.as_str()) != Some("approved"))
        .and_then(|r| r.get("comment").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string();
    [
        ("作业内容", t.instructions.as_str()),
        ("自检", t.self_check_criteria.as_str()),
        ("验收", t.acceptance.as_str()),
        ("派工单备注", t.editor_note.as_str()),
        ("主编退回意见（这一次要改的）", review.as_str()),
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
        max_bytes: csw_collector_harvest::download::DEFAULT_MAX_BYTES,
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
    links: Option<&[String]>,
    // 回测：藏起每条自己那条决定、不补读（要可复现）
    hide_self: bool,
    // 深核：正式轮做；重判要看深核效果时也做。深核过的待核带着条目卡重判一次，不再留在待核
    deep: bool,
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
        links.unwrap_or(&[]),
        caches,
    )
    .await?;

    // 四、对照：每条检索 + 重排。一期里最慢的一段之一（重排全局串行），单独记一步、报进度
    let mat = rounds::begin_step(
        conn,
        round.id,
        StepCode::Materials,
        &round.instructions_hash,
    )?;
    let items =
        match judge_items(conn, round.id, &prepared, svc, cfg, hide_self, Some(mat.id)).await {
            Ok(items) => {
                rounds::end_step(
                    conn,
                    mat.id,
                    StepStatus::Succeeded,
                    &serde_json::json!({ "条": items.len() }),
                    "",
                )?;
                items
            }
            Err(e) => {
                rounds::end_step(
                    conn,
                    mat.id,
                    StepStatus::Failed,
                    &serde_json::json!({}),
                    &format!("{e:#}"),
                )?;
                return Err(e);
            }
        };
    // 三、五：合并与逐条判断在同一个流水线里做，合并不单独成步（页面上标「随判断」）
    let step = rounds::begin_step(conn, round.id, StepCode::Judge, &round.instructions_hash)?;
    // 判之前先把 Van 否过的同一件事挡掉。挡下来的不进模型，单独出一行台账。
    let (mut items, excluded) = apply_exclusions(conn, round, svc, items).await;
    let cache = JudgeCache { conn };
    let checkpoint = JudgeCheckpoint {
        path: cfg.db_path(),
    };
    // 已确认的准则卡照送；没确认的按开关标「草稿」送（09-27 用户拍板，默认开）
    let confirmed_rules =
        csw_collector_judge::memory::rules_for_judge(conn, cfg.features.send_draft_rules_to_model)
            .unwrap_or_else(|e| {
                tracing::warn!("读准则卡失败，这一轮不带准则卡：{e:#}");
                Vec::new()
            });
    let deps = csw_collector_judge::pipeline::Deps {
        model: &svc.model,
        jev: svc.jev.as_ref(),
        work_standard: standard,
        batch_concurrency: cfg.model.concurrency,
        on_batch: None,
        // 预取轮判过、且输入一点没变的，直接拿
        cached: caches
            .judgements
            .then_some(&cache as &dyn csw_collector_judge::pipeline::Cached),
        confirmed_rules: &confirmed_rules,
        // 回测要可复现，不接断点；正式轮、预取轮都接
        checkpoint: caches
            .judgements
            .then_some(&checkpoint as &dyn csw_collector_judge::pipeline::Checkpoint),
    };
    let mut outcome = csw_collector_judge::pipeline::run(&items, &deps).await;

    // 定点补读：关键内容在外链、没读到的，抓一次外链正文再重判。
    // 回测要可复现，不补读。
    let refetch = if !hide_self && cfg.limits.refetch_per_round > 0 {
        refetch_and_rejudge(
            conn,
            round,
            &mut items,
            &mut outcome,
            &deps,
            cfg.limits.refetch_per_round,
        )
        .await
    } else {
        RefetchCounts::default()
    };

    // 六、深核：放在落库、登记之前，深核的结果才进得了台账与引擎
    let deep_outcomes = if deep {
        deepcheck_and_rejudge(
            conn,
            round,
            cfg,
            standard,
            &prepared,
            &mut items,
            &mut outcome,
            &deps,
        )
        .await
    } else {
        Vec::new()
    };
    let deepchecked = deep_outcomes.iter().filter(|o| o.done()).count();

    for j in outcome.judgements.iter().chain(excluded.iter()) {
        let mut flags: Vec<String> = outcome
            .rule_notes
            .get(&j.candidate_key)
            .cloned()
            .unwrap_or_default();
        flags.extend(
            outcome
                .flags
                .iter()
                .filter(|f| f.candidate_key == j.candidate_key)
                .map(|f| f.note()),
        );
        // 一条写不进去不该让整轮停下——它会留在「没判」里被自查抓到
        if let Err(e) =
            ledger::put_judgement(conn, round.id, j, &flags, &cfg.model.model, RUBRIC_VERSION)
        {
            tracing::warn!(候选 = %j.candidate_key, 原因 = %format!("{e:#}"), "这条判断没落库");
        }
    }

    // 选题层：同产品、同事件的多帖合成一个选题，推荐位按选题算
    let mut judgements = outcome.judgements;
    judgements.extend(excluded.iter().cloned());
    let by_key: HashMap<String, Candidate> = prepared
        .iter()
        .map(|p| (p.candidate.candidate_key.clone(), p.candidate.clone()))
        .collect();
    let topics = build_topics(svc, &outcome.groups, &judgements, &by_key).await;
    if let Err(e) = csw_collector_core::topics::put_topics(conn, round.id, &topics) {
        tracing::warn!("选题没落库：{e:#}");
    }
    let topic_counts = csw_collector_judge::topic::counts(&topics, &judgements);

    let notes: usize = outcome.rule_notes.values().map(Vec::len).sum();
    rounds::end_step(
        conn,
        step.id,
        if outcome.unjudged.is_empty() {
            StepStatus::Succeeded
        } else {
            StepStatus::Partial
        },
        &serde_json::json!({
            "判了": judgements.len() - excluded.len(),
            "其中复用": outcome.reused.len(),
            "未判": outcome.unjudged.len(),
            "规则排除": excluded.len(),
            "合并成事件": outcome.groups.len(),
            "被标出的依据": outcome.flags.len(),
            "口径留痕": notes,
            "口径重判": outcome.rejudged.len(),
            "补读": refetch.tried,
            "补读取到": refetch.fetched,
            "补读后重判": refetch.rejudged,
            "深核": deep_outcomes.len(),
            "深核做成": deepchecked,
            "选题": topic_counts,
        }),
        &outcome.unjudged.join("、"),
    )?;

    Ok(Judged {
        by_key,
        deepchecked,
        reused_judgements: outcome.reused.len(),
        reused_recognition: prepared.iter().filter(|p| p.reused).count(),
        prepared,
        sweeps,
        judgements,
        topics,
    })
}

#[derive(Debug, Default, Clone, Copy)]
struct RefetchCounts {
    tried: usize,
    fetched: usize,
    rejudged: usize,
}

/// 正文里提示「全文在外链」的说法。命中且 Jev 觉得有价值的，也补读。
const LINK_HINTS: [&str; 10] = [
    "link in bio",
    "linkinbio",
    "主页链接",
    "全文",
    "官网",
    "详见",
    "完整内容",
    "プロフィールのリンク",
    "詳しくは",
    "read more",
];

/// 定点补读（09-22 反馈第三项）：关键内容没取到的待核条目，抓一次外链正文再重判一次。
///
/// **只抓正文里的地址**，经出网防护（封内网、回环、云元数据地址）；每轮至多 `cap` 条。
/// 抓没抓到都如实回写缺口的「已尝试」，抓不到的把下一步交给主编。
async fn refetch_and_rejudge(
    conn: &Connection,
    round: &Round,
    items: &mut [csw_collector_judge::pipeline::Item<'_>],
    out: &mut csw_collector_judge::pipeline::Outcome,
    deps: &csw_collector_judge::pipeline::Deps<'_>,
    cap: usize,
) -> RefetchCounts {
    use csw_collector_core::types::{Dim, GapLevel, GapOwner, Tier};
    use csw_collector_harvest::fetch;
    use csw_collector_judge::rules;

    let mut counts = RefetchCounts::default();
    let gain_of = |key: &str| {
        out.triages
            .iter()
            .find(|t| t.candidate_key == key)
            .and_then(|t| {
                t.dims
                    .iter()
                    .find(|(d, _)| *d == Dim::Gain)
                    .map(|(_, p)| *p)
            })
            .unwrap_or(0.0)
    };
    let targets: Vec<String> = out
        .judgements
        .iter()
        .filter(|j| j.tier == Tier::PendingCheck && j.image_seen)
        .filter(|j| {
            rules::wants_refetch(j)
                || items
                    .iter()
                    .find(|it| it.candidate.candidate_key == j.candidate_key)
                    .is_some_and(|it| {
                        let t = format!("{}\n{}", it.candidate.text, it.candidate.translated)
                            .to_lowercase();
                        LINK_HINTS.iter().any(|h| t.contains(h)) && gain_of(&j.candidate_key) >= 0.5
                    })
        })
        .map(|j| j.candidate_key.clone())
        .take(cap)
        .collect();
    if targets.is_empty() {
        return counts;
    }
    let client = match csw_collector_harvest::netguard::guarded_client(fetch::TIMEOUT, false) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("补读客户端建不起来，这一轮不补读：{e:#}");
            return counts;
        }
    };

    // key → 每个地址的结果（中文）
    let mut tried: HashMap<String, Vec<String>> = HashMap::new();
    let mut again: Vec<usize> = Vec::new();
    for key in &targets {
        let Some(i) = items
            .iter()
            .position(|it| &it.candidate.candidate_key == key)
        else {
            continue;
        };
        counts.tried += 1;
        let text = format!(
            "{}\n{}",
            items[i].candidate.text, items[i].candidate.translated
        );
        let urls = fetch::urls_in(&text);
        if urls.is_empty() {
            tried.insert(key.clone(), vec!["正文里没有可补读的链接".into()]);
            continue;
        }
        let mut got = Vec::new();
        let mut notes = Vec::new();
        for u in urls {
            // 预取轮（或上一期）刚抓过、抓到了的就不再抓：同一页正文，指纹也就对得上
            let f = match csw_collector_core::topics::recent_refetch(conn, key, &u, 36) {
                Ok(Some(text)) => fetch::Fetched {
                    url: u.clone(),
                    status: fetch::FetchStatus::Ok,
                    http_status: Some(200),
                    bytes: text.len(),
                    text,
                    error: String::new(),
                },
                _ => fetch::fetch_text(&client, &u, false).await,
            };
            let _ = csw_collector_core::topics::put_refetch(
                conn,
                round.id,
                &csw_collector_core::topics::RefetchRow {
                    candidate_key: key.clone(),
                    url: f.url.clone(),
                    status: f.status.as_str().into(),
                    http_status: f.http_status,
                    bytes: f.bytes,
                    text: f.text.clone(),
                    error: f.error.clone(),
                },
            );
            notes.push(format!("补读 {}：{}", f.url, f.status.cn()));
            if f.status == fetch::FetchStatus::Ok && !f.text.trim().is_empty() {
                got.push(csw_collector_judge::verdict::Refetched {
                    url: f.url,
                    text: f.text.chars().take(fetch::JUDGE_CHARS).collect(),
                });
            }
        }
        tried.insert(key.clone(), notes);
        if !got.is_empty() {
            counts.fetched += 1;
            items[i].refetched = got;
            again.push(i);
        }
    }

    if !again.is_empty() {
        let redone =
            csw_collector_judge::pipeline::judge_again(items, &again, deps, &out.triages).await;
        counts.rejudged = redone.judgements.len();
        for j in redone.judgements {
            let key = j.candidate_key.clone();
            if let Some(pos) = out.judgements.iter().position(|x| x.candidate_key == key) {
                out.judgements[pos] = j;
            }
            out.flags.retain(|f| f.candidate_key != key);
            let notes = out.rule_notes.entry(key.clone()).or_default();
            notes.clear();
            notes.extend(redone.rule_notes.get(&key).cloned().unwrap_or_default());
        }
        out.flags.extend(redone.flags);
        for k in redone.rejudged {
            if !out.rejudged.contains(&k) {
                out.rejudged.push(k);
            }
        }
    }

    // 「已尝试」回写进影响判断的缺口；还没结的下一步交给主编
    for j in out.judgements.iter_mut() {
        let Some(notes) = tried.get(&j.candidate_key) else {
            continue;
        };
        let still_pending = j.tier == Tier::PendingCheck;
        // 写到指向外部内容的那条缺口上；没有就另起一条，不去改一条讲别的事的缺口
        let about_link = |g: &csw_collector_core::types::Gap| {
            g.level == GapLevel::Decision
                && ["外链", "链接", "主页", "全文", "官网", "完整内容", "link"]
                    .iter()
                    .any(|k| format!("{}{}", g.what, g.next).contains(k))
        };
        if !j.gaps.iter().any(about_link) && still_pending {
            j.gaps.push(csw_collector_core::types::Gap::decision(
                "完整内容在外链，需补读",
            ));
        }
        if let Some(g) = j.gaps.iter_mut().find(|g| about_link(g)) {
            for n in notes {
                // 预取轮已经写过的同一句不再重复
                if !g.tried.contains(n.as_str()) {
                    if !g.tried.is_empty() {
                        g.tried.push('；');
                    }
                    g.tried.push_str(n);
                }
            }
            if still_pending {
                g.owner = GapOwner::Editor;
                g.next = "补读没解决：请人工打开原帖与外链核对后改档".into();
            }
        }
    }
    counts
}

/// 深核，并让深核过的待核带着条目卡重判一次（09-25 用户：深核过的最好不要还是待核）。
///
/// - 挑谁：主编点名的 → 待核的 → 推荐的（见 [`super::finish::first_batch`]）；同一合并组只核代表那条，
///   待核的例外（每条的缺口不一样）。
/// - 深核做成了的待核：条目卡作为材料重判，提示词与 R9 都不许再判待核——深核后仍缺关键资料的
///   定为「备选·待补证」，缺口交主编。
/// - 其余深核过的：条目卡里的「仍缺」并进缺口（影响成稿），档不动。
#[allow(clippy::too_many_arguments)]
async fn deepcheck_and_rejudge(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    standard: &str,
    prepared: &[Prepared],
    items: &mut [csw_collector_judge::pipeline::Item<'_>],
    out: &mut csw_collector_judge::pipeline::Outcome,
    deps: &csw_collector_judge::pipeline::Deps<'_>,
) -> Vec<csw_collector_deepcheck::run::Outcome> {
    use csw_collector_core::types::Tier;

    let by_key: HashMap<String, Candidate> = prepared
        .iter()
        .map(|p| (p.candidate.candidate_key.clone(), p.candidate.clone()))
        .collect();
    let secondary: HashSet<&str> = out
        .groups
        .iter()
        .flat_map(|g| g.members.iter().filter(|m| **m != g.primary))
        .map(String::as_str)
        .collect();
    let eligible: Vec<Judgement> = out
        .judgements
        .iter()
        .filter(|j| j.tier == Tier::PendingCheck || !secondary.contains(j.candidate_key.as_str()))
        .cloned()
        .collect();
    let outcomes =
        super::finish::deepcheck(conn, round, cfg, &eligible, &by_key, prepared, standard).await;
    if outcomes.is_empty() {
        return outcomes;
    }

    let pending: HashSet<String> = out
        .judgements
        .iter()
        .filter(|j| j.tier == Tier::PendingCheck)
        .map(|j| j.candidate_key.clone())
        .collect();
    let mut again = Vec::new();
    for o in &outcomes {
        let Some(card) = o.card.as_ref().filter(|_| o.done()) else {
            continue;
        };
        if !pending.contains(&o.candidate_key) {
            continue;
        }
        if let Some(i) = items
            .iter()
            .position(|it| it.candidate.candidate_key == o.candidate_key)
        {
            items[i].deep_card = super::finish::card_text(card);
            again.push(i);
        }
    }
    if !again.is_empty() {
        let mut redone =
            csw_collector_judge::pipeline::judge_again(items, &again, deps, &out.triages).await;
        // 重判没回来的（网关超时之类）再试一次：不能让深核过的因为一次网关抖动留在待核
        let missed: Vec<usize> = again
            .iter()
            .copied()
            .filter(|i| {
                !redone
                    .judgements
                    .iter()
                    .any(|j| j.candidate_key == items[*i].candidate.candidate_key)
            })
            .collect();
        if !missed.is_empty() {
            tracing::warn!(条数 = missed.len(), "深核后重判有几条没回来，再试一次");
            let more =
                csw_collector_judge::pipeline::judge_again(items, &missed, deps, &out.triages)
                    .await;
            redone.judgements.extend(more.judgements);
            redone.rule_notes.extend(more.rule_notes);
            redone.flags.extend(more.flags);
            redone.rejudged.extend(more.rejudged);
        }
        tracing::info!(
            条数 = redone.judgements.len(),
            "深核过的待核带着条目卡重判了"
        );
        for j in redone.judgements {
            let key = j.candidate_key.clone();
            if let Some(pos) = out.judgements.iter().position(|x| x.candidate_key == key) {
                out.judgements[pos] = j;
            }
            out.flags.retain(|f| f.candidate_key != key);
            let notes = out.rule_notes.entry(key.clone()).or_default();
            notes.clear();
            notes.extend(redone.rule_notes.get(&key).cloned().unwrap_or_default());
        }
        out.flags.extend(redone.flags);
        for k in redone.rejudged {
            if !out.rejudged.contains(&k) {
                out.rejudged.push(k);
            }
        }
    }
    // 深核过、仍是待核的（含重判两次都没成的）：与 R9 同一个处理——待核·已深核，缺口交主编补证
    let deepchecked: HashSet<&str> = outcomes
        .iter()
        .filter(|o| o.done())
        .map(|o| o.candidate_key.as_str())
        .collect();
    for j in out
        .judgements
        .iter_mut()
        .filter(|j| j.tier == Tier::PendingCheck && deepchecked.contains(j.candidate_key.as_str()))
    {
        if !j.has_decision_gap() {
            j.gaps.push(csw_collector_core::types::Gap::decision(
                "深核后仍缺决定选题的关键资料",
            ));
        }
        for g in j
            .gaps
            .iter_mut()
            .filter(|g| g.level == csw_collector_core::types::GapLevel::Decision)
        {
            g.owner = csw_collector_core::types::GapOwner::Editor;
            if g.next.trim().is_empty() {
                g.next = "人工补证后再定是否推荐".into();
            }
        }
        let notes = out.rule_notes.entry(j.candidate_key.clone()).or_default();
        if !notes.iter().any(|n| n.contains("已深核")) {
            notes.push(format!(
                "{}已深核，仍缺核心证据：待核，缺口交主编补证",
                csw_collector_judge::rules::NOTE
            ));
        }
    }

    // 深核补出来的「仍缺」并进缺口：只留在深核日志里等于没人看得见
    let gaps = super::finish::deepcheck_gaps(&outcomes);
    super::finish::merge_deepcheck_gaps(&mut out.judgements, &gaps);
    outcomes
}

/// 建选题，并对多帖的推荐 / 备选选题做一次综合。综合失败不影响选题本身。
async fn build_topics(
    svc: &super::services::Services,
    groups: &[csw_collector_core::types::EventGroup],
    js: &[Judgement],
    by_key: &HashMap<String, Candidate>,
) -> Vec<csw_collector_core::types::Topic> {
    use csw_collector_judge::topic;
    let mut topics = topic::build(groups, js);
    let todo: Vec<String> = topics
        .iter()
        .filter(|t| topic::needs_synthesis(t))
        .map(|t| t.topic_key.clone())
        .collect();
    for key in todo {
        let Some(t) = topics.iter().find(|t| t.topic_key == key).cloned() else {
            continue;
        };
        let members: Vec<(&Candidate, &Judgement)> = t
            .members
            .iter()
            .filter_map(|k| Some((by_key.get(k)?, js.iter().find(|j| &j.candidate_key == k)?)))
            .collect();
        if members.len() < 2 {
            continue;
        }
        match topic::synthesize(&svc.model, &t, &members).await {
            Ok(mut s) => {
                if let Some(jev) = svc.jev.as_ref() {
                    match topic::verify_new_info(jev, &s.synthesis.per_member, |k| {
                        by_key
                            .get(k)
                            .map(|c| format!("{}\n{}", c.text, c.translated))
                    })
                    .await
                    {
                        Ok(bad) => s.synthesis.unsupported = bad,
                        Err(e) => tracing::warn!(选题 = %key, "新增信息核对失败：{e:#}"),
                    }
                }
                topic::apply(&mut topics, &key, s, js);
            }
            Err(e) => tracing::warn!(选题 = %key, "选题综合失败，保留逐帖信息：{e:#}"),
        }
    }
    topics
}

/// 第 2–5 步的产物。
struct Judged {
    prepared: Vec<Prepared>,
    sweeps: Vec<SweepCount>,
    judgements: Vec<Judgement>,
    /// 选题层：每条有结论的候选恰好属于一个选题
    topics: Vec<csw_collector_core::types::Topic>,
    by_key: HashMap<String, Candidate>,
    /// 深核做成了几条
    deepchecked: usize,
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
    let j = harvest_and_judge(
        conn,
        round,
        cfg,
        svc,
        &standard,
        Caches::default(),
        None,
        false,
        true,
    )
    .await?;
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
            &j.topics,
            &j.sweeps,
            &j.by_key,
            &carried,
        )?;
    } else {
        tracing::info!("手动轮不写引擎，只落本地");
    }

    let mut counts = count_round(conn, round.id, &j.prepared)?;

    // 深核已在判断流程里做过（登记之前），深核补的缺口已经并进台账
    counts.deepchecked = j.deepchecked;

    Ok((
        counts,
        Finished {
            judgements: j.judgements,
            topics: j.topics,
            sweeps: j.sweeps,
            by_key: j.by_key,
            deep_gaps: HashMap::new(),
        },
    ))
}

/// 返工：**在原轮次上改，不重新采集**（09-28 用户：退回的话不要重新发起一轮采集，
/// 就在原轮次里修改，改完了再交给主编审核）。
///
/// 1. 从库里读回原轮次的判断、选题、候选与图；
/// 2. 按引擎窗口的止点收口：窗外的移出本期（台账里留着，不登记、不进交付物）；
/// 3. 挑要改的：主编退回意见里点名的账号，加上推荐 / 备选选题代表帖的前几条；
/// 4. 这几条**重新深核**（不复用旧条目卡）并带着条目卡与退回意见重判，其余保持原判；
/// 5. 选题档位按成员重算，采集轮沿用上次的、窗口内条数按收口后的重算，再登记。
///
/// 交付物与提交在调用方（与正式轮同一段）。
pub async fn rework_in_place(
    conn: &Connection,
    prev: &Round,
    detail: &TaskDetail,
    cfg: &Config,
    svc: &super::services::Services,
    engine_end: Option<Timestamp>,
    calibrations: &[super::calibration::Calibration],
) -> Result<(RoundCounts, Finished)> {
    use csw_collector_core::types::Tier;

    let standard = work_standard(detail);
    // 退回意见 = 意见 + 方向 + 位置。**点名的条目键常在方向与位置里**，只读意见就漏
    //（09-28 r56 v8：satisfyrunning-b499a6、asimocrafts-8b3c65 写在位置里）
    let review = detail
        .latest_review
        .as_ref()
        .map(|r| {
            ["comment", "return_direction", "return_location"]
                .iter()
                .filter_map(|k| r.get(*k).and_then(|c| c.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let review_raw = review.clone();
    let review = review.to_lowercase();

    // 一、读回
    let (mut judgements, mut topics, mut by_key) = load_round_state(conn, prev.id)?;
    tracing::info!(
        轮次 = prev.id,
        判断 = judgements.len(),
        选题 = topics.len(),
        "返工：读回原轮次"
    );

    // 二、窗口收口：首次入库不早于引擎窗口止点的，不算本期
    let now = Timestamp::now();
    let end = engine_end.filter(|e| *e < now);
    let outside: HashSet<String> = by_key
        .values()
        .filter(|c| matches!((end, c.posted_at.or(c.ingested_at)), (Some(e), Some(t)) if t >= e))
        .map(|c| c.candidate_key.clone())
        .collect();
    if !outside.is_empty() {
        tracing::info!(条数 = outside.len(), "返工：窗口外的移出本期");
    }
    judgements.retain(|j| !outside.contains(&j.candidate_key));
    for t in &mut topics {
        t.members.retain(|m| !outside.contains(m));
        if outside.contains(&t.primary_key) {
            t.primary_key = t.members.first().cloned().unwrap_or_default();
        }
    }
    topics.retain(|t| !t.members.is_empty());

    // 二·补：本期止点晚于这一轮当初的止点——开工时引擎止点还没到，按开工时刻收了口，
    // 中间那一段入库的没扫到。**只补采、补判缺的那一段**，原来判过的一条不动
    //（09-30 r58：06:00:12 开工、止点 07:00，主编退回「先解释 07:00 截止与 06:00:12 快照差异」）
    let mut win_end = prev.window_end.clone();
    if let Some(e) = end
        && let Ok(w) = prev.window_end.parse::<Timestamp>()
        && w < e
    {
        let slice = Round {
            window_start: prev.window_end.clone(),
            window_end: e.to_string(),
            ..prev.clone()
        };
        tracing::info!(从 = %slice.window_start, 到 = %slice.window_end, "返工：补采窗口缺的那一段");
        let kept_trace =
            csw_collector_core::window_trace::of_round(conn, prev.id).unwrap_or_default();
        let j = harvest_and_judge(
            conn,
            &slice,
            cfg,
            svc,
            &standard,
            Caches::default(),
            None,
            false,
            true,
        )
        .await?;
        record_patch(conn, prev.id, &slice, &j.sweeps);
        // 留痕：原来的全留着（补采那一趟的宽取会整份覆盖），只把这一段新入窗的接在后面
        let known: HashSet<String> = kept_trace.iter().map(|t| t.source_id.clone()).collect();
        let slice_trace =
            csw_collector_core::window_trace::of_round(conn, prev.id).unwrap_or_default();
        let mut by_sweep: std::collections::BTreeMap<
            String,
            Vec<csw_collector_core::window_trace::TraceRow>,
        > = Default::default();
        for t in kept_trace {
            by_sweep.entry(t.sweep_key.clone()).or_default().push(t);
        }
        for t in slice_trace
            .into_iter()
            .filter(|t| t.outcome == "in_window" && !known.contains(&t.source_id))
        {
            by_sweep.entry(t.sweep_key.clone()).or_default().push(t);
        }
        for (sweep, rows) in &by_sweep {
            if let Err(e) = csw_collector_core::window_trace::put(conn, prev.id, sweep, rows) {
                tracing::warn!("补采后留痕没合上：{e:#}");
            }
        }
        let have: HashSet<String> = judgements.iter().map(|j| j.candidate_key.clone()).collect();
        let added: Vec<Judgement> = j
            .judgements
            .into_iter()
            .filter(|x| !have.contains(&x.candidate_key))
            .collect();
        tracing::info!(补判 = added.len(), "返工：补采那一段判完");
        judgements.extend(added);
        let have_topics: HashSet<String> = topics.iter().map(|t| t.topic_key.clone()).collect();
        topics.extend(
            j.topics
                .into_iter()
                .filter(|t| !have_topics.contains(&t.topic_key)),
        );
        by_key.extend(j.by_key);
        conn.execute(
            "UPDATE rounds SET window_end = ?2 WHERE id = ?1",
            rusqlite::params![prev.id, e.to_string()],
        )?;
        win_end = e.to_string();
    }

    // 三、挑要改的：退回意见点名的条目优先，再补推荐 / 备选选题的代表帖
    // 二·校准：主编定过去向的条目直接采用，不交给模型重判（09-30 r58：连退十几版的根因）
    let calibrated: HashSet<String> = apply_calibrations(
        conn,
        prev.id,
        cfg,
        calibrations,
        &mut judgements,
        &mut topics,
    );
    let mut focus: Vec<String> = named_in_review(&review_raw, &judgements, &by_key);
    focus.retain(|k| !calibrated.contains(k));
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let named = focus.len();
    // 同类误用：以「同一事实无增量」判了不推荐，命中里却没有一条是正式发布、已推草稿箱或 03 决定
    //（只有上一轮台账、生成稿）。主编 v8：「其余同类仅检查同类误用」「不整池重评」——
    // 只查**这一轮登记过条目**的（主编看得见、点得着的那些），由代码找出来一并重判。
    // 没登记过的同类判断不在这次返工里动，由新规则在下一期正式轮纠正
    let registered = register::item_of_with_history(conn, prev.id, &topics);
    let misuse: Vec<String> = judgements
        .iter()
        .filter(|j| {
            registered.contains_key(&j.candidate_key)
                && j.tier == Tier::NotRecommend
                && j.comparison.verdict
                    == csw_collector_core::types::ComparisonVerdict::SameFactNoGain
                && !j.comparison.hits.iter().any(|h| {
                    matches!(
                        h.state,
                        csw_collector_core::types::HitState::Published
                            | csw_collector_core::types::HitState::Draft
                            | csw_collector_core::types::HitState::Decision
                    )
                })
        })
        .map(|j| j.candidate_key.clone())
        .collect();
    for k in &misuse {
        if !focus.contains(k) {
            focus.push(k.clone());
        }
    }
    let must = focus.len();
    // 推荐 / 备选的代表帖只在第一次返工时补进来重核；点名的与同类误用的每次都重判
    let first = prev.target_version <= 2;
    for tier in [Tier::Recommend, Tier::Alternate] {
        if !first {
            break;
        }
        for t in topics.iter().filter(|t| t.tier == Some(tier)) {
            if focus.len() >= REWORK_FOCUS.max(named) {
                break;
            }
            if !focus.contains(&t.primary_key) {
                focus.push(t.primary_key.clone());
            }
        }
    }
    focus.truncate(REWORK_FOCUS_MAX.max(must));
    // 之后的退回不整池重评（主编「禁止另一次模型重评」「不再整池重评」，09-28 r56 #615），
    // 但**点名的条目要定点重判**——否则同一版意见永远改不动，一轮轮退回成死循环（09-29 r56 v8）
    tracing::info!(
        点名 = named,
        同类误用 = misuse.len(),
        共 = focus.len(),
        第一次返工 = first,
        "返工：要重核重判的"
    );

    // 四、五：这几条重新深核（不复用旧条目卡）并带着条目卡与退回意见重判
    let (prepared, outcomes) = evidence_pass(
        conn,
        prev,
        cfg,
        svc,
        &standard,
        &focus,
        &mut judgements,
        &by_key,
        "【返工】按主编退回意见重新深核、重判",
    )
    .await?;

    // 主编退回意见里明确要「停止 / 移出」的对象：不交给模型，直接移出本期主备选
    //（09-28 r56：v1 就说停止 KEEN 访谈、RAYWOOD 纯促销，重判后 KEEN 仍在备选）
    let stops = stop_words(&review);
    for j in judgements.iter_mut() {
        let acc = by_key
            .get(&j.candidate_key)
            .map(|c| norm(&c.account))
            .unwrap_or_default();
        // 主编校准过的锁死：品牌词会误中（09-30 r58 v19：「停止项保持停止」把定为继续的 Coleman 冷藏箱移出、又在引擎里撤掉）
        if matches!(j.tier, Tier::Recommend | Tier::Alternate)
            && !calibrated.contains(&j.candidate_key)
            && stops.iter().any(|w| acc.contains(w.as_str()))
        {
            j.tier = Tier::NotRecommend;
            j.gaps
                .retain(|g| g.level != csw_collector_core::types::GapLevel::Decision);
            let flags =
                vec!["【返工】主编退回意见要求移出本期主备选（不是品牌永久排除）".to_string()];
            if let Err(e) =
                ledger::put_judgement(conn, prev.id, j, &flags, &cfg.model.model, RUBRIC_VERSION)
            {
                tracing::warn!(候选 = %j.candidate_key, 原因 = %format!("{e:#}"), "移出主备选没落库");
            }
            tracing::info!(候选 = %j.candidate_key, "返工：按退回意见移出主备选");
        }
    }

    // 旧口径留下的「备选·待补证」（备选但挂着影响选题判断的缺口）：核心证据不足，一律待核
    //（主编验收：「核心证据不足落 pending_check 并同步统计」）
    for j in judgements
        .iter_mut()
        .filter(|j| j.tier == Tier::Alternate && j.has_decision_gap())
        // 主编校准过的锁死，定为备选就是备选
        .filter(|j| !calibrated.contains(&j.candidate_key))
    {
        j.tier = Tier::PendingCheck;
        for g in j
            .gaps
            .iter_mut()
            .filter(|g| g.level == csw_collector_core::types::GapLevel::Decision)
        {
            g.owner = csw_collector_core::types::GapOwner::Editor;
        }
        let flags = vec!["【口径】已深核，仍缺核心证据：待核，缺口交主编补证".to_string()];
        if let Err(e) =
            ledger::put_judgement(conn, prev.id, j, &flags, &cfg.model.model, RUBRIC_VERSION)
        {
            tracing::warn!(候选 = %j.candidate_key, 原因 = %format!("{e:#}"), "改待核没落库");
        }
    }

    // 六、选题档位按成员重算
    retier_topics(&mut topics, &judgements);
    if let Err(e) = csw_collector_core::topics::put_topics(conn, prev.id, &topics) {
        tracing::warn!("返工的选题没落库：{e:#}");
    }

    // 七、采集轮沿用上次的，窗口内条数按收口后的重算
    let mut sweeps = register::previous_sweeps(conn, prev.id);
    for sw in &mut sweeps {
        let col = register::collector_of(&sw.sweep_key);
        let n = judgements
            .iter()
            .filter(|j| {
                by_key
                    .get(&j.candidate_key)
                    .is_some_and(|c| c.collector == col)
            })
            .count() as i64;
        sw.in_window = n;
        sw.reviewed = n;
        sw.unreviewed = 0;
        if col == register::collector_of("csw-window") {
            // 查询文字写本期窗口：旧的写的是开工时刻的止点（主编 #615：query 仍是旧止点）
            sw.query = format!(
                "按原始披露时间收口：{} ~ {}（本期窗口，左闭右开）；接口按发布时间取宽，取回 {} 条、去重 {} 条",
                prev.window_start, win_end, sw.found, sw.fetched_unique
            );
        }
    }

    // 八、登记（上次登记过、这次不列的撤下）。窗口止点可能刚被补采改过，按库里的最新值
    let prev_now = rounds::get(conn, prev.id)?.unwrap_or_else(|| prev.clone());
    if let Some(run_id) = prev.run_id {
        register_and_enqueue(
            conn,
            &prev_now,
            run_id,
            &judgements,
            &topics,
            &sweeps,
            &by_key,
            &HashSet::new(),
        )?;
    }

    let mut counts = count_round(conn, prev.id, &prepared)?;
    counts.deepchecked = outcomes.iter().filter(|o| o.done()).count();
    Ok((
        counts,
        Finished {
            judgements,
            topics,
            sweeps,
            by_key,
            deep_gaps: HashMap::new(),
        },
    ))
}

/// 套用主编校准：改档、待核补一条交主编的决定级缺口、单独成一个选题（登记的条目键就是它自己），
/// 留痕写明来源。返回套用了的条目键。
fn apply_calibrations(
    conn: &Connection,
    round_id: i64,
    cfg: &Config,
    calibrations: &[super::calibration::Calibration],
    judgements: &mut [Judgement],
    topics: &mut Vec<csw_collector_core::types::Topic>,
) -> HashSet<String> {
    use csw_collector_core::types::{Gap, GapLevel, GapOwner, Tier, Topic};
    let mut done = HashSet::new();
    for c in calibrations {
        let Some(j) = judgements.iter_mut().find(|j| j.candidate_key == c.key) else {
            continue;
        };
        let before = j.tier;
        j.tier = c.tier;
        if c.tier == Tier::PendingCheck && !j.has_decision_gap() {
            j.gaps.push(Gap {
                level: GapLevel::Decision,
                what: format!("主编校准为待核（{}），核心证据待补", c.source),
                owner: GapOwner::Editor,
                tried: String::new(),
                next: "按主编退回意见补证后再定档".into(),
            });
        }
        let flags = vec![format!(
            "【主编校准】{}（{}；工作台原判 {before:?}，不交模型重判）",
            c.label(),
            c.source
        )];
        if let Err(e) =
            ledger::put_judgement(conn, round_id, j, &flags, &cfg.model.model, RUBRIC_VERSION)
        {
            tracing::warn!(候选 = %c.key, "主编校准没落库：{e:#}");
        }
        // 单独成一个选题：条目键就是它，登记状态跟着主编走，不被组里别的帖子带偏
        let solo = topics
            .iter()
            .any(|t| t.topic_key == c.key && t.members.len() == 1);
        if !solo {
            for t in topics.iter_mut() {
                t.members.retain(|m| m != &c.key);
                if t.primary_key == c.key {
                    t.primary_key = t.members.first().cloned().unwrap_or_default();
                    t.topic_key = t.primary_key.clone();
                }
            }
            topics.retain(|t| !t.members.is_empty());
            topics.push(Topic {
                topic_key: c.key.clone(),
                primary_key: c.key.clone(),
                members: vec![c.key.clone()],
                merge_note: "主编校准单列".into(),
                tier: Some(c.tier),
                headline: j.headline.clone(),
                synthesis: Default::default(),
            });
        }
        done.insert(c.key.clone());
    }
    if !done.is_empty() {
        tracing::info!(条数 = done.len(), "返工：套用主编校准");
    }
    done
}

/// 记一次补采（主力窗口采集器那一路）。0 条也记：这就是「这一段扫过、没有新入库」的证据。
pub fn record_patch(conn: &Connection, round_id: i64, slice: &Round, sweeps: &[SweepCount]) {
    let Some(sw) = sweeps.iter().find(|s| s.sweep_key == "csw-window") else {
        return;
    };
    let p = csw_collector_core::window_trace::Patch {
        window_from: slice.window_start.clone(),
        window_to: slice.window_end.clone(),
        fetched_at: if sw.ended_at.is_empty() {
            Timestamp::now().to_string()
        } else {
            sw.ended_at.clone()
        },
        found: sw.found,
        fetched_unique: sw.fetched_unique,
        in_window: sw.in_window,
        query: sw.query.clone(),
    };
    if let Err(e) = csw_collector_core::window_trace::put_patch(conn, round_id, &p) {
        tracing::warn!("补采记录没存下：{e:#}");
    }
}

/// 退回意见点名了哪些条目。
///
/// 1. 意见里写了条目键的，**只认条目键**。
/// 2. 没写条目键的，认意见里**大写的品牌词串**（「ZANE ARTS YOMA」「HILLS FIELD BIG TOP」）：
///    词串的前几个词拼起来是账号前缀（zanearts、hillsfield），剩下的词是产品（YOMA、BIG TOP），
///    再用产品词在这个账号的帖子里筛；筛不出就取这个账号全部帖子。
///
/// 以前是「意见里 4 个字母以上的英文词，账号里含这个词就算」：09-30 r58 意见里的 field、arts、
/// trace、window 把 22 条不相干的帖子拉进来重判。
pub fn named_in_review(
    review: &str,
    judgements: &[Judgement],
    by_key: &HashMap<String, Candidate>,
) -> Vec<String> {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let lower = review.to_lowercase();
    let by_keyname: Vec<String> = judgements
        .iter()
        .filter(|j| lower.contains(&j.candidate_key.to_lowercase()))
        .map(|j| j.candidate_key.clone())
        .collect();
    if !by_keyname.is_empty() {
        return by_keyname;
    }
    static RUN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[A-Z][A-Z0-9]+(?:[ \-_.&][A-Z][A-Z0-9]+)*").expect("正则")
    });
    let account = |k: &str| by_key.get(k).map(|c| norm(&c.account)).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for m in RUN.find_iter(review) {
        let words: Vec<String> = m
            .as_str()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(norm)
            .collect();
        // 最长的、能当账号前缀的词头
        let Some((k, prefix)) = (1..=words.len())
            .rev()
            .map(|k| (k, words[..k].concat()))
            .filter(|(_, p)| p.len() >= 4)
            .find(|(_, p)| {
                judgements
                    .iter()
                    .any(|j| account(&j.candidate_key).starts_with(p.as_str()))
            })
        else {
            continue;
        };
        let of_brand: Vec<&Judgement> = judgements
            .iter()
            .filter(|j| account(&j.candidate_key).starts_with(prefix.as_str()))
            .collect();
        let products: Vec<&String> = words[k..].iter().filter(|w| w.len() >= 2).collect();
        let product_text = |j: &Judgement| {
            let c = by_key.get(&j.candidate_key);
            norm(&format!(
                "{} {} {}",
                j.headline,
                c.map(|c| c.text.as_str()).unwrap_or(""),
                c.map(|c| c.translated.as_str()).unwrap_or("")
            ))
        };
        let hit: Vec<&&Judgement> = if products.is_empty() {
            Vec::new()
        } else {
            let joined = products.iter().map(|w| w.as_str()).collect::<String>();
            of_brand
                .iter()
                .filter(|j| {
                    let t = product_text(j);
                    t.contains(&joined) || products.iter().all(|w| t.contains(w.as_str()))
                })
                .collect()
        };
        let chosen: Vec<&Judgement> = if hit.is_empty() {
            of_brand
        } else {
            hit.into_iter().copied().collect()
        };
        for j in chosen {
            if !out.contains(&j.candidate_key) {
                out.push(j.candidate_key.clone());
            }
        }
    }
    out
}

/// 读回的一轮：判断、选题、候选（按条目键）
pub type RoundState = (
    Vec<Judgement>,
    Vec<csw_collector_core::types::Topic>,
    HashMap<String, Candidate>,
);

/// 从库里读回一轮：判断、选题、候选（带图片清单与采集来源）。返工与定向补证都走它。
///
/// 采集来源不在 candidates 表里，在这一轮的 round_candidates 里。不读回来的话
/// 采集轮的窗口内条数对不上（算成 0）、条目的 discovered_via 也写不出（09-28 r56 #615 退回）。
pub fn load_round_state(conn: &Connection, round_id: i64) -> Result<RoundState> {
    let judgements = ledger::judgements_of_round(conn, round_id)?;
    let topics = csw_collector_core::topics::topics(conn, round_id)?;
    let collectors: HashMap<String, String> = {
        let mut st = conn
            .prepare("SELECT candidate_key, collector FROM round_candidates WHERE round_id = ?1")?;
        st.query_map([round_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(Result::ok)
            .collect()
    };
    let mut by_key: HashMap<String, Candidate> = HashMap::new();
    for j in &judgements {
        if let Some(mut c) = ledger::get_candidate(conn, &j.candidate_key)? {
            c.media = media::media_refs(conn, &c.candidate_key)?;
            if let Some(col) = collectors.get(&c.candidate_key) {
                c.collector = col.clone();
            }
            by_key.insert(c.candidate_key.clone(), c);
        }
    }
    Ok((judgements, topics, by_key))
}

/// 选题档位按成员重算：取组内最好的那一档。
pub fn retier_topics(topics: &mut [csw_collector_core::types::Topic], judgements: &[Judgement]) {
    use csw_collector_core::types::Tier;
    let rank = |t: Tier| match t {
        Tier::Recommend => 3,
        Tier::Alternate => 2,
        Tier::PendingCheck => 1,
        Tier::NotRecommend => 0,
    };
    for t in topics.iter_mut() {
        t.tier = judgements
            .iter()
            .filter(|j| t.members.contains(&j.candidate_key))
            .map(|j| j.tier)
            .max_by_key(|x| rank(*x));
    }
}

/// 定向补证：这几条**重新深核**（不复用旧条目卡；深核能联网就去找官网、商品页、全文）
/// 并带着条目卡重判，写回原轮次。返工与 `csw-collector evidence` 都走它。
#[allow(clippy::too_many_arguments)]
pub async fn evidence_pass(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    standard: &str,
    focus: &[String],
    judgements: &mut [Judgement],
    by_key: &HashMap<String, Candidate>,
    flag: &str,
) -> Result<(Vec<Prepared>, Vec<csw_collector_deepcheck::run::Outcome>)> {
    // 四、准备这几条：从库里还原，复用识图，不重新采集
    let cands: Vec<Candidate> = focus
        .iter()
        .filter_map(|k| by_key.get(k).cloned())
        .collect();
    let cache = DescCache { conn };
    let (prepared, _) = pipeline::prepare(
        cands,
        &pipeline::Deps {
            downloader: &svc.downloader,
            model: &svc.model,
            vector: &svc.vector,
            image_only: true,
            concurrency: cfg.model.concurrency,
            cache: Some(&cache as &dyn pipeline::Descriptions),
            image_vectors: cfg.features.image_vectors,
            progress: None,
        },
    )
    .await;
    let mut items = judge_items(conn, round.id, &prepared, svc, cfg, false, None).await?;

    // 五、重新深核（不复用旧条目卡），带着条目卡与退回意见重判
    let step = rounds::begin_step(
        conn,
        round.id,
        StepCode::Deepcheck,
        &round.instructions_hash,
    )?;
    let targets: Vec<&Judgement> = judgements
        .iter()
        .filter(|j| focus.contains(&j.candidate_key))
        .collect();
    let outcomes = super::finish::deepcheck_these(
        conn, round, cfg, step.id, targets, by_key, &prepared, standard, false,
    )
    .await;
    for o in outcomes.iter().filter(|o| o.done()) {
        if let (Some(card), Some(it)) = (
            o.card.as_ref(),
            items
                .iter_mut()
                .find(|it| it.candidate.candidate_key == o.candidate_key),
        ) {
            it.deep_card = super::finish::card_text(card);
        }
    }
    let rules =
        csw_collector_judge::memory::rules_for_judge(conn, cfg.features.send_draft_rules_to_model)
            .unwrap_or_default();
    let deps = csw_collector_judge::pipeline::Deps {
        model: &svc.model,
        jev: svc.jev.as_ref(),
        work_standard: standard,
        batch_concurrency: cfg.model.concurrency,
        on_batch: None,
        cached: None,
        confirmed_rules: &rules,
        checkpoint: None,
    };
    let idx: Vec<usize> = (0..items.len()).collect();
    let redone = csw_collector_judge::pipeline::judge_again(&items, &idx, &deps, &[]).await;
    let mut changed = 0;
    for j in redone.judgements {
        let mut flags: Vec<String> = redone
            .rule_notes
            .get(&j.candidate_key)
            .cloned()
            .unwrap_or_default();
        flags.push(flag.to_string());
        if let Err(e) =
            ledger::put_judgement(conn, round.id, &j, &flags, &cfg.model.model, RUBRIC_VERSION)
        {
            tracing::warn!(候选 = %j.candidate_key, 原因 = %format!("{e:#}"), "返工的判断没落库");
            continue;
        }
        if let Some(pos) = judgements
            .iter()
            .position(|x| x.candidate_key == j.candidate_key)
        {
            judgements[pos] = j;
            changed += 1;
        }
    }
    super::finish::merge_deepcheck_gaps(judgements, &super::finish::deepcheck_gaps(&outcomes));
    tracing::info!(
        重判 = changed,
        未判 = redone.unjudged.len(),
        "补证：重判完成"
    );
    Ok((prepared, outcomes))
}

/// 退回意见里要「停止 / 移出 / 不进入」的对象：取这类句子里 4 个字母以上的英文词。
fn stop_words(review: &str) -> Vec<String> {
    review
        .split(['。', '；', '\n', ';'])
        .filter(|sent| {
            ["停止", "移出", "不进入", "不再进入", "停掉"]
                .iter()
                .any(|k| sent.contains(k))
        })
        .flat_map(|sent| {
            sent.split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| w.len() >= 4 && w.chars().any(|c| c.is_ascii_alphabetic()))
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 返工时至少重核重判几条（不含主编点名的）、至多几条
const REWORK_FOCUS: usize = 8;
const REWORK_FOCUS_MAX: usize = 12;

/// 预取轮：跑第 2–6 步（含深核），**不写引擎、不群播报、不交付**。
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
    // 预取轮也深核：正式轮复用它的条目卡与深核后的结论，05:30 那一轮才在 40 分钟内交得出
    run_local(conn, round, cfg, svc, Caches::default(), "预取轮", true).await
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
    run_local(conn, round, cfg, svc, caches, "手动轮", false).await
}

async fn run_local(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    caches: Caches,
    label: &str,
    deep: bool,
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
    let j = harvest_and_judge(conn, round, cfg, svc, &standard, caches, None, false, deep).await?;
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

/// M3 回测：把这几条**已经判过**的贴文当新候选，走一遍第 2–5 步。
///
/// 与手动轮只差两处：候选来自给定的短码（窗口是空的），判断时**藏起每条自己那条决定**
/// （见 [`csw_collector_kb::search::Query::exclude_urls`]）。只写本地，不写引擎。
pub async fn run_backtest(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    caches: Caches,
    links: &[String],
) -> Result<Vec<Judgement>> {
    let standard = mirror::latest_work_standard(conn, "intake")?.unwrap_or_default();
    let j = harvest_and_judge(
        conn,
        round,
        cfg,
        svc,
        &standard,
        caches,
        Some(links),
        true,
        false,
    )
    .await?;
    Ok(j.judgements)
}

/// 重判：把某一轮的候选按**现在的**口径再走一遍第 2–5 步（含选题记忆、上一轮台账、补读、选题层）。
///
/// 与回测不同，它不藏任何材料——要看的正是「新口径在真实条件下会判成什么样」。
/// 只写本地一个新轮，**不写引擎**。
pub async fn run_rejudge(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    svc: &super::services::Services,
    links: &[String],
    // 也做深核（要 codex）：看「深核过的不再待核」这一段
    deep: bool,
) -> Result<Vec<Judgement>> {
    let standard = mirror::latest_work_standard(conn, "intake")?.unwrap_or_default();
    let j = harvest_and_judge(
        conn,
        round,
        cfg,
        svc,
        &standard,
        Caches {
            descriptions: true,
            judgements: false,
        },
        Some(links),
        false,
        deep,
    )
    .await?;
    Ok(j.judgements)
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
    fn notes(&self, candidate_key: &str, inputs_hash: &str) -> Vec<String> {
        ledger::flags_by_hash(self.conn, candidate_key, inputs_hash).unwrap_or_default()
    }

    fn get(&self, candidate_key: &str, inputs_hash: &str) -> Option<Judgement> {
        ledger::judgement_by_hash(self.conn, candidate_key, inputs_hash)
            .unwrap_or_else(|e| {
                tracing::warn!(候选 = %candidate_key, 原因 = %format!("{e:#}"), "读判断缓存失败，重判一遍");
                None
            })
    }
}

/// 判断断点落在本地库。各批并发写，**每次开一个短连接**：主连接不能跨线程共用，
/// 用 WAL 与 busy_timeout 排队。写不进去只记一笔——断点只是加速，丢了不影响正确性。
struct JudgeCheckpoint {
    path: std::path::PathBuf,
}

impl JudgeCheckpoint {
    fn open(&self) -> Option<Connection> {
        let c = Connection::open(&self.path).ok()?;
        c.busy_timeout(std::time::Duration::from_secs(10)).ok()?;
        Some(c)
    }
}

impl csw_collector_judge::pipeline::Checkpoint for JudgeCheckpoint {
    fn get(&self, candidate_key: &str, inputs_hash: &str) -> Option<Judgement> {
        let c = self.open()?;
        let json: String = c
            .query_row(
                "SELECT judgement_json FROM judge_checkpoints WHERE candidate_key = ?1 AND inputs_hash = ?2",
                rusqlite::params![candidate_key, inputs_hash],
                |r| r.get(0),
            )
            .ok()?;
        serde_json::from_str(&json).ok()
    }

    fn put(&self, judgements: &[Judgement]) {
        let Some(c) = self.open() else {
            tracing::warn!("判断断点没存下：打不开本地库");
            return;
        };
        let now = Timestamp::now().to_string();
        for j in judgements {
            let Ok(json) = serde_json::to_string(j) else {
                continue;
            };
            if let Err(e) = c.execute(
                "INSERT OR REPLACE INTO judge_checkpoints (candidate_key, inputs_hash, judgement_json, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![j.candidate_key, j.inputs_hash, json, now],
            ) {
                tracing::warn!(候选 = %j.candidate_key, "判断断点没存下：{e:#}");
            }
        }
        // 断点只为接上同一轮：三天前的清掉
        let _ = c.execute(
            "DELETE FROM judge_checkpoints WHERE created_at < ?1",
            rusqlite::params![
                Timestamp::now()
                    .checked_sub(jiff::SignedDuration::from_hours(72))
                    .map(|t| t.to_string())
                    .unwrap_or_default()
            ],
        );
    }
}

/// 一轮跑到登记为止的产物，交给「自查 → 交付物 → 提交」那三步。
pub struct Finished {
    pub judgements: Vec<Judgement>,
    pub topics: Vec<csw_collector_core::types::Topic>,
    pub sweeps: Vec<SweepCount>,
    pub by_key: HashMap<String, Candidate>,
    /// 深核补出来的缺口，按条目键
    pub deep_gaps: HashMap<String, Vec<String>>,
}

/// 六、硬性排除：Van 否过的同一件事，没有新料的不送判断。
///
/// 返回（留下来要判的, 被挡下的那几条的台账行）。
///
/// # 三道闸都在这儿
///
/// 1. **Jev 不在就整段跳过。** 没有判据就不排除，整轮照常判。
/// 2. **没读到实图的不排除。** 那种条目只有正文能给 Jev 看，「同一事实」
///    本来就判不准；让它走正常流程落 `pending_check` 才对。
/// 3. **一条判断失败只影响那一条。** 排除是省钱的优化，不是必须成功的一步。
async fn apply_exclusions<'a>(
    conn: &Connection,
    round: &Round,
    svc: &super::services::Services,
    items: Vec<csw_collector_judge::pipeline::Item<'a>>,
) -> (Vec<csw_collector_judge::pipeline::Item<'a>>, Vec<Judgement>) {
    use csw_collector_judge::exclude;

    let Some(jev) = svc.jev.as_ref() else {
        return (items, Vec::new());
    };
    let rules = match csw_collector_core::exclusion::active(conn) {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => return (items, Vec::new()),
        Err(e) => {
            tracing::warn!("读排除规则失败，这一轮不排除：{e:#}");
            return (items, Vec::new());
        }
    };

    let mut keep = Vec::with_capacity(items.len());
    let mut out = Vec::new();
    for it in items {
        // 连图都没读到的不排除——见上面第 2 条
        if !it.image_seen {
            keep.push(it);
            continue;
        }
        let descriptions = it
            .descriptions
            .iter()
            .map(|d| d.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let hits = svc.brands.hits(&format!(
            "{}\n{}\n{}",
            it.candidate.text, it.candidate.translated, descriptions
        ));
        let mut hit = None;
        for rule in exclude::candidates_for(&hits, &rules) {
            match exclude::ask(jev, it.candidate, &descriptions, rule).await {
                Ok(m) if m.holds() => {
                    hit = Some((rule.clone(), m));
                    break;
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    候选 = %it.candidate.candidate_key,
                    规则 = %rule.decision_ref,
                    "排除判定失败，按不排除处理：{e:#}"
                ),
            }
        }
        match hit {
            Some((rule, m)) => {
                if let Err(e) = csw_collector_core::exclusion::record_hit(
                    conn,
                    round.id,
                    &it.candidate.candidate_key,
                    rule.id,
                    m.same_fact,
                    m.new_substance,
                ) {
                    // 记不下来就不排除：台账上解释不了的排除等于凭空消失一条
                    tracing::warn!(候选 = %it.candidate.candidate_key, "排除没记下来，按不排除处理：{e:#}");
                    keep.push(it);
                    continue;
                }
                tracing::info!(
                    候选 = %it.candidate.candidate_key,
                    规则 = %rule.decision_ref,
                    "规则排除：{}",
                    exclude::explain(&rule, &m)
                );
                out.push(exclude::excluded_judgement(it.candidate, &rule, &m));
            }
            None => keep.push(it),
        }
    }
    (keep, out)
}

/// 把采集产物与检索结果拼成判断那一步要的输入。
async fn judge_items<'a>(
    conn: &Connection,
    round_id: i64,
    prepared: &'a [Prepared],
    svc: &'a super::services::Services,
    cfg: &Config,
    hide_self: bool,
    progress_step: Option<i64>,
) -> Result<Vec<csw_collector_judge::pipeline::Item<'a>>> {
    use csw_collector_kb::search::{Query, Retriever};

    // 「上一轮台账」取这一轮之前的。重判某一轮时从被重判的那一轮往前取——
    // 否则被重判那一轮自己的旧结论会被当成材料送进去，新口径的试判就被旧答案带偏了
    let prior_before: i64 = conn
        .query_row(
            "SELECT CASE WHEN kind = 'backtest' THEN COALESCE(parent_round_id, id) ELSE id END
             FROM rounds WHERE id = ?1",
            [round_id],
            |r| r.get(0),
        )
        .unwrap_or(round_id);
    // 选题记忆：全部采用 / 否决案例，每轮算一次「品牌｜对象」的向量，
    // 好给每条候选挑跨品牌、对象相近的案例（同品牌的案例太少，09-25 M3）
    let cases = csw_collector_judge::memory::load_cases(conn).unwrap_or_else(|e| {
        tracing::warn!("读选题记忆失败，这一轮不带案例：{e:#}");
        Vec::new()
    });
    let case_vectors: Option<Vec<Vec<f32>>> = if cases.is_empty() {
        None
    } else {
        let inputs: Vec<csw_collector_core::vector::EmbedInput> = cases
            .iter()
            .map(|c| csw_collector_core::vector::EmbedInput::Text(c.embed_text()))
            .collect();
        match svc.vector.embed(&inputs).await {
            Ok(v) => Some(v),
            Err(e) => {
                // 算不出向量只是少了跨品牌的相似案例，同品牌的照送
                tracing::warn!("选题记忆算向量失败，只送同品牌案例：{e:#}");
                None
            }
        }
    };
    let mut out = Vec::with_capacity(prepared.len());
    let total = prepared.len();
    // 每条候选与案例的最高相似度、送出去的跨品牌案例数：阈值定得对不对，只能看真实分布
    let mut best_sims: Vec<f32> = Vec::new();
    let mut cross_sent = 0usize;
    for (i, p) in prepared.iter().enumerate() {
        if let Some(id) = progress_step {
            let _ = rounds::set_progress(conn, id, i, total);
        }
        let text = judge_text(p);
        let own_url = [p.candidate.url.clone()];
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
                exclude_urls: if hide_self { &own_url } else { &[] },
                rerank_max: cfg.limits.rerank_per_candidate,
                snippet_chars: cfg.limits.rerank_snippet_chars,
            },
        )
        .await
        .unwrap_or_default();

        // 命中的品牌键：查重核「同品牌历史正文缺失」、挑选题记忆、同品牌合并都用它
        let mut brand_keys: Vec<String> = svc
            .brands
            .hits(&text)
            .keys()
            .map(|b| csw_collector_kb::brands::brand_key(b))
            .filter(|k| !k.is_empty())
            .collect();
        brand_keys.sort();
        brand_keys.dedup();

        // 第五类：这条候选在之前几轮的结论（预取轮不算）。回测时藏起来，免得拿答案当材料。
        let prior: Vec<csw_collector_judge::materials::PriorLedgerItem> = if hide_self {
            Vec::new()
        } else {
            ledger::prior_for(conn, prior_before, &p.candidate.candidate_key, 3)
                .unwrap_or_default()
                .into_iter()
                .map(
                    |(tier, at, headline)| csw_collector_judge::materials::PriorLedgerItem {
                        candidate_key: p.candidate.candidate_key.clone(),
                        title: headline,
                        tier,
                        decided_at: at,
                        note: String::new(),
                    },
                )
                .collect()
        };
        let mut materials = csw_collector_judge::materials::assemble(&retrieved, &prior);
        // Van 的原话送不送模型要单独拍板，没拍板前只送结论与理由码
        if !cfg.features.send_van_quotes_to_model {
            csw_collector_judge::materials::strip_van_quotes(&mut materials);
        }
        // 选题记忆：同品牌的采用 / 否决案例优先，空位由跨品牌、对象相近的补；
        // 只带对象与结论，**不带原话**。这条贴文自己的那条案例在 `pick` 里按链接排除，
        // 所以回测也照样给——以前回测整段不给，M3 就从没检验过选题记忆起不起作用
        let sims: Option<Vec<f32>> = match (&case_vectors, &p.fused) {
            (Some(cv), Some(f)) => Some(
                cv.iter()
                    .map(|v| csw_collector_judge::memory::cosine(v, f))
                    .collect(),
            ),
            _ => None,
        };
        if let Some(best) = sims
            .as_deref()
            .and_then(|v| v.iter().copied().reduce(f32::max))
        {
            best_sims.push(best);
        }
        let picked = csw_collector_judge::memory::pick(
            &cases,
            &brand_keys,
            &p.candidate.url,
            sims.as_deref(),
        );
        cross_sent += picked.iter().filter(|m| m.title.contains("跨品牌")).count();
        materials.extend(picked);
        // **真的把图读出来送进判断。**
        //
        // 这里曾经传的是 `|_| None`，于是 `pick_images` 的 filter_map 把每一张都
        // 滤掉了——判断那一步只拿到图片的**文字描述**，从没看见过原图。
        // 线上第一次跑真数据时它就自己暴露了：76 条里 36 条的 gaps 第一句都是
        // 「未读到实图」，待核率 45%。模型是如实回答的，图确实没给它。
        //
        // 「每张图都识别」这条口径因此只做了一半：识别成了文字，判断却看不见画面。
        let blob_dir = cfg.blob_dir();
        let images_b64 = csw_collector_judge::verdict::pick_images(&p.descriptions, |hash| {
            let path = csw_collector_harvest::download::blob_path(&blob_dir, hash);
            std::fs::read(&path)
                .ok()
                .map(|bytes| csw_collector_harvest::recognize::b64(&bytes))
        });
        if i + 1 == total && !best_sims.is_empty() {
            let mut v = best_sims.clone();
            v.sort_by(f32::total_cmp);
            let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
            tracing::info!(
                候选 = v.len(),
                最高相似度_中位 = q(0.5),
                最高相似度_九成 = q(0.9),
                最高相似度_最大 = q(1.0),
                阈值 = csw_collector_judge::memory::MIN_SIMILARITY,
                送出跨品牌案例 = cross_sent,
                "选题记忆：跨品牌相似案例"
            );
        }
        out.push(csw_collector_judge::pipeline::Item {
            candidate: &p.candidate,
            descriptions: &p.descriptions,
            images_b64,
            materials,
            fused: p.fused.as_deref(),
            image_seen: p.image_seen(),
            image_gap: p.failed_media.join("；"),
            heat_note: csw_collector_judge::order::heat_note(&p.candidate, None),
            brand_keys,
            refetched: Vec::new(),
            deep_card: String::new(),
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

/// 把各采集器的逐条去向存下来。写不进去不拦这一轮（交付包里会少这份留痕，并如实写明）。
pub fn save_window_trace(
    conn: &Connection,
    round_id: i64,
    sweeps: &[pipeline::SweepCount],
    source: &str,
) {
    for s in sweeps.iter().filter(|s| !s.trace.is_empty()) {
        let rows: Vec<_> = s
            .trace
            .iter()
            .map(|t| csw_collector_core::window_trace::TraceRow {
                sweep_key: s.sweep_key.clone(),
                source_id: t.source_id.clone(),
                candidate_key: t.candidate_key.clone(),
                account: t.account.clone(),
                url: t.url.clone(),
                posted_at: t.posted_at.clone(),
                ingested_at: t.ingested_at.clone(),
                media: t.media.clone(),
                outcome: t.outcome.as_str().into(),
                dup_of: t.dup_of.clone(),
                source: source.into(),
            })
            .collect();
        if let Err(e) = csw_collector_core::window_trace::put(conn, round_id, &s.sweep_key, &rows) {
            tracing::warn!(轮次 = round_id, 采集器 = %s.sweep_key, "宽取留痕没存下：{e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 主编校准直接改档并单独成题() {
        use crate::serve::calibration::Calibration;
        use csw_collector_core::types::{Tier, Topic};
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = rounds::open_round(
            &conn,
            &rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                run_id: Some(58),
                task_id: Some(614),
                stage_code: Some("intake".into()),
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v".into(),
                kb_snapshot: "k".into(),
                instructions_hash: String::new(),
            },
        )
        .unwrap();
        let mut js = vec![
            Judgement::fixture("coleman-e9591e", Tier::NotRecommend),
            Judgement::fixture("coleman-aaaaaa", Tier::Recommend),
            Judgement::fixture("drlv-cf21b4", Tier::Recommend),
        ];
        let mut topics = vec![Topic {
            topic_key: "coleman-aaaaaa".into(),
            primary_key: "coleman-aaaaaa".into(),
            members: vec!["coleman-aaaaaa".into(), "coleman-e9591e".into()],
            merge_note: String::new(),
            tier: Some(Tier::Recommend),
            headline: String::new(),
            synthesis: Default::default(),
        }];
        let cal = [
            Calibration {
                key: "coleman-e9591e".into(),
                tier: Tier::Recommend,
                source: "v6 退回意见".into(),
            },
            Calibration {
                key: "drlv-cf21b4".into(),
                tier: Tier::PendingCheck,
                source: "引擎条目".into(),
            },
        ];
        let cfg = Config::default();
        let done = apply_calibrations(&conn, r.id, &cfg, &cal, &mut js, &mut topics);
        assert_eq!(done.len(), 2);
        assert_eq!(js[0].tier, Tier::Recommend, "主编说继续，模型的不推荐不算");
        assert_eq!(js[2].tier, Tier::PendingCheck);
        assert!(js[2].has_decision_gap(), "待核要有交主编的决定级缺口");
        // 从合并的选题里拆出来单列，条目键就是它自己
        assert!(
            topics
                .iter()
                .any(|t| t.topic_key == "coleman-e9591e" && t.members == ["coleman-e9591e"])
        );
        assert!(
            topics
                .iter()
                .any(|t| t.topic_key == "coleman-aaaaaa" && t.members == ["coleman-aaaaaa"])
        );
    }

    #[test]
    fn 点名认品牌词串与产品词不误中技术词() {
        use csw_collector_core::types::{Platform, Tier};
        let c = |key: &str, account: &str, text: &str| Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw_window".into(),
            account: account.into(),
            url: String::new(),
            text: text.into(),
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
        };
        let cands = [
            c("zanearts-1", "zanearts", "YOMA 自主召回"),
            c("zanearts-2", "zanearts", "新款地布"),
            c("hillsfield-1", "hills_field", "BIG TOP 配置"),
            c("fieldsahara-1", "fieldsahara", "沙漠营地"),
            c("tracegear-1", "tracegear", "window 帐篷"),
        ];
        let by_key: HashMap<String, Candidate> = cands
            .iter()
            .map(|x| (x.candidate_key.clone(), x.clone()))
            .collect();
        let js: Vec<Judgement> = cands
            .iter()
            .map(|x| Judgement::fixture(&x.candidate_key, Tier::Recommend))
            .collect();
        // 09-30 r58 退回意见里的原话片段
        let review = "主编优先核ZANE ARTS YOMA召回、HILLS FIELD BIG TOP配置关系；trace/window_ids.jsonl、window_summary.json、sweeps.jsonl 须与接口同名含platform/tool/reviewed";
        let got = named_in_review(review, &js, &by_key);
        assert_eq!(got, ["zanearts-1", "hillsfield-1"]);
        // 写了条目键就只认条目键
        assert_eq!(
            named_in_review("核 zanearts-2 与 ZANE ARTS YOMA", &js, &by_key),
            ["zanearts-2"]
        );
    }

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
            dispatch: None,
            deliverables: vec![],
        };
        let s = work_standard(&t);
        assert!(!s.contains("退回意见"), "没退回过就不出这一段");
        let mut back = t.clone();
        back.latest_review =
            Some(serde_json::json!({"verdict": "returned", "comment": "缺 images"}));
        assert!(work_standard(&back).contains("【主编退回意见（这一次要改的）】\n缺 images"));
        assert!(s.contains("【作业内容】\n每条都判，不设 top K"));
        assert!(s.contains("【自检】"));
        assert!(s.contains("【派工单备注】"));
        // 空的那段不留一个空标题
        assert!(!s.contains("【验收】"), "{s}");
    }
}
