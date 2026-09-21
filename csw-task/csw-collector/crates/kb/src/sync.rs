//! 知识库同步：把四类对照材料从引擎与 csw 拉进本地库。
//!
//! ## 粒度是想清楚的，不是顺手定的
//!
//! - **第一类「正式已发布的条目」按条目**（拆条）建 doc：查重问的是
//!   「这条资讯我们发过没有」，不是「这一篇我们发过没有」。一篇合集里的五条
//!   各有各的品牌与角度，合成一条就查不准了。
//! - **第二类「范例」按整篇**建 doc：范例的价值在**写法**，而写法是整篇的属性。
//!   所以范例的 doc 带整篇正文，而已发条目的 doc 只带条目自己的字段——
//!   否则同一篇里的五个条目会拿到五个几乎一样的向量（都被那段共享正文主导），
//!   检索时它们会一起涌上来，把别的材料挤掉。
//!
//! ## 增量靠重叠窗口，不靠精确游标
//!
//! 引擎的 `since` 过滤的是 `published_at`，**不是「改动时间」**：一篇旧文后来被
//! 标成范例、或状态从草稿变成已发，按 published_at 的游标就永远拉不到它。
//! 所以每次都往回多拉 [`OVERLAP_DAYS`] 天。重拉不贵——`content_hash` 没变的行
//! 只更新元数据，不会重算向量。真正的补漏靠偶尔一次 `--full`。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use csw_collector_core::vector::{EmbedInput, VectorClient};
use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::{Decision, LedgerPost};
use csw_collector_harvest::csw::{CswClient, RawPost};

use crate::brands::BrandIndex;
use crate::docs::{self, KbDoc, KbKind};
use crate::fts::{self, Tokenizer};
use crate::vectors::{DocVector, VectorStore};

pub const SRC_LEDGER_POSTS: &str = "ledger_posts";
pub const SRC_LEDGER_DECISIONS: &str = "ledger_decisions";
pub const SRC_CSW_GENERATED: &str = "csw_generated";
pub const SRC_EXAMPLES: &str = "examples";

/// 每次往回多拉多少天。见模块文档「增量靠重叠窗口」。
pub const OVERLAP_DAYS: i64 = 45;
/// `--full` 时往回拉多少天（十年，等于全量）
pub const FULL_DAYS: i64 = 3650;
/// 一次决定拉多少条
const DECISIONS_LIMIT: u32 = 500;
/// csw 已生成贴文最多翻多少页（每页 100）
const GENERATED_MAX_PAGES: u32 = 30;
/// 一次向量化处理多少条。GPU 串行，批太大只是让失败重来得更贵。
pub const EMBED_BATCH: usize = 32;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceReport {
    pub source: String,
    /// 从上游拿到多少条
    pub fetched: usize,
    pub inserted: usize,
    /// 内容变了、要重算向量的
    pub changed: usize,
    /// 拿到了但内容没变
    pub unchanged: usize,
    /// 上游给了但缺关键字段，跳过
    pub skipped: usize,
}

impl SourceReport {
    fn new(source: &str) -> Self {
        Self {
            source: source.into(),
            ..Default::default()
        }
    }
    fn record(&mut self, r: &docs::Upserted) {
        if r.inserted {
            self.inserted += 1;
        }
        if r.content_changed {
            self.changed += 1;
        } else {
            self.unchanged += 1;
        }
    }
}

// ───────────────────────────── 游标 ─────────────────────────────

pub fn cursor(conn: &Connection, source: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT cursor FROM kb_cursors WHERE source = ?1",
            [source],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .filter(|s| !s.is_empty()))
}

pub fn set_cursor(conn: &Connection, source: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO kb_cursors(source, cursor, synced_at) VALUES (?1,?2,?3)
         ON CONFLICT(source) DO UPDATE SET cursor=excluded.cursor, synced_at=excluded.synced_at",
        params![source, value, jiff::Timestamp::now().to_string()],
    )?;
    Ok(())
}

/// 这一轮该从哪天开始拉。`full` 时等于全量。
fn since(conn: &Connection, source: &str, full: bool) -> Result<String> {
    let days = if full {
        FULL_DAYS
    } else {
        match cursor(conn, source)? {
            Some(_) => OVERLAP_DAYS,
            // 没有游标 = 第一次同步，直接全量
            None => FULL_DAYS,
        }
    };
    Ok(format!("{days}d"))
}

// ─────────────────────── 第一类 + 第二类：台账 ───────────────────────

pub async fn sync_ledger_posts(
    conn: &Connection,
    engine: &EngineClient,
    tok: &Tokenizer,
    full: bool,
) -> Result<SourceReport> {
    let since = since(conn, SRC_LEDGER_POSTS, full)?;
    let resp = engine
        .ledger_posts(&since, "", true)
        .await
        .context("拉台账发布记录")?;
    let mut rep = SourceReport::new(SRC_LEDGER_POSTS);
    rep.fetched = resp.posts.len();
    for p in &resp.posts {
        for doc in ledger_docs(p) {
            let r = docs::upsert(conn, &doc)?;
            if r.content_changed {
                fts::index_doc(conn, r.id, tok, &doc.text())?;
            }
            rep.record(&r);
        }
    }
    set_cursor(conn, SRC_LEDGER_POSTS, &jiff::Timestamp::now().to_string())?;
    Ok(rep)
}

/// 一篇台账记录拆成几条 kb_doc。见模块文档「粒度」。
fn ledger_docs(p: &LedgerPost) -> Vec<KbDoc> {
    let mut out = Vec::new();

    // 范例：整篇一条，带全文。写法是整篇的属性。
    if p.is_reference && !p.body_text.trim().is_empty() {
        out.push(KbDoc {
            kind: KbKind::Example.as_str().into(),
            ref_id: p.post_id.clone(),
            platform: p.platform.clone(),
            post_id: p.post_id.clone(),
            title: p.title.clone(),
            body: p.body_text.clone(),
            url: p.url.clone(),
            brand: p.items.first().map(|i| i.brand.clone()).unwrap_or_default(),
            published_at: non_empty(&p.published_at),
            publish_state: p.state.clone(),
            is_reference: true,
            ..Default::default()
        });
    }

    // 已发条目：一条拆条一个 doc，只带条目自己的字段
    for it in &p.items {
        if it.item_key.trim().is_empty() {
            continue;
        }
        out.push(KbDoc {
            kind: KbKind::PublishedItem.as_str().into(),
            ref_id: it.item_key.clone(),
            platform: p.platform.clone(),
            post_id: p.post_id.clone(),
            title: it.title.clone(),
            body: item_body(it),
            url: if it.source_url.is_empty() {
                p.url.clone()
            } else {
                it.source_url.clone()
            },
            brand: it.brand.clone(),
            published_at: non_empty(&p.published_at),
            publish_state: p.state.clone(),
            is_reference: p.is_reference,
            ..Default::default()
        });
    }

    // 没有拆条的篇：整篇一条，别让它整个消失
    if p.items.is_empty() && !p.is_reference && !p.post_id.trim().is_empty() {
        out.push(KbDoc {
            kind: KbKind::PublishedItem.as_str().into(),
            ref_id: p.post_id.clone(),
            platform: p.platform.clone(),
            post_id: p.post_id.clone(),
            title: p.title.clone(),
            body: p.body_text.clone(),
            url: p.url.clone(),
            published_at: non_empty(&p.published_at),
            publish_state: p.state.clone(),
            ..Default::default()
        });
    }
    out
}

fn item_body(it: &csw_collector_engineapi::types::LedgerPostItem) -> String {
    [
        it.brand.as_str(),
        it.product.as_str(),
        it.angle.as_str(),
        it.title.as_str(),
    ]
    .iter()
    .filter(|s| !s.trim().is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join("｜")
}

// ─────────────────────── 第四类：03 决定 ───────────────────────

/// 决定里带 Van 的原话。原话只进**本地**库、只送**本机** GPU 算向量，
/// 不出网、不送第三方——这条在总方案里是硬约束。
pub async fn sync_decisions(
    conn: &Connection,
    engine: &EngineClient,
    tok: &Tokenizer,
    full: bool,
) -> Result<SourceReport> {
    let since = since(conn, SRC_LEDGER_DECISIONS, full)?;
    let list = engine
        .ledger_decisions(&since, DECISIONS_LIMIT)
        .await
        .context("拉历史决定")?;
    let mut rep = SourceReport::new(SRC_LEDGER_DECISIONS);
    rep.fetched = list.len();
    for d in &list {
        let Some(doc) = decision_doc(d) else {
            rep.skipped += 1;
            continue;
        };
        let r = docs::upsert(conn, &doc)?;
        if r.content_changed {
            fts::index_doc(conn, r.id, tok, &doc.text())?;
        }
        rep.record(&r);
    }
    set_cursor(
        conn,
        SRC_LEDGER_DECISIONS,
        &jiff::Timestamp::now().to_string(),
    )?;
    Ok(rep)
}

fn decision_doc(d: &Decision) -> Option<KbDoc> {
    if d.item_key.trim().is_empty() {
        return None;
    }
    // 同一个 item_key 在不同 run 里可能被决定过两次，run_id 进键才不会互相覆盖
    let ref_id = format!("{}#{}", d.run_id, d.item_key);
    let body = [
        format!("结论：{}", d.status),
        non_blank("品牌", &d.brand),
        non_blank("理由", &d.reason),
        non_blank("理由码", &d.reason_code),
        // 原话是这一类材料的核心，没有它就只剩一个结论标签
        non_blank("原话", &d.quote_ref),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    Some(KbDoc {
        kind: KbKind::Decision.as_str().into(),
        ref_id,
        title: d.title.clone(),
        body,
        url: d.source_url.clone(),
        brand: d.brand.clone(),
        published_at: non_empty(&d.published_at),
        ..Default::default()
    })
}

fn non_blank(label: &str, v: &str) -> String {
    if v.trim().is_empty() {
        String::new()
    } else {
        format!("{label}：{v}")
    }
}

// ─────────────────── 第三类：生成过文章的贴文 ───────────────────

/// csw 的 `posts/generated`。这一类**没有增量接口**，每次都全量翻页；
/// 一千九百条翻三十页，几秒钟的事，不值得为它做游标。
pub async fn sync_generated(
    conn: &Connection,
    csw: &CswClient,
    tok: &Tokenizer,
    brands: &BrandIndex,
) -> Result<SourceReport> {
    let posts = csw
        .generated(GENERATED_MAX_PAGES)
        .await
        .context("拉已生成贴文")?;
    let mut rep = SourceReport::new(SRC_CSW_GENERATED);
    rep.fetched = posts.len();
    for p in &posts {
        let Some(doc) = generated_doc(p, brands) else {
            rep.skipped += 1;
            continue;
        };
        let r = docs::upsert(conn, &doc)?;
        if r.content_changed {
            fts::index_doc(conn, r.id, tok, &doc.text())?;
        }
        rep.record(&r);
    }
    set_cursor(conn, SRC_CSW_GENERATED, &jiff::Timestamp::now().to_string())?;
    Ok(rep)
}

fn generated_doc(p: &RawPost, brands: &BrandIndex) -> Option<KbDoc> {
    if p.post_id.trim().is_empty() {
        return None;
    }
    let body = [p.description.as_str(), p.translated_text.as_str()]
        .iter()
        .filter(|s| !s.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if body.trim().is_empty() {
        return None;
    }
    Some(KbDoc {
        kind: KbKind::GeneratedPost.as_str().into(),
        ref_id: p.post_id.clone(),
        platform: "instagram".into(),
        post_id: p.post_id.clone(),
        body,
        url: p.url.clone(),
        // 这一类上游不给品牌，用别名表从账号名与正文里认
        brand: pick_brand(brands, &format!("{} {}", p.account, p.description)),
        published_at: non_empty(&p.date_posted),
        ..Default::default()
    })
}

/// 文本里认出的品牌，取别名最长的那个（最长 ≈ 最specific）。
fn pick_brand(brands: &BrandIndex, text: &str) -> String {
    brands
        .hits(text)
        .into_iter()
        .max_by_key(|(_, alias)| alias.chars().count())
        .map(|(brand, _)| brand)
        .unwrap_or_default()
}

// ─────────────────────── 范例的本地导入 ───────────────────────

/// 从本地 JSONL 导入范例，格式与引擎侧 `syncer kb-import` 那份一致。
///
/// 正常路径是范例先进引擎台账、再由 [`sync_ledger_posts`] 带回来；
/// 这条路是给开发与回放用的——**不连引擎也能把库建起来**。
pub fn import_examples(
    conn: &Connection,
    tok: &Tokenizer,
    path: &std::path::Path,
) -> Result<SourceReport> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(default)]
        platform: String,
        #[serde(default)]
        post_id: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        published_at: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        body_text: String,
        #[serde(default)]
        items: Vec<RowItem>,
    }
    #[derive(serde::Deserialize, Default)]
    struct RowItem {
        #[serde(default)]
        brand: String,
    }

    let text = std::fs::read_to_string(path).with_context(|| format!("读 {}", path.display()))?;
    let mut rep = SourceReport::new(SRC_EXAMPLES);
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        rep.fetched += 1;
        // 单行坏掉不该让整份导入失败——如实记一笔，继续下一行
        let row: Row = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(行 = i + 1, 原因 = %e, "范例这一行不是合法 JSON，跳过");
                rep.skipped += 1;
                continue;
            }
        };
        if row.post_id.trim().is_empty() || row.body_text.trim().is_empty() {
            tracing::warn!(行 = i + 1, "范例缺 post_id 或 body_text，跳过");
            rep.skipped += 1;
            continue;
        }
        let doc = KbDoc {
            kind: KbKind::Example.as_str().into(),
            ref_id: row.post_id.clone(),
            platform: if row.platform.is_empty() {
                "wechat".into()
            } else {
                row.platform
            },
            post_id: row.post_id,
            title: row.title,
            body: row.body_text,
            url: row.url,
            brand: row
                .items
                .first()
                .map(|i| i.brand.clone())
                .unwrap_or_default(),
            published_at: non_empty(&row.published_at),
            publish_state: "published".into(),
            is_reference: true,
            ..Default::default()
        };
        let r = docs::upsert(conn, &doc)?;
        if r.content_changed {
            fts::index_doc(conn, r.id, tok, &doc.text())?;
        }
        rep.record(&r);
    }
    set_cursor(conn, SRC_EXAMPLES, &jiff::Timestamp::now().to_string())?;
    Ok(rep)
}

// ─────────────────────────── 向量化 ───────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbedReport {
    pub embedded: usize,
    pub failed: usize,
    /// 还剩多少没算
    pub remaining: usize,
}

/// 把还没算过向量的文档算上。
///
/// **失败的不标记**，下一轮自然会再挑到它——这就是重试，不需要单独的重试表。
/// 一次只做 `EMBED_BATCH` 条：GPU 是串行的，批大只会让一次失败重来得更贵。
pub async fn embed_pending(
    conn: &Connection,
    store: &VectorStore,
    vector: &VectorClient,
    model: &str,
    max_docs: usize,
) -> Result<EmbedReport> {
    let mut rep = EmbedReport::default();
    loop {
        let todo = docs::needs_embedding(conn, model, EMBED_BATCH.min(max_docs - rep.embedded))?;
        if todo.is_empty() {
            break;
        }
        let inputs: Vec<EmbedInput> = todo.iter().map(|d| EmbedInput::Text(d.text())).collect();
        match vector.embed(&inputs).await {
            Ok(vs) if vs.len() == todo.len() => {
                let rows: Vec<DocVector> = todo
                    .iter()
                    .zip(vs)
                    .map(|(d, v)| DocVector {
                        key: d.vector_key(),
                        kind: d.kind.clone(),
                        doc_id: d.id,
                        post_id: d.post_id.clone(),
                        brand: d.brand.clone(),
                        at: d.published_at.clone().unwrap_or_default(),
                        is_reference: d.is_reference,
                        vector: v,
                    })
                    .collect();
                store.put_docs(&rows).await?;
                // **先写向量库再标记**：反过来的话中途崩溃会留下
                // 「库里说算过了、向量库里没有」的行，而那种不一致没人查得出来
                let ids: Vec<i64> = todo.iter().map(|d| d.id).collect();
                docs::mark_embedded(conn, &ids, model)?;
                rep.embedded += todo.len();
            }
            Ok(vs) => {
                tracing::warn!(
                    要 = todo.len(),
                    给 = vs.len(),
                    "向量服务返回条数对不上，这批不标记"
                );
                rep.failed += todo.len();
                break;
            }
            Err(e) => {
                tracing::warn!(原因 = %format!("{e:#}"), "这批向量化失败，不标记，下轮重试");
                rep.failed += todo.len();
                break;
            }
        }
        if rep.embedded >= max_docs {
            break;
        }
    }
    rep.remaining = docs::needs_embedding(conn, model, usize::MAX)?.len();
    Ok(rep)
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_engineapi::types::LedgerPostItem;

    fn conn() -> Connection {
        csw_collector_core::store::open_in_memory().unwrap()
    }

    fn item(key: &str, brand: &str, title: &str) -> LedgerPostItem {
        LedgerPostItem {
            seq: 1,
            brand: brand.into(),
            product: "轻量背包".into(),
            title: title.into(),
            angle: "复刻".into(),
            item_key: key.into(),
            source_url: String::new(),
            split_by: "auto".into(),
        }
    }

    fn post(post_id: &str, is_ref: bool, body: &str, items: Vec<LedgerPostItem>) -> LedgerPost {
        LedgerPost {
            platform: "wechat".into(),
            account: "营事编集室".into(),
            post_id: post_id.into(),
            url: format!("https://example.com/{post_id}"),
            published_at: "2026-09-18T00:00:00Z".into(),
            title: "营事编集室 vol.10".into(),
            state: "published".into(),
            source: "sync".into(),
            is_reference: is_ref,
            body_text: body.into(),
            publish_evidence: String::new(),
            items,
        }
    }

    #[test]
    fn 已发条目按拆条建而不是按篇() {
        let p = post(
            "p1",
            false,
            "整篇正文",
            vec![
                item("k1", "山と道", "条目一"),
                item("k2", "NANGA", "条目二"),
            ],
        );
        let ds = ledger_docs(&p);
        assert_eq!(ds.len(), 2, "两条拆条就该是两个 doc");
        assert!(ds.iter().all(|d| d.kind == "published_item"));
        assert_eq!(ds[0].ref_id, "k1");
        assert_eq!(ds[0].brand, "山と道");
        assert_eq!(ds[1].brand, "NANGA");
        // 关键：条目的 body 只带条目自己的字段。带上整篇正文的话，
        // 同一篇里的几个条目会拿到几乎一样的向量，检索时一起涌上来挤掉别的材料
        assert!(!ds[0].body.contains("整篇正文"), "{}", ds[0].body);
        assert!(ds[0].body.contains("山と道") && ds[0].body.contains("复刻"));
        // 同篇的条目共用 post_id，检索时按它排掉自己
        assert!(ds.iter().all(|d| d.post_id == "p1"));
    }

    #[test]
    fn 范例按整篇建并带全文() {
        let p = post(
            "p2",
            true,
            "这是一篇范例的整篇正文",
            vec![item("k1", "山と道", "条目一")],
        );
        let ds = ledger_docs(&p);
        // 范例的价值在写法，写法是整篇的属性 → 整篇一条；拆条另算已发条目
        let ex: Vec<_> = ds.iter().filter(|d| d.kind == "example").collect();
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].ref_id, "p2");
        assert!(ex[0].body.contains("整篇正文"));
        assert!(ex[0].is_reference);
        assert_eq!(ex[0].brand, "山と道", "整篇的品牌取首条拆条的");
        assert_eq!(ds.iter().filter(|d| d.kind == "published_item").count(), 1);
    }

    #[test]
    fn 没有拆条的篇也不该整个消失() {
        let ds = ledger_docs(&post("p3", false, "只有正文没拆条", vec![]));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].kind, "published_item");
        assert_eq!(ds[0].ref_id, "p3");
        assert!(ds[0].body.contains("只有正文"));
    }

    #[test]
    fn 范例没有正文就不出整篇那一条() {
        // 正文是空的范例只是个壳，建成 doc 只会污染检索
        let ds = ledger_docs(&post(
            "p4",
            true,
            "   ",
            vec![item("k1", "山と道", "条目一")],
        ));
        assert!(ds.iter().all(|d| d.kind != "example"));
        assert_eq!(ds.len(), 1);
    }

    #[test]
    fn 决定的键要带run_id() {
        let d = Decision {
            run_id: 48,
            subject: String::new(),
            item_key: "nanga-ab12cd".into(),
            title: "羽绒进城".into(),
            brand: "NANGA".into(),
            source_url: "https://x".into(),
            published_at: "2026-09-18T00:00:00Z".into(),
            status: "adopted".into(),
            decision_source: "van".into(),
            decided_at: "2026-09-18T09:00:00Z".into(),
            reason_code: String::new(),
            reason: "角度新".into(),
            quote_ref: "这个可以做".into(),
            actor_role: "editor".into(),
        };
        let doc = decision_doc(&d).unwrap();
        // 同一个条目在不同 run 里可能被决定两次，run_id 进键才不会互相覆盖
        assert_eq!(doc.ref_id, "48#nanga-ab12cd");
        assert!(doc.body.contains("结论：adopted"));
        assert!(
            doc.body.contains("原话：这个可以做"),
            "原话是这一类材料的核心"
        );
        assert!(doc.body.contains("理由：角度新"));
        // 没有理由码就不该留一个空标签
        assert!(!doc.body.contains("理由码"));

        let mut no_key = d.clone();
        no_key.item_key = "  ".into();
        assert!(decision_doc(&no_key).is_none());
    }

    #[test]
    fn 已生成贴文要从账号与正文里认品牌() {
        let brands = BrandIndex::new(
            [("山と道", "山と道"), ("and wander", "andwander")]
                .iter()
                .filter_map(|(b, a)| crate::brands::Alias::new(b, a))
                .collect(),
        );
        let mut p = RawPost {
            post_id: "abc123".into(),
            account: "andwander".into(),
            description: "新色上市".into(),
            ..Default::default()
        };
        let doc = generated_doc(&p, &brands).unwrap();
        assert_eq!(doc.kind, "generated_post");
        assert_eq!(doc.brand, "and wander", "账号名里就带着品牌");

        // 正文空的跳过：没有正文就没有可嵌入的东西
        p.description = String::new();
        p.translated_text = String::new();
        assert!(generated_doc(&p, &brands).is_none());
    }

    #[test]
    fn 第一次同步全量之后按重叠窗口() {
        let c = conn();
        assert_eq!(
            since(&c, SRC_LEDGER_POSTS, false).unwrap(),
            "3650d",
            "没游标就全量"
        );
        set_cursor(&c, SRC_LEDGER_POSTS, "2026-09-18T00:00:00Z").unwrap();
        assert_eq!(since(&c, SRC_LEDGER_POSTS, false).unwrap(), "45d");
        assert_eq!(
            since(&c, SRC_LEDGER_POSTS, true).unwrap(),
            "3650d",
            "--full 永远全量"
        );
        assert_eq!(
            cursor(&c, SRC_LEDGER_POSTS).unwrap().as_deref(),
            Some("2026-09-18T00:00:00Z")
        );
        assert!(cursor(&c, "没这个源").unwrap().is_none());
    }

    /// 起一个假的向量服务：按请求里的 `input` 条数回同样多条 8 维向量
    struct Echo;
    impl wiremock::Respond for Echo {
        fn respond(&self, req: &wiremock::Request) -> wiremock::ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let n = body["input"].as_array().map(Vec::len).unwrap_or(0);
            let data: Vec<serde_json::Value> = (0..n)
                .map(|i| serde_json::json!({"embedding": vec![i as f32 + 1.0; 8]}))
                .collect();
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data}))
        }
    }

    async fn embed_fixture(
        status: u16,
    ) -> (
        Connection,
        VectorStore,
        VectorClient,
        tempdir::TempDir,
        wiremock::MockServer,
    ) {
        let server = wiremock::MockServer::start().await;
        let m = wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/embeddings"));
        if status == 200 {
            m.respond_with(Echo).mount(&server).await;
        } else {
            m.respond_with(wiremock::ResponseTemplate::new(status))
                .mount(&server)
                .await;
        }
        let dir = tempdir::TempDir::new("emb").unwrap();
        let store = VectorStore::open(dir.path(), 8, "测试模型").await.unwrap();
        let vector = VectorClient::new(csw_collector_core::vector::VectorConfig {
            base_url: server.uri(),
            timeout: std::time::Duration::from_secs(5),
            ..Default::default()
        })
        .unwrap();
        let c = conn();
        let tok = Tokenizer::new();
        for i in 1..=3 {
            let d = KbDoc {
                kind: KbKind::Example.as_str().into(),
                ref_id: format!("e{i}"),
                body: format!("第 {i} 条范例正文"),
                ..Default::default()
            };
            let r = docs::upsert(&c, &d).unwrap();
            fts::index_doc(&c, r.id, &tok, &d.text()).unwrap();
        }
        (c, store, vector, dir, server)
    }

    #[tokio::test]
    async fn 向量算完写进去并标记() {
        let (c, store, vector, _d, _s) = embed_fixture(200).await;
        let rep = embed_pending(&c, &store, &vector, "测试模型", 100)
            .await
            .unwrap();
        assert_eq!((rep.embedded, rep.failed, rep.remaining), (3, 0, 0));
        assert_eq!(store.counts().await.unwrap().0, 3);
        assert!(
            docs::needs_embedding(&c, "测试模型", 10)
                .unwrap()
                .is_empty()
        );

        // 再跑一次没活干
        let again = embed_pending(&c, &store, &vector, "测试模型", 100)
            .await
            .unwrap();
        assert_eq!(again.embedded, 0);
    }

    #[tokio::test]
    async fn 算失败就不标记下轮自然重试() {
        let (c, store, vector, _d, _s) = embed_fixture(500).await;
        let rep = embed_pending(&c, &store, &vector, "测试模型", 100)
            .await
            .unwrap();
        assert_eq!(rep.embedded, 0);
        assert_eq!(rep.failed, 3);
        // 关键不变式：失败的一条都不能标记。标了就会留下
        // 「库里说算过了、向量库里没有」的行，那种不一致没人查得出来
        assert_eq!(store.counts().await.unwrap().0, 0);
        assert_eq!(rep.remaining, 3);
        assert_eq!(docs::needs_embedding(&c, "测试模型", 10).unwrap().len(), 3);
    }

    #[test]
    fn 范例导入坏行跳过好行照收() {
        let c = conn();
        let tok = Tokenizer::new();
        let dir = tempdir::TempDir::new("kbimp").unwrap();
        let path = dir.path().join("kb_import.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"post_id":"a1","title":"范例一","body_text":"正文一","items":[{"brand":"山と道"}]}"#,
                "\n",
                "这一行不是 JSON\n",
                "\n",
                r#"{"post_id":"a2","body_text":""}"#,
                "\n",
                r#"{"post_id":"a3","title":"范例三","body_text":"正文三"}"#,
                "\n",
            ),
        )
        .unwrap();

        let rep = import_examples(&c, &tok, &path).unwrap();
        // 一行坏掉不该让整份导入失败
        assert_eq!(rep.fetched, 4);
        assert_eq!(rep.inserted, 2);
        assert_eq!(rep.skipped, 2, "坏 JSON 与缺正文各一");
        assert_eq!(
            docs::counts_by_kind(&c).unwrap(),
            [("example".to_string(), 2)]
        );
        let d = docs::get(&c, 1).unwrap().unwrap();
        assert!(d.is_reference && d.brand == "山と道");

        // 重跑幂等：内容没变就不重算
        let again = import_examples(&c, &tok, &path).unwrap();
        assert_eq!(again.inserted, 0);
        assert_eq!(again.changed, 0);
        assert_eq!(again.unchanged, 2);
    }
}
