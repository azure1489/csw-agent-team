//! 杂志背景库的检索（方案 §6「HTTP / MCP」、步骤 6）。
//!
//! 与判断路径完全分开：判断用 [`crate::search::Retriever`]，只看参考库四类；
//! 这里只看 `magazine_item`，不重排（重排占 GPU，页面上连搜几下会堵正式轮）。
//!
//! - **文字检索**：全文（`fts::search_kind`）∪ 向量（`docs` 表 `kind = magazine_item`
//!   前置过滤），按名次倒数融合（RRF）排序。同一本书能返回多条——不做 `dedupe_by_post`（护栏 6）。
//! - **以图搜图**：查询图的纯图向量在 `images` 表找最近的 blake3，经 `kb_doc_images`
//!   回到条目（护栏 2）。`images` 表也存候选图，回不到杂志条目的命中直接丢掉。
//! - **条目详情**、**品牌页杂志一节**：纯查库。

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::docs::MAGAZINE;
use crate::fts::{self, Tokenizer};
use crate::vectors::{DocFilter, VectorStore};

/// 融合时每条的累计：(RRF 分, 命中的路, 向量相似度)
type Fused = (f64, Vec<&'static str>, Option<f32>);

/// RRF 的平滑常数（文献惯用 60）
const RRF_K: f64 = 60.0;

/// 一条杂志条目（检索结果卡与详情页共用）。
#[derive(Debug, Clone, Serialize)]
pub struct MagazineItem {
    pub id: i64,
    pub ref_id: String,
    pub title: String,
    pub brand: String,
    pub body: String,
    /// 刊期（`published_at`）
    pub issue_date: Option<String>,
    pub book_key: String,
    pub image_url: String,
    pub image_blake3: String,
    pub page_url: String,
    pub pdf_index: i64,
    pub printed_page: Option<i64>,
    /// `[x0,y0,x1,y1]`（PDF 点），整页图上标框用
    pub bbox: Value,
    /// `[w,h]`（PDF 点）
    pub page_size: Value,
    pub category: String,
    /// 融合向量 / 纯图向量是否已算
    pub fused_ready: bool,
    pub pure_ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MagazineHit {
    #[serde(flatten)]
    pub item: MagazineItem,
    /// 哪几路命中：`fts` / `vector` / `image`
    pub routes: Vec<&'static str>,
    /// 向量路的余弦相似度（全文路没有）
    pub similarity: Option<f32>,
}

const ITEM_COLS: &str = "d.id, d.ref_id, d.title, d.brand, d.body, d.published_at, i.book_key, i.url, i.blake3,
     i.page_url, i.pdf_index, i.printed_page, i.bbox, i.page_size, i.category, d.embed_model, i.image_embed_model";

fn item_row(r: &rusqlite::Row<'_>, model: &str) -> rusqlite::Result<MagazineItem> {
    let json = |s: String| serde_json::from_str(&s).unwrap_or(Value::Null);
    Ok(MagazineItem {
        id: r.get(0)?,
        ref_id: r.get(1)?,
        title: r.get(2)?,
        brand: r.get(3)?,
        body: r.get(4)?,
        issue_date: r.get(5)?,
        book_key: r.get(6)?,
        image_url: r.get(7)?,
        image_blake3: r.get(8)?,
        page_url: r.get(9)?,
        pdf_index: r.get(10)?,
        printed_page: r.get(11)?,
        bbox: json(r.get(12)?),
        page_size: json(r.get(13)?),
        category: r.get(14)?,
        fused_ready: r.get::<_, String>(15)? == model,
        pure_ready: r.get::<_, String>(16)? == model,
    })
}

/// 按 id 取一条（不是杂志条目返回 None）。`model` 用来判断向量是否已算。
pub fn get(conn: &Connection, id: i64, model: &str) -> Result<Option<MagazineItem>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {ITEM_COLS} FROM kb_docs d JOIN kb_doc_images i ON i.doc_id = d.id
                 WHERE d.id = ?1 AND d.kind = '{MAGAZINE}'"
            ),
            [id],
            |r| item_row(r, model),
        )
        .optional()?)
}

fn get_many(conn: &Connection, ids: &[i64], model: &str) -> Result<HashMap<i64, MagazineItem>> {
    let mut out = HashMap::new();
    for id in ids {
        if let Some(it) = get(conn, *id, model)? {
            out.insert(*id, it);
        }
    }
    Ok(out)
}

/// 文字检索的条件。
#[derive(Debug, Clone, Default)]
pub struct TextQuery<'a> {
    pub text: &'a str,
    /// 查询文字的向量；算不出来就只走全文
    pub vector: Option<&'a [f32]>,
    /// 只看这本书
    pub book_key: Option<&'a str>,
    pub limit: usize,
}

/// 文字检索：全文 ∪ 向量，RRF 融合。
pub async fn search_text(
    conn: &Connection,
    store: &VectorStore,
    tok: &Tokenizer,
    q: &TextQuery<'_>,
    model: &str,
) -> Result<Vec<MagazineHit>> {
    let ids = vector_ids(store, q).await?;
    fuse(conn, tok, q, &ids, model)
}

/// 向量路（要 await，不碰 SQLite）。HTTP 层要在两段之间放掉连接锁，所以拆开给。
pub async fn vector_ids(store: &VectorStore, q: &TextQuery<'_>) -> Result<Vec<(i64, f32)>> {
    let Some(v) = q.vector else {
        return Ok(vec![]);
    };
    let pool = pool_size(q);
    let hits = store
        .search_docs(
            v,
            &DocFilter {
                kinds: vec![MAGAZINE.into()],
                ..Default::default()
            },
            pool,
        )
        .await?;
    Ok(hits.into_iter().map(|h| (h.doc_id, h.similarity)).collect())
}

fn pool_size(q: &TextQuery<'_>) -> usize {
    let limit = q.limit.clamp(1, 100);
    // 按书筛是后置的，多取些免得筛完不够
    if q.book_key.is_some() {
        limit * 10
    } else {
        limit * 3
    }
}

/// 全文路 + 与向量路融合（纯查库）。
pub fn fuse(
    conn: &Connection,
    tok: &Tokenizer,
    q: &TextQuery<'_>,
    vector_hits: &[(i64, f32)],
    model: &str,
) -> Result<Vec<MagazineHit>> {
    let limit = q.limit.clamp(1, 100);
    let fts_hits = fts::search_kind(conn, tok, q.text, MAGAZINE, pool_size(q))?;
    // id → (RRF 分, 路, 相似度)
    let mut acc: HashMap<i64, Fused> = HashMap::new();
    for (rank, h) in fts_hits.iter().enumerate() {
        let e = acc.entry(h.doc_id).or_default();
        e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
        e.1.push("fts");
    }
    for (rank, (id, sim)) in vector_hits.iter().enumerate() {
        let e = acc.entry(*id).or_default();
        e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
        e.1.push("vector");
        e.2 = Some(*sim);
    }
    let mut ranked: Vec<(i64, Fused)> = acc.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.total_cmp(&a.1.0).then(a.0.cmp(&b.0)));
    let ids: Vec<i64> = ranked.iter().map(|(id, _)| *id).collect();
    let mut items = get_many(conn, &ids, model)?;
    Ok(ranked
        .into_iter()
        .filter_map(|(id, (_, routes, similarity))| {
            let item = items.remove(&id)?;
            if q.book_key.is_some_and(|b| b != item.book_key) {
                return None;
            }
            Some(MagazineHit {
                item,
                routes,
                similarity,
            })
        })
        .take(limit)
        .collect())
}

/// 以图搜图：`images` 表最近的 blake3 → `kb_doc_images` → 条目。
///
/// 同一张图在几期里出现，几条都返回（相似度相同，按刊期新的在前）。
pub async fn search_image(
    store: &VectorStore,
    query: &[f32],
    limit: usize,
) -> Result<Vec<(String, f32)>> {
    let limit = limit.clamp(1, 100);
    // images 表里还有候选图，回不到杂志的要丢，多取些
    let hits = store.search_images(query, limit * 3).await?;
    Ok(hits.into_iter().map(|h| (h.blake3, h.similarity)).collect())
}

/// 以图搜图的查库那一段。
pub fn image_hits(
    conn: &Connection,
    hits: &[(String, f32)],
    limit: usize,
    model: &str,
) -> Result<Vec<MagazineHit>> {
    let limit = limit.clamp(1, 100);
    let mut st = conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM kb_doc_images i JOIN kb_docs d ON d.id = i.doc_id
         WHERE i.blake3 = ?1 ORDER BY d.published_at IS NULL, d.published_at DESC, d.id"
    ))?;
    let mut out = Vec::new();
    for (b3, sim) in hits {
        let rows = st.query_map([b3], |r| item_row(r, model))?;
        for item in rows.filter_map(Result::ok) {
            out.push(MagazineHit {
                item,
                routes: vec!["image"],
                similarity: Some(*sim),
            });
        }
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    Ok(out)
}

/// 品牌页「杂志里出现过」：按品牌键比（`SNOW PEAK` = `snowpeak`），刊期新的在前。
pub fn by_brand(
    conn: &Connection,
    brand: &str,
    limit: usize,
    model: &str,
) -> Result<Vec<MagazineItem>> {
    let key = crate::brands::brand_key(brand);
    if key.is_empty() {
        return Ok(vec![]);
    }
    let spellings: Vec<String> = conn
        .prepare(&format!(
            "SELECT DISTINCT brand FROM kb_docs WHERE brand <> '' AND kind = '{MAGAZINE}'"
        ))?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|b| crate::brands::brand_key(b) == key)
        .collect();
    let mut out = Vec::new();
    let mut st = conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM kb_docs d JOIN kb_doc_images i ON i.doc_id = d.id
         WHERE d.kind = '{MAGAZINE}' AND d.brand = ?1"
    ))?;
    for b in spellings {
        out.extend(
            st.query_map([b], |r| item_row(r, model))?
                .filter_map(Result::ok),
        );
    }
    out.sort_by(|a, b| {
        b.issue_date
            .cmp(&a.issue_date)
            .then(a.book_key.cmp(&b.book_key))
            .then(a.pdf_index.cmp(&b.pdf_index))
    });
    out.truncate(limit);
    Ok(out)
}

/// `/api/kb/status` 的杂志一节：本数、条数、两遍向量进度、最近同步的书。
#[derive(Debug, Clone, Serialize, Default)]
pub struct MagazineStatus {
    pub books: i64,
    pub docs: i64,
    pub eligible: i64,
    pub fused_done: i64,
    /// 纯图向量按不同的图算
    pub pure_target: i64,
    pub pure_done: i64,
    /// 最近入库的书（按条目 id 最大）
    pub last_book: Option<String>,
}

pub fn status(conn: &Connection, model: &str) -> Result<MagazineStatus> {
    let (books, docs, eligible, fused_done) = conn.query_row(
        "SELECT COUNT(DISTINCT i.book_key), COUNT(*),
                COALESCE(SUM(i.vector_eligible), 0),
                COALESCE(SUM(i.vector_eligible = 1 AND d.embed_model = ?1), 0)
         FROM kb_doc_images i JOIN kb_docs d ON d.id = i.doc_id",
        [model],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let (pure_target, pure_done) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(done), 0) FROM (
           SELECT blake3, MAX(image_embed_model = ?1) AS done
           FROM kb_doc_images WHERE vector_eligible = 1 GROUP BY blake3)",
        [model],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let last_book = conn
        .query_row(
            "SELECT book_key FROM kb_doc_images ORDER BY doc_id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(MagazineStatus {
        books,
        docs,
        eligible,
        fused_done,
        pure_target,
        pure_done,
        last_book,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::{self, KbDoc};
    use crate::magazine::{ingest_lines, tests::line};
    use crate::vectors::{DocVector, ImageVector};

    const M: &str = "m";
    const DIM: i32 = 8;

    fn unit(i: usize) -> Vec<f32> {
        let mut v = vec![0.0; DIM as usize];
        v[i % DIM as usize] = 1.0;
        v
    }

    /// 两本书：b1 三条（TNF，第 0、1 条同一张图）、b2 一条（snow peak）
    fn conn_with_books() -> (Connection, Tokenizer) {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let tok = Tokenizer::new();
        let b1: Vec<_> = (0..3)
            .map(|i| {
                line(
                    "b1",
                    "T",
                    i,
                    0,
                    Some("双肩包 背负系统"),
                    "TNF",
                    if i < 2 { 1 } else { 2 },
                )
            })
            .collect();
        ingest_lines(&c, &tok, "b1", &b1).unwrap();
        let mut l = line("b2", "U", 5, 0, Some("帐篷 双肩包"), "SNOW PEAK", 3);
        l.issue_date = Some("2026-01-01".into());
        ingest_lines(&c, &tok, "b2", &[l]).unwrap();
        (c, tok)
    }

    fn ids(c: &Connection) -> Vec<i64> {
        let mut st = c
            .prepare("SELECT doc_id FROM kb_doc_images ORDER BY doc_id")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// 护栏 6：同一本书能返回多条；按书筛
    #[test]
    fn 文字检索同一本返回多条_按书筛() {
        let (c, tok) = conn_with_books();
        let q = TextQuery {
            text: "双肩包",
            limit: 10,
            ..Default::default()
        };
        let hits = fuse(&c, &tok, &q, &[], M).unwrap();
        assert_eq!(hits.len(), 4);
        assert_eq!(hits.iter().filter(|h| h.item.book_key == "b1").count(), 3);
        assert!(hits.iter().all(|h| h.routes == vec!["fts"]));
        let q = TextQuery {
            book_key: Some("b2"),
            ..q
        };
        let hits = fuse(&c, &tok, &q, &[], M).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.brand, "SNOW PEAK");
    }

    #[test]
    fn 两路都命中的排前面() {
        let (c, tok) = conn_with_books();
        let all = ids(&c);
        // 向量路只命中 b2 那条
        let q = TextQuery {
            text: "双肩包",
            limit: 10,
            ..Default::default()
        };
        let hits = fuse(&c, &tok, &q, &[(all[3], 0.9)], M).unwrap();
        assert_eq!(hits[0].item.id, all[3]);
        assert_eq!(hits[0].routes, vec!["fts", "vector"]);
        assert_eq!(hits[0].similarity, Some(0.9));
    }

    /// 护栏 2：以图搜图命中经 kb_doc_images 回到条目与 OSS 地址；候选图丢掉
    #[tokio::test]
    async fn 以图搜图回到条目() {
        let (c, _) = conn_with_books();
        let dir = tempdir::TempDir::new("magsearch").unwrap();
        let store = VectorStore::open(dir.path(), DIM, M).await.unwrap();
        let b3 = |n: u8| format!("{:064x}", n);
        store
            .put_images(&[
                ImageVector {
                    blake3: b3(1),
                    vector: unit(0),
                },
                ImageVector {
                    blake3: b3(3),
                    vector: unit(1),
                },
                // 候选图，不属于任何杂志条目
                ImageVector {
                    blake3: b3(99),
                    vector: unit(0),
                },
            ])
            .await
            .unwrap();
        let raw = search_image(&store, &unit(0), 10).await.unwrap();
        let hits = image_hits(&c, &raw, 10, M).unwrap();
        let top: Vec<_> = hits.iter().take(2).collect();
        assert!(
            top.iter().all(|h| h.item.image_blake3 == b3(1)),
            "同一张图的两条都在最前"
        );
        assert_eq!(top.len(), 2);
        assert!(top[0].item.image_url.ends_with(&format!("{}.jpg", b3(1))));
        assert!(hits.iter().all(|h| h.item.image_blake3 != b3(99)));
        assert_eq!(top[0].routes, vec!["image"]);
        assert!(top[0].similarity.unwrap() > 0.99);
    }

    #[test]
    fn 详情与品牌页与状态() {
        let (c, _) = conn_with_books();
        let all = ids(&c);
        let it = get(&c, all[0], M).unwrap().unwrap();
        assert_eq!((it.book_key.as_str(), it.pdf_index), ("b1", 0));
        assert_eq!(it.bbox, serde_json::json!([1.0, 2.0, 3.0, 4.0]));
        assert!(!it.fused_ready);
        // 参考库条目不是杂志
        let r = docs::upsert(
            &c,
            &KbDoc {
                kind: "example".into(),
                ref_id: "e1".into(),
                body: "x".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(get(&c, r.id, M).unwrap().is_none());

        let sp = by_brand(&c, "snowpeak", 10, M).unwrap();
        assert_eq!(sp.len(), 1);
        assert_eq!(by_brand(&c, "TNF", 10, M).unwrap().len(), 3);

        c.execute(
            "UPDATE kb_docs SET embed_model = 'm' WHERE id = ?1",
            [all[0]],
        )
        .unwrap();
        c.execute(
            "UPDATE kb_doc_images SET image_embed_model = 'm' WHERE doc_id = ?1",
            [all[0]],
        )
        .unwrap();
        let s = status(&c, M).unwrap();
        assert_eq!((s.books, s.docs, s.eligible, s.fused_done), (2, 4, 4, 1));
        assert_eq!((s.pure_target, s.pure_done), (3, 1), "纯图按不同的图算");
        assert_eq!(s.last_book.as_deref(), Some("b2"));
    }

    /// 护栏 1：库里塞 10 万条杂志向量（都比参考库更近），按四类检索 top-K 一条不丢——
    /// 证明 LanceDB `only_if` 是前置过滤。若是后置过滤，前 K 全是杂志，筛完就空了。
    #[tokio::test]
    async fn 十万条杂志不挤掉四类检索() {
        let dir = tempdir::TempDir::new("only_if").unwrap();
        let store = VectorStore::open(dir.path(), DIM, M).await.unwrap();
        let near = |j: usize| {
            let mut v = unit(0);
            v[1] = 0.001 * (j % 7) as f32;
            v
        };
        for chunk in (0..100_000).collect::<Vec<usize>>().chunks(25_000) {
            let rows: Vec<DocVector> = chunk
                .iter()
                .map(|&j| DocVector {
                    key: format!("kb:{MAGAZINE}:{j}"),
                    kind: MAGAZINE.into(),
                    doc_id: j as i64,
                    post_id: String::new(),
                    brand: String::new(),
                    at: String::new(),
                    is_reference: false,
                    vector: near(j),
                })
                .collect();
            store.put_docs(&rows).await.unwrap();
        }
        let refs: Vec<DocVector> = (0..5)
            .map(|j| {
                let mut v = unit(0);
                v[2] = 0.5 + j as f32 * 0.1; // 比杂志远
                DocVector {
                    key: format!("kb:published_item:{j}"),
                    kind: "published_item".into(),
                    doc_id: 1_000_000 + j,
                    post_id: format!("p{j}"),
                    brand: String::new(),
                    at: String::new(),
                    is_reference: false,
                    vector: v,
                }
            })
            .collect();
        store.put_docs(&refs).await.unwrap();
        let four: Vec<String> = docs::KbKind::ALL
            .iter()
            .map(|k| k.as_str().to_string())
            .collect();
        let hits = store
            .search_docs(
                &unit(0),
                &DocFilter {
                    kinds: four,
                    ..Default::default()
                },
                5,
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 5, "四类 top-5 一条不丢");
        assert!(hits.iter().all(|h| h.kind == "published_item"));
        // 不加过滤时前 5 全是杂志（说明上面确实是前置过滤起的作用）
        let raw = store
            .search_docs(&unit(0), &DocFilter::default(), 5)
            .await
            .unwrap();
        assert!(raw.iter().all(|h| h.kind == MAGAZINE));
    }
}
