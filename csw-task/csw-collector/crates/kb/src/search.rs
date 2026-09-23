//! 检索：**三路召回 → 合并去重 → rerank → 按类分组**。
//!
//! 三路是互补的，不是互为备份：
//! - **向量**认得「说的是同一件事」，但对专名不敏感——`and wander` 与 `nanamica`
//!   在向量空间里离得并不远。
//! - **品牌精确匹配**认得专名，但只认得写出来的那个名字。
//! - **全文**捞回前两路都漏掉的字面重合（型号、材质、联名对象）。
//!
//! 合并之后统一 rerank，再按类分组。**分组后按类补齐**是关键一步：
//! 对照材料五类缺一不判，而 rerank 只按相关度排，很容易让某一类整体挤不进前八，
//! 于是一条本来判得了的候选被卡成待核。补齐就是防这个。
//!
//! ## rerank 的用量是算过的
//!
//! 实测每段 127–165 ms，而且 GPU 是全局串行的。每条候选排 24 段 ≈ 3.5 秒，
//! 一期 358 条 ≈ 21 分钟——这已经是整轮时间预算里排第二大的一块。
//! 所以 [`RERANK_MAX`] 不是随手写的数，**要调它先去看时间预算**。

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::Connection;

use csw_collector_core::vector::VectorClient;

use crate::brands::BrandIndex;
use crate::docs::{self, KbDoc, KbKind};
use crate::fts::{self, Tokenizer};
use crate::vectors::{DocFilter, VectorStore};

/// 向量路召回几条
pub const VECTOR_TOP_K: usize = 24;
/// 品牌路每个命中的品牌召回几条
pub const BRAND_PER_BRAND: usize = 8;
/// 全文路最多用几个关键词去查
pub const FTS_TERMS: usize = 8;
/// 每个关键词召回几条
pub const FTS_PER_TERM: usize = 4;
/// 全文路总共最多召回几条
pub const FTS_TOP_N: usize = 16;
/// 关键词的最短长度。单字一律不要；拉丁两字母（`GO`、`XL`）也太泛。
const MIN_TERM_LATIN: usize = 3;
const MIN_TERM_CJK: usize = 2;
/// 一个词命中超过参考库的这个比例就算「满库都有」，剔掉。
const FTS_DF_RATIO: usize = 20; // 5%
/// 小库上比例算不出东西，给个下限
const FTS_DF_FLOOR: usize = 16;
/// 送进 rerank 的段数上限。**改它之前先看模块文档里那笔账。**
pub const RERANK_MAX: usize = 24;
/// 最终给判断那一步的条数上限
pub const FINAL_MAX: usize = 8;
/// 送进 rerank 的每段截多长。太长既费时间又让重点被稀释。
pub const SNIPPET_CHARS: usize = 300;

/// 告诉重排器什么叫「相关」。空着的话它只会按字面相似度排，
/// 而我们要的是「能用来判断这条新贴文值不值得做」。
const RERANK_INSTRUCTION: &str = "判断这份历史材料能否用来对照评估一条新的户外品牌贴文：\
     同一品牌、同一产品线、同一类事件（上新、联名、复刻、活动）都算相关；\
     只是同属户外品类不算。";

/// 一条召回是从哪条路来的。**一条材料可能同时从多条路来**，
/// 台账里要如实写清——只记一条路会让「这三路各自有没有用」变成无法回答的问题。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Routes {
    pub vector: bool,
    pub brand: bool,
    pub fts: bool,
}

impl Routes {
    pub fn names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.vector {
            v.push("向量");
        }
        if self.brand {
            v.push("品牌");
        }
        if self.fts {
            v.push("全文");
        }
        v
    }
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub doc: KbDoc,
    pub routes: Routes,
    /// 重排分；没跑 rerank 时是 None
    pub score: Option<f64>,
    /// 这条是为了「缺一不判」补进来的，不是靠相关度挤进来的。
    /// 台账要标出来——补齐来的材料说服力本来就弱一档。
    pub backfilled: bool,
}

/// 三路各召回了多少、去重后多少。写进采集轮台账。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecallCounts {
    pub vector: usize,
    pub brand: usize,
    pub fts: usize,
    /// 去重后送进 rerank 的
    pub merged: usize,
    /// 因为 [`RERANK_MAX`] 截掉的
    pub truncated: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Retrieved {
    /// 按相关度排好的最终结果
    pub docs: Vec<Scored>,
    /// 参考库里一条都没找着的类。判断那一步据此决定落不落待核。
    pub missing_kinds: Vec<&'static str>,
    pub counts: RecallCounts,
    /// 品牌路是靠哪几个品牌命中的
    pub brands_hit: Vec<String>,
}

impl Retrieved {
    /// 按类分组，顺序与 [`KbKind::ALL`] 一致。
    pub fn by_kind(&self) -> Vec<(KbKind, Vec<&Scored>)> {
        KbKind::ALL
            .into_iter()
            .map(|k| {
                let v = self
                    .docs
                    .iter()
                    .filter(|s| s.doc.kind == k.as_str())
                    .collect();
                (k, v)
            })
            .collect()
    }
}

/// 一次检索要什么。
pub struct Query<'a> {
    /// 候选的正文 + 图片描述。品牌路与全文路都用它，rerank 也用它当 query。
    pub text: &'a str,
    /// 融合向量。没有就只走品牌与全文两路——**这不是错误**，
    /// 向量化失败时检索差一点，但不该让这条候选判不了。
    pub vector: Option<&'a [f32]>,
    /// 排掉这条贴文自己（它可能已经在参考库里）
    pub exclude_post_id: Option<String>,
    /// 最终要几条
    pub limit: usize,
    /// 缺的那一类要不要从库里补一条进来。
    ///
    /// **判断那一步要开，检索工具要关。** 判断受「五类缺一不判」约束，
    /// 某一类整体缺席会把条目卡成待核；而检索工具被问「有没有关于 X 的」时，
    /// 补齐会让答案永远是「有」——那不是回答，是糊弄。
    pub backfill_kinds: bool,
    /// 送进重排的召回上限。默认 [`RERANK_MAX`]；正式一轮取配置 `limits.rerank_per_candidate`
    pub rerank_max: usize,
    /// 重排每段截多少字。默认 [`SNIPPET_CHARS`]；正式一轮取配置 `limits.rerank_snippet_chars`
    pub snippet_chars: usize,
}

impl Default for Query<'_> {
    fn default() -> Self {
        Self {
            text: "",
            vector: None,
            exclude_post_id: None,
            limit: FINAL_MAX,
            // 默认不补：补齐是判断那一步的特殊需要，不是检索的常态
            backfill_kinds: false,
            rerank_max: RERANK_MAX,
            snippet_chars: SNIPPET_CHARS,
        }
    }
}

/// 三路召回 + 重排。
///
/// # 为什么 `conn` 是参数而不是字段
///
/// `rusqlite::Connection` 不是 `Sync`，所以**把 `&Connection` 跨 `await` 持有的
/// future 不是 `Send`**。检索里有两段是 async 的（向量路、重排），一旦连接
/// 存在结构体里，整个 `search` 就再也不能在 axum 的 handler 里跑——
/// 而工作台的知识库页恰恰要在那里跑。
///
/// 所以：连接只在**同步**的那几段里出现，且只以参数形式。
/// [`Retriever::search`] 一次跑完（给不要求 `Send` 的地方用：判断那一步、MCP），
/// axum 那边按 [`Retriever::vector_route`] → [`Retriever::recall`] →
/// [`Retriever::rerank`] → [`Retriever::finish`] 自己排，
/// 每段之间把连接的锁放掉。
pub struct Retriever<'a> {
    pub store: &'a VectorStore,
    pub brands: &'a BrandIndex,
    pub tok: &'a Tokenizer,
    /// 只用来 rerank。None = 不重排，按召回顺序给。
    pub reranker: Option<&'a VectorClient>,
}

/// 第二段的产物：召回到的文档、各路计数、认出的品牌。
pub struct Recalled {
    pub scored: Vec<Scored>,
    pub counts: RecallCounts,
    pub brands_hit: Vec<String>,
}

impl Retriever<'_> {
    /// 一次跑完。**这个 future 不是 `Send`**（它跨 await 持有 `&Connection`），
    /// 只能在不要求 `Send` 的地方用。
    pub async fn search(&self, conn: &Connection, q: &Query<'_>) -> Result<Retrieved> {
        let vector_ids = self.vector_route(q).await?;
        let mut r = self.recall(conn, q, &vector_ids)?;
        self.rerank(q, &mut r.scored).await;
        self.finish(conn, q, r)
    }

    /// 第一段：向量路。**不碰库**，所以它的 future 是 `Send` 的。
    pub async fn vector_route(&self, q: &Query<'_>) -> Result<Vec<i64>> {
        let Some(v) = q.vector else {
            return Ok(Vec::new());
        };
        let filter = DocFilter {
            kinds: KbKind::ALL.iter().map(|k| k.as_str().to_string()).collect(),
            exclude_post_id: q.exclude_post_id.clone(),
            ..Default::default()
        };
        let hits = self
            .store
            .search_docs(v, &filter, VECTOR_TOP_K)
            .await
            .context("向量召回")?;
        Ok(hits.into_iter().map(|h| h.doc_id).collect())
    }

    /// 第二段：品牌路 + 全文路 + 合并 + 取回文档。**只碰库，一次 `await` 都没有。**
    pub fn recall(&self, conn: &Connection, q: &Query<'_>, vector_ids: &[i64]) -> Result<Recalled> {
        let mut routes: HashMap<i64, Routes> = HashMap::new();
        let mut counts = RecallCounts {
            vector: vector_ids.len(),
            ..Default::default()
        };

        for id in vector_ids {
            routes.entry(*id).or_default().vector = true;
        }

        // ── 二、品牌路 ──
        let hits = self.brands.hits(q.text);
        let mut brands_hit: Vec<String> = hits.keys().cloned().collect();
        brands_hit.sort();
        let mut brand_order: Vec<i64> = Vec::new();
        for brand in &brands_hit {
            for d in docs::by_brand(conn, brand, BRAND_PER_BRAND)? {
                counts.brand += 1;
                routes.entry(d.id).or_default().brand = true;
                brand_order.push(d.id);
            }
        }

        // ── 三、全文路 ──
        // **不要把整段正文当一条短语丢进去**。`phrase_query` 把查询整体拼成一个
        // FTS5 短语，那是为搜索框里那种短查询设计的；一条贴文正文几百字，
        // 作为短语去匹配任何一篇历史材料，结果永远是零。
        // 这一路要的是「型号、材质、联名对象」这类字面重合，所以先从正文里挑出
        // 值得逐字匹配的关键词，一个词一次短语查。
        // （这个错是测试抓出来的：只有 only_fts 那条怎么都召不回来。）
        let df_cap = self.df_cap(conn)?;
        let mut fts_order: Vec<i64> = Vec::new();
        for term in key_phrases(self.tok, q.text) {
            if counts.fts >= FTS_TOP_N {
                break;
            }
            // 满库都有的词召回的全是噪声，跳过
            if fts::doc_frequency(conn, self.tok, &term)? > df_cap {
                continue;
            }
            for h in fts::search(conn, self.tok, &term, FTS_PER_TERM)? {
                counts.fts += 1;
                routes.entry(h.doc_id).or_default().fts = true;
                fts_order.push(h.doc_id);
            }
        }

        // ── 合并 ──
        let mut ids = merge_order(&routes, [vector_ids, &brand_order, &fts_order]);
        counts.merged = ids.len();
        let cap = q.rerank_max.max(1);
        if ids.len() > cap {
            counts.truncated = ids.len() - cap;
            ids.truncate(cap);
        }

        let found = docs::get_many(conn, &ids)?;
        let mut scored: Vec<Scored> = found
            .into_iter()
            .map(|doc| Scored {
                routes: routes.get(&doc.id).copied().unwrap_or_default(),
                doc,
                score: None,
                backfilled: false,
            })
            .collect();

        // 同一类里同一条贴文只留一条（已发条目与「生成过文章的贴文」是两类，各留各的）
        dedupe_by_post(&mut scored);

        Ok(Recalled {
            scored,
            counts,
            brands_hit,
        })
    }

    /// 第三段：重排。**不碰库**，所以它的 future 是 `Send` 的。
    ///
    /// 重排失败不算错：召回顺序本身已经是可用的次序，硬要个名次不值得让整次检索失败。
    pub async fn rerank(&self, q: &Query<'_>, scored: &mut [Scored]) {
        let Some(rr) = self.reranker else { return };
        if scored.is_empty() || q.text.trim().is_empty() {
            return;
        }
        let n = q.snippet_chars.max(1);
        let snippets: Vec<String> = scored.iter().map(|s| snippet(&s.doc, n)).collect();
        match rr
            .rerank(&snippet_text(q.text, n), &snippets, RERANK_INSTRUCTION)
            .await
        {
            Ok(items) => {
                for it in &items {
                    if let Some(s) = scored.get_mut(it.index) {
                        s.score = Some(it.relevance_score);
                    }
                }
                // 分高的在前；没拿到分的排最后而不是当成 0 分——
                // 「没排到」和「排了但不相关」是两回事
                scored.sort_by(|a, b| match (a.score, b.score) {
                    (Some(x), Some(y)) => y.total_cmp(&x),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                });
            }
            Err(e) => {
                tracing::warn!(原因 = %format!("{e:#}"), "重排失败，按召回顺序给");
            }
        }
    }

    /// 第四段：截断到 `limit`，按需补齐缺的那几类。**只碰库。**
    pub fn finish(&self, conn: &Connection, q: &Query<'_>, r: Recalled) -> Result<Retrieved> {
        let mut out: Vec<Scored> = r.scored.iter().take(q.limit.max(1)).cloned().collect();
        let missing = if q.backfill_kinds {
            self.backfill(conn, &mut out, &r.scored, q)?
        } else {
            KbKind::ALL
                .into_iter()
                .filter(|k| !out.iter().any(|s| s.doc.kind == k.as_str()))
                .map(KbKind::as_str)
                .collect()
        };

        Ok(Retrieved {
            docs: out,
            missing_kinds: missing,
            counts: r.counts,
            brands_hit: r.brands_hit,
        })
    }

    /// 一个词命中多少篇就算「满库都有」。
    fn df_cap(&self, conn: &Connection) -> Result<usize> {
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM kb_docs", [], |r| r.get(0))
            .unwrap_or(0);
        Ok((total as usize / FTS_DF_RATIO).max(FTS_DF_FLOOR))
    }

    /// 按类补齐：对照材料**缺一不判**，而 rerank 只按相关度排，
    /// 很容易让某一类整体挤不进前几名。补进来的标 `backfilled`。
    ///
    /// 补的顺序：先从这一轮召回里挑该类分最高的；召回里也没有就去库里按
    /// 命中品牌捞该类最近一条；还没有就如实记成缺口，由判断那一步落待核。
    fn backfill(
        &self,
        conn: &Connection,
        out: &mut Vec<Scored>,
        pool: &[Scored],
        q: &Query<'_>,
    ) -> Result<Vec<&'static str>> {
        let mut missing = Vec::new();
        for kind in KbKind::ALL {
            let k = kind.as_str();
            if out.iter().any(|s| s.doc.kind == k) {
                continue;
            }
            if let Some(s) = pool.iter().find(|s| s.doc.kind == k) {
                let mut s = s.clone();
                s.backfilled = true;
                out.push(s);
                continue;
            }
            let from_lib = self.newest_of_kind(conn, k, q)?;
            match from_lib {
                Some(doc) => out.push(Scored {
                    doc,
                    routes: Routes::default(),
                    score: None,
                    backfilled: true,
                }),
                None => missing.push(k),
            }
        }
        Ok(missing)
    }

    /// 库里这一类最近的一条。先按命中的品牌找，找不到再退回全库最近。
    fn newest_of_kind(
        &self,
        conn: &Connection,
        kind: &str,
        q: &Query<'_>,
    ) -> Result<Option<KbDoc>> {
        for brand in self.brands.hits(q.text).keys() {
            let hit = docs::by_brand(conn, brand, BRAND_PER_BRAND)?
                .into_iter()
                .find(|d| d.kind == kind);
            if hit.is_some() {
                return Ok(hit);
            }
        }
        let mut st = conn.prepare(
            "SELECT id FROM kb_docs WHERE kind = ?1
             ORDER BY published_at IS NULL, published_at DESC, id DESC LIMIT 1",
        )?;
        let id: Option<i64> = st
            .query_map([kind], |r| r.get(0))?
            .filter_map(Result::ok)
            .next();
        match id {
            Some(id) => docs::get(conn, id),
            None => Ok(None),
        }
    }
}

/// 同一类里同一条贴文只留一条。`post_id` 为空的不参与去重——
/// 范例与决定大多没有 post_id，按空串去重会把它们误合成一条。
/// 合并三路的次序，截断前用。
///
/// 命中两路以上的排最前（路数多者先，同数按在各路里最靠前的名次）。
/// **只命中一路的三路轮流取**，每路内部保持它自己的次序（向量按相似度、
/// 品牌按新近、全文按关键词顺序）。
///
/// 原先是「路数 → 有没有向量 → id」一把排：向量路每次都满额给回 24 条，
/// 只从品牌路或全文路进来的文档于是**永远排在第 25 名之后被截掉**——
/// M2 在线上跑出来「只靠品牌路命中的 0 条」，就是这么来的。
/// 同时向量路自己的相似度次序也被按 id 排冲掉了，截掉哪几条等于看 id 大小。
fn merge_order(routes: &HashMap<i64, Routes>, lists: [&[i64]; 3]) -> Vec<i64> {
    let best_rank = |id: i64| {
        lists
            .iter()
            .filter_map(|l| l.iter().position(|x| *x == id))
            .min()
            .unwrap_or(usize::MAX)
    };
    let mut multi: Vec<i64> = routes
        .iter()
        .filter(|(_, r)| r.names().len() >= 2)
        .map(|(id, _)| *id)
        .collect();
    multi.sort_by_key(|id| {
        (
            std::cmp::Reverse(routes[id].names().len()),
            best_rank(*id),
            *id,
        )
    });

    let mut seen: std::collections::HashSet<i64> = multi.iter().copied().collect();
    let mut singles: Vec<std::collections::VecDeque<i64>> = lists
        .iter()
        .map(|l| {
            l.iter()
                .copied()
                .filter(|id| routes.get(id).is_some_and(|r| r.names().len() == 1))
                .collect()
        })
        .collect();
    let mut out = multi;
    loop {
        let mut took = false;
        for q in singles.iter_mut() {
            while let Some(id) = q.pop_front() {
                if seen.insert(id) {
                    out.push(id);
                    took = true;
                    break;
                }
            }
        }
        if !took {
            break;
        }
    }
    out
}

fn dedupe_by_post(scored: &mut Vec<Scored>) {
    let mut seen: std::collections::HashSet<(String, String)> = Default::default();
    scored.retain(|s| {
        if s.doc.post_id.is_empty() {
            return true;
        }
        seen.insert((s.doc.kind.clone(), s.doc.post_id.clone()))
    });
}

/// 从候选正文里挑出值得逐字匹配的关键词。
///
/// 长度在这里只做一道粗筛（单字不要）。**真正的判据是词频**，在调用侧按
/// [`fts::doc_frequency`] 剔——`Dyneema` 七个字母却极specific，「新品」两个字
/// 却满库都是，按长度分辨不了这两者。
///
/// 一开始这里写的是「CJK 三字以上」，结果把关键词全滤光了：分词器会把
/// `山と道` 切成三个单字。品牌名本来就由品牌路精确匹配负责，全文路不必管它。
///
/// 长词排前面：先用信息量大的去查，`FTS_TERMS` 的额度花在刀刃上。
fn key_phrases(tok: &Tokenizer, text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut terms: Vec<String> = tok
        .cut(text)
        .split_whitespace()
        .filter(|t| is_distinctive(t))
        .filter(|t| seen.insert(t.to_lowercase()))
        .map(str::to_string)
        .collect();
    terms.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    terms.truncate(FTS_TERMS);
    terms
}

fn is_distinctive(t: &str) -> bool {
    let n = t.chars().count();
    if t.chars().any(|c| c.is_ascii_alphanumeric()) {
        n >= MIN_TERM_LATIN
    } else {
        n >= MIN_TERM_CJK
    }
}

fn snippet(doc: &KbDoc, n: usize) -> String {
    snippet_text(&doc.text(), n)
}

fn snippet_text(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        return t.to_string();
    }
    t.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brands::Alias;
    use crate::vectors::DocVector;

    const DIM: i32 = 8;

    fn v(i: usize) -> Vec<f32> {
        let mut x = vec![0.01_f32; DIM as usize];
        x[i % DIM as usize] = 1.0;
        x
    }

    struct Fixture {
        conn: Connection,
        store: VectorStore,
        brands: BrandIndex,
        tok: Tokenizer,
        _dir: tempdir::TempDir,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = tempdir::TempDir::new("search").unwrap();
            let store = VectorStore::open(dir.path(), DIM, "test").await.unwrap();
            let brands = BrandIndex::new(
                [
                    ("and wander", "and wander"),
                    ("山と道", "山と道"),
                    ("Snow Peak", "Snow Peak"),
                ]
                .iter()
                .filter_map(|(b, a)| Alias::new(b, a))
                .collect(),
            );
            let tok = Tokenizer::with_brands(&brands.dict_words());
            Self {
                conn: csw_collector_core::store::open_in_memory().unwrap(),
                store,
                brands,
                tok,
                _dir: dir,
            }
        }

        /// 写一条参考库材料，三处身份一起建好
        async fn put(
            &self,
            kind: KbKind,
            ref_id: &str,
            brand: &str,
            body: &str,
            vec_at: Option<usize>,
        ) -> i64 {
            let doc = KbDoc {
                kind: kind.as_str().into(),
                ref_id: ref_id.into(),
                brand: brand.into(),
                post_id: format!("p-{ref_id}"),
                title: String::new(),
                body: body.into(),
                published_at: Some("2026-09-18T00:00:00Z".into()),
                ..Default::default()
            };
            let id = docs::upsert(&self.conn, &doc).unwrap().id;
            fts::index_doc(&self.conn, id, &self.tok, &doc.text()).unwrap();
            if let Some(i) = vec_at {
                self.store
                    .put_docs(&[DocVector {
                        key: doc.vector_key(),
                        kind: doc.kind.clone(),
                        doc_id: id,
                        post_id: doc.post_id.clone(),
                        brand: doc.brand.clone(),
                        at: doc.published_at.clone().unwrap_or_default(),
                        is_reference: false,
                        vector: v(i),
                    }])
                    .await
                    .unwrap();
            }
            id
        }

        fn retriever(&self) -> Retriever<'_> {
            Retriever {
                store: &self.store,
                brands: &self.brands,
                tok: &self.tok,
                reranker: None, // 不连 GPU：重排是排序，召回与补齐才是这里要验的
            }
        }
    }

    #[tokio::test]
    async fn 三路各自都能捞到别人捞不到的() {
        let f = Fixture::new().await;
        // 只有向量能找到：正文里既没有品牌名也没有查询里的词
        let only_vec = f
            .put(
                KbKind::PublishedItem,
                "a",
                "别的牌子",
                "夏季新色上市",
                Some(1),
            )
            .await;
        // 只有品牌路能找到：品牌对得上，但正文与查询没有字面重合，也没有向量
        let only_brand = f
            .put(KbKind::Example, "b", "山と道", "旧款回顾", None)
            .await;
        // 只有全文能找到：品牌不在别名表里，但正文有查询里的专名
        let only_fts = f
            .put(KbKind::Decision, "c", "", "这次用的是 Dyneema 面料", None)
            .await;

        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "山と道 的 Dyneema 新包",
                    vector: Some(&v(1)),
                    limit: 10,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let ids: Vec<i64> = got.docs.iter().map(|s| s.doc.id).collect();
        assert!(ids.contains(&only_vec), "向量路漏了：{ids:?}");
        assert!(ids.contains(&only_brand), "品牌路漏了：{ids:?}");
        assert!(ids.contains(&only_fts), "全文路漏了：{ids:?}");
        assert_eq!(got.brands_hit, ["山と道"]);

        let route_of = |id: i64| got.docs.iter().find(|s| s.doc.id == id).unwrap().routes;
        assert!(route_of(only_vec).vector && !route_of(only_vec).brand);
        assert!(route_of(only_brand).brand);
        assert!(route_of(only_fts).fts);
    }

    #[tokio::test]
    async fn 一条材料从多条路来要都记上() {
        let f = Fixture::new().await;
        let id = f
            .put(KbKind::Example, "a", "山と道", "山と道 新包", Some(2))
            .await;
        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "山と道 新包",
                    vector: Some(&v(2)),
                    limit: 10,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let r = got.docs.iter().find(|s| s.doc.id == id).unwrap().routes;
        // 只记一条路会让「这三路各自有没有用」变成无法回答的问题
        assert_eq!(r.names(), ["向量", "品牌", "全文"]);
    }

    #[tokio::test]
    async fn 缺的那一类要补齐并标出来() {
        let f = Fixture::new().await;
        // 只有 example 会被召回；另外三类库里有，但一条都排不进来
        f.put(KbKind::Example, "e1", "山と道", "山と道 新包", Some(1))
            .await;
        for (k, r) in [
            (KbKind::PublishedItem, "p1"),
            (KbKind::GeneratedPost, "g1"),
            (KbKind::Decision, "d1"),
        ] {
            f.put(k, r, "毫不相干", "毫不相干的内容", None).await;
        }

        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "山と道 新包",
                    vector: Some(&v(1)),
                    limit: 1, // 只要一条：不补齐的话另外三类会整体缺席
                    backfill_kinds: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(got.missing_kinds.is_empty(), "库里都有，不该报缺口");
        let kinds: Vec<&str> = got.docs.iter().map(|s| s.doc.kind.as_str()).collect();
        for k in KbKind::ALL {
            assert!(
                kinds.contains(&k.as_str()),
                "{} 没补上：{kinds:?}",
                k.as_str()
            );
        }
        // 补进来的要标出来：它们的说服力本来就弱一档
        assert_eq!(got.docs.iter().filter(|s| s.backfilled).count(), 3);
        assert!(!got.docs[0].backfilled, "靠相关度进来的那条不该被标成补齐");
    }

    #[tokio::test]
    async fn 不开补齐就不该凭空多出材料() {
        let f = Fixture::new().await;
        f.put(KbKind::Example, "e1", "山と道", "山と道 新包", Some(1))
            .await;
        for (k, r) in [(KbKind::PublishedItem, "p1"), (KbKind::Decision, "d1")] {
            // 用真正不共享词元的内容：「毫不相干」和「完全不相干」共享「不相干」，
            // 那会被全文路正当命中，验不到要验的东西
            f.put(k, r, "无名", "露营灯具的保养方法", None).await;
        }
        // 检索工具被问「有没有关于 X 的」时，补齐会让答案永远是「有」
        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "半导体制程良率",
                    limit: 8,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            got.docs.is_empty(),
            "{:?}",
            got.docs.iter().map(|s| &s.doc.ref_id).collect::<Vec<_>>()
        );
        assert!(got.docs.iter().all(|s| !s.backfilled));
        // 缺口仍然如实报出来，只是不去填它
        assert_eq!(got.missing_kinds.len(), 4);
    }

    #[tokio::test]
    async fn 库里真没有那一类就如实报缺口() {
        let f = Fixture::new().await;
        f.put(KbKind::Example, "e1", "山と道", "山と道 新包", Some(1))
            .await;
        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "山と道 新包",
                    vector: Some(&v(1)),
                    limit: 8,
                    backfill_kinds: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // 补不出来就不要编——判断那一步据此落待核
        assert_eq!(
            got.missing_kinds,
            ["published_item", "generated_post", "decision"]
        );
        assert_eq!(got.docs.len(), 1);
    }

    #[tokio::test]
    async fn 没有向量也照样能检索() {
        let f = Fixture::new().await;
        f.put(KbKind::Example, "e1", "山と道", "山と道 新包", None)
            .await;
        // 向量化失败时检索差一点，但不该让这条候选判不了
        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "山と道 新包",
                    vector: None,
                    limit: 8,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(got.counts.vector, 0);
        assert!(got.counts.brand > 0 && got.counts.fts > 0);
        assert!(!got.docs.is_empty());
    }

    #[tokio::test]
    async fn 排掉自己那条贴文() {
        let f = Fixture::new().await;
        f.put(KbKind::PublishedItem, "self", "山と道", "就是这条", Some(3))
            .await;
        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "毫无关系的查询词",
                    vector: Some(&v(3)),
                    exclude_post_id: Some("p-self".into()),
                    limit: 8,
                    backfill_kinds: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(got.counts.vector, 0, "自己那条不该从向量路回来");
    }

    #[tokio::test]
    async fn 同类同贴文去重但没有post_id的不参与() {
        let mut s = vec![
            scored("published_item", "p1", 1),
            scored("published_item", "p1", 2),
            scored("generated_post", "p1", 3),
            scored("example", "", 4),
            scored("decision", "", 5),
        ];
        dedupe_by_post(&mut s);
        let ids: Vec<i64> = s.iter().map(|x| x.doc.id).collect();
        // 同一类里的重复条去掉；跨类的两个身份各留各的；没有 post_id 的两条都留着
        assert_eq!(ids, [1, 3, 4, 5]);
    }

    fn scored(kind: &str, post_id: &str, id: i64) -> Scored {
        Scored {
            doc: KbDoc {
                id,
                kind: kind.into(),
                post_id: post_id.into(),
                ..Default::default()
            },
            routes: Routes::default(),
            score: None,
            backfilled: false,
        }
    }

    #[tokio::test]
    async fn 召回太多要截断并记下截了多少() {
        let f = Fixture::new().await;
        // 三路各有自己的上限，堆一路是堆不过 RERANK_MAX 的——要两路凑
        for i in 0..VECTOR_TOP_K {
            f.put(
                KbKind::Example,
                &format!("v{i}"),
                "无名",
                "一段无关的内容",
                Some(1),
            )
            .await;
        }
        for i in 0..10 {
            f.put(
                KbKind::Example,
                &format!("t{i}"),
                "无名",
                "这次用的是 Dyneema 面料",
                None,
            )
            .await;
        }
        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "Dyneema 面料的新包",
                    vector: Some(&v(1)),
                    limit: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(got.counts.vector, VECTOR_TOP_K);
        assert!(got.counts.fts > 0, "全文路该捞到 Dyneema 那几条");
        assert!(got.counts.merged > RERANK_MAX, "凑不够就验不到截断");
        assert_eq!(got.counts.truncated, got.counts.merged - RERANK_MAX);
        assert!(got.docs.len() <= RERANK_MAX);
    }

    #[test]
    fn 关键词粗筛只管长度() {
        let tok = Tokenizer::new();
        let terms = key_phrases(&tok, "这次用的是 Dyneema 面料，2026SS 新品上市");
        assert!(
            terms.iter().any(|t| t.eq_ignore_ascii_case("dyneema")),
            "{terms:?}"
        );
        assert!(terms.iter().any(|t| t.contains("2026")), "{terms:?}");
        // 单字不要：它们命中一切
        assert!(!terms.iter().any(|t| t.chars().count() < 2), "{terms:?}");
        // 「新品」「上市」这类**留在这里**，由调用侧按词频剔——
        // 按长度分辨不了「Dyneema」和「新品」，这一层不假装能分辨
        assert!(terms.len() <= FTS_TERMS);
        // 长的排前面：额度花在信息量大的词上
        let lens: Vec<usize> = terms.iter().map(|t| t.chars().count()).collect();
        assert!(lens.windows(2).all(|w| w[0] >= w[1]), "{terms:?}");
    }

    #[tokio::test]
    async fn 满库都有的词要按词频剔掉() {
        let f = Fixture::new().await;
        // 每一条都写着「新品上市」，只有一条写了 Dyneema
        for i in 0..40 {
            f.put(
                KbKind::Example,
                &format!("e{i}"),
                "无名",
                "新品上市 的一条",
                None,
            )
            .await;
        }
        f.put(
            KbKind::Example,
            "special",
            "无名",
            "新品上市 用的是 Dyneema 面料",
            None,
        )
        .await;

        let got = f
            .retriever()
            .search(
                &f.conn,
                &Query {
                    text: "新品上市 用的是 Dyneema 面料",
                    limit: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // 「新品」命中 41 篇，远超 5% 的阈值，不该把整库拉进来
        assert!(
            got.counts.fts <= FTS_PER_TERM * 2,
            "满库都有的词没被剔掉：{:?}",
            got.counts
        );
        assert!(
            got.docs.iter().any(|s| s.doc.ref_id == "special"),
            "Dyneema 那条该被捞回来"
        );
    }

    #[test]
    fn 截断按字符不按字节() {
        let long = "山".repeat(SNIPPET_CHARS + 50);
        // 按字节截会把一个汉字劈成半个，重排看到的就是乱码
        assert_eq!(
            snippet_text(&long, SNIPPET_CHARS).chars().count(),
            SNIPPET_CHARS
        );
        assert_eq!(snippet_text("  短的  ", SNIPPET_CHARS), "短的");
    }

    #[test]
    fn 只命中品牌路的不会被向量路挤掉() {
        let mut routes: HashMap<i64, Routes> = HashMap::new();
        // 向量路满额 24 条（相似度从高到低是 124..101，故意与 id 大小相反）
        let vector: Vec<i64> = (101..=124).rev().collect();
        for id in &vector {
            routes.entry(*id).or_default().vector = true;
        }
        let brand = vec![7, 124];
        for id in &brand {
            routes.entry(*id).or_default().brand = true;
        }
        let fts = vec![9];
        routes.entry(9).or_default().fts = true;

        let mut ids = merge_order(&routes, [&vector, &brand, &fts]);
        ids.truncate(RERANK_MAX);
        assert_eq!(ids[0], 124, "两路都命中的排第一");
        assert!(ids.contains(&7), "只从品牌路来的被截掉了：{ids:?}");
        assert!(ids.contains(&9), "只从全文路来的被截掉了：{ids:?}");
        // 向量路按相似度截：最像的 123 留着，最不像的 101 被截掉
        assert!(ids.contains(&123));
        assert!(!ids.contains(&101));
    }
}
