//! 工作台的登录层：浏览器只拿一枚不透明会话 cookie，引擎的 token 一个都不出服务端。
//!
//! 流程（设计决定 4）：
//! ```text
//! 浏览器 ──POST /api/auth/login──▶ 采集服务 ──POST /admin/login──▶ 引擎
//!        ◀── Set-Cookie: csw_collector_sid（HttpOnly）
//!            + JSON 里带 csrf_token（给 JS 读，写接口回填到请求头）
//! ```
//!
//! 为什么不把引擎的 JWT 直接发给浏览器：那等于把引擎的后台权限散到前端，
//! 且工作台一旦被 XSS，攻击者拿到的就是引擎的管理身份。会话留在服务端，
//! 最坏情况也只丢一枚我们自己能立刻作废的不透明 id。
//!
//! Van 模式：引擎里没有 van 角色。配置里列出的 viewer 用户名映射成 van，
//! 勾选只写本地，不回写引擎。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use csw_collector_engineapi::admin::{AdminClient, EngineSession, now};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// 浏览器侧的会话 cookie 名
pub const SESSION_COOKIE: &str = "csw_collector_sid";
/// 写接口必须回填的 CSRF 头
pub const CSRF_HEADER: &str = "x-csw-csrf";
/// access 还剩这么多秒就提前续期，别等前端撞到 401
const REFRESH_SLACK_SECS: i64 = 60;
/// 会话绝对上限：引擎的 refresh 是 7 天，工作台不超过它
const SESSION_MAX_AGE_SECS: i64 = 7 * 24 * 3600;

#[derive(Clone)]
pub struct Session {
    pub engine: EngineSession,
    pub csrf: String,
    pub created_at: i64,
    /// 映射后的工作台角色：superadmin / operator / viewer / van
    pub role: String,
}

/// 会话存放处。打样期先放内存；阶段 1.1 换成本地库的 `sessions` 表，
/// 换掉后进程重启不掉线，接口不变。
#[derive(Default)]
pub struct SessionStore(RwLock<HashMap<String, Session>>);

impl SessionStore {
    pub fn get(&self, sid: &str) -> Option<Session> {
        let s = self.0.read().ok()?.get(sid).cloned()?;
        if now() - s.created_at > SESSION_MAX_AGE_SECS {
            self.remove(sid);
            return None;
        }
        Some(s)
    }
    pub fn put(&self, sid: String, s: Session) {
        if let Ok(mut g) = self.0.write() {
            g.insert(sid, s);
        }
    }
    pub fn remove(&self, sid: &str) {
        if let Ok(mut g) = self.0.write() {
            g.remove(sid);
        }
    }
    pub fn len(&self) -> usize {
        self.0.read().map(|g| g.len()).unwrap_or(0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct AuthState {
    pub engine: AdminClient,
    pub store: SessionStore,
    /// 这些 viewer 用户名在工作台里当 van
    pub van_usernames: Vec<String>,
    /// 生产必须为 true；本机 http 调试才关
    pub secure_cookie: bool,
}

impl AuthState {
    fn map_role(&self, username: &str, engine_role: &str) -> String {
        if engine_role == "viewer" && self.van_usernames.iter().any(|u| u == username) {
            "van".to_string()
        } else {
            engine_role.to_string()
        }
    }
}

pub fn router(state: Arc<AuthState>) -> axum::Router {
    axum::Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/me", get(me))
        .route("/api/auth/logout", post(logout))
        .with_state(state)
}

#[derive(Deserialize)]
pub struct LoginReq {
    username: String,
    password: String,
}

#[derive(Serialize)]
pub struct MeResp {
    username: String,
    display_name: String,
    /// 工作台角色（可能是引擎没有的 van）
    role: String,
    /// 引擎里的真实角色，界面上要能看出 van 其实是个 viewer
    engine_role: String,
    csrf_token: String,
}

async fn login(
    State(st): State<Arc<AuthState>>,
    Json(req): Json<LoginReq>,
) -> Result<Response, ApiError> {
    let engine = st
        .engine
        .login(&req.username, &req.password)
        .await
        // 不把引擎的原文错误透给浏览器：它可能区分「用户不存在」与「口令错」
        .map_err(|e| ApiError::unauthorized("用户名或口令不对", e))?;

    let sid = random_token();
    let csrf = random_token();
    let role = st.map_role(&engine.user.username, &engine.user.role);
    let resp = MeResp {
        username: engine.user.username.clone(),
        display_name: engine.user.display_name.clone(),
        role: role.clone(),
        engine_role: engine.user.role.clone(),
        csrf_token: csrf.clone(),
    };
    st.store.put(
        sid.clone(),
        Session {
            engine,
            csrf,
            created_at: now(),
            role,
        },
    );

    let cookie = format!(
        "{SESSION_COOKIE}={sid}; Path=/; Max-Age={SESSION_MAX_AGE_SECS}; HttpOnly; SameSite=Lax{}",
        if st.secure_cookie { "; Secure" } else { "" }
    );
    Ok(([(header::SET_COOKIE, cookie)], Json(resp)).into_response())
}

async fn me(State(st): State<Arc<AuthState>>, headers: HeaderMap) -> Result<Response, ApiError> {
    let sid = sid_from(&headers).ok_or_else(|| ApiError::unauth_plain("没有会话"))?;
    let mut s = st
        .store
        .get(&sid)
        .ok_or_else(|| ApiError::unauth_plain("会话已失效"))?;

    // access 快过期就先续。引擎会轮换 refresh，新的那枚要存回去。
    if s.engine.access_expires_at - now() < REFRESH_SLACK_SECS {
        match st.engine.refresh(&s.engine.refresh_cookie).await {
            Ok(fresh) => {
                s.engine = fresh;
                st.store.put(sid.clone(), s.clone());
            }
            Err(e) => {
                st.store.remove(&sid);
                return Err(ApiError::unauthorized("会话已过期，请重新登录", e));
            }
        }
    }

    // 角色以引擎当下的回答为准——后台里改了角色或停用，这里立刻生效
    let user = st
        .engine
        .me(&s.engine.access_token)
        .await
        .map_err(|e| ApiError::unauthorized("会话已失效", e))?;
    let role = st.map_role(&user.username, &user.role);
    if role != s.role {
        s.role = role.clone();
        st.store.put(sid, s.clone());
    }
    Ok(Json(MeResp {
        username: user.username,
        display_name: user.display_name,
        role,
        engine_role: user.role,
        csrf_token: s.csrf,
    })
    .into_response())
}

async fn logout(State(st): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    if let Some(sid) = sid_from(&headers) {
        st.store.remove(&sid);
    }
    let cookie = format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax");
    ([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response()
}

/// 只读接口的守卫：有一枚有效会话就行，**不校验 CSRF**。
///
/// CSRF 防的是「攻击者借受害者的 cookie 发出一个**改状态**的请求」。
/// 读接口不改状态，而跨站发出去的读请求，攻击者也读不到响应（同源策略挡在那里），
/// 所以这里只问「你是谁」。写接口另有 [`check_write`]。
pub fn check_read(st: &AuthState, headers: &HeaderMap) -> Result<Session, ApiError> {
    let sid = sid_from(headers).ok_or_else(|| ApiError::unauth_plain("没有会话"))?;
    st.store
        .get(&sid)
        .ok_or_else(|| ApiError::unauth_plain("会话已失效"))
}

/// 写接口的守卫：会话有效 + CSRF 头与会话里的对得上。
/// SameSite=Lax 挡不住表单式的跨站 POST，所以双提交这一层不能省。
pub fn check_write(st: &AuthState, headers: &HeaderMap) -> Result<Session, ApiError> {
    let s = check_read(st, headers)?;
    let got = headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !constant_time_eq(got.as_bytes(), s.csrf.as_bytes()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "CSRF 校验不通过".into()));
    }
    Ok(s)
}

fn sid_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 给浏览器的是一句人话；引擎的原文只进日志。
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn unauthorized(msg: &str, cause: anyhow::Error) -> Self {
        tracing::warn!(error = %cause, "登录或续期失败");
        Self(StatusCode::UNAUTHORIZED, msg.to_string())
    }
    pub fn unauth_plain(msg: &str) -> Self {
        Self(StatusCode::UNAUTHORIZED, msg.to_string())
    }
    /// 拆给工作台的错误类型用。登录层的错（401 / 403）要原样传到页面上——
    /// 「会话已失效」与「CSRF 校验不通过」是两件要分开处理的事。
    pub fn parts(self) -> (StatusCode, String) {
        (self.0, self.1)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}
