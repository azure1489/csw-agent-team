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

use csw_collector_core::{Config, ledger};

pub struct AppState {
    pub conn: Mutex<rusqlite::Connection>,
    pub cfg: Config,
    /// 进程起来的时刻，给 `/healthz` 的 uptime
    pub started: std::time::Instant,
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

/// 工作台的只读接口。写接口另挂，要过 CSRF。
pub fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/rounds", get(list_rounds))
        .route("/api/rounds/{id}", get(round_detail))
        .route("/api/rounds/{id}/steps", get(round_steps))
        .route("/api/rounds/{id}/judgements", get(judgements))
        .route("/api/rounds/{id}/outbox", get(outbox))
        .route("/api/rounds/{id}/van-marks", get(van_marks))
        .route("/api/pending-check", get(pending_check))
        .route("/api/work", get(work_queue))
        .route("/api/audit", get(audit_log))
        .route("/api/settings", get(settings))
        .with_state(state)
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
        deps: vec![DepStatus {
            name: "sqlite".into(),
            ok: db_ok,
            note: st.cfg.db_path().display().to_string(),
        }],
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
         csw_collector_uptime_seconds {}\n",
        one("SELECT COUNT(*) FROM rounds"),
        one("SELECT COUNT(*) FROM rounds WHERE status='running'"),
        one("SELECT COUNT(*) FROM judgements"),
        one("SELECT COUNT(DISTINCT candidate_key) FROM judgements WHERE tier='pending_check'"),
        one("SELECT COUNT(*) FROM engine_outbox WHERE status='conflict'"),
        st.started.elapsed().as_secs(),
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

async fn settings(State(st): State<Arc<AppState>>) -> Result<Json<Settings>, ApiError> {
    let c = &st.cfg;
    let s = csw_collector_core::Secrets::from_env();
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
            ("CSW_API_KEY".into(), !s.csw_api_key.is_empty()),
            ("SUB2API_API_KEY".into(), !s.sub2api_key.is_empty()),
            ("TYPESAFE_API_KEY".into(), !s.typesafe_key.is_empty()),
            ("CSW_ENGINE_TOKEN".into(), !s.engine_token.is_empty()),
        ],
    }))
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
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<csw_collector_core::workbench::AuditRow>>, ApiError> {
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
