//! 定点补读：抓外链正文（09-22 判断台账反馈第三项）。
//!
//! 「完整内容在主页外链」「全文见官网」的条目，以前没读到就被判了不推荐。
//! 现在对这类**有明确报道潜力、补证路径清楚**的线索，抓一次外链正文再重判。
//!
//! # 边界
//!
//! - 只经 [`netguard::guarded_client`] 出网，发请求前先过 [`netguard::check_url`]：
//!   内网、回环、云元数据地址、非 80/443 端口一律不碰（09-23 用户同意补读的前提）。
//! - 限大小（[`MAX_BYTES`]，边读边数）、限时、只收文本类型。
//! - 抓回来的是**第三方内容**：进判断时过不可信边界（`prompt::fence`），
//!   只作材料，不作指令。
//! - 地址只从正文里取（csw 接口不给账号主页链接），且排除 Instagram 自己与贴文库。

use std::sync::LazyLock;
use std::time::Duration;

use futures::StreamExt;
use regex::Regex;
use url::Url;

use crate::netguard;

/// 一页最多读这么多字节。
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// 一页最多给判断多少字。
pub const MAX_CHARS: usize = 8000;
pub const TIMEOUT: Duration = Duration::from_secs(15);
/// 一条候选最多补读几个地址。
pub const MAX_URLS_PER_CANDIDATE: usize = 3;

/// 补读的结果。`status` 写进本地 `refetches` 表，也回写缺口的「已尝试」。
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub url: String,
    pub status: FetchStatus,
    pub http_status: Option<u16>,
    pub bytes: usize,
    /// 抽出来的正文（HTML 去掉标签）。失败时为空。
    pub text: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStatus {
    Ok,
    /// 被出网防护拦下（内网地址、怪端口、重定向到内网）
    Blocked,
    HttpError,
    TooLarge,
    BadType,
    Failed,
}

impl FetchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Blocked => "blocked",
            Self::HttpError => "http_error",
            Self::TooLarge => "too_large",
            Self::BadType => "bad_type",
            Self::Failed => "failed",
        }
    }

    pub fn cn(self) -> &'static str {
        match self {
            Self::Ok => "取到",
            Self::Blocked => "地址不安全，未访问",
            Self::HttpError => "对方返回错误",
            Self::TooLarge => "页面过大",
            Self::BadType => "不是网页",
            Self::Failed => "访问失败",
        }
    }
}

static URL_IN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https?://[^\s<>"'（）()【】「」《》，。；！？、]+"#).expect("正则")
});

/// 这些主机的链接不补读：Instagram 自己（要登录、读不到）、贴文库（已经在库里了）。
const SKIP_HOSTS: [&str; 5] = [
    "instagram.com",
    "instagr.am",
    "facebook.com",
    "campsomewhere.com",
    "threads.net",
];

/// 从正文里挑补读地址：去重、排除 [`SKIP_HOSTS`]、至多 [`MAX_URLS_PER_CANDIDATE`] 个。
pub fn urls_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in URL_IN_TEXT.find_iter(text) {
        let raw = m.as_str().trim_end_matches(['.', ',', ')', ']']);
        let Ok(u) = Url::parse(raw) else { continue };
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();
        if SKIP_HOSTS
            .iter()
            .any(|h| host == *h || host.ends_with(&format!(".{h}")))
        {
            continue;
        }
        let s = u.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
        if out.len() >= MAX_URLS_PER_CANDIDATE {
            break;
        }
    }
    out
}

/// 抓一页，抽出正文。**不抛错**：任何失败都落在 `status` 里，调用方如实写进缺口。
pub async fn fetch_text(client: &reqwest::Client, url: &str, allow_private: bool) -> Fetched {
    let mut out = Fetched {
        url: url.to_string(),
        status: FetchStatus::Failed,
        http_status: None,
        bytes: 0,
        text: String::new(),
        error: String::new(),
    };
    let parsed = match Url::parse(url) {
        Ok(u) => u,
        Err(e) => {
            out.error = format!("地址不合法：{e}");
            return out;
        }
    };
    // IP 字面量不走解析器，这一道必须在发请求前查
    if let Err(e) = netguard::check_url(&parsed, allow_private) {
        out.status = FetchStatus::Blocked;
        out.error = e;
        return out;
    }
    let resp = match client.get(parsed).timeout(TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("{e:#}");
            out.status = if e.is_redirect() || msg.contains("禁止") {
                FetchStatus::Blocked
            } else {
                FetchStatus::Failed
            };
            out.error = msg;
            return out;
        }
    };
    out.http_status = Some(resp.status().as_u16());
    if !resp.status().is_success() {
        out.status = FetchStatus::HttpError;
        return out;
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_html = ctype.contains("text/html") || ctype.contains("application/xhtml");
    if !(is_html || ctype.contains("text/plain")) {
        out.status = FetchStatus::BadType;
        out.error = format!("类型 {ctype}");
        return out;
    }
    if resp
        .content_length()
        .is_some_and(|n| n as usize > MAX_BYTES)
    {
        out.status = FetchStatus::TooLarge;
        return out;
    }
    let mut body: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(c) => {
                body.extend_from_slice(&c);
                if body.len() > MAX_BYTES {
                    out.status = FetchStatus::TooLarge;
                    out.bytes = body.len();
                    return out;
                }
            }
            Err(e) => {
                out.error = format!("{e:#}");
                return out;
            }
        }
    }
    out.bytes = body.len();
    let raw = String::from_utf8_lossy(&body);
    out.text = if is_html {
        html_to_text(&raw)
    } else {
        collapse(&raw)
    };
    out.status = FetchStatus::Ok;
    out
}

static DROP_BLOCKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<!--.*?-->|<(script|style|noscript|svg|nav|footer|header|form|iframe|template)\b.*?</\s*(script|style|noscript|svg|nav|footer|header|form|iframe|template)\s*>")
        .expect("正则")
});
static BREAKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<br\s*/?>|</(p|div|li|h[1-6]|tr|section|article|blockquote)\s*>")
        .expect("正则")
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("正则"));
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("正则"));

/// HTML → 纯文本：去掉脚本、样式、导航与标签，保留段落换行，截到 [`MAX_CHARS`]。
pub fn html_to_text(html: &str) -> String {
    let title = TITLE
        .captures(html)
        .map(|c| decode(c[1].trim()))
        .unwrap_or_default();
    let s = DROP_BLOCKS.replace_all(html, " ");
    let s = BREAKS.replace_all(&s, "\n");
    let s = TAGS.replace_all(&s, " ");
    let body = collapse(&decode(&s));
    let mut out = if title.is_empty() || body.starts_with(&title) {
        body
    } else {
        format!("{title}\n{body}")
    };
    if out.chars().count() > MAX_CHARS {
        out = out.chars().take(MAX_CHARS).collect();
    }
    out
}

fn decode(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn collapse(s: &str) -> String {
    s.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 正文里挑外链且跳过平台自己() {
        let t = "四款帐篷比较，全文见 https://www.hyperlitemountaingear.com/blogs/news/4-tents。\
                 主页 https://www.instagram.com/hyperlite/ 与 https://linktr.ee/hmg，\
                 重复 https://www.hyperlitemountaingear.com/blogs/news/4-tents";
        let us = urls_in(t);
        assert_eq!(
            us,
            [
                "https://www.hyperlitemountaingear.com/blogs/news/4-tents",
                "https://linktr.ee/hmg"
            ]
        );
    }

    #[test]
    fn 每条至多三个() {
        let t = (0..6)
            .map(|i| format!("https://e{i}.com/"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(urls_in(&t).len(), MAX_URLS_PER_CANDIDATE);
    }

    #[test]
    fn 网页去掉脚本样式导航只留正文() {
        let h = r#"<html><head><title>四款帐篷 &amp; 比较</title><style>p{}</style>
            <script>alert('x')</script></head><body><nav>菜单</nav>
            <p>Mid 1 比 Mid 2 轻 200 克。</p><p>Unbound 是双人款&nbsp;。</p>
            <!-- 注释 --><footer>版权</footer></body></html>"#;
        let t = html_to_text(h);
        assert!(t.starts_with("四款帐篷 & 比较"), "{t}");
        assert!(t.contains("Mid 1 比 Mid 2 轻 200 克。"));
        assert!(t.contains("Unbound 是双人款 。"));
        for x in ["alert", "菜单", "版权", "注释", "p{}"] {
            assert!(!t.contains(x), "{x} 没去掉：{t}");
        }
    }

    #[tokio::test]
    async fn 内网地址直接拦下不发请求() {
        let c = netguard::guarded_client(TIMEOUT, false).unwrap();
        for u in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8022/v1/embeddings",
            "http://2130706433/",
        ] {
            let f = fetch_text(&c, u, false).await;
            assert_eq!(f.status, FetchStatus::Blocked, "{u}：{f:?}");
            assert!(f.text.is_empty());
        }
    }

    #[tokio::test]
    async fn 取到网页正文并限大小与类型() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::path("/ok"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_raw("<p>完整内容</p>", "text/html; charset=utf-8"),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/big"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_raw("x".repeat(MAX_BYTES + 10), "text/html"),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/zip"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_raw(vec![0u8; 10], "application/zip"),
            )
            .mount(&server)
            .await;
        // 测试只能放行回环（wiremock 在 127.0.0.1 上）
        let c = netguard::guarded_client(TIMEOUT, true).unwrap();
        let f = fetch_text(&c, &format!("{}/ok", server.uri()), true).await;
        assert_eq!(f.status, FetchStatus::Ok);
        assert_eq!(f.text, "完整内容");
        let f = fetch_text(&c, &format!("{}/big", server.uri()), true).await;
        assert_eq!(f.status, FetchStatus::TooLarge);
        let f = fetch_text(&c, &format!("{}/zip", server.uri()), true).await;
        assert_eq!(f.status, FetchStatus::BadType);
    }
}
