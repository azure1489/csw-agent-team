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
            let vector = match &c.vector {
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

pub struct KbImageSearch(pub Arc<KbContext>);

impl Tool for KbImageSearch {
    fn name(&self) -> &'static str {
        "kb_image_search"
    }
    fn description(&self) -> &'static str {
        "以图搜图：给一张本机图片的路径，在杂志背景库的裁图里找外观相近的，返回所在杂志条目。\
         占向量服务，一次约一秒。只读。"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "本机图片文件路径（jpg / png / webp）"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT}
            },
            "required": ["path"]
        })
    }
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
        Box::pin(async move {
            let c = &self.0;
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            anyhow::ensure!(!path.is_empty(), "path 不能空");
            let vector = c.vector.as_ref().context("没有向量服务，以图搜图做不了")?;
            let len = std::fs::metadata(path)
                .with_context(|| format!("读不到 {path}"))?
                .len();
            anyhow::ensure!(len <= MAX_IMAGE_BYTES, "图太大（{len} 字节），上限 20MB");
            let bytes = std::fs::read(path).with_context(|| format!("读 {path}"))?;
            let b64 = tokio::task::spawn_blocking(move || encode_b64(&bytes, QUERY_SIDE)).await??;
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
        let e = KbImageSearch(c)
            .call(&json!({"path": "/tmp/x.jpg"}))
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("没有向量服务"), "{e:#}");
    }
}
