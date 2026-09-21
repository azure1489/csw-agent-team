//! 引擎管理后台（`adminsrv`）的鉴权客户端。
//!
//! 工作台自己不发 JWT、不碰 `CSW_JWT_SECRET`——用户名口令原样转给引擎，
//! 引擎发的 access 与 refresh 都只留在采集服务的服务端会话里，浏览器一个都拿不到。
//! 这样引擎始终是唯一的账号真相，工作台停掉也不会留下一套平行的凭据。

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// 引擎下发 refresh token 用的 cookie 名（`Path=/admin`，登录与续期都会轮换）。
const REFRESH_COOKIE: &str = "csw_refresh";

#[derive(Debug, Clone, Deserialize)]
pub struct AdminUser {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    /// superadmin / operator / viewer——引擎里没有 van，van 是工作台按配置映射出来的
    pub role: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct LoginBody {
    access_token: String,
    expires_in: i64,
    user: AdminUser,
}

#[derive(Debug, Deserialize)]
struct MeBody {
    user: AdminUser,
}

/// 一次登录或续期的产物。**这三样东西只许留在服务端。**
#[derive(Debug, Clone)]
pub struct EngineSession {
    pub access_token: String,
    /// access 过期的绝对时刻（Unix 秒）
    pub access_expires_at: i64,
    pub refresh_cookie: String,
    pub user: AdminUser,
}

pub struct AdminClient {
    base: String,
    http: reqwest::Client,
}

impl AdminClient {
    pub fn new(base_url: &str) -> Result<Self> {
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            // 刻意不开 reqwest 的 cookie feature：引擎每次都会轮换 refresh，
            // 自动 cookie store 会把「这枚 refresh 属于哪个用户会话」这件事糊掉。
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()?,
        })
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<EngineSession> {
        let resp = self
            .http
            .post(format!("{}/admin/login", self.base))
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()
            .await
            .context("连引擎 /admin/login")?;
        self.session_from(resp, None).await
    }

    /// 用手里的 refresh cookie 换一枚新的 access。引擎会轮换 refresh，旧的立刻作废。
    pub async fn refresh(&self, refresh_cookie: &str) -> Result<EngineSession> {
        let resp = self
            .http
            .post(format!("{}/admin/refresh", self.base))
            .header(
                reqwest::header::COOKIE,
                format!("{REFRESH_COOKIE}={refresh_cookie}"),
            )
            .send()
            .await
            .context("连引擎 /admin/refresh")?;
        // 万一引擎这次没重发 cookie，就继续用旧的，不要把会话弄丢
        self.session_from(resp, Some(refresh_cookie.to_string()))
            .await
    }

    /// 拿 access 问「我是谁」。角色以引擎为准，不信任会话里缓存的那份。
    pub async fn me(&self, access_token: &str) -> Result<AdminUser> {
        let resp = self
            .http
            .get(format!("{}/admin/me", self.base))
            .bearer_auth(access_token)
            .send()
            .await
            .context("连引擎 /admin/me")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("引擎 /admin/me 返回 {status}：{}", truncate(&text));
        }
        Ok(serde_json::from_str::<MeBody>(&text)
            .context("解析 /admin/me")?
            .user)
    }

    async fn session_from(
        &self,
        resp: reqwest::Response,
        fallback_cookie: Option<String>,
    ) -> Result<EngineSession> {
        let status = resp.status();
        let cookie = extract_cookie(&resp).or(fallback_cookie);
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("引擎返回 {status}：{}", truncate(&text));
        }
        let body: LoginBody = serde_json::from_str(&text).context("解析引擎登录响应")?;
        let refresh_cookie = cookie.context("引擎没有下发 csw_refresh cookie")?;
        Ok(EngineSession {
            access_expires_at: now() + body.expires_in,
            access_token: body.access_token,
            refresh_cookie,
            user: body.user,
        })
    }
}

fn extract_cookie(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| {
            let v = v.split(';').next()?.trim();
            v.strip_prefix(&format!("{REFRESH_COOKIE}="))
                .map(|s| s.to_string())
        })
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 错误体可能带回口令回显之类的东西，截短再进日志
fn truncate(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= 200 {
        s.to_string()
    } else {
        s.chars().take(200).collect::<String>() + "…"
    }
}
