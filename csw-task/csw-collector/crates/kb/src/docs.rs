//! 参考库文档的结构化那一半：SQLite 里的 `kb_docs`。
//!
//! 一条 `KbDoc` 在三个地方各有一份身份：
//! - `kb_docs` 行 —— 正文、标题、品牌、发布状态这些要展示与过滤的字段；
//! - `kb_fts` 行 —— 预分词后的词元串（辅路检索）；
//! - LanceDB `docs` 行 —— 向量（主路检索）。
//!
//! 三份的对齐靠 `kb_docs.id`。**增量同步的关键是 `content_hash`**：
//! 它只覆盖真正被嵌入与索引的那部分内容（标题 + 正文），所以
//! 「发布状态从草稿变成已发」这种改动会更新行、但**不会**白白重算一次向量。
//! 一次全量重建要两小时，这个区分不是优化，是能不能每天跑的分界。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use csw_collector_core::types::MaterialKind;

/// 参考库的四类。
///
/// 刻意不复用 [`MaterialKind`]：那是**五**类对照材料，多出来的「上一轮台账」
/// 永远不进参考库（否则系统自己的判断会被当成 Van 的口味证据）。
/// 两个集合不一样大，硬并成一个枚举就得在某处撒谎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbKind {
    /// 正式已发布的条目
    PublishedItem,
    /// 知识库范例
    Example,
    /// 生成过文章的贴文
    GeneratedPost,
    /// 03 阶段 Van 的决定（含原话）
    Decision,
}

impl KbKind {
    pub const ALL: [KbKind; 4] = [
        Self::PublishedItem,
        Self::Example,
        Self::GeneratedPost,
        Self::Decision,
    ];

    /// 库里存的那个字符串。**改这里要连带改迁移的注释与 API.md。**
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PublishedItem => "published_item",
            Self::Example => "example",
            Self::GeneratedPost => "generated_post",
            Self::Decision => "decision",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl From<KbKind> for MaterialKind {
    fn from(k: KbKind) -> Self {
        match k {
            KbKind::PublishedItem => Self::Published,
            KbKind::Example => Self::Example,
            KbKind::GeneratedPost => Self::GeneratedPost,
            KbKind::Decision => Self::Decision,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct KbDoc {
    /// 入库后才有；新建时是 0
    pub id: i64,
    pub kind: String,
    /// 来源侧的稳定标识：已发条目 id、范例编号、贴文短码、决定 id
    pub ref_id: String,
    pub platform: String,
    /// 同一贴文的多种身份合并到一行（已发条目与「生成过文章的贴文」常是同一条）
    pub post_id: String,
    pub title: String,
    pub body: String,
    pub url: String,
    pub brand: String,
    /// RFC3339；不知道就 None
    pub published_at: Option<String>,
    pub publish_state: String,
    /// Van 标过「可作范例」
    pub is_reference: bool,
    /// 已经算过向量的模型标识；空 = 还没算
    pub embed_model: String,
}

impl KbDoc {
    /// 要被嵌入与全文索引的那段文字。标题在前，检索时标题里的品牌权重自然更高。
    pub fn text(&self) -> String {
        if self.title.is_empty() {
            self.body.clone()
        } else {
            format!("{}\n{}", self.title, self.body)
        }
    }

    /// 内容指纹。**只覆盖 [`Self::text`]**——见模块文档。
    pub fn content_hash(&self) -> String {
        blake3::hash(self.text().as_bytes()).to_hex().to_string()
    }

    /// LanceDB 里的键。与 [`crate::vectors::DocVector::key`] 同一套构造。
    pub fn vector_key(&self) -> String {
        format!("kb:{}:{}", self.kind, self.ref_id)
    }
}

/// 写入结果：给调用方判断「要不要重算向量、要不要重建全文索引」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upserted {
    pub id: i64,
    /// 这是新行
    pub inserted: bool,
    /// 被嵌入与索引的内容变了（新行也算变）
    pub content_changed: bool,
}

impl Upserted {
    /// 要重算向量、重建全文索引吗
    pub fn needs_reindex(&self) -> bool {
        self.content_changed
    }
}

/// 按 `(kind, ref_id)` 写入或更新。
///
/// 内容没变就只更新元数据，并且**保留 `embed_model`**——那正是「不用重算向量」
/// 这件事在库里的记号。内容变了就把 `embed_model` 清空，下一轮同步会重算。
pub fn upsert(conn: &Connection, doc: &KbDoc) -> Result<Upserted> {
    anyhow::ensure!(!doc.kind.is_empty(), "kb_docs.kind 不能空");
    anyhow::ensure!(!doc.ref_id.is_empty(), "kb_docs.ref_id 不能空");
    let hash = doc.content_hash();
    let now = jiff::Timestamp::now().to_string();

    let old: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, content_hash FROM kb_docs WHERE kind = ?1 AND ref_id = ?2",
            params![doc.kind, doc.ref_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .context("查 kb_docs 现有行")?;

    let Some((id, old_hash)) = old else {
        conn.execute(
            "INSERT INTO kb_docs(kind, ref_id, platform, post_id, title, body, url, brand,
                                 published_at, publish_state, is_reference, content_hash,
                                 embed_model, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'',?13)",
            params![
                doc.kind,
                doc.ref_id,
                doc.platform,
                doc.post_id,
                doc.title,
                doc.body,
                doc.url,
                doc.brand,
                doc.published_at,
                doc.publish_state,
                i64::from(doc.is_reference),
                hash,
                now,
            ],
        )
        .context("插 kb_docs")?;
        return Ok(Upserted {
            id: conn.last_insert_rowid(),
            inserted: true,
            content_changed: true,
        });
    };

    let changed = old_hash != hash;
    conn.execute(
        "UPDATE kb_docs SET platform=?2, post_id=?3, title=?4, body=?5, url=?6, brand=?7,
                            published_at=?8, publish_state=?9, is_reference=?10, content_hash=?11,
                            embed_model = CASE WHEN ?12 THEN '' ELSE embed_model END
         WHERE id = ?1",
        params![
            id,
            doc.platform,
            doc.post_id,
            doc.title,
            doc.body,
            doc.url,
            doc.brand,
            doc.published_at,
            doc.publish_state,
            i64::from(doc.is_reference),
            hash,
            changed,
        ],
    )
    .context("更新 kb_docs")?;
    Ok(Upserted {
        id,
        inserted: false,
        content_changed: changed,
    })
}

const COLS: &str = "id, kind, ref_id, platform, post_id, title, body, url, brand,
                    published_at, publish_state, is_reference, embed_model";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<KbDoc> {
    Ok(KbDoc {
        id: r.get(0)?,
        kind: r.get(1)?,
        ref_id: r.get(2)?,
        platform: r.get(3)?,
        post_id: r.get(4)?,
        title: r.get(5)?,
        body: r.get(6)?,
        url: r.get(7)?,
        brand: r.get(8)?,
        published_at: r.get(9)?,
        publish_state: r.get(10)?,
        is_reference: r.get::<_, i64>(11)? == 1,
        embed_model: r.get(12)?,
    })
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<KbDoc>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM kb_docs WHERE id = ?1"),
            [id],
            row,
        )
        .optional()?)
}

/// 按 id 批量取。**返回顺序与传入的 ids 一致**——调用方给的顺序往往就是排序结果，
/// 按库里的顺序返回会把排好的序打乱，那种错很难看出来。
pub fn get_many(conn: &Connection, ids: &[i64]) -> Result<Vec<KbDoc>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let holes = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM kb_docs WHERE id IN ({holes})"))?;
    let got: Vec<KbDoc> = st
        .query_map(rusqlite::params_from_iter(ids.iter()), row)?
        .filter_map(Result::ok)
        .collect();
    Ok(ids
        .iter()
        .filter_map(|id| got.iter().find(|d| d.id == *id).cloned())
        .collect())
}

/// 链接比对用：去掉查询串、片段与末尾斜杠，主机名小写；Instagram 的 `/reel/`、`/tv/`
/// 与 `/p/` 指同一条，统一成 `/p/{短码}`。**短码区分大小写**，不小写。
pub fn norm_url(u: &str) -> String {
    let u = u.trim();
    let u = u
        .split(['?', '#'])
        .next()
        .unwrap_or(u)
        .trim_end_matches('/');
    if let Some(rest) = u.split("instagram.com/").nth(1) {
        let mut parts = rest.split('/').filter(|s| !s.is_empty());
        if let (Some(kind), Some(code)) = (parts.next(), parts.next())
            && matches!(kind, "p" | "reel" | "reels" | "tv")
        {
            return format!("https://www.instagram.com/p/{code}");
        }
    }
    u.to_lowercase()
}

/// 品牌召回路：这个品牌下最近的若干条。
///
/// 按发布时间倒序，**没有发布时间的排在最后**——`published_at` 为空的多是范例
/// 与决定，它们本来就不该挤掉近期的已发条目。
///
/// 品牌按 [`crate::brands::brand_key`] 比：`SNOW PEAK` 与 `snowpeak` 算同一个。
/// 键在 Rust 里算（SQLite 里去不干净 Unicode 标点），所以先捞出库里所有写法、
/// 挑出键相同的那几种再查。去重后的品牌写法只有一两千种，每次扫一遍是毫秒级。
pub fn by_brand(conn: &Connection, brand: &str, limit: usize) -> Result<Vec<KbDoc>> {
    let key = crate::brands::brand_key(brand);
    if key.is_empty() {
        return Ok(Vec::new());
    }
    let spellings: Vec<String> = conn
        .prepare("SELECT DISTINCT brand FROM kb_docs WHERE brand <> ''")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|b| crate::brands::brand_key(b) == key)
        .collect();
    if spellings.is_empty() {
        return Ok(Vec::new());
    }
    let holes = std::iter::repeat_n("?", spellings.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM kb_docs WHERE brand IN ({holes})
         ORDER BY published_at IS NULL, published_at DESC LIMIT ?"
    ))?;
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = spellings
        .into_iter()
        .map(|b| Box::new(b) as Box<dyn rusqlite::ToSql>)
        .collect();
    args.push(Box::new(limit as i64));
    Ok(st
        .query_map(
            rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
            row,
        )?
        .filter_map(Result::ok)
        .collect())
}

/// 还没用当前模型算过向量的行。空的 `embed_model` 与「用别的模型算的」一样都要重算。
pub fn needs_embedding(conn: &Connection, model: &str, limit: usize) -> Result<Vec<KbDoc>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM kb_docs WHERE embed_model <> ?1 ORDER BY id LIMIT ?2"
    ))?;
    Ok(st
        .query_map(params![model, limit as i64], row)?
        .filter_map(Result::ok)
        .collect())
}

/// 记下这些行已经用 `model` 算过向量了。
pub fn mark_embedded(conn: &Connection, ids: &[i64], model: &str) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let holes = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(model.to_string())];
    args.extend(ids.iter().map(|i| Box::new(*i) as Box<dyn rusqlite::ToSql>));
    Ok(conn.execute(
        &format!("UPDATE kb_docs SET embed_model = ? WHERE id IN ({holes})"),
        rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
    )?)
}

/// 各类各多少条，给「知识库」页与同步后的自检用。
pub fn counts_by_kind(conn: &Connection) -> Result<Vec<(String, usize)>> {
    let mut st = conn.prepare("SELECT kind, COUNT(*) FROM kb_docs GROUP BY kind ORDER BY kind")?;
    Ok(st
        .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?
        .filter_map(Result::ok)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        csw_collector_core::store::open_in_memory().unwrap()
    }

    fn d(kind: KbKind, ref_id: &str, title: &str, body: &str) -> KbDoc {
        KbDoc {
            kind: kind.as_str().into(),
            ref_id: ref_id.into(),
            title: title.into(),
            body: body.into(),
            brand: "山と道".into(),
            post_id: format!("p-{ref_id}"),
            published_at: Some("2026-09-18T00:00:00Z".into()),
            ..Default::default()
        }
    }

    #[test]
    fn 四类的库内写法要和迁移里写的一致() {
        // 改了任意一个都要连带改 0001_init.sql 的注释与 API.md
        let all: Vec<&str> = KbKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            all,
            ["published_item", "example", "generated_post", "decision"]
        );
        assert_eq!(KbKind::parse("example"), Some(KbKind::Example));
        assert_eq!(KbKind::parse("prior_ledger"), None, "上一轮台账不进参考库");
    }

    #[test]
    fn 新行要重算旧行要看内容有没有变() {
        let c = conn();
        let mut doc = d(KbKind::Example, "ex-1", "新包上市", "正文一");

        let first = upsert(&c, &doc).unwrap();
        assert!(first.inserted && first.needs_reindex());

        // 原样再写一遍：不是新行，也不用重算
        let again = upsert(&c, &doc).unwrap();
        assert_eq!(again.id, first.id);
        assert!(!again.inserted);
        assert!(!again.needs_reindex(), "内容没变不该重算向量");

        // 只改发布状态这类元数据：更新行，但不该触发重算
        doc.publish_state = "published".into();
        let meta_only = upsert(&c, &doc).unwrap();
        assert!(
            !meta_only.needs_reindex(),
            "改发布状态不该白白重算一次向量——一次全量重建要两小时"
        );
        assert_eq!(
            get(&c, first.id).unwrap().unwrap().publish_state,
            "published"
        );

        // 改正文：必须重算
        doc.body = "正文二".into();
        assert!(upsert(&c, &doc).unwrap().needs_reindex());
    }

    #[test]
    fn 内容变了要把已算过的记号清掉() {
        let c = conn();
        let mut doc = d(KbKind::Example, "ex-1", "标题", "正文一");
        let id = upsert(&c, &doc).unwrap().id;
        mark_embedded(&c, &[id], "qwen3-vl").unwrap();
        assert_eq!(get(&c, id).unwrap().unwrap().embed_model, "qwen3-vl");

        // 元数据改动保留记号
        doc.url = "https://example.com/a".into();
        upsert(&c, &doc).unwrap();
        assert_eq!(
            get(&c, id).unwrap().unwrap().embed_model,
            "qwen3-vl",
            "元数据改动不该让它重排队"
        );

        // 正文改动清掉记号，下一轮同步会重算
        doc.body = "正文二".into();
        upsert(&c, &doc).unwrap();
        assert_eq!(get(&c, id).unwrap().unwrap().embed_model, "");
        assert_eq!(needs_embedding(&c, "qwen3-vl", 10).unwrap().len(), 1);
    }

    #[test]
    fn 同一个refid在不同类下是两条() {
        let c = conn();
        // 已发条目与「生成过文章的贴文」常常指向同一个短码，它们是两条不同的对照材料
        let a = upsert(&c, &d(KbKind::PublishedItem, "abc123", "标题", "正文")).unwrap();
        let b = upsert(&c, &d(KbKind::GeneratedPost, "abc123", "标题", "正文")).unwrap();
        assert_ne!(a.id, b.id);
        assert!(b.inserted);
    }

    #[test]
    fn 待重算的挑出来标记完就空了() {
        let c = conn();
        let ids: Vec<i64> = (1..=3)
            .map(|i| {
                upsert(&c, &d(KbKind::Example, &format!("ex-{i}"), "t", "b"))
                    .unwrap()
                    .id
            })
            .collect();
        assert_eq!(needs_embedding(&c, "qwen3-vl", 10).unwrap().len(), 3);

        assert_eq!(mark_embedded(&c, &ids, "qwen3-vl").unwrap(), 3);
        assert!(needs_embedding(&c, "qwen3-vl", 10).unwrap().is_empty());

        // 换了模型：全部要重算，空的 embed_model 与「别的模型算的」一样对待
        assert_eq!(needs_embedding(&c, "别的模型", 10).unwrap().len(), 3);
    }

    #[test]
    fn 批量取回要保持传入的顺序() {
        let c = conn();
        let ids: Vec<i64> = (1..=3)
            .map(|i| {
                upsert(&c, &d(KbKind::Example, &format!("ex-{i}"), "t", "b"))
                    .unwrap()
                    .id
            })
            .collect();
        // 调用方给的顺序往往就是排好的检索结果，按库里的顺序返回会把它打乱
        let want = vec![ids[2], ids[0], ids[1]];
        let got: Vec<i64> = get_many(&c, &want).unwrap().iter().map(|d| d.id).collect();
        assert_eq!(got, want);

        // 不存在的 id 直接跳过，不占位也不报错
        let mixed = get_many(&c, &[ids[1], 99999]).unwrap();
        assert_eq!(mixed.len(), 1);
        assert_eq!(mixed[0].id, ids[1]);
        assert!(get_many(&c, &[]).unwrap().is_empty());
    }

    #[test]
    fn 品牌召回里没有发布时间的排最后() {
        let c = conn();
        let mut old = d(KbKind::PublishedItem, "p-old", "旧", "b");
        old.published_at = Some("2026-08-01T00:00:00Z".into());
        let mut recent = d(KbKind::PublishedItem, "p-new", "新", "b");
        recent.published_at = Some("2026-09-20T00:00:00Z".into());
        // 范例与决定多半没有发布时间，它们不该挤掉近期的已发条目
        let mut undated = d(KbKind::Example, "ex-1", "范例", "b");
        undated.published_at = None;
        for x in [&undated, &old, &recent] {
            upsert(&c, x).unwrap();
        }

        let got: Vec<String> = by_brand(&c, "山と道", 10)
            .unwrap()
            .into_iter()
            .map(|d| d.title)
            .collect();
        assert_eq!(got, ["新", "旧", "范例"]);
        assert!(by_brand(&c, "别的牌子", 10).unwrap().is_empty());
    }

    #[test]
    fn 向量键与内容指纹() {
        let doc = d(KbKind::Decision, "dec-7", "标题", "正文");
        assert_eq!(doc.vector_key(), "kb:decision:dec-7");
        assert_eq!(doc.text(), "标题\n正文");

        // 指纹只看被嵌入的那段文字：换品牌不动它，换正文动它
        let mut other = doc.clone();
        other.brand = "别的牌子".into();
        assert_eq!(doc.content_hash(), other.content_hash());
        other.body = "改了".into();
        assert_ne!(doc.content_hash(), other.content_hash());

        // 没标题就只有正文，不要留一个空行在前面
        let mut untitled = doc.clone();
        untitled.title = String::new();
        assert_eq!(untitled.text(), "正文");
    }

    #[test]
    fn 空的类或refid直接拒绝() {
        let c = conn();
        let mut bad = d(KbKind::Example, "", "t", "b");
        assert!(upsert(&c, &bad).is_err());
        bad.ref_id = "ex-1".into();
        bad.kind = String::new();
        assert!(upsert(&c, &bad).is_err());
    }

    #[test]
    fn 按类计数() {
        let c = conn();
        upsert(&c, &d(KbKind::Example, "ex-1", "t", "b")).unwrap();
        upsert(&c, &d(KbKind::Example, "ex-2", "t", "b2")).unwrap();
        upsert(&c, &d(KbKind::Decision, "dec-1", "t", "b3")).unwrap();
        assert_eq!(
            counts_by_kind(&c).unwrap(),
            [("decision".to_string(), 1), ("example".to_string(), 2)]
        );
    }

    #[test]
    fn 品牌路认得两种写法() {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        for (kind, r, brand) in [
            ("generated_post", "g1", "SNOW PEAK"),
            ("decision", "d1", "snowpeak"),
            ("decision", "d2", "snow-peak-japan"),
        ] {
            upsert(
                &conn,
                &KbDoc {
                    kind: kind.into(),
                    ref_id: r.into(),
                    brand: brand.into(),
                    body: r.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let got: Vec<String> = by_brand(&conn, "SNOW PEAK", 8)
            .unwrap()
            .into_iter()
            .map(|d| d.ref_id)
            .collect();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.contains(&"d1".to_string()));
        assert!(by_brand(&conn, "", 8).unwrap().is_empty());
    }
}
