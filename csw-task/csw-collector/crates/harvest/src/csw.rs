//! csw 贴文库的 HTTP 客户端。
//!
//! 实测得到、必须长在代码里的四条（阶段 0.3）：
//!
//! 1. **路径前缀是 `/api/v1`**，走 API Key 认证；基址是 `agent-api.campsomewhere.com`
//!    （`agent.campsomewhere.com` 是前端域名，不是这个）。
//! 2. **`posts/window` 按 `date_posted` 过滤，不是 `ingestedAt`**
//!    （`backend/internal/db/posts_window.go:56`）。总方案的窗口口径是「首次入库时间」，
//!    接口不支持——所以这里取一个**更宽的发布时间窗口**，再在本地按 `ingestedAt` 收口。
//! 3. **`/accounts` 的 `total` 是页数不是条数**，条数在 `records`；`page`/`size` 分页，size ≤100。
//! 4. **单页 50 条比 100 稳**：100 条撞上冷启动会碰到 90 秒超时。冷启动首个请求可达 40 秒。
//!
//! 还有一条不是接口的事实：**线上版本落后于本地 main**——返回里没有 `tagList`、
//! `hashtagList`，`mediaList` 里也没有 `shortCaption` / `detailedAlt`。
//! 所以这些字段一律当可缺，标签补丁部署后自然就有了，代码不用改。

use std::time::Duration;

use anyhow::{Context, Result, bail};
use csw_collector_core::types::{Candidate, MediaKind, MediaRef, Platform, Timestamp};
use serde::Deserialize;

/// 取更宽的发布时间窗口再本地按原始披露时间收口（没有披露时间的按首次入库）。
///
/// 为什么是 7 天：一条 9/17 发布的贴文若 9/20 才被抓到，按入库时间它属于 9/20 那一期；
/// 用发布时间开窗就会漏掉它。实测 9/17–9/18 那批两者按日完全重合（216 / 222），
/// 但那只说明爬虫当日入库，不是保证。7 天是「漏掉的代价」与「多取的成本」之间的折中：
/// 多取的条目在本地一筛就掉，成本只是几次分页。
pub const WINDOW_LOOKBACK_DAYS: i64 = 7;

#[derive(Debug, Clone)]
pub struct CswConfig {
    pub base_url: String,
    pub api_key: String,
    /// 单页条数。50 稳，100 撞冷启动会超时。
    pub page_size: u32,
    pub timeout: Duration,
}

pub struct CswClient {
    cfg: CswConfig,
    http: reqwest::Client,
    /// 挂上之后 `replay` 才真的不出网。见 [`Self::with_recorder`]。
    rec: Option<std::sync::Arc<csw_collector_core::record::Recorder>>,
}

/// 外层信封 `{success, data}`。
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    success: bool,
    data: Option<T>,
    #[serde(default)]
    message: String,
}

/// 窗口取数实际发出的请求。
#[derive(Debug, Clone, Default)]
pub struct WindowLog {
    /// 不含 offset 的请求路径
    pub path: String,
    pub pages: Vec<PageLog>,
}

#[derive(Debug, Clone)]
pub struct PageLog {
    pub offset: u32,
    pub n: usize,
    pub has_more: bool,
    pub total: i64,
}

impl WindowLog {
    /// 一句话写进采集轮：请求、翻了几页、每页 offset 与条数、末页 has_more。
    pub fn describe(&self) -> String {
        let pages = self
            .pages
            .iter()
            .map(|p| format!("{}:{}", p.offset, p.n))
            .collect::<Vec<_>>()
            .join(",");
        let last = self.pages.last();
        format!(
            "请求 GET {}&offset=…，共 {} 页（offset:本页条数 {pages}），末页 has_more={}、接口 total={}",
            self.path,
            self.pages.len(),
            last.is_some_and(|p| p.has_more),
            last.map_or(0, |p| p.total)
        )
    }
}

#[derive(Debug, Deserialize)]
struct WindowPage {
    #[serde(default)]
    items: Vec<RawPost>,
    #[serde(default)]
    total: i64,
    #[serde(default)]
    has_more: bool,
}

/// `/accounts` 的分页与别处不同：`total` 是**页数**，条数在 `records`。
#[derive(Debug, Deserialize)]
struct AccountPage {
    #[serde(default)]
    items: Vec<RawAccount>,
    /// 页数
    #[serde(default)]
    total: i64,
    /// 条数
    #[serde(default)]
    records: i64,
}

#[derive(Debug, Deserialize)]
pub struct RawAccount {
    #[serde(default, rename = "accountName")]
    pub account_name: String,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "countryName")]
    pub country_name: String,
    #[serde(default, rename = "typeName")]
    pub type_name: String,
}

/// 贴文原始字段。**驼峰**，且一律给默认值——线上版本落后时缺字段是常态。
#[derive(Default, Debug, Clone, Deserialize)]
pub struct RawPost {
    #[serde(default, rename = "postId")]
    pub post_id: String,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "translatedText")]
    pub translated_text: String,
    #[serde(default)]
    pub likes: Option<serde_json::Value>,
    #[serde(default, rename = "numComments")]
    pub num_comments: Option<serde_json::Value>,
    #[serde(default)]
    pub followers: Option<serde_json::Value>,
    #[serde(default, rename = "datePosted")]
    pub date_posted: String,
    /// 真入库时间（后端取自 `p.created_at`）。窗口按它收口。
    #[serde(default, rename = "ingestedAt")]
    pub ingested_at: String,
    #[serde(default, rename = "contentType")]
    pub content_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "mediaList")]
    pub media_list: Vec<RawMedia>,
    /// 标签补丁未部署时整个字段都不在
    #[serde(default, rename = "tagList")]
    pub tag_list: Vec<RawTag>,
    #[serde(default, rename = "hashtagList")]
    pub hashtag_list: Vec<RawHashtag>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawMedia {
    #[serde(default, rename = "mediaHash")]
    pub media_hash: String,
    #[serde(default, rename = "mediaType")]
    pub media_type: String,
    #[serde(default, rename = "mediaUrl")]
    pub media_url: String,
    /// 线上还没有，部署标签补丁后才会有
    #[serde(default, rename = "shortCaption")]
    pub short_caption: String,
    #[serde(default, rename = "detailedAlt")]
    pub detailed_alt: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTag {
    #[serde(default, rename = "tagName")]
    pub tag_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawHashtag {
    #[serde(default, rename = "hashtagName")]
    pub hashtag_name: String,
}

impl CswClient {
    pub fn new(cfg: CswConfig) -> Result<Self> {
        csw_collector_core::ensure_crypto_provider();
        anyhow::ensure!(!cfg.api_key.is_empty(), "缺 CSW_API_KEY");
        let http = reqwest::Client::builder().timeout(cfg.timeout).build()?;
        Ok(Self {
            cfg,
            http,
            rec: None,
        })
    }

    /// 挂上录制回放层。
    ///
    /// 贴文库是第四个出网客户端，之前只有模型、Jev、向量三个挂了它——
    /// 于是 `replay` 说的「这一轮不出网」其实做不到：采集那一段照样在调真接口。
    pub fn with_recorder(
        mut self,
        rec: std::sync::Arc<csw_collector_core::record::Recorder>,
    ) -> Self {
        self.rec = Some(rec);
        self
    }

    /// 窗口取数。
    ///
    /// `start` / `end` 是**发布时间**窗口（接口只支持这个），调用方应当传一个
    /// 比目标窗口宽 [`WINDOW_LOOKBACK_DAYS`] 天的范围，再用 [`in_ingest_window`] 本地收口。
    ///
    /// 返回的是**全部**页，分页由这里负责：调用方不该关心 `has_more`。
    pub async fn window(&self, start: &str, end: &str) -> Result<Vec<RawPost>> {
        Ok(self.window_logged(start, end).await?.0)
    }

    /// 同 [`window`](Self::window)，另外返回实际发出的每一页请求（offset、本页条数、has_more）。
    /// 主编要核「补采那一趟到底怎么请求、翻到哪页结束」（10-09 r67 v3/v4 退回），只留概要数核不了。
    pub async fn window_logged(&self, start: &str, end: &str) -> Result<(Vec<RawPost>, WindowLog)> {
        let mut out = Vec::new();
        let mut log = WindowLog {
            path: format!(
                "/api/v1/posts/window?start={start}&end={end}&limit={}",
                self.cfg.page_size
            ),
            pages: Vec::new(),
        };
        let mut offset = 0u32;
        loop {
            let page: WindowPage = self
                .get(&format!(
                    "/api/v1/posts/window?start={start}&end={end}&limit={}&offset={offset}",
                    self.cfg.page_size
                ))
                .await
                .with_context(|| format!("取窗口 {start}~{end} offset={offset}"))?;
            let n = page.items.len();
            log.pages.push(PageLog {
                offset,
                n,
                has_more: page.has_more,
                total: page.total,
            });
            out.extend(page.items);
            // 同时看 has_more 与本页条数：接口某天不返回 has_more 也不会死循环
            if !page.has_more || n == 0 {
                break;
            }
            offset += self.cfg.page_size;
            // 窗口再大也不该翻过这么多页；翻到这儿说明参数错了，早点炸比默默取一天强
            if offset > 20_000 {
                bail!("窗口分页超过 20000 条，疑似参数有误：{start}~{end}");
            }
        }
        Ok((out, log))
    }

    /// 单条。注意查询参数是 `include-media`（**连字符**，不是下划线）。
    ///
    /// 给数字 id 或链接里的短码都行。**csw 只认数字 id**——拿短码去查一律 404
    /// （`Post not found`），这是线上跑 M2 时才发现的：本地候选存的是数字 id，
    /// 所以正式一轮一直没踩到；从链接切出来的短码（Van 补的链接、05/11 的回退）全踩。
    pub async fn post(&self, id_or_code: &str) -> Result<RawPost> {
        let id = media_id(id_or_code)
            .with_context(|| format!("认不出贴文编号 {id_or_code}：既不是数字 id，也不是短码"))?;
        self.get(&format!("/api/v1/posts/{id}?include-media=true"))
            .await
    }

    /// 在册账号。品牌别名表的主要来源。
    pub async fn accounts(&self) -> Result<Vec<RawAccount>> {
        let mut out = Vec::new();
        let mut page = 1u32;
        loop {
            let p: AccountPage = self
                .get(&format!("/api/v1/accounts?page={page}&size=100"))
                .await?;
            let pages = p.total.max(1);
            out.extend(p.items);
            if page as i64 >= pages {
                // records 才是条数；对不上就说明分页语义又变了，值得吵一句
                if p.records > 0 && out.len() as i64 != p.records {
                    tracing::warn!(
                        取到 = out.len(),
                        应有 = p.records,
                        "账号条数与 records 对不上"
                    );
                }
                break;
            }
            page += 1;
        }
        Ok(out)
    }

    /// 已生成过文章的贴文。知识库第三类对照材料。
    pub async fn generated(&self, max_pages: u32) -> Result<Vec<RawPost>> {
        let mut out = Vec::new();
        for page in 1..=max_pages {
            let p: WindowPage = self
                .get(&format!("/api/v1/posts/generated?page={page}&size=100"))
                .await?;
            let n = p.items.len();
            out.extend(p.items);
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// 一次 GET。**全部四个取数方法都从这儿出网**，所以回放层只要包住它。
    ///
    /// 夹具键只认 `path`：`base_url` 与密钥换了不该让回放失效，
    /// 那两样跟「这次请求要什么数据」无关。
    ///
    /// 录的是**解包信封之后的 `data`**，不是原始响应。回放是为了不出网跑回归，
    /// 不是为了测信封解析（那有自己的单测），录 data 夹具也小得多、读得懂。
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let v = match self.rec.clone() {
            Some(rec) => {
                let req = serde_json::json!({"path": path});
                rec.wrap("csw", &req, || self.get_raw(path)).await?
            }
            None => self.get_raw(path).await?,
        };
        serde_json::from_value(v).with_context(|| format!("解析 {path} 的 data"))
    }

    async fn get_raw(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        // 冷启动首个请求可达 40 秒，重试要留够耐心
        let mut attempt = 0;
        loop {
            attempt += 1;
            let r = self
                .http
                .get(&url)
                .bearer_auth(&self.cfg.api_key)
                .send()
                .await;
            match r {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        let env: Envelope<serde_json::Value> = serde_json::from_str(&text)
                            .with_context(|| {
                                format!("解析 {path}；原文前 200 字：{}", head(&text))
                            })?;
                        if let Some(d) = env.data {
                            return Ok(d);
                        }
                        bail!(
                            "{path} 返回 success={} 但没有 data：{}",
                            env.success,
                            env.message
                        );
                    }
                    if !(status.as_u16() == 429 || status.is_server_error()) || attempt >= 4 {
                        bail!("{path} 返回 {status}：{}", head(&text));
                    }
                }
                Err(e) if attempt >= 4 => return Err(e).context(format!("请求 {path}")),
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(2u64.pow(attempt.min(4)))).await;
        }
    }
}

fn head(s: &str) -> String {
    s.chars().take(200).collect()
}

/// 数字字段有时是字符串（`"2324"`），有时是数字。两种都收。
fn as_i64(v: &Option<serde_json::Value>) -> Option<i64> {
    match v.as_ref()? {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_ts(s: &str) -> Option<Timestamp> {
    if s.trim().is_empty() {
        return None;
    }
    s.parse::<Timestamp>().ok()
}

/// 这条贴文的入库时间落在目标窗口里吗。
///
/// 窗口是左闭右开：`[from, to)`。边界上的一条被两期都算或都不算，都是错。
/// 贴文编号 → csw 认的数字 id。
///
/// Instagram 的短码就是媒体 id 的 base64（字母表 `A-Z a-z 0-9 - _`），逐位解码即得，
/// 与 csw 的 `postId` 逐条对过。已经是数字就原样返回。
/// 超过 11 位的是私密分享码，解不出公开 id，返回 None 而不是猜。
pub fn media_id(s: &str) -> Option<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let s = s.trim();
    if s.is_empty() || s.len() > 20 {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return Some(s.to_string());
    }
    if s.len() > 11 {
        return None;
    }
    let mut n: u128 = 0;
    for b in s.bytes() {
        let v = ALPHABET.iter().position(|&a| a == b)?;
        n = n * 64 + v as u128;
    }
    Some(n.to_string())
}

pub fn in_ingest_window(p: &RawPost, from: Timestamp, to: Timestamp) -> bool {
    match parse_ts(&p.ingested_at) {
        Some(t) => t >= from && t < to,
        // 取不到入库时间就退回发布时间：宁可多判一条，也不要漏
        None => parse_ts(&p.date_posted)
            .map(|t| t >= from && t < to)
            .unwrap_or(false),
    }
}

/// 归一成统一候选格式。到这一层之后，后面的步骤不必再关心它从哪个采集器来。
pub fn to_candidate(p: &RawPost, collector: &str) -> Candidate {
    let media = p
        .media_list
        .iter()
        .enumerate()
        .map(|(i, m)| MediaRef {
            source_hash: m.media_hash.clone(),
            kind: if m.media_type.eq_ignore_ascii_case("Photo") {
                MediaKind::Photo
            } else {
                MediaKind::Video
            },
            url: m.media_url.clone(),
            blake3: None,
            ordinal: i as u16,
        })
        .collect();
    Candidate {
        candidate_key: candidate_key(&p.account, &p.url, &p.post_id),
        platform: Platform::Instagram,
        source_id: p.post_id.clone(),
        collector: collector.to_string(),
        account: p.account.clone(),
        url: p.url.clone(),
        text: p.description.clone(),
        translated: p.translated_text.clone(),
        posted_at: parse_ts(&p.date_posted),
        ingested_at: parse_ts(&p.ingested_at),
        likes: as_i64(&p.likes),
        comments: as_i64(&p.num_comments),
        followers: as_i64(&p.followers),
        heat_ratio: None,
        content_type: p.content_type.clone(),
        media,
        tags: p
            .tag_list
            .iter()
            .map(|t| t.tag_name.clone())
            .filter(|s| !s.is_empty())
            .collect(),
        hashtags: p
            .hashtag_list
            .iter()
            .map(|h| h.hashtag_name.clone())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

/// 条目键 = 品牌小写 + `-` + 链接 sha256 前 6 位。与交付物里的条目键同一个，生成后不再改。
///
/// 链接为空时退回用 postId——键必须稳定，宁可难看也不能今天一个样明天一个样。
pub fn candidate_key(account: &str, url: &str, post_id: &str) -> String {
    use sha2::Digest;
    let brand: String = account
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let brand = if brand.is_empty() {
        "src".to_string()
    } else {
        brand
    };
    let basis = if url.trim().is_empty() { post_id } else { url };
    // sha2 0.11 的输出不再实现 LowerHex，自己转
    let hex: String = sha2::Sha256::digest(basis.as_bytes())
        .iter()
        .take(3)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{brand}-{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(content_type: &str, kinds: &[&str]) -> RawPost {
        RawPost {
            post_id: "3988056105053822940".into(),
            account: "snowpeak_official".into(),
            description: "正文".into(),
            translated_text: String::new(),
            likes: Some(serde_json::json!("2324")),
            num_comments: Some(serde_json::json!(5)),
            followers: None,
            date_posted: "2026-09-17T05:25:17Z".into(),
            ingested_at: "2026-09-18T01:10:00Z".into(),
            content_type: content_type.into(),
            url: "https://www.instagram.com/p/DbKe9gIkoMb/".into(),
            media_list: kinds
                .iter()
                .map(|k| RawMedia {
                    media_hash: "h".into(),
                    media_type: (*k).into(),
                    media_url: "https://oss/x.jpg".into(),
                    short_caption: String::new(),
                    detailed_alt: String::new(),
                })
                .collect(),
            tag_list: vec![],
            hashtag_list: vec![],
        }
    }

    #[test]
    fn 数字字段字符串与数字都收() {
        let p = raw("Image", &["Photo"]);
        let c = to_candidate(&p, "csw_window");
        assert_eq!(c.likes, Some(2324), "点赞是字符串形态也要收");
        assert_eq!(c.comments, Some(5));
        assert_eq!(c.followers, None);
    }

    #[test]
    fn 只取图文的判据对齐核心类型() {
        assert!(to_candidate(&raw("Image", &["Photo"]), "x").is_image_only());
        assert!(to_candidate(&raw("Carousel", &["Photo", "Photo"]), "x").is_image_only());
        assert!(!to_candidate(&raw("Carousel", &["Photo", "Video"]), "x").is_image_only());
        assert!(!to_candidate(&raw("Reel", &["Video"]), "x").is_image_only());
        assert!(!to_candidate(&raw("Reel", &[]), "x").is_image_only());
    }

    #[test]
    fn 窗口按入库时间收口而不是发布时间() {
        let p = raw("Image", &["Photo"]); // 9/17 发布、9/18 入库
        let d = |s: &str| s.parse::<Timestamp>().unwrap();
        // 按发布时间那天的窗口：不该算进来
        assert!(!in_ingest_window(
            &p,
            d("2026-09-17T00:00:00Z"),
            d("2026-09-18T00:00:00Z")
        ));
        // 按入库时间那天的窗口：算进来
        assert!(in_ingest_window(
            &p,
            d("2026-09-18T00:00:00Z"),
            d("2026-09-19T00:00:00Z")
        ));
    }

    #[test]
    fn 没有入库时间就退回发布时间不漏掉() {
        let mut p = raw("Image", &["Photo"]);
        p.ingested_at = String::new();
        let d = |s: &str| s.parse::<Timestamp>().unwrap();
        assert!(in_ingest_window(
            &p,
            d("2026-09-17T00:00:00Z"),
            d("2026-09-18T00:00:00Z")
        ));
    }

    #[test]
    fn 窗口左闭右开() {
        let mut p = raw("Image", &["Photo"]);
        p.ingested_at = "2026-09-18T00:00:00Z".into();
        let d = |s: &str| s.parse::<Timestamp>().unwrap();
        assert!(
            in_ingest_window(&p, d("2026-09-18T00:00:00Z"), d("2026-09-19T00:00:00Z")),
            "起点含"
        );
        assert!(
            !in_ingest_window(&p, d("2026-09-17T00:00:00Z"), d("2026-09-18T00:00:00Z")),
            "终点不含"
        );
    }

    #[test]
    fn 条目键稳定且随链接变() {
        let a = candidate_key("snowpeak_official", "https://x/p/1", "1");
        assert_eq!(a, candidate_key("snowpeak_official", "https://x/p/1", "1"));
        assert_ne!(a, candidate_key("snowpeak_official", "https://x/p/2", "2"));
        assert!(a.starts_with("snowpeakofficial-"), "实得 {a}");
        // 链接为空时退回 postId，仍然稳定
        let b = candidate_key("小红书号", "", "abc");
        assert!(b.starts_with("src-"), "非 ASCII 账号名退回 src：{b}");
        assert_eq!(b, candidate_key("小红书号", "", "abc"));
    }

    #[test]
    fn 缺标签字段不影响归一() {
        // 线上版本还没有 tagList / hashtagList，缺了也要能解析
        let json = r#"{"postId":"1","account":"a","description":"d","contentType":"Image",
            "mediaList":[{"mediaType":"Photo","mediaUrl":"u"}]}"#;
        let p: RawPost = serde_json::from_str(json).unwrap();
        let c = to_candidate(&p, "csw_window");
        assert!(c.tags.is_empty() && c.hashtags.is_empty());
        assert_eq!(c.media.len(), 1);
    }
    /// 贴文库在回放下同样一次都不该出网。
    ///
    /// 同样的做法：录一遍、关掉服务器、再回放。连接被拒的话测试就红。
    #[tokio::test]
    async fn 回放下不再去调贴文库() {
        use csw_collector_core::record::{Mode, Recorder};
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let dir = std::env::temp_dir().join(format!("csw-fx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let page = serde_json::json!({
            "success": true,
            "data": {"items": [{"postId": "abc123", "account": "snowpeak_official",
                                "description": "正文", "contentType": "Image"}],
                     "total": 1, "hasMore": false}
        });

        let base = {
            let srv = wiremock::MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/posts/window"))
                .respond_with(ResponseTemplate::new(200).set_body_json(page.clone()))
                .mount(&srv)
                .await;
            let base = srv.uri();
            let c = client(&base).with_recorder(Arc::new(Recorder::new(Mode::Record, &dir)));
            let got = c.window("2026-09-17", "2026-09-18").await.unwrap();
            assert_eq!(got.len(), 1);
            base
            // srv 析构
        };

        let c = client(&base).with_recorder(Arc::new(Recorder::new(Mode::Replay, &dir)));
        let got = c.window("2026-09-17", "2026-09-18").await.unwrap();
        assert_eq!(got.len(), 1, "服务器已关，还能取到才说明真没出网");
        assert_eq!(got[0].post_id, "abc123");

        // 换个窗口就是另一条请求，夹具里没有——必须报错，不能去调真接口
        let e = c.window("2026-09-19", "2026-09-20").await.unwrap_err();
        assert!(format!("{e:#}").contains("回放未命中"), "{e:#}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 窗口取数逐页留下请求记录() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, ResponseTemplate};
        let item =
            |id: &str| serde_json::json!({"postId": id, "account": "a", "contentType": "Image"});
        let srv = wiremock::MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/posts/window"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true, "data": {"items": [item("1"), item("2")], "total": 3, "has_more": true}})))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/posts/window"))
            .and(query_param("offset", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true, "data": {"items": [item("3")], "total": 3, "has_more": false}})))
            .mount(&srv)
            .await;
        let (got, log) = client(&srv.uri())
            .window_logged("2026-10-07", "2026-10-10")
            .await
            .unwrap();
        assert_eq!(got.len(), 3);
        let d = log.describe();
        assert!(
            d.contains("start=2026-10-07&end=2026-10-10&limit=100"),
            "{d}"
        );
        assert!(d.contains("共 2 页（offset:本页条数 0:2,100:1）"), "{d}");
        assert!(d.contains("末页 has_more=false"), "{d}");
    }

    fn client(base: &str) -> CswClient {
        CswClient::new(CswConfig {
            base_url: base.into(),
            api_key: "k".into(),
            page_size: 100,
            timeout: Duration::from_secs(5),
        })
        .unwrap()
    }

    #[test]
    fn 短码解成csw认的数字id() {
        // 三对都是线上本地候选里的真实 postId 与链接
        for (code, id) in [
            ("DdfP3uvkUc-", "3989977595332609854"),
            ("DdfRXZVE8zY", "3989984169409367256"),
            ("DdfRlyulxLr", "3989985158753620715"),
        ] {
            assert_eq!(media_id(code).as_deref(), Some(id), "{code}");
        }
        assert_eq!(
            media_id("3989977595332609854").as_deref(),
            Some("3989977595332609854")
        );
        // 路径穿越、私密分享码、空串一律不认
        assert_eq!(media_id("../admin"), None);
        assert_eq!(media_id("DdfP3uvkUc-abcdefgh"), None);
        assert_eq!(media_id(""), None);
    }
}
