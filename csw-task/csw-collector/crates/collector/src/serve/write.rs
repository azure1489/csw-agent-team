//! 工作台的写接口。只读的那几个在 `http.rs`。
//!
//! # 三条规矩
//!
//! **一、每一个都过 CSRF。** `SameSite=Lax` 挡不住表单式的跨站 POST，
//! 双提交那一层不能省（`bff::check_write`）。
//!
//! **二、每一个都留痕。** 谁、什么时候、把什么改成了什么，全进 `audit`。
//! 台账上一条被改过档，追不到是谁改的，这条台账就不能拿去跟 Van 对质。
//!
//! **三、HTTP 不干活。** 开一轮要四十分钟，重跑也是。这里只往 `work_queue`
//! 写一行就返回，真正干活的是常驻循环——它才持有模型与向量客户端。
//!
//! # 角色
//!
//! | 角色 | 能做什么 |
//! |---|---|
//! | `superadmin` / `operator` | 改档、指定首批、开轮、重跑、代录 Van 的勾选 |
//! | `van` | 只能勾选（like / doubt / note） |
//! | `viewer` | 一个写都不行 |
//!
//! Van 的勾选**只写本地，不回写引擎**：进不进评选由主编代录，
//! 那是流程里人的决定，不是这里能替她做的。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use serde::{Deserialize, Serialize};

use csw_collector_core::types::Tier;
use csw_collector_core::workbench;

use crate::bff::{AuthState, Session, check_write};

use super::http::{ApiError, AppState};

#[derive(Clone)]
pub struct WriteState {
    pub app: Arc<AppState>,
    pub auth: Arc<AuthState>,
}

pub fn write_router(app: Arc<AppState>, auth: Arc<AuthState>) -> Router {
    Router::new()
        .route("/api/rounds", post(open_manual_round))
        .route("/api/rounds/{id}/steps/{step}/rerun", post(rerun))
        .route("/api/rounds/{id}/first-batch", post(set_first_batch))
        .route("/api/van/marks", post(van_mark))
        .route("/api/exclusions/{id}/active", post(set_exclusion_active))
        .route(
            "/api/rounds/{id}/exclusions/{key}/restore",
            post(restore_excluded),
        )
        .route(
            "/api/rounds/{id}/judgements/{key}/override",
            post(override_tier),
        )
        .with_state(WriteState { app, auth })
}

/// 这件事要什么身份。
enum Need {
    /// 主编那一档：改档、开轮、重跑
    Editor,
    /// Van 自己或主编代录
    Mark,
}

fn require(st: &WriteState, headers: &HeaderMap, need: Need) -> Result<Session, ApiError> {
    let s = check_write(&st.auth, headers).map_err(ApiError::from_bff)?;
    let ok = match need {
        Need::Editor => matches!(s.role.as_str(), "superadmin" | "operator"),
        Need::Mark => matches!(s.role.as_str(), "superadmin" | "operator" | "van"),
    };
    if !ok {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            format!("{} 这个角色做不了这件事", s.role),
        ));
    }
    Ok(s)
}

/// 会话里的用户名。留痕要记的是人，不是角色。
fn actor(s: &Session) -> String {
    s.engine.user.username.clone()
}

/// 改动与留痕**同进同出**。
///
/// 上面第二条规矩说每一个写都要留痕。那就不能是「改完了顺手记一笔，记不上也算了」：
/// 台账上一条被改过档、却查不到是谁改的，这条台账就不能拿去跟 Van 对质，
/// 而那正是留痕唯一的用处。所以两件事进同一个事务——要么都成，要么都不成，
/// 宁可让主编看见一个错误再点一次，也不要留下一条没有出处的改动。
///
/// `body` 自己产出留痕内容：有些细节（比如真标上了几条）要做完才知道。
fn with_audit<T>(
    conn: &rusqlite::Connection,
    who: &str,
    action: &str,
    target: &str,
    body: impl FnOnce(&rusqlite::Connection) -> Result<(T, serde_json::Value), ApiError>,
) -> Result<T, ApiError> {
    let tx = conn.unchecked_transaction().map_err(ApiError::db)?;
    // 这里出错，tx 走 Drop 回滚，改动一并撤掉
    let (out, detail) = body(&tx)?;
    workbench::audit(&tx, who, action, target, &detail).map_err(ApiError::any)?;
    tx.commit().map_err(ApiError::db)?;
    Ok(out)
}

// ─────────────────────────────── 改档 ───────────────────────────────

#[derive(Deserialize)]
struct OverrideReq {
    to_tier: String,
    reason: String,
}

#[derive(Serialize)]
struct OverrideResp {
    id: i64,
    effective_tier: String,
}

/// 改档、标待核、捞回——**都是同一件事**：把这一条从现在显示的那一档挪到另一档。
///
/// 原判一个字不动，改动另存一行。理由是必填的：
/// 没有理由的改档在台账上与「模型本来就这么判的」分不出来。
async fn override_tier(
    State(st): State<WriteState>,
    Path((id, key)): Path<(i64, String)>,
    headers: HeaderMap,
    Json(req): Json<OverrideReq>,
) -> Result<Json<OverrideResp>, ApiError> {
    let s = require(&st, &headers, Need::Editor)?;
    let to = parse_tier(&req.to_tier)?;
    let conn = st.app.conn.lock().await;
    let who = actor(&s);
    let row = with_audit(&conn, &who, "override", &format!("r{id}/{key}"), |c| {
        let row = workbench::put_override(c, id, &key, to, &req.reason, &who)
            .map_err(ApiError::bad_request)?;
        Ok((
            row,
            serde_json::json!({"to": req.to_tier, "reason": req.reason.trim()}),
        ))
    })?;
    let effective = workbench::effective_tier(&conn, id, &key)
        .map_err(ApiError::any)?
        .unwrap_or_default();
    Ok(Json(OverrideResp {
        id: row,
        effective_tier: effective,
    }))
}

fn parse_tier(s: &str) -> Result<Tier, ApiError> {
    match s {
        "recommend" => Ok(Tier::Recommend),
        "alternate" => Ok(Tier::Alternate),
        "not_recommend" => Ok(Tier::NotRecommend),
        "pending_check" => Ok(Tier::PendingCheck),
        // 四档之外没有别的档，也没有分数
        other => Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("没有 {other} 这一档"),
        )),
    }
}

// ───────────────────────────── 指定首批 ─────────────────────────────

#[derive(Deserialize)]
struct FirstBatchReq {
    keys: Vec<String>,
}

#[derive(Serialize)]
struct CountResp {
    marked: usize,
}

/// 指定这一轮深核的首批。传空数组＝清空指定，退回自动挑法。
async fn set_first_batch(
    State(st): State<WriteState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(req): Json<FirstBatchReq>,
) -> Result<Json<CountResp>, ApiError> {
    let s = require(&st, &headers, Need::Editor)?;
    let conn = st.app.conn.lock().await;
    let who = actor(&s);
    let n = with_audit(&conn, &who, "first_batch", &format!("r{id}"), |c| {
        let n = workbench::set_first_batch(c, id, &req.keys).map_err(ApiError::any)?;
        Ok((n, serde_json::json!({"要的": req.keys.len(), "标上的": n})))
    })?;
    Ok(Json(CountResp { marked: n }))
}

// ──────────────────────────── 硬性排除 ────────────────────────────

#[derive(Deserialize)]
struct ActiveReq {
    active: bool,
    /// 关掉时必填：这条否决为什么不该再挡人
    #[serde(default)]
    reason: String,
}

/// 开或关一条排除规则。
///
/// 关 = 「Van 当初否的那个理由现在不成立了」，或者「这条否决没留原话，
/// 我看过了，不该拿它挡人」。开 = 反过来，多半是确认一条没原话的否决。
///
/// **关必须给理由**：一条规则能让候选不经判断就落定，撤掉它同样要留下交代。
async fn set_exclusion_active(
    State(st): State<WriteState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(req): Json<ActiveReq>,
) -> Result<Json<CountResp>, ApiError> {
    let s = require(&st, &headers, Need::Editor)?;
    if !req.active && req.reason.trim().is_empty() {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "停用排除规则要写明理由"
        )));
    }
    let conn = st.app.conn.lock().await;
    let who = actor(&s);
    let n = with_audit(&conn, &who, "exclusion_active", &format!("e{id}"), |c| {
        let n = csw_collector_core::exclusion::set_active(c, id, req.active, &req.reason, &who)
            .map_err(ApiError::any)?;
        Ok((n, serde_json::json!({"开": req.active, "理由": req.reason})))
    })?;
    Ok(Json(CountResp { marked: n }))
}

#[derive(Deserialize)]
struct RestoreReq {
    #[serde(default)]
    reason: String,
}

/// 捞回一条被规则挡下的候选。**只对这一轮这一条生效**，规则本身还开着。
///
/// 捞回之后它要重新走判断——它当初根本没送模型，没有结论可以拿来改档。
/// 页面上捞回完该提示「重跑判断那一步」。
async fn restore_excluded(
    State(st): State<WriteState>,
    Path((id, key)): Path<(i64, String)>,
    headers: HeaderMap,
    Json(req): Json<RestoreReq>,
) -> Result<Json<CountResp>, ApiError> {
    let s = require(&st, &headers, Need::Editor)?;
    if req.reason.trim().is_empty() {
        return Err(ApiError::bad_request(anyhow::anyhow!("捞回要写明理由")));
    }
    let conn = st.app.conn.lock().await;
    let who = actor(&s);
    let n = with_audit(
        &conn,
        &who,
        "exclusion_restore",
        &format!("r{id}/{key}"),
        |c| {
            let n = csw_collector_core::exclusion::restore(c, id, &key, &who, &req.reason)
                .map_err(ApiError::any)?;
            Ok((n, serde_json::json!({"候选": key, "理由": req.reason})))
        },
    )?;
    if n == 0 {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "这一轮没有这一条被排除的记录"
        )));
    }
    Ok(Json(CountResp { marked: n }))
}

// ──────────────────────────── Van 的勾选 ────────────────────────────

#[derive(Deserialize)]
struct MarkReq {
    /// 不给就落到当期那一轮。Van 模式的页面只看当期，不该逼她先挑轮次
    #[serde(default)]
    round_id: Option<i64>,
    candidate_key: String,
    /// like | doubt | note
    mark: String,
    #[serde(default)]
    note: String,
    /// 撤掉这个勾选
    #[serde(default)]
    remove: bool,
}

async fn van_mark(
    State(st): State<WriteState>,
    headers: HeaderMap,
    Json(req): Json<MarkReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let s = require(&st, &headers, Need::Mark)?;
    let conn = st.app.conn.lock().await;
    let id = match req.round_id {
        Some(i) => i,
        None => current_round(&conn)?,
    };
    let who = actor(&s);
    let target = format!("r{id}/{}", req.candidate_key);
    if req.remove {
        let n = with_audit(&conn, &who, "van_mark_remove", &target, |c| {
            let n = workbench::drop_van_mark(c, id, &req.candidate_key, &req.mark, &who)
                .map_err(ApiError::any)?;
            Ok((n, serde_json::json!({"mark": req.mark})))
        })?;
        return Ok(Json(serde_json::json!({ "removed": n })));
    }
    with_audit(&conn, &who, "van_mark", &target, |c| {
        workbench::put_van_mark(c, id, &req.candidate_key, &req.mark, &req.note, &who)
            .map_err(ApiError::bad_request)?;
        Ok((
            (),
            serde_json::json!({"mark": req.mark, "note": req.note.trim()}),
        ))
    })?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// 当期那一轮：最近开的一轮**派单轮**。
///
/// 不含手动轮与预取轮：Van 看的是今天真派下来的那一期，
/// 勾到一轮凌晨的预取上去，主编代录时会对不上。
fn current_round(conn: &rusqlite::Connection) -> Result<i64, ApiError> {
    conn.query_row(
        "SELECT id FROM rounds WHERE kind='task' ORDER BY id DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
    .map_err(|_| ApiError(StatusCode::NOT_FOUND, "还没有哪一期开工".into()))
}

// ──────────────────────── 手动开轮 / 重跑 ────────────────────────

#[derive(Deserialize)]
struct ManualRoundReq {
    /// 往回取几天。不给就按一天
    #[serde(default)]
    days: Option<u32>,
    /// 或者直接给窗口（RFC3339 UTC）。两个都给时以窗口为准
    #[serde(default)]
    window_start: Option<String>,
    #[serde(default)]
    window_end: Option<String>,
}

#[derive(Serialize)]
struct QueuedResp {
    work_id: i64,
    /// 排在第几位（含正在做的那件）
    queued_ahead: usize,
}

/// 手动开一轮。**只排队，不当场跑**——一轮四十分钟，HTTP 请求等不了。
///
/// 手动轮不写引擎：它是拿来看的，不是拿来交的。
async fn open_manual_round(
    State(st): State<WriteState>,
    headers: HeaderMap,
    Json(req): Json<ManualRoundReq>,
) -> Result<Json<QueuedResp>, ApiError> {
    let s = require(&st, &headers, Need::Editor)?;
    let mut payload = serde_json::json!({ "days": req.days.unwrap_or(1).clamp(1, 30) });
    if let (Some(a), Some(b)) = (&req.window_start, &req.window_end) {
        // 窗口要能解析：解析不了宁可现在就报错，也不要排进队里等四十分钟才发现
        for t in [a, b] {
            t.parse::<jiff::Timestamp>().map_err(|e| {
                ApiError(
                    StatusCode::BAD_REQUEST,
                    format!("窗口时间不是 RFC3339：{t}（{e}）"),
                )
            })?;
        }
        payload["window_start"] = serde_json::json!(a);
        payload["window_end"] = serde_json::json!(b);
    }
    queue(&st, &s, "manual_round", None, payload).await
}

/// 重跑一步：从这一步起真的重算，下游跟着重来。
///
/// 只认 `harvest`（连识别一起重来）与 `judge`（只重判）两个。
/// **登记与提交不在其列**——引擎那边已经收到的台账要改，只能走补件，
/// 那是人的决定，不该是重跑的副作用。
async fn rerun(
    State(st): State<WriteState>,
    Path((id, step)): Path<(i64, String)>,
    headers: HeaderMap,
) -> Result<Json<QueuedResp>, ApiError> {
    let s = require(&st, &headers, Need::Editor)?;
    if !matches!(step.as_str(), "harvest" | "judge") {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("{step} 这一步不支持重跑，只能重跑 harvest 或 judge"),
        ));
    }
    {
        let conn = st.app.conn.lock().await;
        let exists: i64 = conn
            .query_row("SELECT COUNT(*) FROM rounds WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(ApiError::db)?;
        if exists == 0 {
            return Err(ApiError(StatusCode::NOT_FOUND, "没有这一轮".into()));
        }
    }
    queue(
        &st,
        &s,
        "rerun",
        Some(id),
        serde_json::json!({ "from_step": step }),
    )
    .await
}

async fn queue(
    st: &WriteState,
    s: &Session,
    kind: &str,
    round_id: Option<i64>,
    payload: serde_json::Value,
) -> Result<Json<QueuedResp>, ApiError> {
    let conn = st.app.conn.lock().await;
    let who = actor(s);
    let target = round_id.map(|r| format!("r{r}")).unwrap_or_default();
    let id = with_audit(&conn, &who, kind, &target, |c| {
        let id = workbench::enqueue(c, kind, round_id, &payload, &who).map_err(ApiError::any)?;
        Ok((id, payload.clone()))
    })?;
    let ahead: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM work_queue WHERE id < ?1 AND status IN ('queued','running')",
            [id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(Json(QueuedResp {
        work_id: id,
        queued_ahead: ahead as usize,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use csw_collector_core::types::{
        Candidate, Comparison, ComparisonVerdict, Dim, DimJudgement, Judgement, MediaKind,
        MediaRef, Platform, RoundKind, RoundTrigger, ThreeSentences, Unanswered, Verdict,
    };
    use csw_collector_core::{ledger, rounds};
    use tower::ServiceExt;

    const SID: &str = "test-sid";
    const CSRF: &str = "test-csrf";

    /// 建一个挂着会话的工作台。`role` 就是会话里的工作台角色。
    fn app_with(role: &str) -> (Router, std::sync::Arc<AppState>) {
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
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap()
        .0;
        ledger::upsert_candidate(&conn, &cand("k1")).unwrap();
        ledger::attach_candidate(&conn, r.id, "k1", "csw-window", false).unwrap();
        ledger::put_judgement(&conn, r.id, &judgement("k1"), &[], "m", "v1").unwrap();

        let app_state = std::sync::Arc::new(AppState {
            conn: tokio::sync::Mutex::new(conn),
            cfg: csw_collector_core::Config::default(),
            started: std::time::Instant::now(),
            // 测试里不起客户端：知识库那几个接口会如实回 503
            svc: None,
        });
        let auth = std::sync::Arc::new(AuthState {
            engine: csw_collector_engineapi::admin::AdminClient::new("http://127.0.0.1:1").unwrap(),
            store: Default::default(),
            van_usernames: vec![],
            secure_cookie: false,
        });
        auth.store.put(
            SID.into(),
            Session {
                engine: csw_collector_engineapi::admin::EngineSession {
                    access_token: "t".into(),
                    access_expires_at: i64::MAX,
                    refresh_cookie: String::new(),
                    user: csw_collector_engineapi::admin::AdminUser {
                        id: 1,
                        username: "主编".into(),
                        display_name: String::new(),
                        role: if role == "van" { "viewer" } else { role }.into(),
                        status: "active".into(),
                    },
                },
                csrf: CSRF.into(),
                created_at: csw_collector_engineapi::admin::now(),
                role: role.into(),
            },
        );
        (write_router(app_state.clone(), auth), app_state)
    }

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw-window".into(),
            account: "acc".into(),
            url: format!("https://x/{key}"),
            text: "正文".into(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![MediaRef {
                source_hash: "h".into(),
                kind: MediaKind::Photo,
                url: "u".into(),
                blake3: None,
                ordinal: 0,
            }],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn judgement(key: &str) -> Judgement {
        Judgement {
            candidate_key: key.into(),
            tier: Tier::NotRecommend,
            dims: Dim::ALL
                .into_iter()
                .map(|d| {
                    (
                        d,
                        DimJudgement {
                            verdict: Verdict::No,
                            basis: "正文第一句".into(),
                        },
                    )
                })
                .collect(),
            three_sentences: ThreeSentences {
                what_changed: "甲".into(),
                why_it_matters: "乙".into(),
                how_different: "丙".into(),
            },
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
            },
            heat_note: String::new(),
            look: String::new(),
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

    /// 带会话与 CSRF 的请求
    fn req(path: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("cookie", format!("{}={SID}", crate::bff::SESSION_COOKIE))
            .header(crate::bff::CSRF_HEADER, CSRF)
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn status(app: &Router, r: Request<Body>) -> StatusCode {
        app.clone().oneshot(r).await.unwrap().status()
    }

    async fn body_json(app: &Router, r: Request<Body>) -> (StatusCode, serde_json::Value) {
        let resp = app.clone().oneshot(r).await.unwrap();
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
    async fn 没有会话或csrf不对一律挡下() {
        let (app, _) = app_with("operator");
        let body = serde_json::json!({"to_tier": "alternate", "reason": "捞回"});
        let path = "/api/rounds/1/judgements/k1/override";

        // 没有会话
        let bare = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        assert_eq!(status(&app, bare).await, StatusCode::UNAUTHORIZED);

        // 有会话，CSRF 头不对：SameSite=Lax 挡不住表单式的跨站 POST，这一层不能省
        let bad = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("cookie", format!("{}={SID}", crate::bff::SESSION_COOKIE))
            .header(crate::bff::CSRF_HEADER, "猜的")
            .body(Body::from(body.to_string()))
            .unwrap();
        assert_eq!(status(&app, bad).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn 看客一个写都不行() {
        let (app, _) = app_with("viewer");
        for (path, body) in [
            (
                "/api/rounds/1/judgements/k1/override",
                serde_json::json!({"to_tier": "alternate", "reason": "捞回"}),
            ),
            (
                "/api/rounds/1/first-batch",
                serde_json::json!({"keys": ["k1"]}),
            ),
            (
                "/api/van/marks",
                serde_json::json!({"candidate_key": "k1", "mark": "like"}),
            ),
            ("/api/rounds", serde_json::json!({"days": 1})),
            // 排除那两个接口同样是主编档：一个能让候选不经判断就落定，
            // 一个能把已落定的捞回来
            (
                "/api/exclusions/1/active",
                serde_json::json!({"active": false, "reason": "过时了"}),
            ),
            (
                "/api/rounds/1/exclusions/k1/restore",
                serde_json::json!({"reason": "这次有新料"}),
            ),
        ] {
            assert_eq!(
                status(&app, req(path, body)).await,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn van只能勾选改不了档() {
        let (app, _) = app_with("van");
        assert_eq!(
            status(
                &app,
                req(
                    "/api/rounds/1/judgements/k1/override",
                    serde_json::json!({"to_tier": "recommend", "reason": "我喜欢"})
                )
            )
            .await,
            StatusCode::FORBIDDEN,
            "进不进评选由主编代录，不是这里能替她做的"
        );
        assert_eq!(
            status(
                &app,
                req(
                    "/api/van/marks",
                    serde_json::json!({"candidate_key": "k1", "mark": "like"})
                )
            )
            .await,
            StatusCode::OK
        );
    }

    /// 留痕与改动同进同出。
    ///
    /// 把 `audit` 表掀掉来模拟「留痕写不进去」——磁盘满、表坏，现实里都会发生。
    /// 这时改档必须整个失败：一条改过档却查不到出处的台账，比没改还糟，
    /// 因为它看上去就像模型本来就那么判的。
    #[tokio::test]
    async fn 留痕写不进去的时候改动也不留下() {
        let (app, st) = app_with("operator");
        {
            let conn = st.conn.lock().await;
            conn.execute_batch("DROP TABLE audit;").unwrap();
        }
        let code = status(
            &app,
            req(
                "/api/rounds/1/judgements/k1/override",
                serde_json::json!({"to_tier": "alternate", "reason": "主编捞回"}),
            ),
        )
        .await;
        assert!(code.is_server_error() || code.is_client_error(), "{code}");
        let conn = st.conn.lock().await;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM judgement_overrides", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "留痕没写成，改档却留下来了");
    }

    #[tokio::test]
    async fn 停用排除规则要理由而且留痕() {
        let (app, st) = app_with("operator");
        let id = {
            let conn = st.conn.lock().await;
            csw_collector_core::exclusion::upsert(
                &conn,
                &csw_collector_core::exclusion::Exclusion {
                    id: 0,
                    decision_ref: "1#a".into(),
                    item_key: "a".into(),
                    title: "旧条目".into(),
                    brand: "snow peak".into(),
                    source_url: String::new(),
                    quote: "这种普通上新没看点".into(),
                    reason: String::new(),
                    reason_code: String::new(),
                    decided_at: "2026-09-01".into(),
                    actor_role: "van".into(),
                    active: true,
                    inactive_reason: String::new(),
                    changed_by: String::new(),
                    changed_at: String::new(),
                },
            )
            .unwrap();
            csw_collector_core::exclusion::all(&conn).unwrap()[0].id
        };

        // 不给理由挡下：撤掉一条能让候选不经判断落定的规则，同样要留交代
        assert_eq!(
            status(
                &app,
                req(
                    &format!("/api/exclusions/{id}/active"),
                    serde_json::json!({"active": false})
                )
            )
            .await,
            StatusCode::BAD_REQUEST
        );

        let (code, _) = body_json(
            &app,
            req(
                &format!("/api/exclusions/{id}/active"),
                serde_json::json!({"active": false, "reason": "这个角度现在又想要了"}),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::OK);

        let conn = st.conn.lock().await;
        let got = &csw_collector_core::exclusion::all(&conn).unwrap()[0];
        assert!(!got.active);
        assert_eq!(got.inactive_reason, "这个角度现在又想要了");
        assert_eq!(got.changed_by, "主编");
        // 生效清单里没有它了
        assert!(
            csw_collector_core::exclusion::active(&conn)
                .unwrap()
                .is_empty()
        );
        // 留痕同事务落下
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'exclusion_active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn 捞回不存在的记录要报错不能静默成功() {
        let (app, _) = app_with("operator");
        // 静默成功最糟：页面显示捞回了，下一轮它还是被挡着
        assert_eq!(
            status(
                &app,
                req(
                    "/api/rounds/1/exclusions/没有这条/restore",
                    serde_json::json!({"reason": "有新料"})
                )
            )
            .await,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn 改档另存原判不动() {
        let (app, st) = app_with("operator");
        let (code, body) = body_json(
            &app,
            req(
                "/api/rounds/1/judgements/k1/override",
                serde_json::json!({"to_tier": "alternate", "reason": "同一事实但有新角度"}),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["effective_tier"], "alternate");

        let conn = st.conn.lock().await;
        let tier: String = conn
            .query_row(
                "SELECT tier FROM judgements WHERE round_id=1 AND candidate_key='k1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tier, "not_recommend", "原判一个字不动");
        // 留痕：台账被改过却追不到人，这份台账就不能拿去跟 Van 对质
        let (actor, action): (String, String) = conn
            .query_row(
                "SELECT actor, action FROM audit ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((actor.as_str(), action.as_str()), ("主编", "override"));
    }

    #[tokio::test]
    async fn 理由是必填的而且档只有那四个() {
        let (app, _) = app_with("operator");
        let (code, body) = body_json(
            &app,
            req(
                "/api/rounds/1/judgements/k1/override",
                serde_json::json!({"to_tier": "alternate", "reason": "  "}),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        // 这句话页面照着显示就是对的，藏起来只会让人不知道该改什么
        assert!(body["error"].as_str().unwrap().contains("理由"), "{body}");

        let (code, body) = body_json(
            &app,
            req(
                "/api/rounds/1/judgements/k1/override",
                serde_json::json!({"to_tier": "8 分", "reason": "好"}),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("这一档"), "{body}");
    }

    #[tokio::test]
    async fn 手动开轮只排队不当场跑() {
        let (app, st) = app_with("operator");
        let (code, body) =
            body_json(&app, req("/api/rounds", serde_json::json!({"days": 2}))).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["queued_ahead"], 0);
        let id = body["work_id"].as_i64().unwrap();

        let conn = st.conn.lock().await;
        let (kind, status, payload): (String, String, String) = conn
            .query_row(
                "SELECT kind, status, payload_json FROM work_queue WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        // 一轮四十分钟，HTTP 请求等不了
        assert_eq!((kind.as_str(), status.as_str()), ("manual_round", "queued"));
        assert!(payload.contains("\"days\":2"));
        // 这一轮还没开：开轮是干活那一侧的事
        let rounds: i64 = conn
            .query_row("SELECT COUNT(*) FROM rounds WHERE kind='manual'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rounds, 0);
    }

    #[tokio::test]
    async fn 窗口解析不了现在就报错() {
        let (app, _) = app_with("operator");
        let code = status(
            &app,
            req(
                "/api/rounds",
                serde_json::json!({"window_start": "上周一", "window_end": "今天"}),
            ),
        )
        .await;
        // 排进队里等四十分钟才发现写错了，比现在就报错糟得多
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn 重跑只认那两步而且轮次得在() {
        let (app, _) = app_with("operator");
        assert_eq!(
            status(
                &app,
                req("/api/rounds/1/steps/judge/rerun", serde_json::json!({}))
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            status(
                &app,
                req("/api/rounds/1/steps/submit/rerun", serde_json::json!({}))
            )
            .await,
            StatusCode::BAD_REQUEST,
            "登记与提交不许被重跑顺手带走"
        );
        assert_eq!(
            status(
                &app,
                req("/api/rounds/99/steps/judge/rerun", serde_json::json!({}))
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn 勾选不给轮次就落到当期那一轮() {
        let (app, st) = app_with("van");
        assert_eq!(
            status(
                &app,
                req(
                    "/api/van/marks",
                    serde_json::json!({"candidate_key": "k1", "mark": "like"})
                )
            )
            .await,
            StatusCode::OK
        );
        let conn = st.conn.lock().await;
        let round: i64 = conn
            .query_row("SELECT round_id FROM van_marks", [], |r| r.get(0))
            .unwrap();
        // Van 看的是今天真派下来的那一期，不该勾到凌晨那轮预取上去
        assert_eq!(round, 1);
    }

    #[tokio::test]
    async fn 指定首批只认这一轮里有的() {
        let (app, st) = app_with("operator");
        let (code, body) = body_json(
            &app,
            req(
                "/api/rounds/1/first-batch",
                serde_json::json!({"keys": ["k1", "翻篇了"]}),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["marked"], 1);
        let conn = st.conn.lock().await;
        assert_eq!(
            csw_collector_core::workbench::first_batch(&conn, 1).unwrap(),
            ["k1"]
        );
    }

    #[test]
    fn 四档之外没有别的档也没有分数() {
        assert!(parse_tier("recommend").is_ok());
        assert!(parse_tier("pending_check").is_ok());
        assert!(parse_tier("好").is_err());
        assert!(parse_tier("8").is_err());
        assert!(parse_tier("").is_err());
    }
}
