//! 05 原帖图不够图位时，去品牌官网补采同型号的图。
//!
//! 10-19 r82 KingCamp、10-26 r89 鹿牌两次一样：原帖只有一两张图，主编派修订要「同型号官网图」，
//! 工作台只会在原帖里重排，交了同图的新版本，最后靠维护方人工补采。现在：
//!
//! 1. **找页面**：派工单里写的链接 → 01 深核留下的依据链接 → 都没有就让 codex 用
//!    `web_search` / `fetch_page` 找这款产品的官网商品页（只认品牌自己的站或官方商城）。
//! 2. **取图**：页面上的图（含站内相对路径），排掉按钮、横幅、图标，短边够大的才下。
//! 3. **去重**：与原帖同画面的不要（官网常有一张就是原帖那张）。
//! 4. **识别**：与原帖同一套识图提示词，`pick_figures` 只拿它们补原帖之后的空位。
//!
//! 官网图能不能用（转载许可）工作台判不了，每次都写一条缺口交主编与 Van。

use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use csw_collector_core::Config;
use csw_collector_deepcheck::codex::{Codex, CodexConfig, DEFAULT_ENV_PASSTHROUGH, Input};
use csw_collector_deliver::material::{ExternalShot, ItemShots, Shot};
use csw_collector_harvest::recognize;

use super::{hires_match, slots};

/// 包里写的来源标注
pub const ORIGIN: &str = "品牌官网补采";

/// 官网图在 `ItemShots.shots` 里的序号从这里起，与原帖序号分开
const ORDINAL_BASE: u16 = 800;

/// 最多看几个页面
const MAX_PAGES: usize = 3;

/// 补采做了什么，写进包里与失败原因
#[derive(Debug, Default)]
pub struct Report {
    /// 看过的页面
    pub pages: Vec<String>,
    /// 页面从哪来
    pub how: String,
    /// 下载并通过尺寸、去重的张数
    pub kept: usize,
    /// 识别后补进 `item.shots` 的张数
    pub added: usize,
    pub notes: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut s = if self.pages.is_empty() {
            format!("没找到官网商品页（{}）", self.how)
        } else {
            format!(
                "查了 {}（{}），留下 {} 张、补进 {} 张",
                self.pages.join("、"),
                self.how,
                self.kept,
                self.added
            )
        };
        if !self.notes.is_empty() {
            s.push('；');
            s.push_str(&self.notes.join("；"));
        }
        s
    }
}

/// 给 `item` 补官网图。出错不抛：补采是兜底，写进 [`Report`] 交给人看。
pub async fn supplement(
    conn: &Connection,
    cfg: &Config,
    svc: &super::services::Services,
    item: &mut ItemShots,
    post_text: &str,
    dispatch_note: &str,
) -> Report {
    let mut rep = Report::default();
    let (mut pages, how) = known_pages(conn, &item.item_key, dispatch_note);
    rep.how = how;
    let mut hinted_images = Vec::new();
    if pages.is_empty() {
        match search(cfg, item, post_text).await {
            Ok(f) => {
                rep.how = format!("codex 搜索：{}", f.searched.trim());
                pages = f.pages.into_iter().filter(|u| is_page(u)).collect();
                hinted_images = f.image_urls;
            }
            Err(e) => {
                rep.how = "codex 搜索失败".into();
                rep.notes.push(format!("{e:#}"));
            }
        }
    }
    pages.truncate(MAX_PAGES);
    rep.pages = pages.clone();
    if pages.is_empty() {
        return rep;
    }

    let http = match csw_collector_harvest::netguard::guarded_client(
        std::time::Duration::from_secs(60),
        false,
    ) {
        Ok(h) => h,
        Err(e) => {
            rep.notes.push(format!("建 HTTP 客户端失败：{e:#}"));
            return rep;
        }
    };

    // 原帖每张图的指纹：官网常有一张就是原帖那张，不能当第二张
    let post_prints: Vec<(Option<hires_match::Prints>, Option<u64>)> = item
        .shots
        .iter()
        .map(|s| {
            (
                hires_match::prints(&s.bytes).ok(),
                hires_match::lo_print(&s.bytes).ok(),
            )
        })
        .collect();

    let mut kept: Vec<Shot> = Vec::new();
    let mut kept_prints: Vec<(hires_match::Prints, u64)> = Vec::new();
    let mut seen_sha = HashSet::new();
    for page in &pages {
        let urls = match page_image_urls(&http, page).await {
            Ok(u) => u,
            Err(e) => {
                rep.notes.push(format!("{page}：{e:#}"));
                continue;
            }
        };
        let mut urls: Vec<String> = urls.into_iter().filter(|u| !is_junk(u)).collect();
        // codex 给的图地址只认与页面同一站点的
        for u in &hinted_images {
            if same_site(page, u) && !is_junk(u) && !urls.contains(u) {
                urls.push(u.clone());
            }
        }
        for u in urls.into_iter().take(cfg.official.max_images) {
            // 官网图床偶尔掐连接（10-26 实测 captainstag.net 8 张断 2 张），再试一次
            let b = match slots::fetch(&http, &u, 20 << 20).await {
                Ok(b) => b,
                Err(_) => match slots::fetch(&http, &u, 20 << 20).await {
                    Ok(b) => b,
                    Err(e) => {
                        rep.notes.push(format!("{u} 下载失败：{e:#}"));
                        continue;
                    }
                },
            };
            let Some((w, h)) = hires_match::dimensions(&b) else {
                continue;
            };
            if w.min(h) < cfg.official.min_short_side {
                continue;
            }
            let sha = slots::sha256_hex(&b);
            if !seen_sha.insert(sha.clone()) {
                continue;
            }
            let (Ok(p), Ok(lo)) = (hires_match::prints(&b), hires_match::lo_print(&b)) else {
                continue;
            };
            let dup_post = post_prints.iter().any(|(pp, plo)| {
                pp.is_some_and(|pp| hires_match::compare(&pp, lo).ok())
                    || plo.is_some_and(|plo| hires_match::compare(&p, plo).ok())
            });
            let dup_kept = kept_prints.iter().any(|(kp, klo)| {
                hires_match::compare(kp, lo).ok() || hires_match::compare(&p, *klo).ok()
            });
            if dup_post {
                rep.notes.push(format!("{u} 与原帖同画面，不重复用"));
                continue;
            }
            if dup_kept {
                continue;
            }
            kept_prints.push((p, lo));
            let ordinal = ORDINAL_BASE + kept.len() as u16;
            kept.push(Shot {
                blake3: blake3::hash(&b).to_hex().to_string(),
                ordinal,
                url: u.clone(),
                ext: super::item::ext_of(&u),
                desc: None,
                hires: None,
                external: Some(ExternalShot {
                    page_url: page.clone(),
                    original_url: u.clone(),
                    width: w,
                    height: h,
                    bytes: b.len() as u64,
                    sha256: sha,
                    verified: true,
                    note: "官网原文件原样入包，未裁切、未缩放、未重编码".into(),
                    origin: ORIGIN.into(),
                    ..Default::default()
                }),
                bytes: b,
            });
        }
    }
    rep.kept = kept.len();
    if kept.is_empty() {
        return rep;
    }

    // 与原帖同一套识图：描述、对应正文、能不能当配图都按同一把尺子
    let refs: Vec<recognize::ImageRef> = kept
        .iter()
        .map(|s| recognize::ImageRef {
            blake3: s.blake3.clone(),
            b64: recognize::b64(&s.bytes),
            ordinal: s.ordinal,
        })
        .collect();
    let mut descs = Vec::new();
    for batch in refs.chunks(recognize::MAX_IMAGES_PER_CALL) {
        match recognize::recognize_batch(&svc.model, post_text, batch).await {
            Ok(mut d) => descs.append(&mut d),
            Err(e) => rep.notes.push(format!("{} 张官网图识别失败：{e:#}", batch.len())),
        }
    }
    for mut s in kept {
        s.desc = descs.iter().find(|d| d.blake3 == s.blake3).cloned();
        if s.desc.is_some() {
            rep.added += 1;
            item.shots.push(s);
        }
    }
    if rep.added > 0 {
        item.gaps.push(format!(
            "官网补采的图（{}）是品牌商品图，页面未必写明转载许可：能否用于本期、要不要另取许可，由主编与 Van 定",
            rep.pages.join("、")
        ));
    }
    tracing::info!(条目 = %item.item_key, 补采 = %rep.summary(), "官网补采");
    rep
}

/// 不用搜就知道的页面：派工单里写的、01 深核依据里的。
fn known_pages(conn: &Connection, item_key: &str, note: &str) -> (Vec<String>, String) {
    let from_note: Vec<String> = urls_in(note).into_iter().filter(|u| is_page(u)).collect();
    if !from_note.is_empty() {
        return (from_note, "派工单写的链接".into());
    }
    let from_deep: Vec<String> = deepcheck_evidence(conn, item_key)
        .into_iter()
        .flat_map(|e| urls_in(&e))
        .filter(|u| is_page(u))
        .collect();
    if !from_deep.is_empty() {
        let mut seen = HashSet::new();
        let v = from_deep.into_iter().filter(|u| seen.insert(u.clone())).collect();
        return (v, "01 深核依据".into());
    }
    (Vec::new(), String::new())
}

fn deepcheck_evidence(conn: &Connection, item_key: &str) -> Vec<String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT result_json FROM deepchecks WHERE candidate_key = ?1 AND status = 'completed'
             ORDER BY id DESC LIMIT 1",
            params![item_key],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    raw.and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .and_then(|v| v.pointer("/card/evidence").cloned())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default()
}

/// 文字里的 http(s) 链接，去掉尾巴上的标点
pub fn urls_in(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"https?://[^\s"'<>（）()、，。；]+"#).expect("静态正则");
    re.find_iter(text)
        .map(|m| {
            m.as_str()
                .trim_end_matches(['.', ',', ';', ':', ')', ']', '」', '】'])
                .to_string()
        })
        .collect()
}

/// 能当官网页面看的链接：排掉社交平台、我们自己的存储与服务
pub fn is_page(u: &str) -> bool {
    let Ok(url) = url::Url::parse(u) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    const SKIP: [&str; 14] = [
        "instagram.com",
        "cdninstagram.com",
        "facebook.com",
        "fbcdn.net",
        "x.com",
        "twitter.com",
        "threads.net",
        "youtube.com",
        "youtu.be",
        "tiktok.com",
        "xiaohongshu.com",
        "aliyuncs.com",
        "aworld.ltd",
        "campsomewhere.com",
    ];
    !host.is_empty() && !SKIP.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// 按地址就能排掉的：按钮、横幅、图标、logo、社交分享图。按文件名分词比，
/// 免得 `silicone_cup.jpg` 这种因为含 icon 被误排
pub fn is_junk(u: &str) -> bool {
    let lower = u.split('?').next().unwrap_or(u).to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or("");
    const WORDS: [&str; 10] = [
        "btn", "bnr", "banner", "logo", "icon", "ogp", "favicon", "sprite", "badge", "arrow",
    ];
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| WORDS.iter().any(|w| t == *w || (t.starts_with(w) && t.len() <= w.len() + 2)))
        || lower.contains("/share/images/")
        || lower.contains("/item_img/s/")
        || name.ends_with(".svg")
        || name.ends_with(".gif")
}

fn same_site(page: &str, img: &str) -> bool {
    let site = |u: &str| {
        url::Url::parse(u).ok().and_then(|x| x.host_str().map(|h| {
            let parts: Vec<&str> = h.split('.').collect();
            parts[parts.len().saturating_sub(2)..].join(".")
        }))
    };
    match (site(page), site(img)) {
        (Some(a), Some(b)) => a == b || b.ends_with("shopify.com") || b.ends_with("shopifycdn.net"),
        _ => false,
    }
}

async fn page_image_urls(http: &reqwest::Client, page: &str) -> Result<Vec<String>> {
    let html = slots::fetch(http, page, 5 << 20).await?;
    let html = String::from_utf8_lossy(&html).into_owned();
    // Shopify 商品页另有商品 JSON，图库比页面上的全
    let pj = if page.contains("/products/") && !page.ends_with(".html") {
        let ju = format!(
            "{}.json",
            page.split('?').next().unwrap_or(page).trim_end_matches('/')
        );
        slots::fetch(http, &ju, 5 << 20)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    } else {
        None
    };
    Ok(slots::page_images(page, &html, pj.as_ref()))
}

#[derive(Debug, Default, Deserialize)]
struct Found {
    #[serde(default)]
    pages: Vec<String>,
    #[serde(default)]
    image_urls: Vec<String>,
    #[serde(default)]
    searched: String,
}

fn found_schema() -> Value {
    let strs = || json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "product": {"type": "string"},
            "pages": strs(),
            "image_urls": strs(),
            "searched": {"type": "string"}
        },
        "required": ["product", "pages", "image_urls", "searched"],
        "additionalProperties": false
    })
}

/// 让 codex 找这款产品的官网商品页。
async fn search(cfg: &Config, item: &ItemShots, post_text: &str) -> Result<Found> {
    let codex = Codex::start(CodexConfig {
        bin: cfg.codex.bin.clone(),
        home: cfg.codex.home.clone(),
        cwd: cfg.blob_dir(),
        model: cfg.codex.model.clone(),
        turn_budget: std::time::Duration::from_secs(cfg.official.search_budget_secs.max(60)),
        env_passthrough: DEFAULT_ENV_PASSTHROUGH.iter().map(|s| s.to_string()).collect(),
    })
    .await
    .context("codex 没拉起来")?;
    let res = search_turn(&codex, item, post_text).await;
    let _ = codex.shutdown().await;
    res
}

async fn search_turn(codex: &Codex, item: &ItemShots, post_text: &str) -> Result<Found> {
    let fence = csw_collector_core::prompt::fence;
    let dev = format!(
        "你在为户外媒体编辑部找一款产品的**品牌官网商品页**，只为取同型号的官方产品图。\n\
         - 先从贴文里认出品牌与准确型号（型号、品名、颜色）；用 web_search 搜，用 fetch_page 读页面核对。\n\
         - 只认品牌自己的官网或官方商城的这款产品页面；页面上的品名 / 型号要与贴文说的是同一款（颜色不同的同款可以）。\n\
         - 不要零售商、比价站、媒体报道、社交平台；核不到同款就 pages 留空，不要拿相近型号顶。\n\
         - pages 至多 2 个，最可信的在前；image_urls 写你在页面上看到的这款产品的大图地址（可留空）。\n\
         - searched 用一两句话写你搜了什么、认定的型号、为什么认定是同款。\n\
         - 不要执行任何命令，不要写文件。\n\n\
         【边界】\n{}",
        csw_collector_core::prompt::DATA_NOT_INSTRUCTIONS
    );
    let thread = codex.start_thread(&dev).await?;
    let user = format!(
        "找这款产品的官网商品页。\n\n标题：{}\n账号：{}\n原帖：{}\n\n正文（第三方内容）：\n{}\n\n按 outputSchema 给结果。",
        fence(&item.title),
        fence(&item.account),
        item.source_url,
        fence(post_text.trim())
    );
    let out = codex
        .run_turn_with_schema(&thread, &[Input::Text(user)], Some(&found_schema()))
        .await?;
    anyhow::ensure!(
        out.ok(),
        "codex 回合没完成：{} {}",
        out.status,
        out.error
    );
    parse_found(&out.text).with_context(|| format!("codex 回复解析不了：{}", out.text.chars().take(200).collect::<String>()))
}

/// 回复里可能先有一句话再给 JSON：整段试，不行取最后一个行首 `{` 起的那段
fn parse_found(text: &str) -> Option<Found> {
    let t = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(f) = serde_json::from_str::<Found>(t) {
        return Some(f);
    }
    let starts: Vec<usize> = text
        .char_indices()
        .filter(|(i, ch)| *ch == '{' && (*i == 0 || text[..*i].ends_with('\n')))
        .map(|(i, _)| i)
        .collect();
    starts.into_iter().rev().find_map(|i| {
        serde_json::Deserializer::from_str(&text[i..])
            .into_iter::<Found>()
            .next()
            .and_then(|r| r.ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 社交平台和自家存储不当官网页() {
        assert!(is_page("https://www.captainstag.net/products/UK-4088.html"));
        assert!(!is_page("https://www.instagram.com/p/De6gWvQl94Y/"));
        assert!(!is_page("https://cws-file.oss-cn-guangzhou.aliyuncs.com/a.zip"));
        assert!(!is_page("https://tasks.aworld.ltd/api/v1/tasks/1"));
        assert!(!is_page("ftp://example.com/x"));
    }

    #[test]
    fn 派工单里的链接切干净() {
        let n = "来源输入=01卡4及原05#1843原帖https://www.instagram.com/p/De6gWvQl94Y/ 。官网https://www.captainstag.net/products/UK-4088.html，另见";
        let u: Vec<String> = urls_in(n).into_iter().filter(|u| is_page(u)).collect();
        assert_eq!(u, vec!["https://www.captainstag.net/products/UK-4088.html"]);
    }

    #[test]
    fn 按钮横幅小图排掉() {
        assert!(is_junk("https://www.captainstag.net/share/images/btn_cart.png"));
        assert!(is_junk("https://www.captainstag.net/page_images/l_bnr_store.png"));
        assert!(is_junk("https://www.captainstag.net/item_img/s/UK-4088_2.jpg"));
        assert!(is_junk("https://x.jp/assets/logo_w.png"));
        assert!(!is_junk("https://www.captainstag.net/item_img/l/UK-4088_2.jpg"));
        assert!(!is_junk("https://cdn.shopify.com/s/files/1/0/products/KA2628_1.jpg"));
        assert!(!is_junk("https://x.jp/img/silicone_cup_main.jpg"));
    }

    #[test]
    fn codex回复先说话再给json() {
        let t = "我会先搜一下品牌官网。\n{\"product\":\"UK-4088\",\"pages\":[\"https://www.captainstag.net/products/UK-4088.html\"],\"image_urls\":[],\"searched\":\"搜 CAPTAIN STAG ツインライト\"}";
        let f = parse_found(t).unwrap();
        assert_eq!(f.pages.len(), 1);
    }

    #[test]
    fn 同站才收codex给的图() {
        assert!(same_site(
            "https://www.captainstag.net/products/UK-4088.html",
            "https://img.captainstag.net/a.jpg"
        ));
        assert!(!same_site(
            "https://www.captainstag.net/products/UK-4088.html",
            "https://www.amazon.co.jp/a.jpg"
        ));
    }
}

#[cfg(test)]
mod live {
    use super::*;
    #[tokio::test]
    #[ignore]
    async fn 鹿牌官网实取() {
        let http = csw_collector_harvest::netguard::guarded_client(std::time::Duration::from_secs(60), true).unwrap();
        let page = "https://www.captainstag.net/products/UK-4088.html";
        let urls: Vec<String> = page_image_urls(&http, page).await.unwrap().into_iter().filter(|u| !is_junk(u)).collect();
        let post = slots::fetch(&http, "https://cws-file.oss-cn-guangzhou.aliyuncs.com/upload/2026/10/26/FnBqE7ma.jpg", 20 << 20).await.unwrap();
        let (pp, plo) = (hires_match::prints(&post).unwrap(), hires_match::lo_print(&post).unwrap());
        for u in urls.iter().take(12) {
            let b = match slots::fetch(&http, u, 20 << 20).await { Ok(b) => b, Err(e) => { println!("{u} 下载失败 {e:#}"); continue; } };
            let d = hires_match::dimensions(&b);
            let (p, lo) = (hires_match::prints(&b).unwrap(), hires_match::lo_print(&b).unwrap());
            let dup = hires_match::compare(&pp, lo).ok() || hires_match::compare(&p, plo).ok();
            println!("{u} {d:?} 与原帖同画面={dup}");
        }
    }
}
