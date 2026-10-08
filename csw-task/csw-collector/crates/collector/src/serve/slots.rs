//! 05 按已通过的 04 图位说明配图。
//!
//! 为什么（10-07 r65 #751）：Van 批准写的是 PA'LANTE V2，04 的「图位说明」写明图位 2 用 V2 店页肩带图；
//! 05 却只会在原帖（Desert/Joey 补货帖）13 张里按识图自选，三次都挑了 Desert 与 joey 肩带图，三次被退回。
//! 04 与 05 都只依赖 03、并行跑，所以 05 的上游里没有 04：这里自己去引擎找同条目已通过的 04，
//! 按它的图位说明取图。04 还没过就照旧自选（返工时 04 多半已过，会改按 04）。
//!
//! 图的来历：
//! - 04 引的是原帖的图：按画面在我们的高清图里找同一张（不按文件名猜序号）；
//! - 原帖里没有（店页产品图等）：取 04 附件原字节，再回 04 依据的页面找同画面的最大原图，
//!   核到就用页面原图并记原地址、尺寸、字节、SHA；核不到如实写「页面最高原图未核到」。

use std::collections::{BTreeMap, HashSet};
use std::io::Read;

use anyhow::{Context, Result};
use csw_collector_engineapi::client::EngineClient;
use sha2::{Digest, Sha256};

use super::hires_match;

/// 04 图位说明里的一张图。
#[derive(Debug, Clone, Default)]
pub struct PlannedSlot {
    /// 图位序号（1 起）；连续图组 / 备选为 0
    pub n: usize,
    /// 04 写的这个图位的用途
    pub purpose: String,
    /// 04 附件里的相对路径（`images/xxx.jpg`）
    pub file: String,
    pub bytes: Vec<u8>,
    /// 04 manifest：label / supports / cannot_prove / upstream_path
    pub label: String,
    pub supports: String,
    pub cannot: String,
    pub upstream_path: String,
}

/// 同条目已通过的 04。
#[derive(Debug, Clone, Default)]
pub struct WritePlan {
    pub write_task_id: i64,
    pub deliverable_id: i64,
    pub version: i64,
    /// 主图位，按 04 的序号
    pub slots: Vec<PlannedSlot>,
    /// 04 依据的页面（店页、官网），核原图用
    pub evidence_urls: Vec<String>,
}

/// 从 04 正文里切出「图位说明」：编号行是用途，紧跟的 `![](images/…)` 是图。
/// 小标题（### 同图位连续图组参考 等）之后的图不算主图位。
pub fn parse_slots(md: &str) -> Vec<(usize, String, String)> {
    let num = regex::Regex::new(r"^\s*(\d+)[\.、．]\s*(.+)$").expect("静态正则");
    let img = regex::Regex::new(r"!\[[^\]]*\]\(([^)\s]+)\)").expect("静态正则");
    let mut out = Vec::new();
    let mut inside = false;
    let mut cur: Option<(usize, String)> = None;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            inside = t.contains("图位");
            cur = None;
            continue;
        }
        if !inside {
            continue;
        }
        if t.starts_with("### ") {
            cur = None; // 连续图组、备选：不进主图位
            continue;
        }
        if let Some(c) = num.captures(t) {
            cur = Some((c[1].parse().unwrap_or(0), c[2].trim().to_string()));
        }
        if let (Some((n, purpose)), Some(c)) = (cur.as_ref(), img.captures(t))
            && !out.iter().any(|(m, _, _): &(usize, String, String)| m == n)
        {
            out.push((*n, purpose.clone(), c[1].to_string()));
        }
    }
    out
}

/// 去掉 zip 里唯一的顶层目录（有的包带、有的不带）。
fn unzip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("04 包不是 zip")?;
    let mut raw = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        raw.insert(name, buf);
    }
    let prefix = if raw.contains_key("index.md") {
        String::new()
    } else {
        raw.keys()
            .find(|k| k.ends_with("/index.md") && k.matches('/').count() == 1)
            .map(|k| k.trim_end_matches("index.md").to_string())
            .unwrap_or_default()
    };
    Ok(raw
        .into_iter()
        .filter_map(|(k, v)| k.strip_prefix(&prefix).map(|r| (r.to_string(), v)))
        .collect())
}

fn evidence_urls(files: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let re = regex::Regex::new(r#"https?://[^\s"'<>）)\]]+"#).expect("静态正则");
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    // 先认明确写了「原文链接」与 evidence_url 的，再补 draft 里的
    for (k, v) in files {
        if !k.starts_with("source/") {
            continue;
        }
        let text = String::from_utf8_lossy(v);
        for line in text.lines() {
            if !(line.contains("原文链接")
                || line.contains("evidence_url")
                || k.ends_with("draft.json"))
            {
                continue;
            }
            for m in re.find_iter(line) {
                let u = m.as_str().trim_end_matches([',', '.', '"']).to_string();
                if u.contains("instagram.com") || u.contains("cdninstagram") {
                    continue;
                }
                if seen.insert(u.clone()) {
                    out.push(u);
                }
            }
        }
    }
    out.truncate(4);
    out
}

/// 找同条目已通过的 04，下载它的包，切出图位。没有就是 `None`。
pub async fn approved_write_plan(
    engine: &EngineClient,
    http: &reqwest::Client,
    run_id: i64,
    item_key: &str,
) -> Result<Option<WritePlan>> {
    let run = engine.run_detail(run_id).await.context("取期次详情")?;
    let Some(task_id) = run
        .get("tasks")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .find(|t| {
            t.get("stage_code").and_then(|x| x.as_str()) == Some("write")
                && t.get("item_key").and_then(|x| x.as_str()) == Some(item_key)
        })
        .and_then(|t| t.get("id").and_then(|x| x.as_i64()))
    else {
        return Ok(None);
    };
    let detail = engine
        .task_detail(task_id)
        .await
        .context("取 04 任务详情")?;
    let Some(d) = detail
        .deliverables
        .iter()
        .filter(|d| d.get("status").and_then(|x| x.as_str()) == Some("passed"))
        .max_by_key(|d| d.get("version").and_then(|x| x.as_i64()).unwrap_or(0))
    else {
        return Ok(None);
    };
    let url = d
        .get("download_url")
        .and_then(|x| x.as_str())
        .context("04 交付物没有下载地址")?;
    let bytes = fetch(http, url, 64 << 20).await.context("下载 04 包")?;
    let files = unzip(&bytes)?;
    let md = files
        .get("index.md")
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .context("04 包里没有 index.md")?;
    let manifest: Vec<serde_json::Value> = files
        .get("source/image-manifest.json")
        .and_then(|b| serde_json::from_slice(b).ok())
        .unwrap_or_default();
    let field = |file: &str, k: &str| {
        manifest
            .iter()
            .find(|m| m.get("file").and_then(|x| x.as_str()) == Some(file))
            .and_then(|m| m.get(k).and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string()
    };
    let mut slots = Vec::new();
    for (n, purpose, file) in parse_slots(&md) {
        let rel = file.trim_start_matches("./").to_string();
        let Some(b) = files.get(&rel) else { continue };
        slots.push(PlannedSlot {
            n,
            purpose,
            label: field(&rel, "label"),
            supports: field(&rel, "supports"),
            cannot: field(&rel, "cannot_prove"),
            upstream_path: field(&rel, "upstream_path"),
            file: rel,
            bytes: b.clone(),
        });
    }
    Ok(Some(WritePlan {
        write_task_id: task_id,
        deliverable_id: d.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
        version: d.get("version").and_then(|x| x.as_i64()).unwrap_or(0),
        slots,
        evidence_urls: evidence_urls(&files),
    }))
}

pub async fn fetch(http: &reqwest::Client, url: &str, cap: usize) -> Result<Vec<u8>> {
    let mut resp = http
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (csw-collector)")
        .send()
        .await
        .with_context(|| format!("请求 {url}"))?;
    anyhow::ensure!(resp.status().is_success(), "{url} 返回 {}", resp.status());
    let mut out = Vec::new();
    while let Some(c) = resp.chunk().await? {
        anyhow::ensure!(out.len() + c.len() <= cap, "{url} 超过 {cap} 字节上限");
        out.extend_from_slice(&c);
    }
    Ok(out)
}

/// 页面上的图片地址：`<img src / data-src>`、og:image；Shopify 商品页再加商品 JSON 的图库与描述区。
/// 一律去掉查询参数与 Shopify 的尺寸后缀（`_1000x` 等），取上传原图。
pub fn page_images(
    page_url: &str,
    html: &str,
    product_json: Option<&serde_json::Value>,
) -> Vec<String> {
    let re = regex::Regex::new(
        r#"(?:src|data-src|content)="((?:https?:)?//[^"]+\.(?:jpe?g|png|webp)[^"]*)""#,
    )
    .expect("静态正则");
    let size = regex::Regex::new(r"_(?:\d+x\d*|\d*x\d+|pico|icon|thumb|small|compact|medium|large|grande)(\.(?:jpe?g|png|webp))$")
        .expect("静态正则");
    let mut raw: Vec<String> = re.captures_iter(html).map(|c| c[1].to_string()).collect();
    if let Some(p) = product_json.and_then(|v| v.get("product")) {
        for im in p
            .get("images")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(s) = im.get("src").and_then(|x| x.as_str()) {
                raw.push(s.to_string());
            }
        }
        if let Some(b) = p.get("body_html").and_then(|x| x.as_str()) {
            raw.extend(re.captures_iter(b).map(|c| c[1].to_string()));
        }
    }
    let base = url::Url::parse(page_url).ok();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for u in raw {
        let u = if u.starts_with("//") {
            format!("https:{u}")
        } else {
            u
        };
        let Some(abs) = base.as_ref().and_then(|b| b.join(&u).ok()) else {
            continue;
        };
        let mut s = abs.to_string();
        if let Some(i) = s.find('?') {
            s.truncate(i);
        }
        let s = size.replace(&s, "$1").to_string();
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

/// 核到的页面原图。
pub struct Original {
    pub page_url: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// 在 04 依据的页面里找与 04 附件同画面的最大原图。
pub async fn find_original(
    http: &reqwest::Client,
    pages: &[String],
    write_bytes: &[u8],
) -> Result<Option<Original>> {
    let lo = hires_match::lo_print(write_bytes).map_err(anyhow::Error::msg)?;
    let mut best: Option<Original> = None;
    for page in pages {
        let Ok(html) = fetch(http, page, 5 << 20).await else {
            continue;
        };
        let html = String::from_utf8_lossy(&html).into_owned();
        let pj = if page.contains("/products/") {
            let ju = format!(
                "{}.json",
                page.split('?').next().unwrap_or(page).trim_end_matches('/')
            );
            fetch(http, &ju, 5 << 20)
                .await
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        } else {
            None
        };
        for u in page_images(page, &html, pj.as_ref()).into_iter().take(40) {
            let Ok(b) = fetch(http, &u, 20 << 20).await else {
                continue;
            };
            let Ok(p) = hires_match::prints(&b) else {
                continue;
            };
            if !hires_match::compare(&p, lo).ok() {
                continue;
            }
            let Some((w, h)) = hires_match::dimensions(&b) else {
                continue;
            };
            if best.as_ref().is_none_or(|o| {
                u64::from(w) * u64::from(h) > u64::from(o.width) * u64::from(o.height)
            }) {
                best = Some(Original {
                    page_url: page.clone(),
                    url: u,
                    width: w,
                    height: h,
                    data: b,
                });
            }
        }
    }
    Ok(best)
}

pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# 稿\n\n正文……\n\n## 图位说明（对内，不嵌入读者段落）\n\n\
1. 标题下：原帖系列背负场景，图注只写PA’LANTE背包，不指定V2。\n\n![图位1参考](images/moonlightgearofficial-695545_1.jpg)\n\n\
2. 第二段前：明确绑定V2店页的肩带局部，只用此图对应V2结构。\n\n![图位2参考](images/palante-v2-img_001.jpg)\n\n\
3. 第三段前：手探包底袋口动作，图注不指定型号。\n\n![图位3参考](images/moonlightgearofficial-695545_4.jpg)\n\n\
### 同图位连续图组参考\n\n![图组参考](images/palante-v2-img_002.jpg)\n\n## 自检\n\n![x](images/other.jpg)\n";

    #[test]
    fn 切出04的三个图位() {
        let s = parse_slots(MD);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].0, 1);
        assert_eq!(s[1].2, "images/palante-v2-img_001.jpg");
        assert!(s[1].1.contains("V2店页的肩带局部"));
        assert_eq!(s[2].2, "images/moonlightgearofficial-695545_4.jpg");
        assert!(parse_slots("# 没有图位说明\n\n![a](images/a.jpg)").is_empty());
    }

    #[test]
    fn 店页图片地址取上传原图() {
        let html = r#"<img src="//moonlight-gear.com/cdn/shop/files/7_828c.jpg?v=179&width=1000"><meta property="og:image" content="https://x.com/a_1000x.jpg?v=1">"#;
        let pj = serde_json::json!({"product": {"images": [{"src": "https://cdn.shopify.com/s/files/1/x/files/20_4a.jpg?v=1"}],
            "body_html": "<p><img src=\"//cdn.shopify.com/s/files/1/x/files/20241009_Palante_049_1400px.jpg?v=2\"></p>"}});
        let v = page_images(
            "https://moonlight-gear.com/products/palante-new-v2",
            html,
            Some(&pj),
        );
        assert!(v.contains(&"https://moonlight-gear.com/cdn/shop/files/7_828c.jpg".to_string()));
        assert!(v.contains(&"https://x.com/a.jpg".to_string()));
        assert!(v.contains(&"https://cdn.shopify.com/s/files/1/x/files/20_4a.jpg".to_string()));
        assert!(
            v.contains(
                &"https://cdn.shopify.com/s/files/1/x/files/20241009_Palante_049_1400px.jpg"
                    .to_string()
            )
        );
    }

    #[test]
    fn 依据页面只认原文链接与evidence() {
        let mut f = BTreeMap::new();
        f.insert("source/evidence-2.txt".into(), "> 原文链接: https://moonlight-gear.com/products/palante-new-v2\n[x](https://moonlightgear.myshopify.com/pages/haimen_size)".as_bytes().to_vec());
        f.insert(
            "source/task.json".into(),
            br#""evidence_url": "https://www.instagram.com/p/DeJpaFOFNB3/","#.to_vec(),
        );
        assert_eq!(
            evidence_urls(&f),
            vec!["https://moonlight-gear.com/products/palante-new-v2"]
        );
    }
}
