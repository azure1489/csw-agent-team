//! 高清原图服务（`hires-service`）的客户端。
//!
//! 来源库（Bright Data）2026-06 起只给 Instagram 图片的 640px 档，05 / 11 要的原图从这里补：
//! 输入 shortcode，服务用已登录的 Instagram 会话取每张图 `image_versions2` 里最大的一档
//! （原始比例，`dst-jpg` 地址下载下来就是 JPEG），转存 OSS 后把地址、像素、文件名返回。
//!
//! 几条要长在代码里的事实（见 `~/project/hires-service/README.md`）：
//! - **串行 + 限速**：同一时间只有一个请求在访问 Instagram，两次之间 ≥ 8 秒；
//!   同一 shortcode 24 小时内缓存（`cached: true`），重开派工不会再打 Instagram。
//! - **`file_key` 是 CDN 文件名**，同一张图不管哪个分辨率都相同——来源库那张 640 图的
//!   原始地址里也是它。工作台手里没有原始地址（csw 接口不返回），所以按轮播序号对应，
//!   再用画面校验兜底（见 collector 的 `hires_match`）。
//! - **`source_url` 会过期**（`oe=` 是过期时间），长期用 `oss.url`。
//! - **熔断**：cookie 失效后服务对所有请求回 503 `session_invalid`，要人换 cookie 再
//!   `POST /v1/session/reload`。这不是工作台能自己修的，报失败时要把它原样说出来。

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct HiresConfig {
    pub base_url: String,
    pub token: String,
    pub timeout: Duration,
}

/// 一张图（或一个视频）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HiresItem {
    /// 轮播里的位置，0 起；单图帖就是 0
    pub index: usize,
    /// `Photo` | `Video`
    #[serde(rename = "type")]
    pub kind: String,
    pub width: u32,
    pub height: u32,
    /// CDN 文件名，不含查询串。同一张图所有分辨率都一样
    pub file_key: String,
    /// Instagram CDN 签名地址，会过期；只作证据记录
    #[serde(default)]
    pub source_url: String,
    /// 没上传（视频默认不传）或上传失败时为空
    #[serde(default)]
    pub oss: Option<HiresOss>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HiresOss {
    pub url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub file_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HiresResult {
    pub shortcode: String,
    #[serde(default)]
    pub pk: String,
    #[serde(default)]
    pub cached: bool,
    #[serde(default)]
    pub items: Vec<HiresItem>,
}

impl HiresResult {
    /// 只要图片，按轮播顺序。
    pub fn photos(&self) -> Vec<&HiresItem> {
        let mut v: Vec<&HiresItem> = self.items.iter().filter(|i| i.kind == "Photo").collect();
        v.sort_by_key(|i| i.index);
        v
    }
}

/// 服务回的错误。**原因要能原样进失败报告**：主编按它决定是换 cookie 还是等明天。
#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum HiresError {
    #[error("hires-service 的 Instagram 会话已失效（服务已熔断，需换 cookie 后 reload）：{0}")]
    SessionInvalid(String),
    #[error("hires-service 当天对 Instagram 的请求次数已达上限：{0}")]
    DailyLimit(String),
    #[error("hires-service 排队请求太多：{0}")]
    QueueFull(String),
    #[error("Instagram 上找不到这条帖子（已删或不公开）：{0}")]
    NotFound(String),
    #[error("hires-service 的 token 不对或缺失")]
    Unauthorized,
    #[error("hires-service 调 Instagram 失败：{0}")]
    Upstream(String),
    #[error("hires-service 返回 HTTP {0}：{1}")]
    Http(u16, String),
    #[error("连不上 hires-service：{0}")]
    Transport(String),
    #[error("hires-service 返回的内容读不懂：{0}")]
    BadResponse(String),
}

#[derive(Debug, Deserialize)]
struct ErrBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
}

pub struct HiresClient {
    cfg: HiresConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for HiresClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HiresClient")
            .field("base_url", &self.cfg.base_url)
            .field("token", &format!("{} 字符", self.cfg.token.len()))
            .finish()
    }
}

impl HiresClient {
    pub fn new(cfg: HiresConfig) -> Result<Self> {
        csw_collector_core::ensure_crypto_provider();
        anyhow::ensure!(!cfg.token.trim().is_empty(), "缺 HIRES_TOKEN");
        anyhow::ensure!(
            cfg.base_url.starts_with("https://") || cfg.base_url.starts_with("http://127.0.0.1"),
            "hires base_url 必须是 https（token 明文走线）"
        );
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .build()
            .context("建 hires 客户端")?;
        Ok(Self { cfg, http })
    }

    pub fn base_url(&self) -> &str {
        &self.cfg.base_url
    }

    /// 一条帖子的全部图片（视频不传 OSS）。
    pub async fn media(&self, shortcode: &str) -> std::result::Result<HiresResult, HiresError> {
        let sc = shortcode.trim();
        if sc.is_empty()
            || !sc
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(HiresError::BadResponse(format!("shortcode 不合法：{sc:?}")));
        }
        let url = format!("{}/v1/media/{sc}", self.cfg.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.cfg.token)
            .send()
            .await
            .map_err(|e| HiresError::Transport(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| HiresError::Transport(e.to_string()))?;
        if status.is_success() {
            return serde_json::from_str::<HiresResult>(&body)
                .map_err(|e| HiresError::BadResponse(format!("{e}（{}）", head(&body))));
        }
        let eb: ErrBody = serde_json::from_str(&body).unwrap_or(ErrBody {
            error: String::new(),
            message: head(&body),
        });
        let msg = if eb.message.is_empty() {
            eb.error.clone()
        } else {
            eb.message.clone()
        };
        Err(match (status.as_u16(), eb.error.as_str()) {
            (401, _) => HiresError::Unauthorized,
            (404, _) => HiresError::NotFound(msg),
            (429, "daily_limit") => HiresError::DailyLimit(msg),
            (429, _) => HiresError::QueueFull(msg),
            (503, _) => HiresError::SessionInvalid(msg),
            (502, _) => HiresError::Upstream(msg),
            (code, _) => HiresError::Http(code, msg),
        })
    }
}

fn head(s: &str) -> String {
    s.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(base: &str) -> HiresClient {
        // wiremock 的地址就是 http://127.0.0.1:端口，正好过「只许 https 或本机」那道检查
        HiresClient::new(HiresConfig {
            base_url: base.to_string(),
            token: "t0ken-at-least-16-chars".into(),
            timeout: Duration::from_secs(5),
        })
        .unwrap()
    }

    const OK: &str = r#"{"shortcode":"Dd8nAZVS5n6","pk":"3998242120212912634","cached":false,
      "items":[
        {"index":0,"type":"Photo","width":1350,"height":1687,"file_key":"830177726_18010723259976841_5490932647440578128_n.jpg",
         "source_url":"https://scontent.cdninstagram.com/v/a.jpg?stp=dst-jpg_e35_tt6&oe=1",
         "oss":{"url":"https://cws-file.oss-cn-guangzhou.aliyuncs.com/upload/2026/10/05/HD4uOKNS.jpg","size":376572,"mime_type":"image/jpeg","file_hash":"6f98"}},
        {"index":1,"type":"Video","width":720,"height":900,"file_key":"v.mp4","source_url":"https://x/v.mp4","oss":null},
        {"index":2,"type":"Photo","width":1080,"height":1350,"file_key":"b.jpg","source_url":"https://x/b.jpg","oss":null,"error":"upload failed: 502"}
      ]}"#;

    #[tokio::test]
    async fn 成功时按轮播顺序只取图片() {
        let srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/media/Dd8nAZVS5n6"))
            .and(header("authorization", "Bearer t0ken-at-least-16-chars"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(OK, "application/json"))
            .mount(&srv)
            .await;
        let r = client(&srv.uri()).media("Dd8nAZVS5n6").await.unwrap();
        assert_eq!(r.pk, "3998242120212912634");
        let ph = r.photos();
        assert_eq!(ph.len(), 2, "视频不算");
        assert_eq!(ph[0].width, 1350);
        assert_eq!(
            ph[0].oss.as_ref().unwrap().url,
            "https://cws-file.oss-cn-guangzhou.aliyuncs.com/upload/2026/10/05/HD4uOKNS.jpg"
        );
        assert!(ph[1].oss.is_none(), "上传失败的 oss 为空");
        assert_eq!(ph[1].error.as_deref(), Some("upload failed: 502"));
    }

    #[tokio::test]
    async fn 错误码各归各类_原因原样带回() {
        let srv = MockServer::start().await;
        for (sc, code, body) in [
            (
                "a1",
                503,
                r#"{"error":"session_invalid","message":"redirected to login"}"#,
            ),
            ("a2", 429, r#"{"error":"daily_limit","message":"300/300"}"#),
            ("a3", 429, r#"{"error":"queue_full","message":"20 queued"}"#),
            ("a4", 404, r#"{"error":"not_found","message":"gone"}"#),
            ("a5", 401, r#"{"error":"unauthorized","message":""}"#),
            (
                "a6",
                502,
                r#"{"error":"upstream_error","message":"ig 500"}"#,
            ),
            ("a7", 500, "<html>oops</html>"),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/v1/media/{sc}")))
                .respond_with(ResponseTemplate::new(code).set_body_raw(body, "application/json"))
                .mount(&srv)
                .await;
        }
        let c = client(&srv.uri());
        assert_eq!(
            c.media("a1").await.unwrap_err(),
            HiresError::SessionInvalid("redirected to login".into())
        );
        assert_eq!(
            c.media("a2").await.unwrap_err(),
            HiresError::DailyLimit("300/300".into())
        );
        assert_eq!(
            c.media("a3").await.unwrap_err(),
            HiresError::QueueFull("20 queued".into())
        );
        assert_eq!(
            c.media("a4").await.unwrap_err(),
            HiresError::NotFound("gone".into())
        );
        assert_eq!(c.media("a5").await.unwrap_err(), HiresError::Unauthorized);
        assert_eq!(
            c.media("a6").await.unwrap_err(),
            HiresError::Upstream("ig 500".into())
        );
        assert_eq!(
            c.media("a7").await.unwrap_err(),
            HiresError::Http(500, "<html>oops</html>".into())
        );
        // 失败报告里要能看出是哪类
        assert!(
            HiresError::SessionInvalid("x".into())
                .to_string()
                .contains("换 cookie")
        );
    }

    #[test]
    fn 没有token或不走https建不起来() {
        assert!(
            HiresClient::new(HiresConfig {
                base_url: "https://hires.example".into(),
                token: " ".into(),
                timeout: Duration::from_secs(1),
            })
            .is_err()
        );
        assert!(
            HiresClient::new(HiresConfig {
                base_url: "http://hires.example".into(),
                token: "abcdefghijklmnop".into(),
                timeout: Duration::from_secs(1),
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn 不合法的shortcode不出网() {
        let c = client("http://127.0.0.1:9");
        assert!(matches!(
            c.media("../etc").await.unwrap_err(),
            HiresError::BadResponse(_)
        ));
    }
}
