//! 杂志背景库：读刊译台的清单，一张有译文的裁图一条 `magazine_item`。
//!
//! 契约：`docs/情报收集员工作台_杂志清单契约.md`（v1）。要点——
//! - **有清单 = 这一本的图已全部上传**（刊译台最后写，`.part` + rename）。没有清单、只有 `.part`
//!   的书不入库（护栏 4）；坏行跳过并计数，不让整份失败。
//! - **按本对账**：清单里没有的该书条目直接删。重新解析一本书 `task_id` 会变、`image_id` 跟着变，
//!   旧条目不靠人记得清理（护栏 7）。
//! - 入库完成写 `{book}/ingested.json`，刊译台读它显示入库状态。
//! - 杂志**不进判断**（方案决策 2）：`post_id` 留空（检索去重按 (kind, post_id)，填 book_key 会让
//!   每本只剩一条），判断路径按 [`docs::REFERENCE_KINDS_SQL`] 只看参考库四类。
//!
//! 向量不在这里算：融合向量 / 纯图向量由回填任务按 `vector_eligible` 另算。这里只负责
//! 结构化字段、全文索引与清理，并把要从 LanceDB 删的键交给调用方。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::docs::{self, KbDoc, MAGAZINE};
use crate::fts::Tokenizer;

pub const MANIFEST: &str = "manifest.jsonl";
pub const INGESTED: &str = "ingested.json";
pub const SRC_MAGAZINE: &str = "magazine";
/// 本工具认得的清单版本
pub const MANIFEST_VERSION: i64 = 1;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Product {
    #[serde(default)]
    pub brand: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub price_jpy: Option<i64>,
    #[serde(default)]
    pub tax_included: Option<bool>,
    #[serde(default)]
    pub specs: Option<String>,
}

/// 清单一行（契约 v1 §2.2）。
#[derive(Debug, Clone, Deserialize)]
pub struct Line {
    pub manifest_version: i64,
    pub generated_at: String,
    pub book_key: String,
    pub magazine: String,
    pub issue: String,
    pub issue_date: Option<String>,
    pub image_id: String,
    pub pdf_index: i64,
    #[serde(default)]
    pub seq_in_page: i64,
    pub printed_page: Option<i64>,
    #[serde(default)]
    pub bbox_pt: Value,
    #[serde(default)]
    pub page_size: Value,
    pub section_title_zh: Option<String>,
    pub category: Option<String>,
    pub has_content: bool,
    pub skipped: bool,
    pub description: Option<String>,
    pub md_zh: Option<String>,
    #[serde(default)]
    pub products: Vec<Product>,
    pub image_blake3: String,
    pub image_url: String,
    pub local_path: String,
    pub page_blake3: String,
    pub page_url: String,
    pub page_local_path: String,
}

impl Line {
    /// 决策 8：只算有内容、没跳过、不是装饰图的裁图的向量
    pub fn vector_eligible(&self) -> bool {
        self.has_content && !self.skipped && self.category.as_deref() != Some("decor")
    }

    fn page_label(&self) -> String {
        self.printed_page
            .map(|p| format!("P.{p}"))
            .unwrap_or_else(|| format!("PDF 第 {} 页", self.pdf_index + 1))
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// 一行 → 一条 doc。没有译文的图不建 doc（方案 §6 字段映射）。
pub fn to_doc(l: &Line) -> Option<KbDoc> {
    let md = non_empty(&l.md_zh)?;
    let first = l.products.first();
    let brand = first.and_then(|p| non_empty(&p.brand)).unwrap_or("");
    let name = first.and_then(|p| non_empty(&p.name)).unwrap_or("");
    let head = match (brand, name) {
        ("", "") => non_empty(&l.section_title_zh).unwrap_or("杂志").to_string(),
        (b, "") => b.to_string(),
        ("", n) => n.to_string(),
        (b, n) => format!("{b} {n}"),
    };
    let title = format!("{head} · {} {} {}", l.magazine, l.issue, l.page_label());
    let mut body = String::new();
    if let Some(d) = non_empty(&l.description) {
        body.push_str(d);
        body.push('\n');
    }
    body.push_str(md);
    // 商品表文字：多品牌时全部品牌写进正文，靠全文命中
    for p in &l.products {
        let mut row = Vec::new();
        for v in [non_empty(&p.brand), non_empty(&p.name)]
            .into_iter()
            .flatten()
        {
            row.push(v.to_string());
        }
        if let Some(y) = p.price_jpy {
            row.push(format!(
                "¥{y}{}",
                match p.tax_included {
                    Some(true) => "（含税）",
                    Some(false) => "（不含税）",
                    None => "",
                }
            ));
        }
        if let Some(s) = non_empty(&p.specs) {
            row.push(s.to_string());
        }
        if !row.is_empty() {
            body.push_str("\n- ");
            body.push_str(&row.join(" · "));
        }
    }
    if let Some(s) = non_empty(&l.section_title_zh) {
        body.push_str(&format!("\n栏目：{s}"));
    }
    Some(KbDoc {
        kind: MAGAZINE.into(),
        ref_id: l.image_id.clone(),
        platform: "magazine".into(),
        post_id: String::new(),
        title,
        body,
        url: l.image_url.clone(),
        brand: brand.to_string(),
        published_at: l
            .issue_date
            .as_deref()
            .filter(|d| d.len() == 10)
            .map(|d| format!("{d}T00:00:00Z")),
        publish_state: "magazine".into(),
        is_reference: false,
        ..Default::default()
    })
}

/// 读一份清单：返回好行与坏行数。坏行 = 不是 JSON、缺字段、版本不认得、book_key 不一致。
pub fn read_manifest(path: &Path, expect_book: &str) -> Result<(Vec<Line>, Vec<String>)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("读 {}", path.display()))?;
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Line>(raw) {
            Ok(l) if l.manifest_version != MANIFEST_VERSION => bad.push(format!(
                "第 {} 行：清单版本 {} 不认得",
                i + 1,
                l.manifest_version
            )),
            Ok(l) if l.book_key != expect_book => bad.push(format!(
                "第 {} 行：book_key {} 与目录 {expect_book} 不一致",
                i + 1,
                l.book_key
            )),
            Ok(l) if l.image_id.is_empty() || l.image_blake3.len() != 64 => {
                bad.push(format!("第 {} 行：image_id / image_blake3 不合法", i + 1))
            }
            Ok(l) => good.push(l),
            Err(e) => bad.push(format!("第 {} 行：{e}", i + 1)),
        }
    }
    Ok((good, bad))
}

#[derive(Debug, Default, Clone)]
pub struct BookReport {
    pub book_key: String,
    pub generated_at: String,
    pub lines: usize,
    pub bad_lines: usize,
    pub inserted: usize,
    pub changed: usize,
    pub unchanged: usize,
    /// 没有译文、不建 doc 的图
    pub no_text: usize,
    /// 对账删掉的旧条目
    pub deleted: usize,
    pub errors: Vec<String>,
    /// 要从 LanceDB docs 表删的键（对账删掉的条目）
    pub drop_doc_keys: Vec<String>,
    /// 要从 LanceDB images 表删的 blake3（已没有任何条目引用）
    pub drop_image_blake3: Vec<String>,
}

/// 从全文索引里删一条：contentless FTS5 要原样给回旧词元。
fn fts_remove(conn: &Connection, doc_id: i64, tokens: &str) -> Result<()> {
    if tokens.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO kb_fts(kb_fts, rowid, tokens) VALUES('delete', ?1, ?2)",
        params![doc_id, tokens],
    )
    .context("从全文索引删旧词元")?;
    Ok(())
}

/// 删一批杂志条目（kb_docs、kb_fts、kb_doc_images），返回要从向量库删的 doc 键与已无人引用的图。
fn delete_docs(conn: &Connection, ids: &[i64]) -> Result<(Vec<String>, Vec<String>)> {
    let mut keys = Vec::new();
    let mut blakes = HashSet::new();
    for id in ids {
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT d.ref_id, COALESCE(i.fts_tokens, ''), COALESCE(i.blake3, '')
                 FROM kb_docs d LEFT JOIN kb_doc_images i ON i.doc_id = d.id
                 WHERE d.id = ?1 AND d.kind = ?2",
                params![id, MAGAZINE],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((ref_id, tokens, b3)) = row else {
            continue;
        };
        fts_remove(conn, *id, &tokens)?;
        conn.execute("DELETE FROM kb_doc_images WHERE doc_id = ?1", [id])?;
        conn.execute("DELETE FROM kb_docs WHERE id = ?1", [id])?;
        keys.push(format!("kb:{MAGAZINE}:{ref_id}"));
        if !b3.is_empty() {
            blakes.insert(b3);
        }
    }
    // 图向量按 blake3 跨书共用：只删已经没有任何条目引用的
    let mut orphan = Vec::new();
    for b in blakes {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM kb_doc_images WHERE blake3 = ?1",
            [&b],
            |r| r.get(0),
        )?;
        if n == 0 {
            orphan.push(b);
        }
    }
    orphan.sort();
    Ok((keys, orphan))
}

/// 入库一本（一个事务）。清单里没有的该书旧条目删掉。
pub fn ingest_lines(
    conn: &Connection,
    tok: &Tokenizer,
    book_key: &str,
    lines: &[Line],
) -> Result<BookReport> {
    let mut rep = BookReport {
        book_key: book_key.to_string(),
        generated_at: lines
            .first()
            .map(|l| l.generated_at.clone())
            .unwrap_or_default(),
        lines: lines.len(),
        ..Default::default()
    };
    let tx = conn.unchecked_transaction()?;
    let mut keep: HashSet<String> = HashSet::new();
    for l in lines {
        let Some(doc) = to_doc(l) else {
            rep.no_text += 1;
            continue;
        };
        keep.insert(doc.ref_id.clone());
        let up = docs::upsert(&tx, &doc)?;
        if up.inserted {
            rep.inserted += 1;
        } else if up.content_changed {
            rep.changed += 1;
        } else {
            rep.unchanged += 1;
        }
        let old_tokens: Option<String> = tx
            .query_row(
                "SELECT fts_tokens FROM kb_doc_images WHERE doc_id = ?1",
                [up.id],
                |r| r.get(0),
            )
            .optional()?;
        let tokens = tok.cut(&doc.text());
        if old_tokens.as_deref() != Some(tokens.as_str()) {
            if let Some(old) = &old_tokens {
                fts_remove(&tx, up.id, old)?;
            }
            tx.execute(
                "INSERT INTO kb_fts(rowid, tokens) VALUES (?1, ?2)",
                params![up.id, tokens],
            )?;
        }
        tx.execute(
            "INSERT INTO kb_doc_images(doc_id, book_key, image_id, blake3, url, page_blake3, page_url, pdf_index,
                                       printed_page, bbox, page_size, local_path, page_local_path, category,
                                       vector_eligible, fts_tokens)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(doc_id) DO UPDATE SET
               book_key=excluded.book_key, image_id=excluded.image_id,
               image_embed_model = CASE WHEN kb_doc_images.blake3 = excluded.blake3 THEN kb_doc_images.image_embed_model ELSE '' END,
               blake3=excluded.blake3, url=excluded.url, page_blake3=excluded.page_blake3, page_url=excluded.page_url,
               pdf_index=excluded.pdf_index, printed_page=excluded.printed_page, bbox=excluded.bbox,
               page_size=excluded.page_size, local_path=excluded.local_path, page_local_path=excluded.page_local_path,
               category=excluded.category, vector_eligible=excluded.vector_eligible, fts_tokens=excluded.fts_tokens",
            params![
                up.id,
                book_key,
                l.image_id,
                l.image_blake3,
                l.image_url,
                l.page_blake3,
                l.page_url,
                l.pdf_index,
                l.printed_page,
                l.bbox_pt.to_string(),
                l.page_size.to_string(),
                l.local_path,
                l.page_local_path,
                l.category.clone().unwrap_or_default(),
                i64::from(l.vector_eligible()),
                tokens,
            ],
        )?;
    }
    // 对账：这本书在库里、清单里没有的条目
    let stale: Vec<i64> = {
        let mut st = tx.prepare(
            "SELECT d.id, d.ref_id FROM kb_doc_images i JOIN kb_docs d ON d.id = i.doc_id WHERE i.book_key = ?1",
        )?;
        let rows = st.query_map([book_key], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.filter_map(Result::ok)
            .filter(|(_, r)| !keep.contains(r))
            .map(|(id, _)| id)
            .collect()
    };
    let (keys, orphan) = delete_docs(&tx, &stale)?;
    rep.deleted = stale.len();
    rep.drop_doc_keys = keys;
    rep.drop_image_blake3 = orphan;
    tx.commit()?;
    Ok(rep)
}

/// 整本清理（`kb purge --book`）：删 kb_docs、kb_fts、kb_doc_images 里这本书的行。
/// 返回要从向量库删的 doc 键、已无人引用的图 blake3。
pub fn purge_book(conn: &Connection, book_key: &str) -> Result<(usize, Vec<String>, Vec<String>)> {
    let tx = conn.unchecked_transaction()?;
    let ids: Vec<i64> = {
        let mut st = tx.prepare("SELECT doc_id FROM kb_doc_images WHERE book_key = ?1")?;
        let rows = st.query_map([book_key], |r| r.get(0))?;
        rows.filter_map(Result::ok).collect()
    };
    let (keys, orphan) = delete_docs(&tx, &ids)?;
    tx.commit()?;
    Ok((ids.len(), keys, orphan))
}

/// 这本书在库里的向量进度：(融合已算, 应算, 纯图已算, 应算)。
pub fn vector_progress(conn: &Connection, book_key: &str) -> Result<(i64, i64, i64, i64)> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(d.embed_model <> ''), 0), COUNT(*),
                COALESCE(SUM(i.image_embed_model <> ''), 0), COUNT(*)
         FROM kb_doc_images i JOIN kb_docs d ON d.id = i.doc_id
         WHERE i.book_key = ?1 AND i.vector_eligible = 1",
        [book_key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?)
}

/// 写 `ingested.json`（契约 §3，`.part` + rename，整份覆盖）。
pub fn write_ingested(conn: &Connection, book_dir: &Path, rep: &BookReport) -> Result<()> {
    let docs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM kb_doc_images WHERE book_key = ?1",
        [&rep.book_key],
        |r| r.get(0),
    )?;
    let (fd, ft, pd, pt) = vector_progress(conn, &rep.book_key)?;
    let mut errors = rep.errors.clone();
    errors.truncate(20);
    let v = json!({
        "manifest_generated_at": rep.generated_at,
        "ingested_at": jiff::Timestamp::now().to_string(),
        "docs": docs,
        "images_total": rep.lines + rep.bad_lines,
        "fused_done": fd, "fused_target": ft,
        "pure_done": pd, "pure_target": pt,
        "deleted": rep.deleted,
        "errors": errors,
        "tool_version": env!("CARGO_PKG_VERSION"),
    });
    let path = book_dir.join(INGESTED);
    let part = book_dir.join(format!("{INGESTED}.part"));
    std::fs::write(&part, serde_json::to_vec_pretty(&v)?)
        .with_context(|| format!("写 {}", part.display()))?;
    std::fs::rename(&part, &path)?;
    Ok(())
}

/// 上次入库依据的清单时间（读 ingested.json）。
fn last_ingested(book_dir: &Path) -> Option<String> {
    let t = std::fs::read_to_string(book_dir.join(INGESTED)).ok()?;
    serde_json::from_str::<Value>(&t)
        .ok()?
        .get("manifest_generated_at")?
        .as_str()
        .map(str::to_string)
}

/// 同步一本。`Ok(None)` = 没清单（未上传完）或清单没变、跳过。
pub fn sync_book(
    conn: &Connection,
    tok: &Tokenizer,
    book_dir: &Path,
    force: bool,
) -> Result<Option<BookReport>> {
    let book_key = book_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .context("书目录没有名字")?;
    let path = book_dir.join(MANIFEST);
    if !path.is_file() {
        return Ok(None); // 只有 .part 或还没写：图没传完，不入库
    }
    let (lines, bad) = read_manifest(&path, &book_key)?;
    if lines.is_empty() {
        anyhow::bail!("{book_key}：清单没有一行可用（坏行 {}）", bad.len());
    }
    if !force && last_ingested(book_dir).as_deref() == Some(lines[0].generated_at.as_str()) {
        return Ok(None);
    }
    let mut rep = ingest_lines(conn, tok, &book_key, &lines)?;
    rep.bad_lines = bad.len();
    rep.errors = bad;
    write_ingested(conn, book_dir, &rep)?;
    Ok(Some(rep))
}

/// 扫产出目录下每本书（有 `manifest.jsonl` 的子目录）。各本各报各的，一本失败不挡别的。
pub fn sync_dir(
    conn: &Connection,
    tok: &Tokenizer,
    dir: &Path,
    force: bool,
) -> Result<Vec<Result<BookReport>>> {
    let mut books: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("读清单目录 {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    books.sort();
    let mut out = Vec::new();
    for b in books {
        match sync_book(conn, tok, &b, force) {
            Ok(Some(r)) => out.push(Ok(r)),
            Ok(None) => {}
            Err(e) => out.push(Err(e)),
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn conn() -> Connection {
        csw_collector_core::store::open_in_memory().unwrap()
    }

    fn b3(n: u8) -> String {
        format!("{:064x}", n)
    }

    pub(crate) fn line(
        book: &str,
        task: &str,
        idx: i64,
        seq: i64,
        md: Option<&str>,
        brand: &str,
        img: u8,
    ) -> Line {
        Line {
            manifest_version: 1,
            generated_at: "2026-10-08T12:00:00Z".into(),
            book_key: book.into(),
            magazine: "GO OUT".into(),
            issue: "2016.08".into(),
            issue_date: Some("2016-08-01".into()),
            image_id: format!("{task}:{idx}:{seq}"),
            pdf_index: idx,
            seq_in_page: seq,
            printed_page: Some(idx + 6),
            bbox_pt: json!([1.0, 2.0, 3.0, 4.0]),
            page_size: json!([1535.0, 1967.0]),
            section_title_zh: Some("发现！超值好物".into()),
            category: Some("product".into()),
            has_content: true,
            skipped: false,
            description: Some("双肩包".into()),
            md_zh: md.map(str::to_string),
            products: vec![Product {
                brand: Some(brand.into()),
                name: Some("Backpack".into()),
                price_jpy: Some(25920),
                tax_included: Some(true),
                specs: None,
            }],
            image_blake3: b3(img),
            image_url: format!("https://x/{}.jpg", b3(img)),
            local_path: "/d/a.jpg".into(),
            page_blake3: b3(200),
            page_url: "https://x/p.jpg".into(),
            page_local_path: "/d/p.jpg".into(),
        }
    }

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn 映射字段() {
        let d = to_doc(&line(
            "goout-16-08日本",
            "T",
            12,
            3,
            Some("**背包**"),
            "TNF",
            1,
        ))
        .unwrap();
        assert_eq!(d.kind, MAGAZINE);
        assert_eq!(d.ref_id, "T:12:3");
        assert_eq!(d.post_id, "", "post_id 留空，不被 dedupe_by_post 合并");
        assert_eq!(d.title, "TNF Backpack · GO OUT 2016.08 P.18");
        assert!(d.body.contains("¥25920（含税）"));
        assert_eq!(d.brand, "TNF");
        assert_eq!(d.published_at.as_deref(), Some("2016-08-01T00:00:00Z"));
        assert!(
            to_doc(&line("b", "T", 1, 0, None, "x", 1)).is_none(),
            "没译文不建 doc"
        );
    }

    #[test]
    fn 入库幂等_按本对账_换图重算() {
        let c = conn();
        let tok = Tokenizer::new();
        let v1 = vec![
            line("B", "T1", 1, 0, Some("甲"), "A", 1),
            line("B", "T1", 1, 1, Some("乙"), "A", 2),
            line("B", "T1", 2, 0, None, "A", 3),
        ];
        let r = ingest_lines(&c, &tok, "B", &v1).unwrap();
        assert_eq!((r.inserted, r.no_text, r.deleted), (2, 1, 0));
        let again = ingest_lines(&c, &tok, "B", &v1).unwrap();
        assert_eq!((again.inserted, again.changed, again.unchanged), (0, 0, 2));
        assert_eq!(count(&c, "SELECT COUNT(*) FROM kb_doc_images"), 2);

        // 护栏 7：重新解析（task_id 变了）→ 旧 ref_id 全部对账删掉，不残留
        let v2 = vec![line("B", "T2", 1, 0, Some("甲"), "A", 1)];
        let r2 = ingest_lines(&c, &tok, "B", &v2).unwrap();
        assert_eq!((r2.inserted, r2.deleted), (1, 2));
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM kb_docs WHERE kind = 'magazine_item'"
            ),
            1
        );
        assert_eq!(r2.drop_doc_keys.len(), 2);
        // 图 1 仍被新条目引用，不删；图 2 已没人引用，删
        assert_eq!(r2.drop_image_blake3, vec![b3(2)]);

        // 换图：同 image_id、文字不变，也要重算
        let mut v3 = v2.clone();
        v3[0].image_blake3 = b3(9);
        v3[0].image_url = format!("https://x/{}.jpg", b3(9));
        let r3 = ingest_lines(&c, &tok, "B", &v3).unwrap();
        assert_eq!(r3.changed, 1);
    }

    #[test]
    fn 全文能搜到杂志但判断路径搜不到() {
        let c = conn();
        let tok = Tokenizer::new();
        ingest_lines(
            &c,
            &tok,
            "B",
            &[line(
                "B",
                "T",
                1,
                0,
                Some("Hard Rock Cooler 冷藏箱"),
                "ORCA",
                1,
            )],
        )
        .unwrap();
        assert!(
            crate::fts::search(&c, &tok, "冷藏箱", 10)
                .unwrap()
                .is_empty(),
            "判断的全文路不召回杂志"
        );
        assert_eq!(
            crate::fts::search_kind(&c, &tok, "冷藏箱", MAGAZINE, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(docs::by_brand(&c, "ORCA", 10).unwrap().is_empty());
    }

    #[test]
    fn 整本清理三处一致_共用图保留() {
        let c = conn();
        let tok = Tokenizer::new();
        ingest_lines(
            &c,
            &tok,
            "A",
            &[
                line("A", "TA", 1, 0, Some("甲"), "X", 1),
                line("A", "TA", 1, 1, Some("乙"), "X", 7),
            ],
        )
        .unwrap();
        // B 本与 A 共用图 7（跨期重复的广告图）
        ingest_lines(&c, &tok, "B", &[line("B", "TB", 1, 0, Some("丙"), "X", 7)]).unwrap();
        let (n, keys, orphan) = purge_book(&c, "A").unwrap();
        assert_eq!(n, 2);
        assert_eq!(keys.len(), 2);
        assert_eq!(orphan, vec![b3(1)], "图 7 还被 B 引用，不删");
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM kb_doc_images WHERE book_key = 'A'"
            ),
            0
        );
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM kb_docs WHERE kind = 'magazine_item'"
            ),
            1
        );
        // 全文索引里 A 的词元也扣掉了
        assert!(
            crate::fts::search_kind(&c, &tok, "甲", MAGAZINE, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            crate::fts::search_kind(&c, &tok, "丙", MAGAZINE, 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn 没清单或只有part不入库_坏行跳过计数() {
        let c = conn();
        let tok = Tokenizer::new();
        let dir = std::env::temp_dir().join(format!("mag-test-{}", std::process::id()));
        let book = dir.join("BK");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&book).unwrap();
        let good = serde_json::to_string(
            &serde_json::to_value(LineSer(&line("BK", "T", 1, 0, Some("甲"), "X", 1))).unwrap(),
        )
        .unwrap();
        std::fs::write(book.join("manifest.jsonl.part"), format!("{good}\n")).unwrap();
        assert!(
            sync_book(&c, &tok, &book, false).unwrap().is_none(),
            "只有 .part 不入库"
        );

        let other = serde_json::to_string(
            &serde_json::to_value(LineSer(&line("别的书", "T", 1, 1, Some("乙"), "X", 2))).unwrap(),
        )
        .unwrap();
        std::fs::write(book.join(MANIFEST), format!("{good}\n{{坏行\n{other}\n")).unwrap();
        let r = sync_book(&c, &tok, &book, false).unwrap().unwrap();
        assert_eq!((r.inserted, r.bad_lines), (1, 2));
        let ing: Value =
            serde_json::from_str(&std::fs::read_to_string(book.join(INGESTED)).unwrap()).unwrap();
        assert_eq!(ing["docs"], 1);
        assert_eq!(ing["fused_target"], 1);
        assert_eq!(ing["errors"].as_array().unwrap().len(), 2);
        // 清单没变：再扫跳过
        assert!(sync_book(&c, &tok, &book, false).unwrap().is_none());
        assert!(
            sync_book(&c, &tok, &book, true).unwrap().is_some(),
            "--force 照样重入"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 测试里把 Line 写回 JSON（生产代码只读不写）
    struct LineSer<'a>(&'a Line);
    impl serde::Serialize for LineSer<'_> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
            let l = self.0;
            json!({
                "manifest_version": l.manifest_version, "generated_at": l.generated_at, "book_key": l.book_key,
                "magazine": l.magazine, "issue": l.issue, "issue_date": l.issue_date, "image_id": l.image_id,
                "pdf_index": l.pdf_index, "seq_in_page": l.seq_in_page, "printed_page": l.printed_page,
                "bbox_pt": l.bbox_pt, "page_size": l.page_size, "section_title_zh": l.section_title_zh,
                "category": l.category, "has_content": l.has_content, "skipped": l.skipped,
                "description": l.description, "md_zh": l.md_zh,
                "products": l.products.iter().map(|p| json!({"brand": p.brand, "name": p.name, "price_jpy": p.price_jpy, "tax_included": p.tax_included, "specs": p.specs})).collect::<Vec<_>>(),
                "image_blake3": l.image_blake3, "image_url": l.image_url, "local_path": l.local_path,
                "page_blake3": l.page_blake3, "page_url": l.page_url, "page_local_path": l.page_local_path,
            })
            .serialize(s)
        }
    }
}
