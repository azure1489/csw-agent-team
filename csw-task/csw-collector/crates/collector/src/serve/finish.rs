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

/// 五栏与目标缺口用到的数。**按独立事件（选题）计**，逐帖数另列；成熟只认主编在引擎里亲手定的，工作台不自定。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct FiveColumns {
    /// 各栏的事件键（选题代表帖的条目键）
    pub lead_keys: Vec<String>,
    pub pending_keys: Vec<String>,
    pub mature_keys: Vec<String>,
    pub van_keys: Vec<String>,
    /// 逐帖口径：推荐 + 备选的帖数、待核的帖数（只作对照，不进五栏）
    pub post_leads: usize,
    pub post_pending: usize,
    /// 工作台初判推荐 / 备选、尚未经主编校准
    pub leads: usize,
    pub pending: usize,
    /// 主编在引擎里定为成熟主选 / 备选的
    pub mature_primary: usize,
    pub mature_alt: usize,
    /// Van 批准可写（含已写成）
    pub van_approved: usize,
    /// 主编校准：停止 / 待核 / 继续评估 各几条
    pub calib_stop: usize,
    pub calib_pending: usize,
    pub calib_continue: usize,
    /// 首批里不是主编点名、按全池排序补上来的
    pub new_cards: usize,
    /// 本期目标（主选，备选）；引擎没给就 None
    pub target: Option<(usize, usize)>,
}

/// 「五栏与目标缺口」一节（10-08 r66 v1 退回：要五栏真实数量、目标差多少 / 为何 / 现池如何补 / 补不齐取舍）。
pub fn five_columns_section(f: &FiveColumns) -> String {
    let mut s = String::from(
        "## 五栏与目标缺口\n\n| 线索 | 待核 | 成熟主选 | 成熟备选 | Van 批准 |\n|---|---|---|---|---|\n",
    );
    s.push_str(&format!(
        "| {} | {} | {} | {} | {} |\n\n",
        f.leads, f.pending, f.mature_primary, f.mature_alt, f.van_approved
    ));
    s.push_str("- 按独立事件（选题）计，各栏的事件键见 `trace/five_columns.json`。线索是工作台初判推荐 / 备选、未经主编校准的事件；主编定为继续评估的不算线索也不算成熟；成熟只认主编在引擎里定的，工作台不自定。\n");
    s.push_str(&format!(
        "- 逐帖口径另列、不混进五栏：推荐 + 备选 {} 帖、待核 {} 帖；引擎登记的 shortlisted / 待核是条目数，也不等于五栏。\n",
        f.post_leads, f.post_pending
    ));
    match f.target {
        Some((p, a)) => {
            let (gp, ga) = (
                p.saturating_sub(f.mature_primary),
                a.saturating_sub(f.mature_alt),
            );
            s.push_str(&format!("- 目标 {p} 主 {a} 备，现差 {gp} 主 {ga} 备。\n"));
            s.push_str(&format!(
                "- 为何：主编校准停止 {} 条、隔离待核 {} 条、继续评估 {} 条（继续评估不等于成熟）。\n",
                f.calib_stop, f.calib_pending, f.calib_continue
            ));
            s.push_str(&format!(
                "- 现池如何补：首批另补 {} 张卡，来自本期全池排序（不重采、不 top K 替代全池），待主编校准。\n",
                f.new_cards
            ));
            s.push_str("- 补不齐的取舍：校准后仍不足就如实报缺口，不把未校准的工作台推荐算成成熟，不换措辞包装弱题。\n");
        }
        None => s.push_str("- 本期派工没给目标数，缺口无法计算。\n"),
    }
    s.push('\n');
    s
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

/// 补采记录里没有逐页请求（10-09 之前的补采只记概要）时，写明缺失与证据等级。
const PAGE_LOG_MISSING: &str = "不存在：当时只记了概要（接口返回条数、翻到 has_more=false），没有逐页 offset 记录。first_seen 复算只限接口返回的集合，未独立证明入口全覆盖；不补造、不补采";

fn page_log_note(p: &csw_collector_core::window_trace::Patch) -> String {
    if p.query.contains("请求 GET") {
        String::new()
    } else {
        format!("\n>\n> 这一趟的逐页请求日志{PAGE_LOG_MISSING}。")
    }
}

/// 在每一行以 `prefix` 开头的行后面插一行（已经插过的不重复插）。
fn insert_after(body: &str, prefix: &str, line: &str) -> String {
    let mut out = String::with_capacity(body.len() + line.len() + 1);
    let mut lines = body.split_inclusive('\n').peekable();
    while let Some(l) = lines.next() {
        out.push_str(l);
        if l.starts_with(prefix) && lines.peek().map(|n| n.trim_end()) != Some(line) {
            if !l.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// 这一轮前几版交付包里 `images/` 下有、`have` 里没有的文件，按原字节取出（同名以较新的一版为准）。
fn archived_images(
    conn: &Connection,
    round_id: i64,
    have: &std::collections::HashSet<String>,
) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let paths: Vec<String> = conn
        .prepare("SELECT zip_path FROM deliverables_local WHERE round_id = ?1 AND deliv_kind = '产出' ORDER BY id")
        .and_then(|mut st| {
            st.query_map([round_id], |r| r.get::<_, String>(0))
                .map(|it| it.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    let mut out: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    for p in paths {
        let Ok(f) = std::fs::File::open(&p) else {
            continue;
        };
        let Ok(mut z) = zip::ZipArchive::new(f) else {
            continue;
        };
        for i in 0..z.len() {
            let Ok(mut e) = z.by_index(i) else { continue };
            let Some(rel) = e.name().split_once('/').map(|(_, r)| r.to_string()) else {
                continue;
            };
            if !rel.starts_with("images/") || have.contains(&rel) {
                continue;
            }
            let mut b = Vec::new();
            if e.read_to_end(&mut b).is_ok() {
                out.insert(rel, b);
            }
        }
    }
    out.into_iter().collect()
}

/// 首批卡与联系图上展示的图：主编指定了代表预览就只放那几张（按指定顺序），否则全部。
/// 指定的张数在本地一张都对不上时退回全部，并不当成没图。
fn shown_previews(
    conn: &Connection,
    cfg: &Config,
    key: &str,
    picks: &HashMap<String, Vec<u32>>,
) -> Vec<intake::Preview> {
    let mut all = all_previews(conn, cfg, key);
    let Some(ns) = picks.get(key) else { return all };
    let mut picked: Vec<intake::Preview> = Vec::new();
    for n in ns {
        if let Some(i) = all
            .iter()
            .position(|p| p.candidate_key == format!("{key}_{n}"))
        {
            picked.push(all.remove(i));
        }
    }
    if picked.is_empty() { all } else { picked }
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
            // 只因「披露早于窗口」没过的窗口合规是口径冲突，不是越界，另起一行说明（见 window_rule_conflict）
            .filter(|c| !is_disclosure_conflict(c))
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

/// 引擎自查「窗口合规」只因「原始披露时间早于窗口」没过：工作台按**首次入库时间**收口，
/// 断料后补入库的贴文披露早、入库在窗口内，两种口径在这里冲突。
fn is_disclosure_conflict(c: &csw_collector_engineapi::types::IntakeCheck) -> bool {
    c.name.contains("窗口合规")
        && c.detail.contains("（早于窗口）")
        && !["（晚于窗口）", "抓取早于披露", "缺披露", "（缺"]
            .iter()
            .any(|x| c.detail.contains(x))
}

/// 规则口径冲突的说明（有才给）：写明两边各按什么、冲突的有几条、证据在哪。**不改日期、不撤条目**。
/// 09-30 r58 主编退回：「对机器 153 越界标为规则口径冲突，逐项保留真实披露时间，不改日期、
/// 不机械 drop；给出首次入库证据与所用 task 规则」。
pub fn window_rule_conflict(r: Option<&IntakeCheckResult>) -> Option<String> {
    let c = r?
        .checks
        .iter()
        .find(|c| !c.ok && !c.skipped && is_disclosure_conflict(c))?;
    let n = c
        .detail
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|x| !x.is_empty())
        .unwrap_or("若干");
    Some(format!(
        "规则口径冲突（不是越界）：引擎自查「{}」按登记条目的原始披露时间判 {n} 个条目早于窗口；工作台按作业口径\
         「窗口按首次入库时间，左闭右开」收口，这些条目的首次入库都在本期窗口内（上游断料期间发布、恢复后补入库）。\
         逐帖统计（含未登记的不推荐帖）见 trace/window_summary.json 的「披露早于窗口（逐帖）」，与条目数口径不同。\
         逐条保留真实披露时间，不改日期、不撤条目；每条的 posted_at（披露）与 first_seen_at（首次入库）\
         见 trace/window_ids.jsonl，口径冲突的逐条标了 disclosure_before_window。请主编定这一期按哪个口径",
        c.name
    ))
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
    reconciled: Option<(&serde_json::Value, &[String])>,
    pinned: &[String],
    calib_labels: &HashMap<String, String>,
    five: Option<FiveColumns>,
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
    // 登记条目的最终归属：每个条目最终什么状态、名下有哪几帖。放在选题一览前面，
    // 与引擎 runs/{id}/items 逐键相等（09-28 r56 v7 退回：index 登记条目最终归属）
    let reg = registration_section(conn, round.id, topics, judgements);
    if !reg.is_empty() {
        body = match body.find("\n## ") {
            Some(i) => format!("{}\n{reg}{}", &body[..i], &body[i..]),
            None => format!("{body}\n{reg}"),
        };
    }
    // 档位是工作台初判：写明不是成熟数、也不是 Van 采用（09-30 r58 退回：首页「76 推荐 99 备选」措辞）
    if let Some(i) = body.find("共 ")
        && let Some(end) = body[i..].find('\n')
    {
        body.insert_str(
            i + end,
            "\n\n> 以上档位是工作台初判，未经主编首批校准，也不是 Van 采用；推荐 / 备选的总清单只作附件核对用。",
        );
    }
    // 窗口行后写补采记录：补过的一段扫了多少、窗口内多少，0 条照写（不是只改止点）
    let patches = csw_collector_core::window_trace::patches_of(conn, round.id).unwrap_or_default();
    if !patches.is_empty()
        && let Some(i) = body.find("窗口（")
        && let Some(end) = body[i..].find('\n')
    {
        let lines: Vec<String> = patches
            .iter()
            .map(|p| {
                if p.query.contains("first_seen 复算") {
                    format!("> {}（{} 补采）{}", p.query, p.fetched_at, page_log_note(p))
                } else {
                    format!(
                        "> 补采 {} ~ {}：{} 补采，接口按发布时间宽取 {} 条，窗口内 {} 条（见 trace/window_summary.json「补采记录」）{}",
                        p.window_from, p.window_to, p.fetched_at, p.found, p.in_window, page_log_note(p)
                    )
                }
            })
            .collect();
        body.insert_str(i + end, &format!("\n\n{}", lines.join("\n>\n")));
    }
    // 返工这一版的解析留痕（没有就是首版）。主编指定的代表预览从这里取
    let review_parse: Option<serde_json::Value> = conn
        .query_row(
            "SELECT detail_json FROM audit WHERE action = 'rework_parse' AND target = ?1 ORDER BY id DESC LIMIT 1",
            [super::round::review_parse_target(round.id, round.target_version.max(1))],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let picks: HashMap<String, Vec<u32>> = review_parse
        .as_ref()
        .and_then(|v| v.get("代表图"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    // 首页先放 4–8 张独立首批卡（点名的优先），其余清单在后（09-30 r58 退回）
    let (cards, card_keys) = first_cards(
        conn,
        cfg,
        judgements,
        topics,
        by_key,
        pinned,
        calib_labels,
        &picks,
    );
    // 五栏与目标缺口紧跟首批卡：「首批另补几张」按实际挑出来的卡算
    let five = five.map(|mut f| {
        f.new_cards = card_keys.iter().filter(|k| !pinned.contains(k)).count();
        f
    });
    let five_trace = five
        .as_ref()
        .and_then(|f| serde_json::to_string_pretty(f).ok());
    let five = five.as_ref().map(five_columns_section).unwrap_or_default();
    body = format!("{cards}{five}{body}");
    // 口径冲突单独说明，不混进红灯
    if let Some((_, lines)) = reconciled
        && let Some(c) = lines.iter().find(|l| l.starts_with("规则口径冲突"))
    {
        body = format!("> **{c}**\n\n{body}");
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
        let all = j.tier == Tier::Recommend
            || (j.tier == Tier::PendingCheck && deep)
            || card_keys.contains(&j.candidate_key);
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
    // 首批卡的联系图与对照表
    let mut sheet_rows: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut legend = String::new();
    for (i, k) in card_keys.iter().enumerate() {
        let ps = shown_previews(conn, cfg, k, &picks);
        let cells: Vec<String> = ps
            .iter()
            .take(3)
            .map(|p| {
                let n = p.candidate_key.rsplit('_').next().unwrap_or("?");
                format!("images/{}.jpg（原帖第 {n} 张）", p.candidate_key)
            })
            .collect();
        let grounds = judgements
            .iter()
            .find(|j| &j.candidate_key == k)
            .map(|j| j.three_sentences.grounds.replace('|', "／"))
            .unwrap_or_default();
        legend.push_str(&format!(
            "| {} | `{k}` | {} | {} |\n",
            i + 1,
            if cells.is_empty() {
                "本地没有落库的图".to_string()
            } else {
                cells.join("<br>")
            },
            grounds
        ));
        sheet_rows.push(ps.into_iter().take(3).map(|p| p.bytes).collect());
    }
    let sheet = contact_sheet(&sheet_rows);
    if sheet.is_some() && !legend.is_empty() {
        let section = format!(
            "## 首批卡联系图\n\n打开 `preview/首批卡联系图.jpg`：一行一张卡，每行至多 3 张同帖图（按原帖图序，左起）。\n\n| 行 | 条目 | 格内图片（左→右） | 正文支持点 |\n|---|---|---|---|\n{legend}\n"
        );
        body = match body.find("# 情报逐条 · 判断台账") {
            Some(i) => format!("{}{section}{}", &body[..i], &body[i..]),
            None => format!("{body}\n{section}"),
        };
    }
    let version = round.target_version.max(1);
    // 上一版包里有、这一版不再展示的原图：从旧包逐文件原样带上，不重下载、不重编码
    //（10-09 r67 v5：换卡后 v4 的 35 张原图从包里消失，主编要原图与冻结材料保留）
    let have: std::collections::HashSet<String> = previews
        .iter()
        .map(|p| {
            format!(
                "images/{}.{}",
                p.candidate_key,
                if p.ext.trim().is_empty() {
                    "jpg"
                } else {
                    p.ext.trim()
                }
            )
        })
        .collect();
    let archived = archived_images(conn, round.id, &have);
    // 主编要原样加的卡面 / 台账说明：放在首批卡与台账这一条的条目键下面，不改六维与历史判断
    //（10-09 r67 v5：「保留历史判断不改六维，旁加主编当前决定即可」）
    if let Some(notes) = review_parse
        .as_ref()
        .and_then(|v| v.get("旁注"))
        .and_then(|v| v.as_array())
    {
        for n in notes {
            let (Some(k), Some(t)) = (
                n.get("条目键").and_then(|v| v.as_str()),
                n.get("原话").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let src = n.get("来源").and_then(|v| v.as_str()).unwrap_or("退回意见");
            let line = format!("- **主编当前决定**（{src}原文）：{t}");
            body = insert_after(&body, &format!("- 条目 `{k}`"), &line);
            body = insert_after(&body, &format!("- 条目键：`{k}`"), &line);
        }
    }
    if !archived.is_empty() {
        let mut by_key: std::collections::BTreeMap<String, usize> = Default::default();
        for (path, _) in &archived {
            let k = path
                .trim_start_matches("images/")
                .rsplit_once('_')
                .map_or(path.as_str(), |(k, _)| k)
                .to_string();
            *by_key.entry(k).or_default() += 1;
        }
        body.push_str(&format!(
            "\n## 归档原图（{} 张）\n\n前几版包里有、这一版首批卡不再展示的原图，从旧包逐文件原样带上（字节不变），只作素材归档，不代表恢复采用资格：\n\n{}\n",
            archived.len(),
            by_key
                .iter()
                .map(|(k, n)| format!("- `{k}`：{n} 张"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
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
            checks: {
                let mut c = self_checks(judgements, extra_gaps);
                // 对账不一致那行已经作为红灯进了 extra_gaps，不再写第二遍
                if let Some((_, lines)) = reconciled {
                    c.extend(lines.iter().filter(|l| !extra_gaps.contains(l)).cloned());
                }
                c
            },
            item_key: String::new(),
        },
        body,
        &previews,
        // 与引擎接口同名同值：登记给引擎的那一份采集轮原样导出；还没登记过的退回旧格式
        //（09-30 r58 退回：「trace/sweeps 须与接口同名含 platform/tool/reviewed/unreviewed/registered」）
        match crate::serve::register::registered_sweeps_jsonl(conn, round.id) {
            Some(s) => s,
            None => trace::to_jsonl(&sweep_lines)?,
        },
        trace::items_jsonl(judgements, lookup)?,
    );
    for (path, bytes) in archived {
        entries.push(pack::Entry::binary(&path, bytes));
    }
    // 逐条判断全文：六维依据、三句话、查重命中、分级缺口都在这里，主编要能直接复核
    let mut jl = String::new();
    for j in judgements {
        jl.push_str(&serde_json::to_string(j)?);
        jl.push('\n');
    }
    entries.push(pack::Entry::text("trace/judgements.jsonl", jl));
    if let Some(bytes) = sheet {
        entries.push(pack::Entry::binary("preview/首批卡联系图.jpg", bytes));
    }
    // 宽取的每一条落到哪儿、窗口内的每条判成什么登记成什么：主编要能从宽取的全部 ID
    // 按 first_seen_at 逐条复算到窗口内的那些（09-28 r56 退回：1431 → 128 只有两个数）
    let (ids, summary) = window_ids(
        conn,
        round.id,
        judgements,
        topics,
        sweeps,
        by_key,
        (&round.window_start, &round.window_end),
    )?;
    entries.push(pack::Entry::text("trace/window_ids.jsonl", ids));
    entries.push(pack::Entry::text(
        "trace/window_summary.json",
        serde_json::to_string_pretty(&summary)?,
    ));
    if let Some(t) = five_trace {
        entries.push(pack::Entry::text("trace/five_columns.json", t));
    }
    // 返工这一版认出了哪些指示、实际做了什么（见 round::record_review_parse）
    if let Some(v) = &review_parse {
        entries.push(pack::Entry::text(
            "trace/review_parse.json",
            serde_json::to_string_pretty(v)?,
        ));
    }
    // 补采那一趟取回的每一条：按首次入库判的这一段内外、资格（10-08 r66 v2 退回要逐键留痕）
    let patch_rows: Vec<String> = csw_collector_core::window_trace::of_round(conn, round.id)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.sweep_key == super::round::PATCH_SWEEP)
        .filter_map(|t| {
            serde_json::to_string(&serde_json::json!({
                "source_id": t.source_id, "candidate_key": t.candidate_key, "account": t.account,
                "url": t.url, "posted_at": t.posted_at, "first_seen": t.ingested_at,
                "media": t.media, "outcome_by_first_seen": t.outcome,
            }))
            .ok()
        })
        .collect();
    if !patch_rows.is_empty() {
        entries.push(pack::Entry::text(
            "trace/window_patch.jsonl",
            patch_rows.join("\n") + "\n",
        ));
    }
    // 本包与引擎逐键对账：条目、判断各一份，不一致的逐键列出
    if let Some((r, _)) = reconciled {
        entries.push(pack::Entry::text(
            "trace/engine_reconcile.json",
            serde_json::to_string_pretty(r)?,
        ));
    }

    let name = format!(
        "情报逐条_情报收集员_r{}_v{version}",
        round.run_id.unwrap_or(0)
    );
    let out = cfg
        .data_dir
        .join("deliverables")
        .join(format!("{name}.zip"));
    let mut built = pack::build(&name, &entries, &out)?;
    built.summary = review_summary(round, cfg, judgements, &card_keys, extra_gaps);

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
            "四档（工作台初判，未经校准）：推荐 {}、备选 {}、待核 {}、不推荐 {}",
            n(Tier::Recommend),
            n(Tier::Alternate),
            n(Tier::PendingCheck),
            n(Tier::NotRecommend)
        ),
    ];
    // 0 条就只说 0 条：写「0 条，已落待核」自相矛盾（09-28 r56 v6 退回点名）
    v.push(if unseen == 0 {
        "未读到实图 0 条（每条的全部图都已识别）".into()
    } else {
        format!("未读到实图 {unseen} 条，已落待核（不是淘汰）")
    });
    v.extend(extra_gaps.iter().cloned());
    v
}

/// 对账里「本包已撤、引擎仍挂着」的条目：补发撤下。返回补发了几条。
///
/// 09-28 r56：v5 撤下的 7 条被主编手动改回待核（当时包里确实是待核），之后联网补证重判成不推荐；
/// 工作台的登记历史里它们「已撤」，不会再发，引擎就一直挂着待核。以引擎的**现状**为准纠正，写明原因。
/// 只纠正「撤下」：别的状态要整套条目字段，由正常登记负责。
pub fn heal_dropped(
    conn: &Connection,
    round: &Round,
    run_id: i64,
    report: &serde_json::Value,
) -> Result<usize> {
    let titles = crate::serve::register::registered_items(conn, round.id);
    let items: Vec<_> = report
        .pointer("/条目/不一致")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|d| d.get("本包").and_then(|v| v.as_str()) == Some("dropped"))
        .filter_map(|d| {
            let key = d.get("item_key")?.as_str()?.to_string();
            let was = d.get("引擎").and_then(|v| v.as_str()).unwrap_or("无");
            Some(csw_collector_engineapi::types::ItemInput {
                title: titles.get(&key).map(|(_, t)| t.clone()).unwrap_or_default(),
                item_key: key,
                status: "dropped".into(),
                reason_code: "superseded".into(),
                reason: format!("与交付包同步：包内已判为不推荐或移出本期（引擎此前为 {was}）"),
                ..Default::default()
            })
        })
        .collect();
    if items.is_empty() {
        return Ok(0);
    }
    crate::serve::register::enqueue_items(conn, round.id, run_id, &items)?;
    Ok(items.len())
}

/// 登记发出去之后，读回引擎里的条目与判断台账，与本包逐键对账。
///
/// 返回（对账报告, 自检行, 是否一致）。不一致不拦提交——如实写进自检与 `trace/engine_reconcile.json`，
/// 主编看得见；拦了反而是一期交不出东西（09-28 r56 v6 退回：包里待核 6、引擎里还挂着 13）。
pub async fn reconcile(
    conn: &Connection,
    round: &Round,
    engine: &EngineClient,
    run_id: i64,
    judgements: &[Judgement],
    topics: &[csw_collector_core::types::Topic],
) -> (serde_json::Value, Vec<String>, bool) {
    let want_items = crate::serve::register::registered_items(conn, round.id);
    let item_of = crate::serve::register::item_of_with_history(conn, round.id, topics);
    let (eng_items, eng_js) = match (
        engine.run_items(run_id).await,
        engine.intake_judgements(run_id).await,
    ) {
        (Ok(i), Ok(j)) => (i, j),
        (i, j) => {
            let why = [
                i.err().map(|e| format!("条目：{e:#}")),
                j.err().map(|e| format!("判断：{e:#}")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("；");
            return (
                serde_json::json!({"对账": "没做成", "原因": why}),
                vec![format!(
                    "引擎逐键对账没做成（{why}），本包与引擎是否一致未核"
                )],
                false,
            );
        }
    };
    let got_items: HashMap<&str, &str> = eng_items
        .items
        .iter()
        .map(|i| (i.item_key.as_str(), i.status.as_str()))
        .collect();
    let mut item_diff = Vec::new();
    for (k, (st, _)) in &want_items {
        match got_items.get(k.as_str()) {
            Some(g) if g == st => {}
            g => item_diff.push(serde_json::json!({"item_key": k, "本包": st, "引擎": g})),
        }
    }
    for (k, g) in &got_items {
        if !want_items.contains_key(*k) {
            item_diff.push(serde_json::json!({"item_key": k, "本包": null, "引擎": g}));
        }
    }
    // 引擎判断台账：同一条多次上报取最后一次
    let mut got_js: HashMap<String, (String, String)> = HashMap::new();
    for j in eng_js
        .get("judgements")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let s = |k: &str| j.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        got_js.insert(s("candidate_key"), (s("tier"), s("item_key")));
    }
    let mut j_diff = Vec::new();
    for j in judgements {
        let tier = serde_json::to_value(j.tier)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let item = item_of.get(&j.candidate_key).cloned().unwrap_or_default();
        match got_js.get(&j.candidate_key) {
            Some((t, i)) if *t == tier && *i == item => {}
            g => j_diff.push(serde_json::json!({
                "candidate_key": j.candidate_key,
                "本包": {"tier": tier, "item_key": item},
                "引擎": g.map(|(t, i)| serde_json::json!({"tier": t, "item_key": i})),
            })),
        }
    }
    let package_keys: std::collections::HashSet<&str> = judgements
        .iter()
        .map(|j| j.candidate_key.as_str())
        .collect();
    for k in got_js.keys().filter(|k| !package_keys.contains(k.as_str())) {
        j_diff.push(serde_json::json!({"candidate_key": k, "本包": null, "引擎": "有"}));
    }
    let count = |st: &str| want_items.values().filter(|(s, _)| s == st).count();
    let consistent = item_diff.is_empty() && j_diff.is_empty();
    let mut lines = vec![format!(
        "工作台登记：shortlisted {}（只是登记，不等于成熟或 Van 采用）、待核 {}、已撤 {}（本轮历次登记的最终状态）",
        count("shortlisted"),
        count("pending_check"),
        count("dropped")
    )];
    lines.push(if consistent {
        format!(
            "引擎逐键对账一致：条目 {} 个、判断 {} 条（trace/engine_reconcile.json）",
            want_items.len(),
            judgements.len()
        )
    } else {
        format!(
            "引擎逐键对账不一致：条目 {} 个、判断 {} 条，逐键见 trace/engine_reconcile.json",
            item_diff.len(),
            j_diff.len()
        )
    });
    let report = serde_json::json!({
        "核对时间": jiff::Timestamp::now().to_string(),
        "run_id": run_id,
        "条目": {"本包应有": want_items.len(), "引擎": got_items.len(), "不一致": item_diff},
        "判断": {"本包": judgements.len(), "引擎": got_js.len(), "不一致": j_diff},
        "口径": "本包应有的条目 = 这一轮历次登记、每个键取最后一次的状态；判断按 candidate_key 比档位与所属条目",
    });
    (report, lines, consistent)
}

/// 交给主编的摘要：各档条数、首批卡标题、自查没过的判据、台账链接。
/// 随提交作 `self_check` 发出，引擎「待审」那条群消息里 @主编时原样带上——
/// 每天 06:00 自动开期后，主编在群里就能看到这一期采到了什么（10-09 用户）
fn review_summary(
    round: &Round,
    cfg: &Config,
    judgements: &[Judgement],
    card_keys: &[String],
    extra_gaps: &[String],
) -> String {
    let n = |t: Tier| judgements.iter().filter(|j| j.tier == t).count();
    // 北京时间，读不出来就原样
    let bj = |s: &str| {
        s.parse::<jiff::Timestamp>().map_or_else(
            |_| s.to_string(),
            |t| {
                t.to_zoned(jiff::tz::Offset::constant(8).to_time_zone())
                    .strftime("%m-%d %H:%M")
                    .to_string()
            },
        )
    };
    let mut out = format!(
        "窗口 {} ~ {}（北京时间），判了 {} 条：推荐 {} · 备选 {} · 待核 {} · 不推荐 {}",
        bj(&round.window_start),
        bj(&round.window_end),
        judgements.len(),
        n(Tier::Recommend),
        n(Tier::Alternate),
        n(Tier::PendingCheck),
        n(Tier::NotRecommend)
    );
    if !card_keys.is_empty() {
        out.push_str(&format!("\n首批 {} 张：", card_keys.len()));
        for (i, k) in card_keys.iter().enumerate() {
            let title = judgements
                .iter()
                .find(|j| &j.candidate_key == k)
                .map(|j| j.headline.trim())
                .filter(|h| !h.is_empty())
                .unwrap_or(k);
            out.push_str(&format!("\n{}. {title}", i + 1));
        }
    }
    if !extra_gaps.is_empty() {
        out.push_str(&format!("\n自查没过 {} 项：", extra_gaps.len()));
        for g in extra_gaps.iter().take(3) {
            out.push_str(&format!("\n- {}", g.chars().take(80).collect::<String>()));
        }
    }
    let base = cfg.web.public_url.trim_end_matches('/');
    if !base.is_empty() {
        out.push_str(&format!("\n台账：{base}/ledger?round={}", round.id));
    }
    out.push_str("\n交付物：");
    out
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
        self_check: built.summary.clone(),
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

/// 「登记条目最终归属」一节：这一轮历次登记、每个条目取最后一次的状态，列出名下贴文与档位。
/// 一个条目名下有几帖的（同产品、同事件合成一个选题）逐帖写出——引擎里一个条目键对应多条判断不是冲突。
fn registration_section(
    conn: &Connection,
    round_id: i64,
    topics: &[csw_collector_core::types::Topic],
    judgements: &[Judgement],
) -> String {
    let items = crate::serve::register::registered_items(conn, round_id);
    if items.is_empty() {
        return String::new();
    }
    let item_of = crate::serve::register::item_of_with_history(conn, round_id, topics);
    let tier: HashMap<&str, Tier> = judgements
        .iter()
        .map(|j| (j.candidate_key.as_str(), j.tier))
        .collect();
    let mut members: HashMap<&str, Vec<&str>> = HashMap::new();
    for (c, item) in &item_of {
        members.entry(item.as_str()).or_default().push(c.as_str());
    }
    fn status_cn(s: &str) -> &str {
        match s {
            "shortlisted" => "shortlisted（登记，非采用）",
            "pending_check" => "待核",
            "dropped" => "已撤",
            other => other,
        }
    }
    let count = |st: &str| items.values().filter(|(s, _)| s == st).count();
    let mut out = format!(
        "## 登记条目最终归属（{} 个：shortlisted {}、待核 {}、已撤 {}；shortlisted 是工作台登记，不等于采用）\n\n\
         本轮历次登记、每个条目取最后一次的状态，与引擎 `runs/{{id}}/items` 逐键相等（见 `trace/engine_reconcile.json`）。\
         一个条目名下有多帖的，是同产品或同事件合成的一个选题，逐帖列出。\n\n\
         | 条目键 | 最终状态 | 名下贴文（档位） |\n|---|---|---|\n",
        items.len(),
        count("shortlisted"),
        count("pending_check"),
        count("dropped")
    );
    for (key, (st, _)) in &items {
        let mut ms = members.get(key.as_str()).cloned().unwrap_or_default();
        ms.sort_by_key(|m| (*m != key.as_str(), *m));
        let list = if ms.is_empty() {
            "（本轮已无贴文归到它）".to_string()
        } else {
            ms.iter()
                .map(|m| {
                    let t = tier.get(m).map(|t| intake::tier_name(*t)).unwrap_or("未判");
                    format!("`{m}`（{t}）")
                })
                .collect::<Vec<_>>()
                .join("、")
        };
        out.push_str(&format!("| `{key}` | {} | {list} |\n", status_cn(st)));
    }
    out.push('\n');
    out
}

/// 首页的独立首批卡：点名的优先，再按推荐选题的代表帖补到 8 张；不足 4 张如实交实际数，不凑。
/// 每张卡内联：完整原文、原始披露时间、首次入库、同帖图（至多 6 张）、真实查重依据、
/// 能改变采用决定的缺口（09-30 r58 退回点名的卡片要素）。返回（正文, 上卡的条目键）。
#[allow(clippy::too_many_arguments)]
fn first_cards(
    conn: &Connection,
    cfg: &Config,
    judgements: &[Judgement],
    topics: &[csw_collector_core::types::Topic],
    by_key: &HashMap<String, Candidate>,
    pinned: &[String],
    calib_labels: &HashMap<String, String>,
    picks: &HashMap<String, Vec<u32>>,
) -> (String, Vec<String>) {
    const MAX: usize = 8;
    let by_j: HashMap<&str, &Judgement> = judgements
        .iter()
        .map(|j| (j.candidate_key.as_str(), j))
        .collect();
    // 并进别的事件的成员不单占一张卡：素材随主事件（10-09 r67 v5：53e957 已并入 fasunaa，
    // 仍作卡 6 与卡 4 重复占 SATISFY 同题）
    let merged_member = |k: &str| {
        topics
            .iter()
            .any(|t| t.primary_key != k && t.members.iter().any(|m| m == k))
    };
    let mut keys: Vec<String> = pinned
        .iter()
        .filter(|k| by_j.contains_key(k.as_str()) && !merged_member(k))
        .take(MAX)
        .cloned()
        .collect();
    // 补位：同品牌只占一张（10-08 r66 v2：卡 7、卡 8 同是 cargocontainer）；
    // 同事实无增量、或历史上已批准写作 / 已写成的不回流（卡 8 是 r62 写成的 MODULE X）
    let brand = |k: &str| -> String {
        by_key
            .get(k)
            .map(|c| {
                c.account
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_lowercase()
            })
            .unwrap_or_default()
    };
    let mut brands: std::collections::HashSet<String> = keys
        .iter()
        .map(|k| brand(k))
        .filter(|b| !b.is_empty())
        .collect();
    for t in topics.iter().filter(|t| t.tier == Some(Tier::Recommend)) {
        if keys.len() >= MAX {
            break;
        }
        let Some(j) = by_j.get(t.primary_key.as_str()) else {
            continue;
        };
        if j.tier != Tier::Recommend || j.has_decision_gap() || keys.contains(&t.primary_key) {
            continue;
        }
        if j.comparison.verdict == csw_collector_core::types::ComparisonVerdict::SameFactNoGain
            || (j.comparison.note.contains("历史：r")
                && (j.comparison.note.contains("已批准写作")
                    || j.comparison.note.contains("已写成")))
        {
            continue;
        }
        let b = brand(&t.primary_key);
        if !b.is_empty() && !brands.insert(b) {
            continue;
        }
        keys.push(t.primary_key.clone());
    }
    if keys.is_empty() {
        return (String::new(), keys);
    }
    let mut s = format!(
        "# 首批卡（{} 张{}）\n\n主编点名的优先，其余按推荐选题的代表帖；档位是工作台初判，待主编首批校准。\n\n",
        keys.len(),
        if keys.len() < 4 {
            "，不足 4 张，如实交实际数"
        } else {
            ""
        }
    );
    for (n, k) in keys.iter().enumerate() {
        let j = by_j[k.as_str()];
        let c = by_key.get(k);
        s.push_str(&format!(
            "## 卡 {} ｜{}\n\n",
            n + 1,
            if j.headline.is_empty() {
                k.as_str()
            } else {
                j.headline.as_str()
            }
        ));
        s.push_str(&format!(
            "- 条目 `{k}` ｜档位：{}{}\n",
            intake::tier_name(j.tier),
            match calib_labels.get(k) {
                Some(l) => format!("｜{l}；供 02 研究，不是 Van 采用"),
                None if pinned.contains(k) => "（主编点名）".to_string(),
                None => String::new(),
            }
        ));
        if let Some(c) = c {
            s.push_str(&format!("- 原帖：@{} {}\n", c.account, c.url));
            s.push_str(&format!(
                // 来源库的 posted_at 是平台贴文时间，不等于核过的首次披露（10-01 r59 v10 退回）
                "- 贴文发布时间（来源库 posted_at，首次披露未另核）：{}\n- 首次入库（first_seen）：{}\n",
                c.posted_at
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "缺".into()),
                c.ingested_at
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "缺：未核，不以发布时间顶替".into())
            ));
        }
        let imgs = shown_previews(conn, cfg, k, picks);
        if let Some(ns) = picks.get(k.as_str()).filter(|_| !imgs.is_empty()) {
            let n: Vec<String> = ns.iter().map(|n| format!("第 {n} 张")).collect();
            s.push_str(&format!(
                "- 代表预览（主编指定原帖{}；同帖其余原图留在 images/ 作原始素材，不作这一则的配图，正式配图由 05 补）：\n\n",
                n.join("、")
            ));
            for p in &imgs {
                s.push_str(&format!("  ![{0}](images/{0}.jpg)\n", p.candidate_key));
            }
            s.push('\n');
        } else if imgs.is_empty() {
            s.push_str("- 同帖图：本地没有落库的图\n");
        } else {
            s.push_str(&format!(
                "- 同帖图（共 {} 张，列前 6 张）：\n\n",
                imgs.len()
            ));
            for p in imgs.iter().take(6) {
                s.push_str(&format!("  ![{0}](images/{0}.jpg)\n", p.candidate_key));
            }
            s.push('\n');
        }
        let t = &j.three_sentences;
        s.push_str(&format!(
            "- 是什么：{}\n- 为什么值得看：{}\n- 依据：{}\n",
            t.what, t.why_worth, t.grounds
        ));
        let cmp = &j.comparison;
        s.push_str(&format!("- 查重：{}", intake::comparison_name(cmp.verdict)));
        if cmp.hits.is_empty() {
            s.push_str("（没有命中）");
        } else {
            let hits: Vec<String> = cmp
                .hits
                .iter()
                .map(|h| format!("《{}》{}", h.title, intake::hit_state_name(h.state)))
                .collect();
            s.push_str(&format!("：{}", hits.join("、")));
        }
        if !cmp.note.trim().is_empty() {
            s.push_str(&format!("；{}", cmp.note.trim()));
        }
        s.push('\n');
        // 深核找到的证据原样列出：完整 URL、标题、日期、关键原文都在这里，不只留编号
        //（09-30 r58 v18 退回：跑道卡「补齐 E2/E3 完整 URL、标题/日期、关键原文摘录」）
        if let Some((from, card)) = recent_card(conn, k, 72) {
            s.push_str(&format!("- 深核（第 {from} 轮）："));
            if !card.disclosed_at.trim().is_empty() {
                s.push_str(&format!("核到的原始披露 {}；", card.disclosed_at.trim()));
            }
            if !card.original_source.trim().is_empty() {
                s.push_str(&format!("原始来源 {}", card.original_source.trim()));
            }
            s.push('\n');
            for (name, xs) in [
                ("证据", &card.evidence),
                ("核到的事实", &card.facts),
                ("仍缺", &card.gaps),
            ] {
                for (n, x) in xs.iter().enumerate() {
                    s.push_str(&format!("  - {name} E{}：{}\n", n + 1, x.trim()));
                }
            }
        }
        let decision: Vec<String> = j
            .gaps
            .iter()
            .filter(|g| g.level == csw_collector_core::types::GapLevel::Decision)
            .map(|g| {
                format!(
                    "{}（{}；下一步：{}）",
                    g.what,
                    intake::owner_name(g.owner),
                    if g.next.is_empty() {
                        "—"
                    } else {
                        g.next.as_str()
                    }
                )
            })
            .collect();
        s.push_str(&format!(
            "- 能改变采用决定的缺口：{}\n",
            if decision.is_empty() {
                "无".to_string()
            } else {
                decision.join("；")
            }
        ));
        if let Some(c) = c {
            s.push_str(&format!(
                "\n原文：\n\n> {}\n",
                c.text.trim().replace('\n', "\n> ")
            ));
            if !c.translated.trim().is_empty() {
                s.push_str(&format!(
                    "\n译文：\n\n> {}\n",
                    c.translated.trim().replace('\n', "\n> ")
                ));
            }
        }
        s.push_str("\n---\n\n");
    }
    (s, keys)
}

/// 首批卡的联系图：一行一张卡，每行至多 3 张同帖图（按原帖图序），拼成一张 JPEG。
/// 主编的看图通道打不开包里的零散图时，打开这一张就能逐卡核图（09-30 r58 v18/v19 退回）。
/// 格子里不写字（不带字体），行列与条目、图序的对照写在首页。
fn contact_sheet(rows: &[Vec<Vec<u8>>]) -> Option<Vec<u8>> {
    use image::{GenericImage, ImageEncoder, Rgb, RgbImage, imageops::FilterType};
    const W: u32 = 320;
    const H: u32 = 320;
    const GAP: u32 = 8;
    const COLS: u32 = 3;
    if rows.is_empty() {
        return None;
    }
    let n = rows.len() as u32;
    let mut canvas = RgbImage::from_pixel(
        COLS * W + (COLS + 1) * GAP,
        n * H + (n + 1) * GAP,
        Rgb([255, 255, 255]),
    );
    for (r, imgs) in rows.iter().enumerate() {
        for (c, bytes) in imgs.iter().take(COLS as usize).enumerate() {
            let Ok(img) = image::load_from_memory(bytes) else {
                continue;
            };
            let thumb = img.resize_to_fill(W, H, FilterType::Triangle).to_rgb8();
            let x = GAP + c as u32 * (W + GAP);
            let y = GAP + r as u32 * (H + GAP);
            let _ = canvas.copy_from(&thumb, x, y);
        }
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
        .write_image(
            canvas.as_raw(),
            canvas.width(),
            canvas.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
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
    (round_start, round_end): (&str, &str),
) -> Result<(String, serde_json::Value)> {
    use csw_collector_harvest::pipeline::TraceOutcome;
    let item_of = crate::serve::register::item_of_with_history(conn, round_id, topics);
    let current = crate::serve::register::registered_item_of(topics);
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
            // 这一轮登记过、后来撤掉的条目：判断仍挂在它名下
            (Some(_), _) if !current.contains_key(key) => "dropped",
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
            "first_seen_source": if r.ingested_at.is_some() { "ingestedAt（csw 首次入库）" } else { "缺：未核，不以发布时间顶替" },
            "posted_at": r.posted_at,
            // 窗口内、但披露早于窗口起点：按首次入库收口属于本期，按披露口径算越界——口径冲突，保留真实披露时间
            "disclosure_before_window": r.outcome == "in_window"
                && r.posted_at.as_deref().zip(Some(round_start)).is_some_and(|(p, s)| p < s),
            "media": r.media,
            "has_video": r.media.contains("视频"),
            // 去重按（平台, 来源 id）：同一条贴文不论从哪个采集器来都只算一次
            "dedup_key": format!("instagram:{}", r.source_id),
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
        // 返工补采的记录：0 条也记，这是「这一段扫过」的证据（09-30 r58 退回：不能只改 window_to）
        "补采记录": csw_collector_core::window_trace::patches_of(conn, round_id)
            .unwrap_or_default()
            .iter()
            .map(|p| serde_json::json!({
                "时段": format!("{} ~ {}", p.window_from, p.window_to),
                "补采时间": p.fetched_at,
                "接口返回": p.found,
                "去重后": p.fetched_unique,
                "窗口内": p.in_window,
                "query": p.query,
                "逐页请求日志": if p.query.contains("请求 GET") { "见 query" } else { PAGE_LOG_MISSING },
            }))
            .collect::<Vec<_>>(),
        // 引擎自查按登记条目数；这里是逐帖数，两者口径不同
        "披露早于窗口（逐帖）": rows
            .iter()
            .filter(|r| r.outcome == "in_window" && r.posted_at.as_deref().is_some_and(|p| p < round_start))
            .count(),
        "判过": judgements.len(),
        "判过但不在留痕窗口内": extra,
        // 主编要能核实际按哪个字段过滤、与首次入库口径差在哪些帖（10-01 r60 v1–v11 退回）
        "实际筛选依据": window_basis(&rows, round_start, round_end),
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
            "判定次序": "先去重，再筛图文，再按 posted_at（平台贴文时间）优先、缺失按首次入库，落在 [起, 止) 左闭右开（代码事实）；派工单口径为 first_seen_at，两口径逐键差异见「实际筛选依据」",
        },
    });
    Ok((out, summary))
}

/// 窗口实际按哪个字段筛、代码在哪、与「按首次入库」相比差在哪些帖（逐键）。
/// 两种入窗口径（按 posted_at / 按首次入库）逐键差异：
/// （按贴文时间在窗内而按入库不在，按入库在窗内而按贴文时间不在，没有贴文时间按入库判）。
/// 窗口起止读不出来是 None。
pub fn two_scope_diff(
    rows: &[csw_collector_core::window_trace::TraceRow],
    from: &str,
    to: &str,
) -> Option<(Vec<String>, Vec<String>, Vec<String>)> {
    let ts = |s: &Option<String>| s.as_deref().and_then(|x| x.parse::<jiff::Timestamp>().ok());
    let (Ok(f), Ok(t)) = (
        from.parse::<jiff::Timestamp>(),
        to.parse::<jiff::Timestamp>(),
    ) else {
        return None;
    };
    let inside = |x: Option<jiff::Timestamp>| x.is_some_and(|x| x >= f && x < t);
    let mut only_posted = Vec::new();
    let mut only_seen = Vec::new();
    let mut fallback = Vec::new();
    for r in rows {
        if matches!(
            r.outcome.as_str(),
            "dup_in_sweep" | "dup_across" | "not_image_only"
        ) {
            continue;
        }
        let (p, i) = (ts(&r.posted_at), ts(&r.ingested_at));
        if p.is_none() {
            if inside(i) {
                fallback.push(r.candidate_key.clone());
            }
            continue;
        }
        match (inside(p), inside(i)) {
            (true, false) => only_posted.push(r.candidate_key.clone()),
            (false, true) => only_seen.push(r.candidate_key.clone()),
            _ => {}
        }
    }
    Some((only_posted, only_seen, fallback))
}

/// 两口径差异的一句话，写进采集轮 query 与窗口摘要。
pub fn two_scope_sentence(conn: &Connection, round_id: i64, from: &str, to: &str) -> String {
    let rows = csw_collector_core::window_trace::of_round(conn, round_id).unwrap_or_default();
    match two_scope_diff(&rows, from, to) {
        None => "两口径差异未比对（窗口起止读不出来）".into(),
        Some((a, b, c)) if a.is_empty() && b.is_empty() && c.is_empty() => {
            "本批按 posted_at 与按首次入库筛出的集合相同（差异 0 条）".into()
        }
        Some((a, b, c)) => format!(
            "本批两口径有差异：按贴文时间在窗内而按入库不在 {} 条、按入库在窗内而按贴文时间不在 {} 条、无贴文时间按入库判 {} 条，逐键见 trace/window_summary.json「实际筛选依据」",
            a.len(),
            b.len(),
            c.len()
        ),
    }
}

fn window_basis(
    rows: &[csw_collector_core::window_trace::TraceRow],
    from: &str,
    to: &str,
) -> serde_json::Value {
    let Some((only_posted, only_seen, fallback)) = two_scope_diff(rows, from, to) else {
        return serde_json::json!({"说明": "窗口起止读不出来，未逐键比对"});
    };
    serde_json::json!({
        "窗口": format!("{from} ~ {to}（UTC，左闭右开）"),
        "实际过滤字段": "posted_at（csw 接口 postedAt，平台贴文时间）优先；posted_at 缺失时用 ingested_at（csw 首次入库）",
        "代码位置": "csw-collector crates/harvest/src/pipeline.rs window_outcome / in_window：c.posted_at.or(c.ingested_at) 落在 [起, 止) 内",
        "说明": "现行派工单（daily_news v9）与反馈 22 的入窗口径是 first_seen_at（首次入库）；工作台实际执行的是 posted_at 优先，这是代码事实如实记录，不代表现行规则。posted_at 只是平台贴文时间，不是另行核实过的首次披露——独立核到的首次披露另列在卡片上。定义 v10 草稿把口径改为原始披露，激活前以派工单为准",
        "first_seen_at 入口": "csw 接口 ingestedAt（贴文首次进入来源库的时间），逐条写在 trace/items.jsonl 与 trace/window_ids.jsonl 的 first_seen_at；两口径的逐键差异列在下面三组",
        "两口径结论": if only_posted.is_empty() && only_seen.is_empty() && fallback.is_empty() {
            "按原始披露与按首次入库筛出的集合相同（差异 0 条）；本期按哪个口径结果一致".to_string()
        } else {
            format!(
                "两口径有差异：按披露在窗内而按入库不在 {} 条、按入库在窗内而按披露不在 {} 条、无披露时间按入库判 {} 条，逐键见下",
                only_posted.len(),
                only_seen.len(),
                fallback.len()
            )
        },
        "按披露在窗内_按首次入库不在": {"条数": only_posted.len(), "条目": only_posted},
        "按首次入库在窗内_按披露不在（晚抓到的旧帖，已排除）": {"条数": only_seen.len(), "条目": only_seen},
        "无披露时间_按首次入库判": {"条数": fallback.len(), "条目": fallback},
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn 联系图一行一卡每行至多三张() {
        let jpeg = |c: u8| {
            let img = image::RgbImage::from_pixel(40, 30, image::Rgb([c, c, c]));
            let mut b = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Jpeg)
                .unwrap();
            b
        };
        let rows = vec![vec![jpeg(10), jpeg(20), jpeg(30), jpeg(40)], vec![jpeg(50)]];
        let out = contact_sheet(&rows).expect("该拼出来");
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!(
            (img.width(), img.height()),
            (3 * 320 + 4 * 8, 2 * 320 + 3 * 8)
        );
        assert!(contact_sheet(&[]).is_none());
    }

    use super::*;

    fn j(key: &str, tier: Tier) -> Judgement {
        let mut j = Judgement::fixture(key, tier);
        j.three_sentences.what = "换了背板".into();
        j.image_seen = tier != Tier::PendingCheck;
        j.inputs_hash = "ih".into();
        j
    }

    #[test]
    fn 主编旁注插在条目键下面且不重复插() {
        let body = "## 卡 2 ｜Comma9\n\n- 条目 `commanine-e30841` ｜档位：推荐\n- 其他\n\n### x｜y\n\n- 条目键：`commanine-e30841`\n";
        let line = "- **主编当前决定**（v5 退回意见原文）：旧译文仅留溯源";
        let once = insert_after(body, "- 条目 `commanine-e30841`", line);
        let once = insert_after(&once, "- 条目键：`commanine-e30841`", line);
        assert_eq!(once.matches(line).count(), 2, "{once}");
        let twice = insert_after(&once, "- 条目 `commanine-e30841`", line);
        assert_eq!(twice, once, "再跑一次不重复插");
    }

    #[test]
    fn 旧包里有这版没有的原图逐字节归档() {
        let dir = tempdir::TempDir::new("arch").unwrap();
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = rounds::open_round(
            &conn,
            &rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                run_id: Some(67),
                task_id: Some(763),
                stage_code: Some("intake".into()),
                target_version: 5,
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
        let v4 = dir.path().join("v4.zip");
        pack::build(
            "pkg_v4",
            &[
                pack::Entry::binary("images/drlv-4f2957_1.jpg", vec![1, 2, 3]),
                pack::Entry::binary("images/ankorau0-615184_1.jpg", vec![9]),
                pack::Entry::text("index.md", "x"),
            ],
            &v4,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deliverables_local(round_id, deliv_kind, item_key, zip_path, zip_sha256, zip_bytes, idem_key, created_at)
             VALUES (?1,'产出','',?2,'s',1,'i','t')",
            params![r.id, v4.to_string_lossy()],
        )
        .unwrap();
        let have: std::collections::HashSet<String> = ["images/ankorau0-615184_1.jpg".to_string()]
            .into_iter()
            .collect();
        let got = archived_images(&conn, r.id, &have);
        assert_eq!(
            got,
            vec![("images/drlv-4f2957_1.jpg".to_string(), vec![1, 2, 3])]
        );
    }

    #[test]
    fn 交付摘要带各档条数首批标题缺口和台账链接() {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = rounds::open_round(
            &conn,
            &rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                run_id: Some(67),
                task_id: Some(763),
                stage_code: Some("intake".into()),
                target_version: 1,
                parent_round_id: None,
                window_start: "2026-10-08T22:00:00Z".into(),
                window_end: "2026-10-09T22:00:00Z".into(),
                plan_version: 1,
                rubric_version: "v".into(),
                kb_snapshot: "k".into(),
                instructions_hash: String::new(),
            },
        )
        .unwrap();
        let mut a = j("a-111111", Tier::Recommend);
        a.headline = "品牌A 背包｜侧袋结构值得讲".into();
        let js = [
            a,
            j("b-222222", Tier::PendingCheck),
            j("c-333333", Tier::NotRecommend),
        ];
        let s = review_summary(
            &r,
            &Config::default(),
            &js,
            &["a-111111".into(), "b-222222".into()],
            &["每条都判：缺 2 条".into()],
        );
        assert!(
            s.starts_with("窗口 10-09 06:00 ~ 10-10 06:00（北京时间）"),
            "{s}"
        );
        assert!(
            s.contains("判了 3 条：推荐 1 · 备选 0 · 待核 1 · 不推荐 1"),
            "{s}"
        );
        assert!(s.contains("1. 品牌A 背包｜侧袋结构值得讲"), "{s}");
        assert!(s.contains("2. b-222222"), "没写标题的用条目键：{s}");
        assert!(s.contains("自查没过 1 项"), "{s}");
        assert!(
            s.contains(&format!(
                "https://collector.aworld.ltd/ledger?round={}",
                r.id
            )),
            "{s}"
        );
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

#[cfg(test)]
mod five_columns_tests {
    use super::*;

    #[test]
    fn 五栏与缺口如实写() {
        let f = FiveColumns {
            leads: 253,
            pending: 342,
            mature_primary: 0,
            mature_alt: 0,
            van_approved: 0,
            calib_stop: 4,
            calib_pending: 2,
            calib_continue: 2,
            new_cards: 4,
            target: Some((6, 2)),
            post_leads: 245,
            post_pending: 344,
            ..Default::default()
        };
        let s = five_columns_section(&f);
        assert!(s.contains("| 253 | 342 | 0 | 0 | 0 |"));
        assert!(s.contains("现差 6 主 2 备"));
        assert!(s.contains("停止 4 条、隔离待核 2 条、继续评估 2 条"));
        assert!(s.contains("首批另补 4 张卡"));
        assert!(s.contains("推荐 + 备选 245 帖、待核 344 帖"));
        assert!(s.contains("trace/five_columns.json"));
        assert!(five_columns_section(&FiveColumns::default()).contains("没给目标数"));
    }
}
