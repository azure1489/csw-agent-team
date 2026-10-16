//! 后台管理远程 MCP 令牌（`POST /mcp` 用的那种）：列出、签发、吊销。
//!
//! - **只给 superadmin**：令牌等于把知识库开给公司外的一台机器，读接口也不给 operator。
//! - **明文只在签发的那一次响应里**（`Cache-Control: no-store`），库里只存 SHA-256；
//!   之后列表里只有名字、权限、时间与用量，查不回令牌。
//! - **签发与吊销都进 `audit`**，与改动同一个事务；留痕里记名字与权限，**不记令牌**。
//! - 写接口照工作台规矩过 CSRF。
//!
//! 令牌本身的规则（格式、权限范围、memory 拒签）在 [`super::mcp_http`]，CLI 与这里共用。

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use csw_collector_core::workbench;

use crate::bff::{Session, check_read, check_write};

use super::http::ApiError;
use super::mcp_http;
use super::write::WriteState;

pub fn router(st: WriteState) -> Router {
    Router::new()
        .route("/api/mcp-tokens", get(list).post(issue))
        .route("/api/mcp-tokens/{id}/revoke", post(revoke))
        .with_state(st)
}

fn superadmin(s: Session) -> Result<Session, ApiError> {
    if s.role != "superadmin" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "只有超级管理员能管理 MCP 令牌".into(),
        ));
    }
    Ok(s)
}

#[derive(Serialize)]
struct TokenRow {
    id: i64,
    name: String,
    scopes: Vec<String>,
    created_at: String,
    expires_at: Option<String>,
    revoked_at: Option<String>,
    last_used_at: Option<String>,
    /// 近 7 天的调用次数（含失败）
    calls_7d: i64,
    /// 是否还能用：没吊销、没过期
    active: bool,
}

async fn list(
    State(st): State<WriteState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TokenRow>>, ApiError> {
    superadmin(check_read(&st.auth, &headers).map_err(ApiError::from_bff)?)?;
    let conn = st.app.conn.lock().await;
    let mut q = conn
        .prepare(
            "SELECT t.id, t.name, t.scopes, t.created_at, t.expires_at, t.revoked_at, t.last_used_at,
                    (SELECT count(*) FROM mcp_calls c WHERE c.token_id = t.id
                       AND c.created_at > strftime('%Y-%m-%dT%H:%M:%SZ','now','-7 days')),
                    (t.revoked_at IS NULL AND (t.expires_at IS NULL
                       OR t.expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now')))
             FROM mcp_tokens t ORDER BY t.id DESC",
        )
        .map_err(ApiError::db)?;
    let rows = q
        .query_map([], |r| {
            Ok(TokenRow {
                id: r.get(0)?,
                name: r.get(1)?,
                scopes: r
                    .get::<_, String>(2)?
                    .split(',')
                    .map(str::to_string)
                    .collect(),
                created_at: r.get(3)?,
                expires_at: r.get(4)?,
                revoked_at: r.get(5)?,
                last_used_at: r.get(6)?,
                calls_7d: r.get(7)?,
                active: r.get(8)?,
            })
        })
        .map_err(ApiError::db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(ApiError::db)?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct IssueReq {
    name: String,
    scopes: Vec<String>,
    /// 多少天后过期；不给或 null 就不过期
    #[serde(default)]
    days: Option<u32>,
}

/// 签发。响应里的 `token` 是**唯一一次**能看到明文的地方。
async fn issue(
    State(st): State<WriteState>,
    headers: HeaderMap,
    Json(req): Json<IssueReq>,
) -> Result<Response, ApiError> {
    let s = superadmin(check_write(&st.auth, &headers).map_err(ApiError::from_bff)?)?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "名字要写（给谁、哪台机器用），不超过 60 字".into(),
        ));
    }
    if let Some(d) = req.days
        && !(1..=3650).contains(&d)
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "有效期要在 1 到 3650 天之间；不过期就不填".into(),
        ));
    }
    let scopes = mcp_http::parse_scopes(&req.scopes.join(",")).map_err(ApiError::bad_request)?;
    let who = s.engine.user.username.clone();
    let conn = st.app.conn.lock().await;
    let tx = conn.unchecked_transaction().map_err(ApiError::db)?;
    let (id, token) =
        mcp_http::issue(&tx, name, &scopes, req.days).map_err(ApiError::bad_request)?;
    workbench::audit(
        &tx,
        &who,
        "mcp_token_issue",
        &format!("mcp-token#{id}"),
        &serde_json::json!({"名字": name, "权限": scopes, "有效期天数": req.days}),
    )
    .map_err(ApiError::any)?;
    tx.commit().map_err(ApiError::db)?;
    tracing::info!(令牌 = id, 名字 = %name, 权限 = %scopes, 签发人 = %who, "后台签发了一枚 MCP 令牌");
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "id": id,
            "name": name,
            "scopes": scopes.split(',').collect::<Vec<_>>(),
            "days": req.days,
            "token": token,
        })),
    )
        .into_response())
}

/// 吊销。立即生效：下一次带这枚令牌的请求就是 401。
async fn revoke(
    State(st): State<WriteState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let s = superadmin(check_write(&st.auth, &headers).map_err(ApiError::from_bff)?)?;
    let who = s.engine.user.username.clone();
    let conn = st.app.conn.lock().await;
    let tx = conn.unchecked_transaction().map_err(ApiError::db)?;
    let name: Option<String> = tx
        .query_row("SELECT name FROM mcp_tokens WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .ok();
    let Some(name) = name else {
        return Err(ApiError(StatusCode::NOT_FOUND, format!("没有 #{id}")));
    };
    if !mcp_http::revoke(&tx, id).map_err(ApiError::any)? {
        return Err(ApiError(StatusCode::CONFLICT, format!("#{id} 早已吊销")));
    }
    workbench::audit(
        &tx,
        &who,
        "mcp_token_revoke",
        &format!("mcp-token#{id}"),
        &serde_json::json!({"名字": name}),
    )
    .map_err(ApiError::any)?;
    tx.commit().map_err(ApiError::db)?;
    tracing::info!(令牌 = id, 名字 = %name, 吊销人 = %who, "后台吊销了一枚 MCP 令牌");
    Ok(Json(serde_json::json!({"id": id, "revoked": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bff::AuthState;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tower::ServiceExt;

    const SID: &str = "sid";
    const CSRF: &str = "csrf";

    fn app(role: &str) -> (Router, Arc<super::super::http::AppState>) {
        let app_state = Arc::new(super::super::http::AppState {
            conn: tokio::sync::Mutex::new(csw_collector_core::store::open_in_memory().unwrap()),
            cfg: csw_collector_core::Config::default(),
            started: std::time::Instant::now(),
            svc: None,
        });
        let auth = Arc::new(AuthState {
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
                        username: "管理员".into(),
                        display_name: String::new(),
                        role: role.into(),
                        status: "active".into(),
                    },
                },
                csrf: CSRF.into(),
                created_at: csw_collector_engineapi::admin::now(),
                role: role.into(),
            },
        );
        (
            router(WriteState {
                app: app_state.clone(),
                auth,
            }),
            app_state,
        )
    }

    fn req(method: &str, path: &str, body: Option<serde_json::Value>, csrf: bool) -> Request<Body> {
        let mut b = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", format!("{}={SID}", crate::bff::SESSION_COOKIE))
            .header("content-type", "application/json");
        if csrf {
            b = b.header(crate::bff::CSRF_HEADER, CSRF);
        }
        b.body(body.map_or(Body::empty(), |v| Body::from(v.to_string())))
            .unwrap()
    }

    async fn send(app: &Router, r: Request<Body>) -> (StatusCode, HeaderMap, serde_json::Value) {
        let resp = app.clone().oneshot(r).await.unwrap();
        let (p, b) = resp.into_parts();
        let b = b.collect().await.unwrap().to_bytes();
        (
            p.status,
            p.headers,
            serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn 签发列出吊销并留痕() {
        let (app, st) = app("superadmin");
        let (s, h, v) = send(
            &app,
            req(
                "POST",
                "/api/mcp-tokens",
                Some(serde_json::json!({"name": "甲的 codex", "scopes": ["magazine", "kb"]})),
                true,
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(h.get("cache-control").unwrap(), "no-store");
        let token = v["token"].as_str().unwrap().to_string();
        assert!(token.starts_with("cswkb_"));
        let id = v["id"].as_i64().unwrap();
        {
            let c = st.conn.lock().await;
            assert!(mcp_http::verify(&c, &token).unwrap().is_some());
        }

        let (s, _, v) = send(&app, req("GET", "/api/mcp-tokens", None, false)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v[0]["name"], "甲的 codex");
        assert_eq!(v[0]["active"], true);
        assert_eq!(v[0]["expires_at"], serde_json::Value::Null);
        assert!(!v.to_string().contains(&token), "列表里不能有令牌");

        let (s, _, _) = send(
            &app,
            req("POST", &format!("/api/mcp-tokens/{id}/revoke"), None, true),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let (s, _, _) = send(
            &app,
            req("POST", &format!("/api/mcp-tokens/{id}/revoke"), None, true),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT);
        let (_, _, v) = send(&app, req("GET", "/api/mcp-tokens", None, false)).await;
        assert_eq!(v[0]["active"], false);

        let c = st.conn.lock().await;
        assert!(mcp_http::verify(&c, &token).unwrap().is_none());
        let audit: Vec<(String, String)> = c
            .prepare("SELECT action, detail_json FROM audit ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].0, "mcp_token_issue");
        assert_eq!(audit[1].0, "mcp_token_revoke");
        assert!(
            audit.iter().all(|(_, d)| !d.contains(&token)),
            "留痕不能有令牌"
        );
    }

    #[tokio::test]
    async fn 只有超级管理员且写要csrf() {
        for role in ["operator", "viewer", "van"] {
            let (app, _) = app(role);
            let (s, _, _) = send(&app, req("GET", "/api/mcp-tokens", None, false)).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{role}");
            let (s, _, _) = send(
                &app,
                req(
                    "POST",
                    "/api/mcp-tokens",
                    Some(serde_json::json!({"name": "x", "scopes": ["magazine"]})),
                    true,
                ),
            )
            .await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{role}");
        }
        let (app, _) = app("superadmin");
        let (s, _, _) = send(
            &app,
            req(
                "POST",
                "/api/mcp-tokens",
                Some(serde_json::json!({"name": "x", "scopes": ["magazine"]})),
                false,
            ),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "没 CSRF 也不行");
    }

    #[tokio::test]
    async fn 参数不对如实报() {
        let (app, _) = app("superadmin");
        for (body, want) in [
            (serde_json::json!({"name": " ", "scopes": ["kb"]}), "名字"),
            (
                serde_json::json!({"name": "x", "scopes": ["memory"]}),
                "不对外",
            ),
            (serde_json::json!({"name": "x", "scopes": []}), "至少"),
            (
                serde_json::json!({"name": "x", "scopes": ["kb"], "days": 0}),
                "有效期",
            ),
        ] {
            let (s, _, v) = send(&app, req("POST", "/api/mcp-tokens", Some(body), true)).await;
            assert_eq!(s, StatusCode::BAD_REQUEST);
            assert!(v["error"].as_str().unwrap().contains(want), "{v}");
        }
        let (s, _, v) = send(
            &app,
            req(
                "POST",
                "/api/mcp-tokens",
                Some(serde_json::json!({"name": "限期", "scopes": ["magazine"], "days": 30})),
                true,
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let (_, _, v) = send(&app, req("GET", "/api/mcp-tokens", None, false)).await;
        assert!(v[0]["expires_at"].is_string());
    }
}
