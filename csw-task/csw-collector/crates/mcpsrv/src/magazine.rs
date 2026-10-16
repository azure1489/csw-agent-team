//! 杂志背景库的两个只读工具：`kb_magazine_search`、`kb_image_search`。
//!
//! 与参考库那三个分开：杂志是**背景**，不是编辑部的口味证据（方案决策 2），
//! 工具名与说明都要让模型分得清。不重排，理由同 `kb_search`。

use std::sync::Arc;

use anyhow::Context;
use serde_json::{Value, json};

use csw_collector_core::vector::EmbedInput;
use csw_collector_kb::magazine_embed::encode_b64;
use csw_collector_kb::magazine_search::{self as ms, MagazineHit, TextQuery};

use crate::server::{BoxFut, Tool};
use crate::tools::KbContext;

const DEFAULT_LIMIT: usize = 6;
const MAX_LIMIT: usize = 12;
/// 查询图缩到多宽，与入库的 `[magazine].image_side` 默认值一致
const QUERY_SIDE: u32 = 768;
/// 查询图最大字节数
const MAX_IMAGE_BYTES: u64 = 20 << 20;

fn limit_of(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT)
}

/// 给模型看的一段文字：刊名期号页码、品牌、相似度、图址，正文截一段。
fn render(hits: &[MagazineHit], empty: &str) -> String {
    if hits.is_empty() {
        return empty.to_string();
    }
    let mut out =
        String::from("【杂志背景】（日文杂志译文，只作背景参照，不是编辑部的选题口径）\n");
    for h in hits {
        let it = &h.item;
        out.push_str(&format!("- #{} {}", it.id, it.title));
        if let Some(s) = h.similarity {
            out.push_str(&format!("（相似度 {s:.2}，命中 {}）", h.routes.join("+")));
        } else {
            out.push_str(&format!("（命中 {}）", h.routes.join("+")));
        }
        out.push('\n');
        let snippet: String = it.body.chars().take(160).collect();
        out.push_str(&format!("  {}\n", snippet.replace('\n', " ")));
        out.push_str(&format!("  图：{}\n", it.image_url));
    }
    out
}

// ─────────────────────── kb_magazine_search ───────────────────────

pub struct KbMagazineSearch(pub Arc<KbContext>);

impl Tool for KbMagazineSearch {
    fn name(&self) -> &'static str {
        "kb_magazine_search"
    }
    fn description(&self) -> &'static str {
        "在杂志背景库（GO OUT 等日文户外杂志的中文译文、商品表、裁图）里按文字检索。\
         全文与向量两路。用来查某个品牌、产品过去在杂志里怎么出现过。只读。"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "品牌名、产品名或一句话"},
                "book_key": {"type": "string", "description": "只看这一本（可选）"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT}
            },
            "required": ["query"]
        })
    }
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
        Box::pin(async move {
            let c = &self.0;
            let text = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            anyhow::ensure!(!text.is_empty(), "query 不能空");
            // 正式轮时段 vector_now 是 None：只走全文，不碰 GPU
            let vector = match c.vector_now() {
                Some(v) => v
                    .embed(&[EmbedInput::Text(text.chars().take(2000).collect())])
                    .await
                    .ok()
                    .and_then(|mut vs| (!vs.is_empty()).then(|| vs.remove(0))),
                None => None,
            };
            let q = TextQuery {
                text,
                vector: vector.as_deref(),
                book_key: args.get("book_key").and_then(Value::as_str),
                limit: limit_of(args),
            };
            let ids = ms::vector_ids(&c.store, &q).await?;
            let conn = c.conn.lock().await;
            let hits = ms::fuse(&conn, &c.tok, &q, &ids, c.store.model())?;
            Ok(render(
                &hits,
                "杂志背景库里没有找到。这只说明已入库的杂志里没有，不说明没出现过。",
            ))
        })
    }
}

// ─────────────────────── kb_image_search ───────────────────────

/// 远程版：`image_url` 拉图的上限
const MAX_URL_BYTES: usize = 10 << 20;
/// 远程版：`image_base64` 解码后的上限
const MAX_B64_BYTES: usize = 5 << 20;
/// 远程版：拉图限时
const URL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 以图搜图。两种用法：
/// - **本机**（stdio）：给本机图片路径；
/// - **远程**（HTTP `/mcp`）：给图片的 https 地址或 base64。远程版**不认路径**——
///   对方机器上的路径在这里没有意义，认了反倒让外人能读服务器上的文件。
pub struct KbImageSearch {
    ctx: Arc<KbContext>,
    /// 远程版拉图用的客户端（出网防护：封内网、回环与云元数据地址）。None = 本机版
    remote: Option<reqwest::Client>,
}

impl KbImageSearch {
    pub fn local(ctx: Arc<KbContext>) -> Self {
        Self { ctx, remote: None }
    }

    pub fn remote(ctx: Arc<KbContext>) -> anyhow::Result<Self> {
        let http = csw_collector_harvest::netguard::guarded_client(URL_TIMEOUT, false)?;
        Ok(Self {
            ctx,
            remote: Some(http),
        })
    }

    /// 取查询图的字节
    async fn image_bytes(&self, args: &Value) -> anyhow::Result<Vec<u8>> {
        let s = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let Some(http) = &self.remote else {
            let path = s("path");
            anyhow::ensure!(!path.is_empty(), "path 不能空");
            let len = std::fs::metadata(&path)
                .with_context(|| format!("读不到 {path}"))?
                .len();
            anyhow::ensure!(len <= MAX_IMAGE_BYTES, "图太大（{len} 字节），上限 20MB");
            return std::fs::read(&path).with_context(|| format!("读 {path}"));
        };
        anyhow::ensure!(
            s("path").is_empty(),
            "远程调用不接受 path，请给 image_url 或 image_base64"
        );
        let (u, b) = (s("image_url"), s("image_base64"));
        match (u.is_empty(), b.is_empty()) {
            (false, true) => fetch_image(http, &u, false).await,
            (true, false) => {
                // 容许 data URL 的前缀
                let raw = b.rsplit_once(";base64,").map_or(b.as_str(), |(_, r)| r);
                // 解码前先按长度粗拦：base64 每 4 字符 3 字节
                anyhow::ensure!(
                    raw.len() / 4 * 3 <= MAX_B64_BYTES + 3,
                    "图太大，image_base64 解码后上限 5MB"
                );
                let bytes = b64_decode(raw).context("image_base64 不是合法的 base64")?;
                anyhow::ensure!(
                    bytes.len() <= MAX_B64_BYTES,
                    "图太大（{} 字节），image_base64 解码后上限 5MB",
                    bytes.len()
                );
                Ok(bytes)
            }
            (true, true) => anyhow::bail!("要给 image_url 或 image_base64 其中一个"),
            (false, false) => anyhow::bail!("image_url 与 image_base64 只能给一个"),
        }
    }
}

/// 拉一张图：只许 https（测试时 `allow_private` 放开 http 与回环），限 10MB、15 秒。
async fn fetch_image(
    http: &reqwest::Client,
    raw: &str,
    allow_private: bool,
) -> anyhow::Result<Vec<u8>> {
    let url = url::Url::parse(raw).with_context(|| format!("地址不合法：{raw}"))?;
    anyhow::ensure!(
        allow_private || url.scheme() == "https",
        "image_url 只许 https"
    );
    // IP 字面量不走解析器，这一道必须在发请求前查；之后每一跳重定向由客户端再查
    csw_collector_harvest::netguard::check_url(&url, allow_private)
        .map_err(|e| anyhow::anyhow!("image_url 被拒：{e}"))?;
    let mut resp = http
        .get(url)
        .timeout(URL_TIMEOUT)
        .send()
        .await
        .context("拉图失败")?
        .error_for_status()
        .context("拉图失败")?;
    anyhow::ensure!(
        resp.content_length()
            .is_none_or(|n| n as usize <= MAX_URL_BYTES),
        "图太大，image_url 上限 10MB"
    );
    let mut body = Vec::new();
    while let Some(c) = resp.chunk().await.context("拉图中断")? {
        body.extend_from_slice(&c);
        anyhow::ensure!(body.len() <= MAX_URL_BYTES, "图太大，image_url 上限 10MB");
    }
    anyhow::ensure!(!body.is_empty(), "拉到的图是空的");
    Ok(body)
}

/// base64 解码（标准表，容许缺尾部的 `=` 与夹杂的空白）。
///
/// 不引 base64 crate，理由同 `harvest::recognize::b64`。
pub fn b64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    }
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let body = clean
        .strip_suffix(b"==")
        .or_else(|| clean.strip_suffix(b"="))
        .unwrap_or(&clean);
    anyhow::ensure!(body.len() % 4 != 1, "长度不对");
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = val(c).with_context(|| format!("非法字符 {:?}", c as char))?;
            n |= v << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

impl Tool for KbImageSearch {
    fn name(&self) -> &'static str {
        "kb_image_search"
    }
    fn description(&self) -> &'static str {
        if self.remote.is_some() {
            "以图搜图：给一张图片（image_url 为 https 地址，或 image_base64），在杂志背景库的裁图里\
             找外观相近的，返回所在杂志条目。占向量服务，一次约一秒；每天正式轮时段暂停。只读。"
        } else {
            "以图搜图：给一张本机图片的路径，在杂志背景库的裁图里找外观相近的，返回所在杂志条目。\
             占向量服务，一次约一秒。只读。"
        }
    }
    fn input_schema(&self) -> Value {
        let limit = json!({"type": "integer", "minimum": 1, "maximum": MAX_LIMIT});
        if self.remote.is_some() {
            json!({
                "type": "object",
                "properties": {
                    "image_url": {"type": "string", "description": "图片的 https 地址（jpg / png / webp，≤10MB）"},
                    "image_base64": {"type": "string", "description": "图片内容的 base64（≤5MB），与 image_url 二选一"},
                    "limit": limit
                }
            })
        } else {
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "本机图片文件路径（jpg / png / webp）"},
                    "limit": limit
                },
                "required": ["path"]
            })
        }
    }
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
        Box::pin(async move {
            let c = &self.ctx;
            anyhow::ensure!(
                !c.gpu_is_paused(),
                "正式轮时段以图搜图暂停（让 GPU 给每天的正式轮），过了这段再试；文字检索照常可用"
            );
            let vector = c.vector.as_ref().context("没有向量服务，以图搜图做不了")?;
            let bytes = self.image_bytes(args).await?;
            let b64 = tokio::task::spawn_blocking(move || encode_b64(&bytes, QUERY_SIDE))
                .await?
                .context("图片读不出来（只认 jpg / png / webp）")?;
            let qv = vector
                .embed(&[EmbedInput::Image(b64)])
                .await?
                .into_iter()
                .next()
                .context("向量服务没返回向量")?;
            let limit = limit_of(args);
            let raw = ms::search_image(&c.store, &qv, limit).await?;
            let conn = c.conn.lock().await;
            let hits = ms::image_hits(&conn, &raw, limit, c.store.model())?;
            Ok(render(
                &hits,
                "杂志背景库里没有相近的图（纯图向量可能还在回填）。",
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_kb::brands::BrandIndex;
    use csw_collector_kb::fts::Tokenizer;
    use csw_collector_kb::magazine::{Line, ingest_lines};
    use csw_collector_kb::vectors::VectorStore;
    use tokio::sync::Mutex;

    async fn ctx(dir: &std::path::Path) -> Arc<KbContext> {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let tok = Tokenizer::new();
        let lines: Vec<Line> = (0..2)
            .map(|i| {
                serde_json::from_value(json!({
                    "manifest_version": 1, "generated_at": "2026-10-08T12:00:00Z",
                    "book_key": "b1", "magazine": "GO OUT", "issue": "2026.01",
                    "issue_date": "2026-01-01", "image_id": format!("T:{i}:0"), "pdf_index": i,
                    "printed_page": 10 + i, "section_title_zh": "露营", "category": "product",
                    "has_content": true, "skipped": false, "description": "帐篷",
                    "md_zh": "这顶帐篷很轻", "products": [{"brand": "snow peak", "name": "Tent"}],
                    "image_blake3": format!("{:064x}", i), "image_url": format!("https://x/{i}.jpg"),
                    "local_path": "", "page_blake3": "", "page_url": "", "page_local_path": ""
                }))
                .unwrap()
            })
            .collect();
        ingest_lines(&conn, &tok, "b1", &lines).unwrap();
        Arc::new(KbContext {
            conn: Mutex::new(conn),
            store: VectorStore::open(dir, 8, "测试").await.unwrap(),
            brands: BrandIndex::new(vec![]),
            tok,
            vector: None,
            send_van_quotes: false,
            gpu_paused: None,
        })
    }

    #[tokio::test]
    async fn 文字检索给出刊期页码与图址() {
        let d = tempdir::TempDir::new("mcpmag").unwrap();
        let c = ctx(d.path()).await;
        let out = KbMagazineSearch(c.clone())
            .call(&json!({"query": "帐篷"}))
            .await
            .unwrap();
        assert!(out.contains("杂志背景"), "{out}");
        assert!(out.contains("GO OUT 2026.01 P.10"), "{out}");
        assert!(
            out.contains("https://x/0.jpg") && out.contains("https://x/1.jpg"),
            "{out}"
        );
        let none = KbMagazineSearch(c.clone())
            .call(&json!({"query": "滑板"}))
            .await
            .unwrap();
        assert!(none.contains("没有找到"), "{none}");
        assert!(
            KbMagazineSearch(c)
                .call(&json!({"query": " "}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn 以图搜图没有向量服务如实报错() {
        let d = tempdir::TempDir::new("mcpimg").unwrap();
        let c = ctx(d.path()).await;
        let e = KbImageSearch::local(c)
            .call(&json!({"path": "/tmp/x.jpg"}))
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("没有向量服务"), "{e:#}");
    }

    #[test]
    fn base64来回一致() {
        for n in 0..40usize {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let enc = csw_collector_harvest::recognize::b64(&bytes);
            assert_eq!(b64_decode(&enc).unwrap(), bytes, "n={n}");
            // 没有尾部 = 也认
            assert_eq!(b64_decode(enc.trim_end_matches('=')).unwrap(), bytes);
        }
        assert!(b64_decode("ab$d").is_err());
        assert!(b64_decode("abcde").is_err());
    }

    fn remote(c: Arc<KbContext>) -> KbImageSearch {
        KbImageSearch::remote(c).unwrap()
    }

    #[tokio::test]
    async fn 远程版不认路径只认地址或base64() {
        let d = tempdir::TempDir::new("mcpimg").unwrap();
        let c = ctx(d.path()).await;
        let t = remote(c);
        let schema = t.input_schema().to_string();
        assert!(!schema.contains("\"path\""), "{schema}");
        assert!(!t.description().contains("路径"));
        let r = t.image_bytes(&json!({"path": "/etc/passwd"})).await;
        assert!(format!("{:#}", r.unwrap_err()).contains("不接受 path"));
        let r = t.image_bytes(&json!({})).await;
        assert!(format!("{:#}", r.unwrap_err()).contains("其中一个"));
        let r = t
            .image_bytes(&json!({"image_url": "https://a/x.jpg", "image_base64": "AAAA"}))
            .await;
        assert!(format!("{:#}", r.unwrap_err()).contains("只能给一个"));
    }

    #[tokio::test]
    async fn 远程版拦内网与http与超大base64() {
        let d = tempdir::TempDir::new("mcpimg").unwrap();
        let t = remote(ctx(d.path()).await);
        for u in [
            "http://example.com/x.jpg",
            "https://127.0.0.1/x.jpg",
            "https://169.254.169.254/latest",
            "https://localhost/x.jpg",
        ] {
            let r = t.image_bytes(&json!({"image_url": u})).await;
            assert!(r.is_err(), "{u} 该被拦");
        }
        let big = csw_collector_harvest::recognize::b64(&vec![0u8; (5 << 20) + 10]);
        let r = t.image_bytes(&json!({"image_base64": big})).await;
        assert!(format!("{:#}", r.unwrap_err()).contains("上限 5MB"));
        let ok = csw_collector_harvest::recognize::b64(b"hello");
        let r = t
            .image_bytes(&json!({"image_base64": format!("data:image/jpeg;base64,{ok}")}))
            .await
            .unwrap();
        assert_eq!(r, b"hello");
    }

    #[tokio::test]
    async fn 远程版拉图限大小() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1u8; 64], "image/jpeg"))
            .mount(&srv)
            .await;
        let http = csw_collector_harvest::netguard::guarded_client(URL_TIMEOUT, true).unwrap();
        let got = fetch_image(&http, &format!("{}/a.jpg", srv.uri()), true)
            .await
            .unwrap();
        assert_eq!(got.len(), 64);

        let srv2 = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(vec![1u8; MAX_URL_BYTES + 1], "image/jpeg"),
            )
            .mount(&srv2)
            .await;
        let e = fetch_image(&http, &format!("{}/b.jpg", srv2.uri()), true)
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("上限 10MB"), "{e:#}");
    }

    #[tokio::test]
    async fn 让卡时以图搜图暂停文字检索照常() {
        let d = tempdir::TempDir::new("mcpimg").unwrap();
        let c = ctx(d.path()).await;
        let mut inner = Arc::try_unwrap(c).ok().unwrap();
        inner.gpu_paused = Some(Arc::new(|| true));
        let c = Arc::new(inner);
        let e = KbImageSearch::local(c.clone())
            .call(&json!({"path": "/tmp/x.jpg"}))
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("正式轮时段"), "{e:#}");
        let out = KbMagazineSearch(c)
            .call(&json!({"query": "帐篷"}))
            .await
            .unwrap();
        assert!(out.contains("GO OUT 2026.01 P.10"), "{out}");
    }
}
