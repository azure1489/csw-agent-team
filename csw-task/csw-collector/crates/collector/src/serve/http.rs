//! 工作台 HTTP。契约见 `API.md`，这里只做实现。
//!
//! # 三条贯穿全文的规矩
//!
//! **一、接口里没有「分数」字段，也不会有。** Jev 初评的概率只在服务端决定
//! 「先判哪条」，任何响应里都不出现。这不是疏漏，是口径。
//!
//! **二、时间一律 RFC3339 UTC**，界面负责换算北京时间。
//! 接口返回本地时间的话，换个时区部署就全错了，而且错得很难看出来。
//!
//! **三、引擎的原文错误只进日志，给浏览器的是一句人话。**
//! 原文里可能带着引擎的内部路径与字段名，那是给我们排障用的，不是给页面看的。

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use rusqlite::OptionalExtension;

use csw_collector_core::{Config, ledger};

use crate::bff::{AuthState, Session};

pub struct AppState {
    pub conn: Mutex<rusqlite::Connection>,
    pub cfg: Config,
    /// 进程起来的时刻，给 `/healthz` 的 uptime
    pub started: std::time::Instant,
    /// 知识库检索要用的那几件（向量库、别名表、分词器、向量服务）。
    ///
    /// `None` 表示还没建起来——那时候知识库页要如实回 503，
    /// 而不是回一个空结果让人以为「库里什么都没有」。
    pub svc: Option<Arc<super::services::Services>>,
}

/// 无鉴权的两个：`/healthz` 与 `/metrics`。
///
/// `/metrics` **只在回环可达**——它会暴露轮次数、失败数这些内部计数，
/// 不该挂到公网上。限制由反代做（只 listen 127.0.0.1），这里再写一遍是提醒。
pub fn ops_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .with_state(state)
}

/// 工作台的只读接口。**每一个都要求一枚有效会话**。
///
/// 这里读得到的东西没有一样是可以匿名给出去的：整期的判断台账、谁把哪一条改了档、
/// Van 说过的原话（选题记忆里存的就是她的原话）。服务挂在公网域名上，
/// 前端那层 `Guard` 只是体验——真正拦人的是这道 `route_layer`。
///
/// 不校验 CSRF：读接口不改状态，跨站发起的读请求攻击者也读不到响应。
/// 校验会话的同时把 [`Session`] 放进 extensions，
/// 个别要再看角色的接口（`/api/settings`）自己从那里取。
pub fn api_router(state: Arc<AppState>, auth: Arc<AuthState>) -> Router {
    Router::new()
        .route("/api/rounds", get(list_rounds))
        .route("/api/rounds/{id}", get(round_detail))
        .route("/api/rounds/{id}/steps", get(round_steps))
        .route("/api/rounds/{id}/judgements", get(judgements))
        .route("/api/rounds/{id}/outbox", get(outbox))
        .route("/api/rounds/{id}/van-marks", get(van_marks))
        .route("/api/rounds/{id}/exclusions", get(round_exclusions))
        .route("/api/exclusions", get(exclusions))
        .route("/api/rounds/{id}/judgements/{key}", get(judgement_detail))
        .route("/api/rounds/{id}/coverage", get(coverage))
        .route("/api/rounds/{id}/media", get(round_media))
        .route("/api/van/today", get(van_today))
        .route("/api/rubric", get(rubric))
        .route("/api/memory/rules", get(memory_rules))
        .route("/api/memory/cases", get(memory_cases))
        .route("/api/metrics/selection", get(selection_metrics))
        .route("/api/kb/status", get(kb_status))
        .route("/api/kb/search", get(kb_search))
        .route("/api/kb/similar-selected", get(kb_similar))
        .route("/api/kb/brands/{brand}", get(kb_brand))
        .route("/api/kb/backfill", get(kb_backfill))
        .route("/api/pending-check", get(pending_check))
        .route("/api/work", get(work_queue))
        .route("/api/audit", get(audit_log))
        .route("/api/settings", get(settings))
        // 与 `/healthz` 同一个 handler，区别只在**这个要会话**。
        //
        // 裸的 `/healthz` 得留着不鉴权（探活与看门狗都从回环读它），但它会露出
        // 库路径与磁盘百分比，所以反代那层不往公网放。页面上要显示服务状态，
        // 走这个带鉴权的。
        .route("/api/healthz", get(healthz))
        // route_layer 只套在匹配到的路由上：没有的路径照常 404，不会先要求登录
        .route_layer(axum::middleware::from_fn_with_state(auth, require_session))
        .with_state(state)
}

/// 运维面的两个接口（`/api/settings`、`/api/audit`）要主编那一档。
///
/// `API.md` 的表里它们标的就是 operator，这里把那句话变成代码。
fn need_operator(s: &Session, what: &str) -> Result<(), ApiError> {
    if matches!(s.role.as_str(), "superadmin" | "operator") {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::FORBIDDEN,
            format!("{} 看不了{what}", s.role),
        ))
    }
}

/// 会话守卫。校验过的会话进 extensions，handler 需要角色时从那里取。
async fn require_session(
    State(auth): State<Arc<AuthState>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, ApiError> {
    let s = crate::bff::check_read(&auth, req.headers()).map_err(ApiError::from_bff)?;
    req.extensions_mut().insert(s);
    Ok(next.run(req).await)
}

// ─────────────────────────────── 运维 ───────────────────────────────

#[derive(Serialize)]
struct Health {
    ok: bool,
    uptime_secs: u64,
    /// 各依赖在不在。**不可达不等于整体不健康**——
    /// 向量服务挂了检索会差一点，但接单与登记照样能做。
    deps: Vec<DepStatus>,
    /// 要人核实的 outbox 条目数。不是 0 就该有人去看
    outbox_conflicts: usize,
}

#[derive(Serialize)]
struct DepStatus {
    name: String,
    ok: bool,
    note: String,
}

async fn healthz(State(st): State<Arc<AppState>>) -> Response {
    let conn = st.conn.lock().await;
    let db_ok = conn
        .query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
        .is_ok();
    let conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM engine_outbox WHERE status='conflict'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    drop(conn);

    let h = Health {
        // 本地库读不了才是真不健康：那时候什么都做不了
        ok: db_ok,
        uptime_secs: st.started.elapsed().as_secs(),
        deps: vec![
            DepStatus {
                name: "sqlite".into(),
                ok: db_ok,
                note: st.cfg.db_path().display().to_string(),
            },
            {
                use csw_collector_core::disk;
                let pct = disk::used_pct(&st.cfg.data_dir);
                let lv = disk::level(
                    pct,
                    st.cfg.limits.disk_warn_pct,
                    st.cfg.limits.disk_block_pct,
                );
                DepStatus {
                    name: "disk".into(),
                    // 到告警线就标不 ok：看板上要能一眼看见，别等它到拒开新轮那条线
                    ok: lv == disk::Level::Ok,
                    note: disk::note(
                        pct,
                        st.cfg.limits.disk_warn_pct,
                        st.cfg.limits.disk_block_pct,
                    ),
                }
            },
        ],
        outbox_conflicts: conflicts as usize,
    };
    let code = if h.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(h)).into_response()
}

/// Prometheus 文本格式。**故意只有计数，没有分数**。
async fn metrics(State(st): State<Arc<AppState>>) -> Response {
    let conn = st.conn.lock().await;
    let one = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    // 最近一个派单轮。查一次，七个指标共用——否则同一个子查询要写七遍，改漏一处就对不上
    let last = one(
        "SELECT COALESCE((SELECT id FROM rounds WHERE kind='task' ORDER BY id DESC LIMIT 1), 0)",
    );
    let of_last = |sql: &str| -> i64 {
        if last == 0 {
            return 0;
        }
        conn.query_row(sql, [last], |r| r.get(0)).unwrap_or(0)
    };
    // 开工时刻取 unix 秒：gauge 放不下 RFC3339，而看门狗要拿它算「都几点了还没动」
    let started = if last == 0 {
        0
    } else {
        of_last("SELECT CAST(strftime('%s', created_at) AS INTEGER) FROM rounds WHERE id=?1")
    };
    let body = format!(
        "# HELP csw_collector_rounds_total 开过的轮次数\n\
         # TYPE csw_collector_rounds_total counter\n\
         csw_collector_rounds_total {}\n\
         # HELP csw_collector_rounds_running 还在跑的轮次\n\
         # TYPE csw_collector_rounds_running gauge\n\
         csw_collector_rounds_running {}\n\
         # HELP csw_collector_judgements_total 判过的条目数\n\
         # TYPE csw_collector_judgements_total counter\n\
         csw_collector_judgements_total {}\n\
         # HELP csw_collector_pending_check 未结的待核条目\n\
         # TYPE csw_collector_pending_check gauge\n\
         csw_collector_pending_check {}\n\
         # HELP csw_collector_outbox_conflict 要人核实的写引擎条目\n\
         # TYPE csw_collector_outbox_conflict gauge\n\
         csw_collector_outbox_conflict {}\n\
         # HELP csw_collector_uptime_seconds 进程活了多久\n\
         # TYPE csw_collector_uptime_seconds gauge\n\
         csw_collector_uptime_seconds {}\n\
         # HELP csw_collector_disk_used_pct 数据盘用掉了百分之几（-1 = 问不出来）\n\
         # TYPE csw_collector_disk_used_pct gauge\n\
         csw_collector_disk_used_pct {}\n\
         # HELP csw_collector_work_queued 还排着队的手动活\n\
         # TYPE csw_collector_work_queued gauge\n\
         csw_collector_work_queued {}\n\
         # HELP csw_collector_last_task_round_started_unix 最近一个派单轮的开工时刻（0 = 从来没开过）\n\
         # TYPE csw_collector_last_task_round_started_unix gauge\n\
         csw_collector_last_task_round_started_unix {}\n\
         # HELP csw_collector_last_task_round_step 它做完了几步（满分 10）\n\
         # TYPE csw_collector_last_task_round_step gauge\n\
         csw_collector_last_task_round_step {}\n\
         # HELP csw_collector_last_task_round_candidates 这一轮的候选数\n\
         # TYPE csw_collector_last_task_round_candidates gauge\n\
         csw_collector_last_task_round_candidates {}\n\
         # HELP csw_collector_last_task_round_judged 其中判完的\n\
         # TYPE csw_collector_last_task_round_judged gauge\n\
         csw_collector_last_task_round_judged {}\n\
         # HELP csw_collector_last_task_round_media 这一轮候选的图片总数\n\
         # TYPE csw_collector_last_task_round_media gauge\n\
         csw_collector_last_task_round_media {}\n\
         # HELP csw_collector_last_task_round_media_described 其中识别出描述的\n\
         # TYPE csw_collector_last_task_round_media_described gauge\n\
         csw_collector_last_task_round_media_described {}\n\
         # HELP csw_collector_last_task_round_delivered 这一轮已经交出去的件数\n\
         # TYPE csw_collector_last_task_round_delivered gauge\n\
         csw_collector_last_task_round_delivered {}\n",
        one("SELECT COUNT(*) FROM rounds"),
        one("SELECT COUNT(*) FROM rounds WHERE status='running'"),
        one("SELECT COUNT(*) FROM judgements"),
        one("SELECT COUNT(DISTINCT candidate_key) FROM judgements WHERE tier='pending_check'"),
        one("SELECT COUNT(*) FROM engine_outbox WHERE status='conflict'"),
        st.started.elapsed().as_secs(),
        csw_collector_core::disk::used_pct(&st.cfg.data_dir)
            .map(i64::from)
            .unwrap_or(-1),
        one("SELECT COUNT(*) FROM work_queue WHERE status IN ('queued','running')"),
        // 下面七个是给看门狗用的：只 curl 这一个接口就够判断今早那一轮走到哪了，
        // 不必在生产机上装 sqlite3、也不必拿一枚会话。
        // **只看派单轮**——手动轮与预取轮不算，看门狗盯的是真派下来的那一期。
        started,
        of_last(
            "SELECT COUNT(DISTINCT step) FROM round_steps
             WHERE round_id=?1 AND status IN ('succeeded','partial')",
        ),
        of_last("SELECT COUNT(*) FROM round_candidates WHERE round_id=?1"),
        of_last("SELECT COUNT(*) FROM judgements WHERE round_id=?1"),
        of_last(
            "SELECT COUNT(*) FROM media m
             JOIN round_candidates rc ON rc.candidate_key = m.candidate_key
             WHERE rc.round_id=?1 AND m.kind='photo'",
        ),
        of_last(
            "SELECT COUNT(DISTINCT d.blake3) FROM media_descriptions d
             JOIN round_candidates rc ON rc.candidate_key = d.candidate_key
             WHERE rc.round_id=?1",
        ),
        of_last("SELECT COUNT(*) FROM deliverables_local WHERE round_id=?1"),
    );
    ([("content-type", "text/plain; version=0.0.4")], body).into_response()
}

// ─────────────────────────────── 轮次 ───────────────────────────────

#[derive(Deserialize)]
struct RoundsQuery {
    kind: Option<String>,
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Serialize)]
struct RoundBrief {
    id: i64,
    kind: String,
    trigger: String,
    run_id: Option<i64>,
    task_id: Option<i64>,
    window_start: String,
    window_end: String,
    status: String,
    note: String,
    created_at: String,
}

async fn list_rounds(
    State(st): State<Arc<AppState>>,
    Query(q): Query<RoundsQuery>,
) -> Result<Json<Vec<RoundBrief>>, ApiError> {
    let conn = st.conn.lock().await;
    let mut sql = String::from(
        "SELECT id, kind, trigger, run_id, task_id, window_start, window_end, status, note, created_at
         FROM rounds WHERE 1=1",
    );
    // 参数化，不拼值：kind 与 status 是从查询串来的，不是可信输入
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(k) = &q.kind {
        sql.push_str(" AND kind = ?");
        args.push(Box::new(k.clone()));
    }
    if let Some(s) = &q.status {
        sql.push_str(" AND status = ?");
        args.push(Box::new(s.clone()));
    }
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    args.push(Box::new(q.limit.clamp(1, 500) as i64));

    let mut stmt = conn.prepare(&sql).map_err(ApiError::db)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
            |r| {
                Ok(RoundBrief {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    trigger: r.get(2)?,
                    run_id: r.get(3)?,
                    task_id: r.get(4)?,
                    window_start: r.get(5)?,
                    window_end: r.get(6)?,
                    status: r.get(7)?,
                    note: r.get(8)?,
                    created_at: r.get(9)?,
                })
            },
        )
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

#[derive(Serialize)]
struct RoundDetail {
    #[serde(flatten)]
    brief: RoundBrief,
    rubric_version: String,
    kb_snapshot: String,
    instructions_hash: String,
    tiers: Vec<(String, usize)>,
    candidates: usize,
    carried: usize,
    unjudged: usize,
}

async fn round_detail(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<RoundDetail>, ApiError> {
    let conn = st.conn.lock().await;
    let (brief, rubric_version, kb_snapshot, instructions_hash) = conn
        .query_row(
            "SELECT id, kind, trigger, run_id, task_id, window_start, window_end, status, note,
                    created_at, rubric_version, kb_snapshot, instructions_hash
             FROM rounds WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    RoundBrief {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        trigger: r.get(2)?,
                        run_id: r.get(3)?,
                        task_id: r.get(4)?,
                        window_start: r.get(5)?,
                        window_end: r.get(6)?,
                        status: r.get(7)?,
                        note: r.get(8)?,
                        created_at: r.get(9)?,
                    },
                    r.get::<_, String>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
                ))
            },
        )
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "没有这一轮".into()))?;

    let (candidates, carried) = ledger::round_candidate_count(&conn, id).map_err(ApiError::any)?;
    Ok(Json(RoundDetail {
        brief,
        rubric_version,
        kb_snapshot,
        instructions_hash,
        tiers: ledger::tier_counts(&conn, id).map_err(ApiError::any)?,
        candidates,
        carried,
        unjudged: ledger::unjudged_keys(&conn, id)
            .map_err(ApiError::any)?
            .len(),
    }))
}

#[derive(Serialize)]
struct StepRow {
    step: String,
    attempt: i64,
    status: String,
    input_hash: String,
    counts: serde_json::Value,
    error: String,
    started_at: Option<String>,
    ended_at: Option<String>,
}

/// **每步的全部 attempt 都给**，不只是最后一次。
/// 「第一次是怎么失败的」正是排障时最想看的东西。
async fn round_steps(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<StepRow>>, ApiError> {
    let conn = st.conn.lock().await;
    let mut stmt = conn
        .prepare(
            "SELECT step, attempt, status, input_hash, counts_json, error, started_at, ended_at
             FROM round_steps WHERE round_id = ?1 ORDER BY id",
        )
        .map_err(ApiError::db)?;
    let rows = stmt
        .query_map([id], |r| {
            Ok(StepRow {
                step: r.get(0)?,
                attempt: r.get(1)?,
                status: r.get(2)?,
                input_hash: r.get(3)?,
                counts: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                error: r.get(5)?,
                started_at: r.get(6)?,
                ended_at: r.get(7)?,
            })
        })
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

#[derive(Deserialize)]
struct JudgementsQuery {
    tier: Option<String>,
    #[serde(default)]
    has_gap: bool,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Serialize)]
struct JudgementRow {
    candidate_key: String,
    /// 模型的原判。**人工改过档也不动它**
    tier: String,
    /// 现在生效的那一档：改过就是改到的，没改过就是原判
    effective_tier: String,
    /// 改档的理由与人；没改过是空的
    override_reason: String,
    override_actor: String,
    /// 主编指定进了深核首批
    first_batch: bool,
    dims: serde_json::Value,
    three_sentences: serde_json::Value,
    comparison: serde_json::Value,
    heat_note: String,
    image_seen: bool,
    gaps: serde_json::Value,
    check_flags: serde_json::Value,
    account: String,
    url: String,
}

async fn judgements(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<JudgementsQuery>,
) -> Result<Json<Vec<JudgementRow>>, ApiError> {
    let conn = st.conn.lock().await;
    // 生效档 = 最后一次改档改到的那一档，没改过就是原判。
    // **筛选与排序都按生效档**：主编把一条捞回成备选之后，台账还把它排在
    // 不推荐那一组里的话，那次捞回等于没发生。
    let mut sql = String::from(
        "SELECT j.candidate_key, j.tier, j.dims_json, j.three_json, j.comparison_json,
                j.heat_note, j.image_seen, j.gaps_json, j.check_flags_json,
                COALESCE(c.account,''), COALESCE(c.url,''),
                COALESCE(o.to_tier, j.tier), COALESCE(o.reason,''), COALESCE(o.actor,''),
                COALESCE(rc.first_batch, 0)
         FROM judgements j
         LEFT JOIN candidates c ON c.candidate_key = j.candidate_key
         LEFT JOIN round_candidates rc
                ON rc.round_id = j.round_id AND rc.candidate_key = j.candidate_key
         LEFT JOIN judgement_overrides o
                ON o.id = (SELECT MAX(id) FROM judgement_overrides
                           WHERE round_id = j.round_id AND candidate_key = j.candidate_key)
         WHERE j.round_id = ?",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(id)];
    if let Some(t) = &q.tier {
        sql.push_str(" AND COALESCE(o.to_tier, j.tier) = ?");
        args.push(Box::new(t.clone()));
    }
    if q.has_gap {
        sql.push_str(" AND j.gaps_json <> '[]'");
    }
    // 排序由服务端定：**没有分数可排**，按档再按键
    sql.push_str(
        " ORDER BY CASE COALESCE(o.to_tier, j.tier)
                     WHEN 'recommend' THEN 0 WHEN 'alternate' THEN 1
                     WHEN 'pending_check' THEN 2 ELSE 3 END, j.candidate_key LIMIT ?",
    );
    args.push(Box::new(q.limit.clamp(1, 1000) as i64));

    let mut stmt = conn.prepare(&sql).map_err(ApiError::db)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
            |r| {
                let j = |s: String| serde_json::from_str(&s).unwrap_or_default();
                Ok(JudgementRow {
                    candidate_key: r.get(0)?,
                    tier: r.get(1)?,
                    effective_tier: r.get(11)?,
                    override_reason: r.get(12)?,
                    override_actor: r.get(13)?,
                    first_batch: r.get::<_, i64>(14)? == 1,
                    dims: j(r.get(2)?),
                    three_sentences: j(r.get(3)?),
                    comparison: j(r.get(4)?),
                    heat_note: r.get(5)?,
                    image_seen: r.get::<_, i64>(6)? == 1,
                    gaps: j(r.get(7)?),
                    check_flags: j(r.get(8)?),
                    account: r.get(9)?,
                    url: r.get(10)?,
                })
            },
        )
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

#[derive(Serialize)]
struct OutboxRow {
    seq: i64,
    kind: String,
    status: String,
    attempts: i64,
    last_error: String,
    /// `conflict` 的要人核实，**不要换幂等键重试**
    needs_human: bool,
}

async fn outbox(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<OutboxRow>>, ApiError> {
    let conn = st.conn.lock().await;
    let mut stmt = conn
        .prepare(
            "SELECT seq, kind, status, attempts, last_error FROM engine_outbox
             WHERE round_id = ?1 ORDER BY seq",
        )
        .map_err(ApiError::db)?;
    let rows = stmt
        .query_map([id], |r| {
            let status: String = r.get(2)?;
            Ok(OutboxRow {
                seq: r.get(0)?,
                kind: r.get(1)?,
                needs_human: status == "conflict" || status == "dead",
                status,
                attempts: r.get(3)?,
                last_error: r.get(4)?,
            })
        })
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

/// 跨轮未结的待核。**待核不是淘汰**，补齐材料后重评。
async fn pending_check(State(st): State<Arc<AppState>>) -> Result<Json<Vec<String>>, ApiError> {
    let conn = st.conn.lock().await;
    Ok(Json(
        ledger::open_pending_checks(&conn, 500).map_err(ApiError::any)?,
    ))
}

#[derive(Serialize)]
struct Settings {
    engine_base_url: String,
    csw_base_url: String,
    vector_base_url: String,
    model: String,
    fallback_model: String,
    embed_model: String,
    jev_enabled: bool,
    features: serde_json::Value,
    /// 密钥**只说在不在**，不返回值
    secrets: Vec<(String, bool)>,
}

async fn settings(
    State(st): State<Arc<AppState>>,
    axum::Extension(s): axum::Extension<Session>,
) -> Result<Json<Settings>, ApiError> {
    // 前端那页挂着 `Guard need="operator"`，服务端得说同一句话。
    // 这里露的是引擎地址、网关地址、开了哪些开关——运维面的东西，不给 viewer 与 van。
    need_operator(&s, "运行设置")?;
    let c = &st.cfg;
    let secrets = csw_collector_core::Secrets::from_env();
    Ok(Json(Settings {
        engine_base_url: c.engine.base_url.clone(),
        csw_base_url: c.csw.base_url.clone(),
        vector_base_url: c.vector.base_url.clone(),
        model: c.model.model.clone(),
        fallback_model: c.model.fallback_model.clone(),
        embed_model: c.vector.embed_model.clone(),
        jev_enabled: c.jev.enabled,
        features: serde_json::to_value(&c.features).unwrap_or_default(),
        // 只回「已配置 / 缺」。返回值等于把密钥送到浏览器里
        secrets: vec![
            ("CSW_API_KEY".into(), !secrets.csw_api_key.is_empty()),
            ("SUB2API_API_KEY".into(), !secrets.sub2api_key.is_empty()),
            ("TYPESAFE_API_KEY".into(), !secrets.typesafe_key.is_empty()),
            ("CSW_ENGINE_TOKEN".into(), !secrets.engine_token.is_empty()),
            // 告警地址同样只回「已配置 / 缺」：它是一个谁拿到都能往群里发消息的地址。
            // 但「有没有配」要让人看得见——没配的话，磁盘满了也没人会知道
            (
                "CSW_COLLECTOR_ALERT_WEBHOOK".into(),
                !c.alert.webhook_url.is_empty(),
            ),
        ],
    }))
}

/// 一条的全貌：候选原文、每张图与它的描述、判断、改档、深核、模型调用。
///
/// **一次给全**：这一页是人在「这条为什么是这个档」上打转的地方，
/// 分七个接口去拿，页面上就会出现七个各自转圈的小方块。
#[derive(Serialize)]
struct JudgementDetail {
    candidate: serde_json::Value,
    judgement: Option<serde_json::Value>,
    effective_tier: String,
    overrides: Vec<csw_collector_core::workbench::Override>,
    marks: Vec<csw_collector_core::workbench::VanMark>,
    images: Vec<ImageRow>,
    deepcheck: Option<serde_json::Value>,
    model_calls: Vec<ModelCallRow>,
    first_batch: bool,
}

#[derive(Serialize)]
struct ImageRow {
    blake3: String,
    ordinal: i64,
    url: String,
    failed: bool,
    kind: String,
    content: String,
    matches_text: String,
    missing_from_text: String,
    usable_as_figure: bool,
    model: String,
    prompt_version: String,
}

#[derive(Serialize)]
struct ModelCallRow {
    purpose: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    latency_ms: i64,
    attempts: i64,
    status: String,
    error: String,
    created_at: String,
}

async fn judgement_detail(
    State(st): State<Arc<AppState>>,
    Path((id, key)): Path<(i64, String)>,
) -> Result<Json<JudgementDetail>, ApiError> {
    let conn = st.conn.lock().await;
    let cand = ledger::get_candidate(&conn, &key)
        .map_err(ApiError::any)?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "台账里没有这一条".into()))?;

    let judgement: Option<serde_json::Value> = conn
        .query_row(
            "SELECT tier, dims_json, three_json, unanswered, comparison_json, heat_note, look,
                    image_seen, gaps_json, priority_hits_json, lower_hits_json, jev_disagreement,
                    check_flags_json, kb_refs_json, memory_refs_json, inputs_hash, model,
                    rubric_version, created_at
             FROM judgements WHERE round_id=?1 AND candidate_key=?2",
            rusqlite::params![id, key],
            |r| {
                let j = |i: usize| -> serde_json::Value {
                    serde_json::from_str(&r.get::<_, String>(i).unwrap_or_default())
                        .unwrap_or_default()
                };
                Ok(serde_json::json!({
                    "tier": r.get::<_, String>(0)?,
                    "dims": j(1),
                    "three_sentences": j(2),
                    "unanswered": r.get::<_, String>(3)?,
                    "comparison": j(4),
                    "heat_note": r.get::<_, String>(5)?,
                    "look": r.get::<_, String>(6)?,
                    "image_seen": r.get::<_, i64>(7)? == 1,
                    "gaps": j(8),
                    "priority_hits": j(9),
                    "lower_hits": j(10),
                    "jev_disagreement": r.get::<_, String>(11)?,
                    "check_flags": j(12),
                    "kb_refs": j(13),
                    "memory_refs": j(14),
                    "inputs_hash": r.get::<_, String>(15)?,
                    "model": r.get::<_, String>(16)?,
                    "rubric_version": r.get::<_, String>(17)?,
                    "created_at": r.get::<_, String>(18)?,
                }))
            },
        )
        .optional()
        .map_err(ApiError::db)?;

    let mut st_img = conn
        .prepare(
            "SELECT m.blake3, m.ordinal, m.url, m.failed,
                    COALESCE(d.kind,''), COALESCE(d.content,''), COALESCE(d.matches_text,''),
                    COALESCE(d.missing_from_text,''), COALESCE(d.usable_as_figure,0),
                    COALESCE(d.model,''), COALESCE(d.prompt_version,'')
             FROM media m
             LEFT JOIN media_descriptions d
                    ON d.blake3 = m.blake3 AND d.candidate_key = m.candidate_key
             WHERE m.candidate_key = ?1 ORDER BY m.ordinal",
        )
        .map_err(ApiError::db)?;
    let images: Vec<ImageRow> = st_img
        .query_map([&key], |r| {
            Ok(ImageRow {
                blake3: r.get(0)?,
                ordinal: r.get(1)?,
                url: r.get(2)?,
                failed: r.get::<_, i64>(3)? == 1,
                kind: r.get(4)?,
                content: r.get(5)?,
                matches_text: r.get(6)?,
                missing_from_text: r.get(7)?,
                usable_as_figure: r.get::<_, i64>(8)? == 1,
                model: r.get(9)?,
                prompt_version: r.get(10)?,
            })
        })
        .map_err(ApiError::db)?
        .filter_map(Result::ok)
        .collect();

    let deepcheck: Option<serde_json::Value> = conn
        .query_row(
            "SELECT status, result_json, started_at, COALESCE(ended_at,'')
             FROM deepchecks WHERE round_id=?1 AND candidate_key=?2",
            rusqlite::params![id, key],
            |r| {
                Ok(serde_json::json!({
                    "status": r.get::<_, String>(0)?,
                    "result": serde_json::from_str::<serde_json::Value>(
                        &r.get::<_, String>(1)?).unwrap_or_default(),
                    "started_at": r.get::<_, String>(2)?,
                    "ended_at": r.get::<_, String>(3)?,
                }))
            },
        )
        .optional()
        .map_err(ApiError::db)?;

    let mut st_calls = conn
        .prepare(
            "SELECT purpose, model, input_tokens, output_tokens, latency_ms, attempts,
                    status, error, created_at
             FROM model_calls WHERE round_id=?1 ORDER BY id DESC LIMIT 50",
        )
        .map_err(ApiError::db)?;
    let model_calls: Vec<ModelCallRow> = st_calls
        .query_map([id], |r| {
            Ok(ModelCallRow {
                purpose: r.get(0)?,
                model: r.get(1)?,
                input_tokens: r.get(2)?,
                output_tokens: r.get(3)?,
                latency_ms: r.get(4)?,
                attempts: r.get(5)?,
                status: r.get(6)?,
                error: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .map_err(ApiError::db)?
        .filter_map(Result::ok)
        .collect();

    let first_batch: i64 = conn
        .query_row(
            "SELECT COALESCE(first_batch,0) FROM round_candidates
             WHERE round_id=?1 AND candidate_key=?2",
            rusqlite::params![id, &key],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let all_marks = csw_collector_core::workbench::van_marks(&conn, id).map_err(ApiError::any)?;
    let all_ovr =
        csw_collector_core::workbench::latest_overrides(&conn, id).map_err(ApiError::any)?;

    Ok(Json(JudgementDetail {
        effective_tier: csw_collector_core::workbench::effective_tier(&conn, id, &key)
            .map_err(ApiError::any)?
            .unwrap_or_default(),
        candidate: serde_json::to_value(&cand).unwrap_or_default(),
        judgement,
        overrides: all_ovr
            .into_iter()
            .filter(|o| o.candidate_key == key)
            .collect(),
        marks: all_marks
            .into_iter()
            .filter(|m| m.candidate_key == key)
            .collect(),
        images,
        deepcheck,
        model_calls,
        first_batch: first_batch == 1,
    }))
}

/// 每个采集器一行。**数字来自我们自己发出去的那一份**（outbox 里的字节），
/// 不是现场再算一遍——页面上看到的要与引擎收到的是同一份。
async fn coverage(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let body: Option<String> = conn
        .query_row(
            "SELECT body_json FROM engine_outbox
             WHERE round_id=?1 AND kind='sweeps' ORDER BY seq DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(ApiError::db)?;
    let sweeps = body
        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("sweeps").cloned())
        .unwrap_or_else(|| serde_json::json!([]));

    // 采集那一步的耗时账在 round_steps 里，一并给出来：
    // 「取到多少」与「花了多久」总是一起看的
    let harvest: Option<serde_json::Value> = conn
        .query_row(
            "SELECT counts_json FROM round_steps
             WHERE round_id=?1 AND step='harvest' ORDER BY attempt DESC LIMIT 1",
            [id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(ApiError::db)?
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

    Ok(Json(serde_json::json!({
        "sweeps": sweeps,
        "harvest": harvest.unwrap_or_else(|| serde_json::json!({})),
    })))
}

#[derive(Deserialize)]
struct MediaQuery {
    /// failed = 只看没下到或没识别成的
    #[serde(default)]
    state: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

async fn round_media(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<MediaQuery>,
) -> Result<Json<Vec<ImageRow>>, ApiError> {
    let conn = st.conn.lock().await;
    let only_failed = q.state == "failed";
    let mut stmt = conn
        .prepare(
            "SELECT m.blake3, m.ordinal, m.url, m.failed,
                    COALESCE(d.kind,''), COALESCE(d.content,''), COALESCE(d.matches_text,''),
                    COALESCE(d.missing_from_text,''), COALESCE(d.usable_as_figure,0),
                    COALESCE(d.model,''), COALESCE(d.prompt_version,'')
             FROM media m
             JOIN round_candidates rc ON rc.candidate_key = m.candidate_key AND rc.round_id = ?1
             LEFT JOIN media_descriptions d
                    ON d.blake3 = m.blake3 AND d.candidate_key = m.candidate_key
             WHERE (?2 = 0 OR m.failed = 1)
             ORDER BY m.candidate_key, m.ordinal LIMIT ?3",
        )
        .map_err(ApiError::db)?;
    let rows = stmt
        .query_map(
            rusqlite::params![id, i64::from(only_failed), q.limit.clamp(1, 2000) as i64],
            |r| {
                Ok(ImageRow {
                    blake3: r.get(0)?,
                    ordinal: r.get(1)?,
                    url: r.get(2)?,
                    failed: r.get::<_, i64>(3)? == 1,
                    kind: r.get(4)?,
                    content: r.get(5)?,
                    matches_text: r.get(6)?,
                    missing_from_text: r.get(7)?,
                    usable_as_figure: r.get::<_, i64>(8)? == 1,
                    model: r.get(9)?,
                    prompt_version: r.get(10)?,
                })
            },
        )
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

/// Van 模式的当期视图：**只给推荐与备选**，每条三句话与代表图。
///
/// 不给不推荐的：她那一页是手机上看的，翻三百条不是在帮她。
/// 要看全部去判断台账——那是主编的页面。
#[derive(Serialize)]
struct VanItem {
    candidate_key: String,
    tier: String,
    title: String,
    brand: String,
    url: String,
    three_sentences: serde_json::Value,
    heat_note: String,
    /// 代表图（第一张能当配图的）的内容描述
    look: String,
    marks: Vec<String>,
}

async fn van_today(State(st): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let round: Option<i64> = conn
        .query_row(
            "SELECT id FROM rounds WHERE kind='task' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(ApiError::db)?;
    let Some(round) = round else {
        // 还没开工不是错误：页面显示「今天还没开始」比显示一个错误体强
        return Ok(Json(serde_json::json!({ "round_id": null, "items": [] })));
    };
    let marks = csw_collector_core::workbench::van_marks(&conn, round).map_err(ApiError::any)?;
    let mut stmt = conn
        .prepare(
            "SELECT j.candidate_key, COALESCE(o.to_tier, j.tier), j.three_json, j.heat_note,
                    j.look, COALESCE(c.account,''), COALESCE(c.url,'')
             FROM judgements j
             LEFT JOIN candidates c ON c.candidate_key = j.candidate_key
             LEFT JOIN judgement_overrides o
                    ON o.id = (SELECT MAX(id) FROM judgement_overrides
                               WHERE round_id = j.round_id AND candidate_key = j.candidate_key)
             WHERE j.round_id = ?1 AND COALESCE(o.to_tier, j.tier) IN ('recommend','alternate')
             ORDER BY CASE COALESCE(o.to_tier, j.tier) WHEN 'recommend' THEN 0 ELSE 1 END,
                      j.candidate_key",
        )
        .map_err(ApiError::db)?;
    let items: Vec<VanItem> = stmt
        .query_map([round], |r| {
            let key: String = r.get(0)?;
            let three: serde_json::Value =
                serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default();
            Ok(VanItem {
                title: three
                    .get("what_changed")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                tier: r.get(1)?,
                three_sentences: three,
                heat_note: r.get(3)?,
                look: r.get(4)?,
                brand: r.get(5)?,
                url: r.get(6)?,
                marks: vec![],
                candidate_key: key,
            })
        })
        .map_err(ApiError::db)?
        .filter_map(Result::ok)
        .map(|mut it| {
            it.marks = marks
                .iter()
                .filter(|m| m.candidate_key == it.candidate_key)
                .map(|m| m.mark.clone())
                .collect();
            it
        })
        .collect();
    Ok(Json(
        serde_json::json!({ "round_id": round, "items": items }),
    ))
}

/// 判断框架与锚点。**从代码里的常量来**，不是从库里——
/// 它要跟着提示词一起走，改动经 `RUBRIC_VERSION` 进版本。
async fn rubric() -> Json<serde_json::Value> {
    use csw_collector_judge::rubric as r;
    Json(serde_json::json!({
        "version": r::RUBRIC_VERSION,
        "core_question": r::CORE_QUESTION,
        "three_questions": r::THREE_QUESTIONS,
        "priority": r::PRIORITY,
        "lower": r::LOWER,
        "dim_anchors": r::DIM_ANCHORS
            .iter()
            .map(|(k, v)| serde_json::json!({ "dim": k, "anchor": v }))
            .collect::<Vec<_>>(),
        "not_dimensions": r::NOT_DIMENSIONS,
    }))
}

#[derive(Serialize)]
struct MemoryRule {
    rule_key: String,
    text: String,
    version: String,
    confirmed_by_van: bool,
    updated_at: String,
}

async fn memory_rules(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<MemoryRule>>, ApiError> {
    let conn = st.conn.lock().await;
    let mut stmt = conn
        .prepare(
            "SELECT rule_key, text, version, confirmed_by_van, updated_at
             FROM memory_rules ORDER BY confirmed_by_van DESC, rule_key LIMIT ?1",
        )
        .map_err(ApiError::db)?;
    let rows = stmt
        .query_map([q.limit.clamp(1, 500) as i64], |r| {
            Ok(MemoryRule {
                rule_key: r.get(0)?,
                text: r.get(1)?,
                version: r.get(2)?,
                confirmed_by_van: r.get::<_, i64>(3)? == 1,
                updated_at: r.get(4)?,
            })
        })
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

#[derive(Serialize)]
struct MemoryCase {
    case_key: String,
    decision: String,
    /// Van 原话。**一字不改**——改写过的原话不能拿去跟她对质
    quote: String,
    source_url: String,
    decided_at: String,
}

async fn memory_cases(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<MemoryCase>>, ApiError> {
    let conn = st.conn.lock().await;
    let mut stmt = conn
        .prepare(
            "SELECT case_key, decision, quote, source_url, COALESCE(decided_at,'')
             FROM memory_cases ORDER BY decided_at DESC, case_key LIMIT ?1",
        )
        .map_err(ApiError::db)?;
    let rows = stmt
        .query_map([q.limit.clamp(1, 500) as i64], |r| {
            Ok(MemoryCase {
                case_key: r.get(0)?,
                decision: r.get(1)?,
                quote: r.get(2)?,
                source_url: r.get(3)?,
                decided_at: r.get(4)?,
            })
        })
        .map_err(ApiError::db)?;
    Ok(Json(rows.filter_map(Result::ok).collect()))
}

/// 选题指标。**都是计数，没有一个是分数。**
///
/// 「模型判了什么」与「人改成了什么」分开算：两者重合得越少，
/// 说明判断框架离 Van 的口味越远——那正是要看的东西。
async fn selection_metrics(
    State(st): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let one = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    Ok(Json(serde_json::json!({
        "轮次": one("SELECT COUNT(*) FROM rounds WHERE kind='task'"),
        "判过的条目": one("SELECT COUNT(*) FROM judgements"),
        "推荐": one("SELECT COUNT(*) FROM judgements WHERE tier='recommend'"),
        "备选": one("SELECT COUNT(*) FROM judgements WHERE tier='alternate'"),
        "待核": one("SELECT COUNT(*) FROM judgements WHERE tier='pending_check'"),
        "不推荐": one("SELECT COUNT(*) FROM judgements WHERE tier='not_recommend'"),
        "没读到实图": one("SELECT COUNT(*) FROM judgements WHERE image_seen=0"),
        "人工改档": one("SELECT COUNT(*) FROM judgement_overrides"),
        "被捞回的": one(
            "SELECT COUNT(*) FROM judgement_overrides
             WHERE from_tier IN ('not_recommend','pending_check')
               AND to_tier IN ('recommend','alternate')"
        ),
        "被压下的": one(
            "SELECT COUNT(*) FROM judgement_overrides
             WHERE from_tier IN ('recommend','alternate')
               AND to_tier IN ('not_recommend','pending_check')"
        ),
        "Van 勾选": one("SELECT COUNT(*) FROM van_marks"),
        "未结待核": one(
            "SELECT COUNT(DISTINCT candidate_key) FROM judgements WHERE tier='pending_check'"
        ),
    })))
}

/// 知识库各来源的水位。**不是「库里有多少」，是「同步到哪天了」**——
/// 前者好看，后者才回答「今天的判断有没有拿到昨天的已发条目」。
async fn kb_status(State(st): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let mut stmt = conn
        .prepare("SELECT source, cursor, synced_at FROM kb_cursors ORDER BY source")
        .map_err(ApiError::db)?;
    let cursors: Vec<serde_json::Value> = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "source": r.get::<_, String>(0)?,
                "cursor": r.get::<_, String>(1)?,
                "synced_at": r.get::<_, String>(2)?,
            }))
        })
        .map_err(ApiError::db)?
        .filter_map(Result::ok)
        .collect();
    let one = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    Ok(Json(serde_json::json!({
        "cursors": cursors,
        "docs": one("SELECT COUNT(*) FROM kb_docs"),
        "docs_by_kind": {
            "published_item": one("SELECT COUNT(*) FROM kb_docs WHERE kind='published_item'"),
            "example": one("SELECT COUNT(*) FROM kb_docs WHERE kind='example'"),
            "generated_post": one("SELECT COUNT(*) FROM kb_docs WHERE kind='generated_post'"),
            "decision": one("SELECT COUNT(*) FROM kb_docs WHERE kind='decision'"),
        },
        // 判据与 kb::docs::needs_embedding 同一条：**换了模型的也算待算**，
        // 不是「没算过的才算」——混着两个模型的向量，检索会悄悄失准且不报错
        "待算向量": conn
            .query_row(
                "SELECT COUNT(*) FROM kb_docs WHERE embed_model <> ?1",
                [&st.cfg.vector.embed_model],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0),
        "品牌": one("SELECT COUNT(*) FROM brands"),
        "别名": one("SELECT COUNT(*) FROM brand_aliases"),
        "embed_model": st.cfg.vector.embed_model,
    })))
}

#[derive(Deserialize)]
struct KbQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

/// 知识库检索。**这条路上不重排**——重排要占 GPU，而 GPU 是全进程串行的；
/// 有人在页面上连着搜几下，正式轮的向量化就堵住了。
///
/// 三段之间把连接的锁放掉：`rusqlite::Connection` 不是 `Sync`，
/// 跨 `await` 持有它的 future 不是 `Send`，axum 的 handler 就编不过。
async fn kb_search(
    State(st): State<Arc<AppState>>,
    Query(q): Query<KbQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if q.q.trim().is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "要搜什么".into()));
    }
    let svc = st.svc.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "知识库还没起来".into(),
    ))?;
    let vector = embed_query(svc, &q.q).await;
    let query = csw_collector_kb::search::Query {
        text: &q.q,
        vector: vector.as_deref(),
        exclude_post_id: None,
        limit: q.limit.clamp(1, 50),
        // 检索工具不补齐：被问「有没有关于 X 的」时，补齐会让答案永远是「有」
        backfill_kinds: false,
    };
    let r = csw_collector_kb::search::Retriever {
        store: &svc.store,
        brands: &svc.brands,
        tok: &svc.tok,
        reranker: None,
    };
    let ids = r.vector_route(&query).await.map_err(ApiError::any)?;
    let recalled = {
        let conn = st.conn.lock().await;
        r.recall(&conn, &query, &ids).map_err(ApiError::any)?
    };
    let out = {
        let conn = st.conn.lock().await;
        r.finish(&conn, &query, recalled).map_err(ApiError::any)?
    };
    Ok(Json(retrieved_json(&out)))
}

/// 与某条候选相似的已采用条目。
///
/// 用候选正文现算一次查询向量——候选的融合向量没存下来（它是一次性的）。
/// 纯文本一次约一百毫秒，页面上点一下等得起。
async fn kb_similar(
    State(st): State<Arc<AppState>>,
    Query(q): Query<KbQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let svc = st.svc.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "知识库还没起来".into(),
    ))?;
    // q 这里是 candidate_key
    let (text, post_id) = {
        let conn = st.conn.lock().await;
        let c = ledger::get_candidate(&conn, q.q.trim())
            .map_err(ApiError::any)?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "台账里没有这一条".into()))?;
        (format!("{}\n{}", c.text, c.translated), c.source_id)
    };
    let vector = embed_query(svc, &text).await;
    let query = csw_collector_kb::search::Query {
        text: &text,
        vector: vector.as_deref(),
        // 把它自己排除掉：拿自己跟自己比没有意义
        exclude_post_id: Some(post_id),
        limit: q.limit.clamp(1, 50),
        backfill_kinds: false,
    };
    let r = csw_collector_kb::search::Retriever {
        store: &svc.store,
        brands: &svc.brands,
        tok: &svc.tok,
        reranker: None,
    };
    let ids = r.vector_route(&query).await.map_err(ApiError::any)?;
    let recalled = {
        let conn = st.conn.lock().await;
        r.recall(&conn, &query, &ids).map_err(ApiError::any)?
    };
    let out = {
        let conn = st.conn.lock().await;
        r.finish(&conn, &query, recalled).map_err(ApiError::any)?
    };
    Ok(Json(retrieved_json(&out)))
}

/// 某品牌的历史覆盖。**纯查库**，不碰 GPU：这一页是拿来翻的，不是拿来搜的。
async fn kb_brand(
    State(st): State<Arc<AppState>>,
    Path(brand): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let docs = csw_collector_kb::docs::by_brand(&conn, &brand, q.limit.clamp(1, 200))
        .map_err(ApiError::any)?;
    Ok(Json(serde_json::json!({
        "brand": brand,
        "docs": docs.iter().map(doc_json).collect::<Vec<_>>(),
    })))
}

/// 待补录清单：出现在选题里、却不在 csw 在册名单里的品牌。
/// 别名表每次现读——它会被 `kb sync --refresh-brands` 与人工加别名改掉，
/// 用常驻那份就对不上刚补录的账号。九百来行，读一次是毫秒级。
async fn kb_backfill(State(st): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let brands = csw_collector_kb::brands::BrandIndex::load(&conn).map_err(ApiError::any)?;
    let rows = csw_collector_kb::coverage::backfill_list(&conn, &brands).map_err(ApiError::any)?;
    Ok(Json(serde_json::json!({
        "registered_brands": brands.len(),
        "rows": rows,
    })))
}

async fn embed_query(svc: &super::services::Services, text: &str) -> Option<Vec<f32>> {
    let inputs = [csw_collector_core::vector::EmbedInput::Text(
        text.chars().take(2000).collect(),
    )];
    match svc.vector.embed(&inputs).await {
        Ok(mut vs) if !vs.is_empty() => Some(vs.remove(0)),
        // 算不出向量不算错：品牌与全文两路照样能召回
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(原因 = %format!("{e:#}"), "算查询向量失败，只走品牌与全文两路");
            None
        }
    }
}

fn doc_json(d: &csw_collector_kb::docs::KbDoc) -> serde_json::Value {
    serde_json::json!({
        "id": d.id,
        "kind": d.kind,
        "ref_id": d.ref_id,
        "title": d.title,
        "brand": d.brand,
        "url": d.url,
        "published_at": d.published_at,
        "publish_state": d.publish_state,
        "is_reference": d.is_reference,
        // 正文只给一小段：这一页是用来挑的，挑中了再去看原文
        "snippet": d.body.chars().take(160).collect::<String>(),
    })
}

fn retrieved_json(r: &csw_collector_kb::search::Retrieved) -> serde_json::Value {
    serde_json::json!({
        "brands_hit": r.brands_hit,
        "missing_kinds": r.missing_kinds,
        "counts": {
            "vector": r.counts.vector,
            "brand": r.counts.brand,
            "fts": r.counts.fts,
            "merged": r.counts.merged,
            "truncated": r.counts.truncated,
        },
        "docs": r.docs.iter().map(|s| {
            let mut v = doc_json(&s.doc);
            v["routes"] = serde_json::json!(s.routes.names());
            v["backfilled"] = serde_json::json!(s.backfilled);
            v
        }).collect::<Vec<_>>(),
    })
}

async fn van_marks(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<csw_collector_core::workbench::VanMark>>, ApiError> {
    let conn = st.conn.lock().await;
    Ok(Json(
        csw_collector_core::workbench::van_marks(&conn, id).map_err(ApiError::any)?,
    ))
}

/// 全部排除规则，停用的也给——那一页要把没生效的也摆出来让人确认。
async fn exclusions(
    State(st): State<Arc<AppState>>,
) -> Result<Json<Vec<csw_collector_core::exclusion::Exclusion>>, ApiError> {
    let conn = st.conn.lock().await;
    Ok(Json(
        csw_collector_core::exclusion::all(&conn).map_err(ApiError::any)?,
    ))
}

/// 这一轮哪几条被规则挡下了，判据是多少，谁捞回过。
///
/// 台账上「它为什么没判」只能从这儿答——被排除的条目没走模型，
/// `judgements` 里那一行的六维全是「不明」。
async fn round_exclusions(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let conn = st.conn.lock().await;
    let hits = csw_collector_core::exclusion::hits(&conn, id).map_err(ApiError::any)?;
    let rules = csw_collector_core::exclusion::all(&conn).map_err(ApiError::any)?;
    let by_id: std::collections::HashMap<i64, _> = rules.iter().map(|r| (r.id, r)).collect();
    let rows: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            let r = by_id.get(&h.exclusion_id);
            serde_json::json!({
                "候选": h.candidate_key,
                "规则": h.exclusion_id,
                "否过的条目": r.map(|r| r.title.clone()).unwrap_or_default(),
                // 原话从本地库直接读给人看，没经过任何外部服务
                "原话": r.map(|r| r.quote.clone()).unwrap_or_default(),
                "同一事实": h.same_fact,
                "新料": h.new_substance,
                "已捞回": h.restored,
                "捞回人": h.restored_by,
                "捞回理由": h.restored_reason,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "挡下": rows.iter().filter(|r| r["已捞回"] == false).count(),
        "捞回": rows.iter().filter(|r| r["已捞回"] == true).count(),
        "明细": rows,
    })))
}

/// 排队的活排到哪了、做成没有、没成是为什么。
async fn work_queue(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<csw_collector_core::workbench::WorkItem>>, ApiError> {
    let conn = st.conn.lock().await;
    Ok(Json(
        csw_collector_core::workbench::recent_work(&conn, q.limit.clamp(1, 200))
            .map_err(ApiError::any)?,
    ))
}

/// 谁在什么时候改了什么。**台账被改过却追不到人，这份台账就不能拿去跟 Van 对质。**
async fn audit_log(
    State(st): State<Arc<AppState>>,
    axum::Extension(s): axum::Extension<Session>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<csw_collector_core::workbench::AuditRow>>, ApiError> {
    need_operator(&s, "留痕")?;
    let conn = st.conn.lock().await;
    Ok(Json(
        csw_collector_core::workbench::recent_audit(&conn, q.limit.clamp(1, 500))
            .map_err(ApiError::any)?,
    ))
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

/// 给浏览器的是一句人话；原文只进日志。
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn db(e: rusqlite::Error) -> Self {
        tracing::warn!(error = %e, "查本地库失败");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "查不到，稍后再试".into())
    }
    pub fn any(e: anyhow::Error) -> Self {
        tracing::warn!(error = %format!("{e:#}"), "接口出错");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "查不到，稍后再试".into())
    }
    /// 参数不合规矩：**原话给浏览器**。
    ///
    /// 与上面两个不同——「改档必须写理由」「没有这一档」这种话，
    /// 页面照着显示就是对的；藏起来只会让人不知道该改什么。
    pub fn bad_request(e: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, format!("{e:#}"))
    }
    /// 登录层给的错原样传下去（401 / 403）
    pub fn from_bff(e: crate::bff::ApiError) -> Self {
        let (code, msg) = e.parts();
        Self(code, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use csw_collector_core::types::{
        Candidate, Comparison, ComparisonVerdict, Dim, DimJudgement, Judgement, MediaDescription,
        MediaKind, MediaRef, Platform, RoundKind, RoundTrigger, ThreeSentences, Tier, Unanswered,
        Verdict,
    };
    use csw_collector_core::{media, rounds, workbench};
    use tower::ServiceExt;

    /// 一轮，两条候选：k1 推荐（两张图，一张没识别成）、k2 不推荐。
    fn app() -> (Router, Arc<AppState>) {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let r = rounds::open_round(
            &conn,
            &rounds::NewRound {
                kind: RoundKind::Task,
                trigger: RoundTrigger::Dispatch,
                run_id: Some(48),
                task_id: Some(1),
                stage_code: Some("intake".into()),
                target_version: 1,
                parent_round_id: None,
                window_start: "2026-09-17T00:00:00Z".into(),
                window_end: "2026-09-18T00:00:00Z".into(),
                plan_version: 1,
                rubric_version: "van-rubric/v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap()
        .0;
        for (k, tier) in [("k1", Tier::Recommend), ("k2", Tier::NotRecommend)] {
            ledger::upsert_candidate(&conn, &cand(k)).unwrap();
            ledger::attach_candidate(&conn, r.id, k, "csw-window", false).unwrap();
            ledger::put_judgement(&conn, r.id, &judgement(k, tier), &[], "m", "van-rubric/v1")
                .unwrap();
        }
        // k1 两张图，只有第一张识别成了
        media::put_prepared(&conn, "k1", &cand("k1").media, &[desc("b0")]).unwrap();

        let state = Arc::new(AppState {
            conn: tokio::sync::Mutex::new(conn),
            cfg: csw_collector_core::Config::default(),
            started: std::time::Instant::now(),
            // 测试里不起客户端：知识库那几个接口会如实回 503
            svc: None,
        });
        // 只读接口全在会话守卫后面，测试也得先有一枚会话
        (api_router(state.clone(), auth_with("operator")), state)
    }

    const SID: &str = "test-sid";

    /// 一把挂着 `role` 会话的钥匙，sid 固定是 [`SID`]。
    fn auth_with(role: &str) -> Arc<AuthState> {
        let auth = Arc::new(AuthState {
            engine: csw_collector_engineapi::admin::AdminClient::new("http://127.0.0.1:1").unwrap(),
            store: Default::default(),
            van_usernames: vec!["van".into()],
            secure_cookie: false,
        });
        auth.store.put(SID.into(), session(role));
        auth
    }

    fn session(role: &str) -> Session {
        Session {
            engine: csw_collector_engineapi::admin::EngineSession {
                access_token: String::new(),
                access_expires_at: i64::MAX,
                refresh_cookie: String::new(),
                user: csw_collector_engineapi::admin::AdminUser {
                    id: 1,
                    username: "editor".into(),
                    display_name: "主编".into(),
                    role: if role == "van" {
                        "viewer".into()
                    } else {
                        role.to_string()
                    },
                    status: "active".into(),
                },
            },
            csrf: "test-csrf".into(),
            created_at: csw_collector_engineapi::admin::now(),
            role: role.to_string(),
        }
    }

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw-window".into(),
            account: "and_wander".into(),
            url: format!("https://www.instagram.com/p/{key}/"),
            text: "新色登場".into(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: Some(369),
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Carousel".into(),
            media: (0..2)
                .map(|i| MediaRef {
                    source_hash: format!("s{i}"),
                    kind: MediaKind::Photo,
                    url: format!("https://img/{key}/{i}.jpg"),
                    blake3: Some(format!("b{i}")),
                    ordinal: i,
                })
                .collect(),
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn desc(b3: &str) -> MediaDescription {
        MediaDescription {
            blake3: b3.into(),
            ordinal: 0,
            matches_text: "正文说的新色".into(),
            content: "灰色背包正面".into(),
            missing_from_text: String::new(),
            kind: csw_collector_core::types::ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: "recognize/v1".into(),
        }
    }

    fn judgement(key: &str, tier: Tier) -> Judgement {
        Judgement {
            candidate_key: key.into(),
            tier,
            dims: Dim::ALL
                .into_iter()
                .map(|d| {
                    (
                        d,
                        DimJudgement {
                            verdict: Verdict::Yes,
                            basis: "正文第一句".into(),
                        },
                    )
                })
                .collect(),
            three_sentences: ThreeSentences {
                what_changed: format!("{key} 换了结构"),
                why_it_matters: "背得更稳".into(),
                how_different: "上一代是软背板".into(),
            },
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
            },
            heat_note: "369 赞".into(),
            look: "灰色主体".into(),
            image_seen: true,
            gaps: vec![],
            priority_hits: vec![],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "ih".into(),
        }
    }

    /// 台账、审计、Van 的原话，一条都不能匿名读出去。
    ///
    /// 服务挂在公网域名上，前端的 Guard 只是体验：真正拦人的是路由上那道守卫。
    #[tokio::test]
    async fn 没登录一个只读接口都打不开() {
        let (app, _) = app();
        for uri in [
            "/api/rounds",
            "/api/rounds/1/judgements",
            "/api/rounds/1/judgements/k1",
            "/api/van/today",
            "/api/audit",
            "/api/memory/rules",
            "/api/memory/cases",
            "/api/kb/search?q=x",
            "/api/settings",
            "/api/pending-check",
        ] {
            let (code, _) = get_as(&app, uri, None).await;
            assert_eq!(code, StatusCode::UNAUTHORIZED, "{uri} 居然匿名能读");
        }
        // 会话对不上号同样不行
        let (code, _) = get_as(&app, "/api/rounds", Some("不存在的会话")).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    /// 没有的路径照常 404——守卫不该把「这个接口不存在」说成「你没登录」。
    #[tokio::test]
    async fn 不存在的路径不会被守卫说成没登录() {
        let (app, _) = app();
        let (code, _) = get_as(&app, "/api/没有这个接口", None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    /// 运行设置里有引擎地址与各网关地址，是运维面的东西。
    /// Van 是个 viewer，她登录了也不该看见。
    #[tokio::test]
    async fn van看不了运行设置但看得了自己那一页() {
        let (_, st) = app();
        let as_van = api_router(st, auth_with("van"));
        for uri in ["/api/settings", "/api/audit"] {
            let (code, _) = get(&as_van, uri).await;
            assert_eq!(code, StatusCode::FORBIDDEN, "{uri}");
        }
        let (code, _) = get(&as_van, "/api/van/today").await;
        assert_eq!(code, StatusCode::OK);
    }

    /// 看门狗只 curl `/metrics` 就要能判断今早那轮走到哪了，
    /// 所以这七个指标缺一不可，名字也不能随手改——改了那边就静默地一直读到 0。
    #[tokio::test]
    async fn 指标里有看门狗要的那七个() {
        // /metrics 在 ops_router 上，不在 api_router 上——它不要会话
        let (_, st) = app();
        let resp = ops_router(st)
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        for m in [
            "csw_collector_last_task_round_started_unix",
            "csw_collector_last_task_round_step",
            "csw_collector_last_task_round_candidates",
            "csw_collector_last_task_round_judged",
            "csw_collector_last_task_round_media",
            "csw_collector_last_task_round_media_described",
            "csw_collector_last_task_round_delivered",
        ] {
            assert!(text.contains(m), "指标里少了 {m}");
        }
        // 夹具那一轮：两条候选、两条判断、k1 两张图识别出一张
        assert!(
            text.contains("csw_collector_last_task_round_candidates 2"),
            "{text}"
        );
        assert!(
            text.contains("csw_collector_last_task_round_judged 2"),
            "{text}"
        );
        assert!(
            text.contains("csw_collector_last_task_round_media 2"),
            "{text}"
        );
        assert!(
            text.contains("csw_collector_last_task_round_media_described 1"),
            "{text}"
        );
    }

    /// `/metrics` 与 `/healthz` 不要会话（探活与抓取都没有 cookie），
    /// 挡它们的是反代。这条测试钉住这个事实——哪天给它们加了守卫，
    /// 看门狗与 Prometheus 会一起瞎掉，得先想清楚。
    #[tokio::test]
    async fn 运维两个接口不要会话() {
        let (_, st) = app();
        let ops = ops_router(st);
        for uri in ["/healthz", "/metrics"] {
            let resp = ops
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_ne!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        get_as(app, uri, Some(SID)).await
    }

    /// `sid` 为 None 就是没登录。
    async fn get_as(app: &Router, uri: &str, sid: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut b = Request::builder().uri(uri);
        if let Some(sid) = sid {
            b = b.header(
                axum::http::header::COOKIE,
                format!("{}={sid}", crate::bff::SESSION_COOKIE),
            );
        }
        let resp = app
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let code = resp.status();
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        (
            code,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::json!(null)),
        )
    }

    #[tokio::test]
    async fn 一条的全貌一次给全() {
        let (app, _) = app();
        let (code, v) = get(&app, "/api/rounds/1/judgements/k1").await;
        assert_eq!(code, StatusCode::OK);
        // 分七个接口去拿，页面上就会出现七个各自转圈的小方块
        assert_eq!(v["candidate"]["account"], "and_wander");
        assert_eq!(v["judgement"]["tier"], "recommend");
        assert_eq!(v["effective_tier"], "recommend");
        assert_eq!(v["images"].as_array().unwrap().len(), 2);
        // 第二张没识别成：描述是空的、failed 是 true
        assert_eq!(v["images"][0]["content"], "灰色背包正面");
        assert_eq!(v["images"][1]["content"], "");
        assert_eq!(v["images"][1]["failed"], true);
        assert_eq!(v["first_batch"], false);
        assert!(v["deepcheck"].is_null());

        let (code, _) = get(&app, "/api/rounds/1/judgements/没这条").await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn 详情里的生效档跟着改档走() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            workbench::put_override(&conn, 1, "k2", Tier::Alternate, "有新角度", "主编").unwrap();
            workbench::set_first_batch(&conn, 1, &["k2".into()]).unwrap();
        }
        let (_, v) = get(&app, "/api/rounds/1/judgements/k2").await;
        // 原判不动，生效档跟着改档走
        assert_eq!(v["judgement"]["tier"], "not_recommend");
        assert_eq!(v["effective_tier"], "alternate");
        assert_eq!(v["overrides"][0]["reason"], "有新角度");
        assert_eq!(v["first_batch"], true);
    }

    #[tokio::test]
    async fn 台账按生效档筛与排() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            workbench::put_override(&conn, 1, "k2", Tier::Recommend, "捞回", "主编").unwrap();
        }
        let (_, v) = get(&app, "/api/rounds/1/judgements?tier=recommend").await;
        let keys: Vec<&str> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["candidate_key"].as_str().unwrap())
            .collect();
        // 捞回之后还把它筛在不推荐那一组里的话，那次捞回等于没发生
        assert_eq!(keys, ["k1", "k2"]);
        let k2 = &v.as_array().unwrap()[1];
        assert_eq!(k2["tier"], "not_recommend", "原判照样给");
        assert_eq!(k2["effective_tier"], "recommend");
        assert_eq!(k2["override_actor"], "主编");
    }

    #[tokio::test]
    async fn van那一页只给推荐与备选() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            workbench::put_van_mark(&conn, 1, "k1", "like", "", "van").unwrap();
        }
        let (code, v) = get(&app, "/api/van/today").await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(v["round_id"], 1);
        let items = v["items"].as_array().unwrap();
        // 手机上翻三百条不是在帮她
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["candidate_key"], "k1");
        assert_eq!(items[0]["title"], "k1 换了结构");
        assert_eq!(items[0]["marks"], serde_json::json!(["like"]));
        assert_eq!(items[0]["three_sentences"]["why_it_matters"], "背得更稳");
    }

    #[tokio::test]
    async fn 还没开工不是错误() {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let state = Arc::new(AppState {
            conn: tokio::sync::Mutex::new(conn),
            cfg: csw_collector_core::Config::default(),
            started: std::time::Instant::now(),
            // 测试里不起客户端：知识库那几个接口会如实回 503
            svc: None,
        });
        let (code, v) = get(&api_router(state, auth_with("operator")), "/api/van/today").await;
        // 显示「今天还没开始」比显示一个错误体强
        assert_eq!(code, StatusCode::OK);
        assert!(v["round_id"].is_null());
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn 失败的图挑得出来() {
        let (app, _) = app();
        let (_, all) = get(&app, "/api/rounds/1/media").await;
        assert_eq!(all.as_array().unwrap().len(), 2);
        let (_, bad) = get(&app, "/api/rounds/1/media?state=failed").await;
        let bad = bad.as_array().unwrap();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0]["blake3"], "b1");
    }

    #[tokio::test]
    async fn 覆盖来自我们发出去的那一份() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            conn.execute(
                "INSERT INTO engine_outbox(seq, round_id, kind, idem_key, body_json, body_sha,
                                           status, created_at)
                 VALUES (1, 1, 'sweeps', 'k', ?1, 'x', 'confirmed', '2026-09-18T06:00:00Z')",
                [r#"{"sweeps":[{"sweep_key":"csw-window","found":438,"in_window":305}]}"#],
            )
            .unwrap();
        }
        let (code, v) = get(&app, "/api/rounds/1/coverage").await;
        assert_eq!(code, StatusCode::OK);
        // 页面上看到的要与引擎收到的是同一份，不是现场再算一遍
        assert_eq!(v["sweeps"][0]["found"], 438);
        assert_eq!(v["sweeps"][0]["in_window"], 305);
    }

    #[tokio::test]
    async fn 判断框架从常量来() {
        let (app, _) = app();
        let (code, v) = get(&app, "/api/rubric").await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(v["version"], csw_collector_judge::rubric::RUBRIC_VERSION);
        assert_eq!(v["priority"].as_array().unwrap().len(), 7);
        assert_eq!(v["lower"].as_array().unwrap().len(), 7);
        assert_eq!(v["dim_anchors"].as_array().unwrap().len(), 6);
        // 「不是维度」的那四项要能看见：它们是这套框架里最容易被偷偷用上的
        assert!(v["not_dimensions"].as_array().unwrap().len() == 4);
    }

    #[tokio::test]
    async fn 指标全是计数没有一个是分数() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            workbench::put_override(&conn, 1, "k2", Tier::Alternate, "捞回", "主编").unwrap();
        }
        let (code, v) = get(&app, "/api/metrics/selection").await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(v["判过的条目"], 2);
        assert_eq!(v["推荐"], 1);
        assert_eq!(v["不推荐"], 1);
        assert_eq!(v["人工改档"], 1);
        // 「模型判了什么」与「人改成了什么」分开算
        assert_eq!(v["被捞回的"], 1);
        assert_eq!(v["被压下的"], 0);
        let obj = v.as_object().unwrap();
        assert!(!obj.keys().any(|k| k.contains("分")), "不该有任何分数字段");
    }

    #[tokio::test]
    async fn 知识库水位回的是同步到哪天() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            conn.execute(
                "INSERT INTO kb_cursors(source, cursor, synced_at)
                 VALUES ('ledger_posts','2026-09-18','2026-09-18T02:30:00Z')",
                [],
            )
            .unwrap();
        }
        let (code, v) = get(&app, "/api/kb/status").await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(v["cursors"][0]["source"], "ledger_posts");
        assert_eq!(v["cursors"][0]["synced_at"], "2026-09-18T02:30:00Z");
        assert_eq!(v["docs"], 0);
    }

    #[tokio::test]
    async fn 知识库没起来就如实回五零三() {
        let (app, _) = app();
        // 回一个空结果会让人以为「库里什么都没有」——那是两件完全不同的事
        let (code, v) = get(&app, "/api/kb/search?q=%E8%83%8C%E5%8C%85").await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert!(v["error"].as_str().unwrap().contains("还没起来"));
        // 空查询是用法错，不是服务不可用
        let (code, _) = get(&app, "/api/kb/search?q=%20%20").await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn 按品牌翻历史不占显卡() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            conn.execute(
                "INSERT INTO kb_docs(kind, ref_id, brand, title, body, url, content_hash,
                                     created_at, published_at)
                 VALUES ('published_item','p1','and wander','40L 背包','正文很长'||?1,
                         'https://x','h','now','2026-09-10')",
                [&"啦".repeat(300)],
            )
            .unwrap();
        }
        // 这一页是拿来翻的，不是拿来搜的：没有向量服务也该能用
        let (code, v) = get(&app, "/api/kb/brands/and%20wander").await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(v["docs"].as_array().unwrap().len(), 1);
        assert_eq!(v["docs"][0]["title"], "40L 背包");
        // 正文只给一小段：挑中了再去看原文
        let snippet = v["docs"][0]["snippet"].as_str().unwrap();
        assert!(
            snippet.chars().count() <= 160,
            "{}",
            snippet.chars().count()
        );
    }

    #[tokio::test]
    async fn 待补录清单只列不在册的() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            conn.execute(
                "INSERT INTO kb_docs(kind, ref_id, brand, title, body, url, content_hash,
                                     created_at, published_at)
                 VALUES ('decision','d1','futurefox','FUTURE FOX炉顶附件',
                         '结论：written'||char(10)||'原话：不外露','https://www.instagram.com/p/X/',
                         'h','now','2026-09-15')",
                [],
            )
            .unwrap();
        }
        let (code, v) = get(&app, "/api/kb/backfill").await;
        assert_eq!(code, StatusCode::OK);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["brand"], "futurefox");
        assert_eq!(rows[0]["adopted"], 1);
        assert!(!v.to_string().contains("不外露"));
    }

    #[tokio::test]
    async fn 选题记忆带着van的原话() {
        let (app, st) = app();
        {
            let conn = st.conn.lock().await;
            conn.execute(
                "INSERT INTO memory_cases(case_key, decision, quote, source_url, decided_at,
                                          updated_at)
                 VALUES ('c1','否决','这条太像广告了','https://x','2026-09-17','2026-09-17')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO memory_rules(rule_key, text, version, confirmed_by_van, updated_at)
                 VALUES ('r1','别发纯联名','v1',1,'2026-09-17')",
                [],
            )
            .unwrap();
        }
        let (_, cases) = get(&app, "/api/memory/cases").await;
        // 原话一字不改——改写过的原话不能拿去跟她对质
        assert_eq!(cases[0]["quote"], "这条太像广告了");
        assert_eq!(cases[0]["decision"], "否决");
        let (_, rules) = get(&app, "/api/memory/rules").await;
        assert_eq!(rules[0]["text"], "别发纯联名");
        assert_eq!(rules[0]["confirmed_by_van"], true);
    }
}
