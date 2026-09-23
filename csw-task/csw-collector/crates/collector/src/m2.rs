//! M2 验收：知识库检索命中率。
//!
//! 计划里写的是「近 30 天已发条目抽检命中率 ≥ 95%」。真做的时候发现**已发条目没有标准答案**：
//! 台账里的已发文章是旧流程发的整篇（52 篇，一半只有标题），不带来源贴文链接；
//! 新流程的决定又都在 9/11 之后，和已发文章的日期对不上。拿文章自己的标题去搜它自己，
//! 测到的只是「索引里有这条」，不是「新贴文来了认得出」。
//!
//! 所以分两组，**只拿第一组当验收**：
//!
//! 1. **历史决定召回（验收）**：近 N 天带 Instagram 原帖链接的决定，取原帖正文当查询
//!    （本地候选有就用本地的，没有就向 csw 取单条），看检索能不能把这条决定找回来。
//!    查询是原帖的外文正文，目标是决定的中文标题与结论——**两段文字不是同一段**，
//!    这才像「一条判过的贴文又冒出来」。同一链接有几条决定，命中任意一条都算。
//! 2. **已发条目自检（下限）**：近 N 天的已发条目，拿去掉栏目后缀的标题去搜。
//!    一半条目正文为空，查询与目标几乎是同一段字，所以**它只证明索引完整、
//!    没被一千八百条已生成贴文淹没**，不证明认得出新贴文。报出来但不算验收。
//!
//! 两个数：**召回命中**（目标进了送重排的那 ≤24 条）与**终选命中**（重排后前 8 条，
//! 只在 `--rerank` 时报）。不开重排时召回池的次序是按命中路数排的，截前 8 条没有意义，
//! 所以不报终选。重排要占 GPU（每条约 3.5 秒），默认不开。
//!
//! **查询只用文字**。正式一轮用的是图文融合向量，这里没下图，向量路会比真实情况弱一点——
//! 测出来的命中率是偏保守的。
//!
//! 输出只有编号、品牌、标题与名次。**决定正文里有 Van 的原话，一个字都不打出来。**

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use csw_collector_core::vector::{EmbedInput, VectorClient, VectorConfig};
use csw_collector_core::{Config, Secrets};
use csw_collector_harvest::csw::{CswClient, CswConfig};
use csw_collector_kb::brands::BrandIndex;
use csw_collector_kb::fts::Tokenizer;
use csw_collector_kb::search::{FINAL_MAX, Query, Retriever};
use csw_collector_kb::vectors::VectorStore;

/// 验收线
pub const PASS_RATE: f64 = 0.95;

pub struct Opts {
    /// 往回看几天（按决定与条目的日期）
    pub days: i64,
    /// 每组最多抽几条。0 = 全部
    pub sample: usize,
    /// 跑重排，报终选命中。占 GPU
    pub rerank: bool,
    /// 本地候选里没有原帖时，向 csw 取单条
    pub fetch: bool,
    /// 报告写到哪
    pub out: PathBuf,
    /// 覆盖配置里的重排段数（`limits.rerank_per_candidate`），比较取舍用
    pub rerank_max: Option<usize>,
    /// 覆盖配置里的每段字数（`limits.rerank_snippet_chars`）
    pub snippet_chars: Option<usize>,
}

/// 一条考题。
#[derive(Debug, Clone)]
struct Case {
    group: Group,
    /// 考题编号：决定组是原帖链接，已发组是条目的 kb_docs.id
    label: String,
    brand: String,
    title: String,
    /// 哪几条 kb_docs 算答对
    targets: Vec<i64>,
    query: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Decision,
    Published,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Self::Decision => "历史决定召回（验收）",
            Self::Published => "已发条目自检（下限）",
        }
    }
}

/// 一条考题的结果。名次从 1 起。
#[derive(Debug, Clone, Default)]
struct Outcome {
    pool_rank: Option<usize>,
    final_rank: Option<usize>,
    /// 命中的那条是从哪几路来的
    routes: Vec<&'static str>,
    pool_size: usize,
}

pub async fn run(cfg: &Config, secrets: &Secrets, o: Opts) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let since = (jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * o.days))
        .strftime("%Y-%m-%d")
        .to_string();

    // ── 出题 ──
    let mut decisions = decision_cases(&conn, &since)?;
    let published = published_cases(&conn, &since)?;
    let local = local_texts(&conn)?;

    let mut no_source = Vec::new();
    let csw = if o.fetch && !secrets.csw_api_key.is_empty() {
        Some(CswClient::new(CswConfig {
            base_url: cfg.csw.base_url.clone(),
            api_key: secrets.csw_api_key.clone(),
            page_size: cfg.csw.window_page,
            timeout: Duration::from_secs(cfg.csw.timeout_secs),
        })?)
    } else {
        None
    };
    decisions = take_sample(decisions, o.sample);
    let mut from_local = 0;
    let mut from_csw = 0;
    for c in decisions.iter_mut() {
        if let Some(t) = local.get(&norm_url(&c.label)) {
            c.query = t.clone();
            from_local += 1;
            continue;
        }
        let Some(code) = short_code(&c.label) else {
            continue;
        };
        let Some(csw) = &csw else { continue };
        match csw.post(&code).await {
            Ok(p) => {
                c.query = join_text(&p.description, &p.translated_text);
                from_csw += 1;
            }
            Err(e) => tracing::warn!(短码 = %code, 原因 = %format!("{e:#}"), "取不到原帖"),
        }
    }
    decisions.retain(|c| {
        let ok = !c.query.trim().is_empty();
        if !ok {
            no_source.push(c.label.clone());
        }
        ok
    });
    let published = take_sample(published, o.sample);

    println!(
        "决定组 {} 条（本地候选 {from_local}、csw 取 {from_csw}；取不到原帖 {} 条不计入）",
        decisions.len(),
        no_source.len()
    );
    println!("已发组 {} 条", published.len());

    // ── 检索 ──
    let brands = BrandIndex::load(&conn).context("读别名表")?;
    let tok = Tokenizer::with_brands(&brands.dict_words());
    let store = VectorStore::open(&cfg.lance_path(), crate::EMBED_DIM, &cfg.vector.embed_model)
        .await
        .context("打开向量库")?;
    let vector = VectorClient::new(VectorConfig {
        base_url: cfg.vector.base_url.clone(),
        timeout: Duration::from_secs(cfg.vector.timeout_secs),
        batch_weight: cfg.vector.text_batch,
        ..Default::default()
    })?;
    let vector_ok = vector.healthy().await;
    if !vector_ok {
        println!(
            "向量服务 {} 不可达：向量路与重排都不跑，结果只反映品牌与全文两路",
            cfg.vector.base_url
        );
    }
    let retriever = Retriever {
        store: &store,
        brands: &brands,
        tok: &tok,
        reranker: (o.rerank && vector_ok).then_some(&vector),
    };

    let cases: Vec<Case> = decisions.into_iter().chain(published).collect();
    let vectors: Vec<Option<Vec<f32>>> = if vector_ok {
        let inputs: Vec<EmbedInput> = cases
            .iter()
            .map(|c| EmbedInput::Text(c.query.clone()))
            .collect();
        match vector.embed(&inputs).await {
            Ok(v) => v.into_iter().map(Some).collect(),
            Err(e) => {
                println!("向量化失败，向量路不跑：{e:#}");
                vec![None; cases.len()]
            }
        }
    } else {
        vec![None; cases.len()]
    };

    let rerank_max = o.rerank_max.unwrap_or(cfg.limits.rerank_per_candidate);
    let snippet_chars = o.snippet_chars.unwrap_or(cfg.limits.rerank_snippet_chars);
    println!("重排段数上限 {rerank_max}、每段 {snippet_chars} 字");
    let t = std::time::Instant::now();
    let mut results = Vec::with_capacity(cases.len());
    for (c, v) in cases.iter().zip(&vectors) {
        let q = Query {
            text: &c.query,
            vector: v.as_deref(),
            exclude_post_id: None,
            limit: FINAL_MAX,
            backfill_kinds: false,
            rerank_max,
            snippet_chars,
        };
        let ids = retriever.vector_route(&q).await?;
        let mut r = retriever.recall(&conn, &q, &ids)?;
        let pool: Vec<(i64, Vec<&'static str>)> = r
            .scored
            .iter()
            .map(|s| (s.doc.id, s.routes.names()))
            .collect();
        let mut out = score(&pool, &c.targets);
        if retriever.reranker.is_some() {
            retriever.rerank(&q, &mut r.scored).await;
            let top: Vec<(i64, Vec<&'static str>)> = r
                .scored
                .iter()
                .take(FINAL_MAX)
                .map(|s| (s.doc.id, s.routes.names()))
                .collect();
            out.final_rank = score(&top, &c.targets).pool_rank;
        }
        results.push(out);
    }
    let secs = t.elapsed().as_secs_f64();
    println!(
        "检索 {} 条用时 {secs:.1}s（每条 {:.2}s）\n",
        cases.len(),
        secs / cases.len().max(1) as f64
    );

    let report = render(
        &cases,
        &results,
        &no_source,
        o.rerank && vector_ok,
        vector_ok,
    );
    print!("{}", summary_lines(&cases, &results, o.rerank && vector_ok));
    if let Some(dir) = o.out.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&o.out, report).with_context(|| format!("写报告 {}", o.out.display()))?;
    println!("\n明细：{}", o.out.display());
    Ok(())
}

/// 决定组的考题：按原帖链接归并，同一链接的几条决定都算答案。
fn decision_cases(conn: &Connection, since: &str) -> Result<Vec<Case>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, brand, title FROM kb_docs
          WHERE kind = 'decision' AND url LIKE '%instagram.com/%'
            -- 发布时间是日期的按日期筛；不是日期的（`unknown`、一句说明）照样出题：
            -- 那是原帖的披露时间写不清，不是决定旧。决定本身都出自新流程这几周
            AND (published_at >= ?1 OR COALESCE(published_at, '') NOT GLOB '[0-9][0-9][0-9][0-9]*')
          ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut by_url: Vec<Case> = Vec::new();
    let mut idx: HashMap<String, usize> = HashMap::new();
    for (id, url, brand, title) in rows {
        let key = norm_url(&url);
        match idx.get(&key) {
            Some(&i) => by_url[i].targets.push(id),
            None => {
                idx.insert(key.clone(), by_url.len());
                by_url.push(Case {
                    group: Group::Decision,
                    label: key,
                    brand,
                    title,
                    targets: vec![id],
                    query: String::new(),
                });
            }
        }
    }
    Ok(by_url)
}

/// 已发组的考题：查询是去掉栏目后缀的标题。
fn published_cases(conn: &Connection, since: &str) -> Result<Vec<Case>> {
    let mut stmt = conn.prepare(
        "SELECT id, brand, title FROM kb_docs
          WHERE kind = 'published_item' AND COALESCE(published_at, '') >= ?1
            AND title <> ''
          ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, brand, title)| Case {
            group: Group::Published,
            label: id.to_string(),
            brand,
            query: strip_column_suffix(&title),
            title,
            targets: vec![id],
        })
        .collect())
}

/// 本地候选的正文，按规范化链接索引。
fn local_texts(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT url, text, translated FROM candidates")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, t, tr)| !t.trim().is_empty() || !tr.trim().is_empty())
        .map(|(u, t, tr)| (norm_url(&u), join_text(&t, &tr)))
        .collect())
}

fn join_text(text: &str, translated: &str) -> String {
    if translated.trim().is_empty() {
        text.to_string()
    } else {
        format!("{text}\n{translated}")
    }
}

/// 链接比对用：去掉查询串、片段与末尾斜杠，主机名小写。`/reel/` 与 `/p/` 指同一条，统一成 `/p/`。
fn norm_url(u: &str) -> String {
    let u = u.trim();
    let u = u.split(['?', '#']).next().unwrap_or(u);
    let u = u.trim_end_matches('/');
    let lower = u.to_lowercase();
    match short_code(u) {
        Some(code) if lower.contains("instagram.com/") => {
            format!("https://www.instagram.com/p/{code}")
        }
        _ => lower,
    }
}

/// Instagram 链接里的短码（`/p/`、`/reel/`、`/tv/` 三种）。**短码区分大小写**，原样返回。
fn short_code(u: &str) -> Option<String> {
    let rest = u.split("instagram.com/").nth(1)?;
    let mut parts = rest.split('/').filter(|s| !s.is_empty());
    let kind = parts.next()?;
    if !matches!(kind, "p" | "reel" | "reels" | "tv") {
        return None;
    }
    let code = parts.next()?.split(['?', '#']).next()?;
    (!code.is_empty()).then(|| code.to_string())
}

/// 去掉公众号标题末尾的栏目后缀（`| 营事编集`、`|《CHECK…`）。
///
/// 只认半角竖线：全角「｜」是「品牌｜标题」的分隔，属于标题本身。
fn strip_column_suffix(t: &str) -> String {
    if let Some(i) = t.rfind('|') {
        let tail = t[i + 1..].trim();
        if tail.starts_with('《') || tail.starts_with("营事") || tail.chars().count() <= 8 {
            return t[..i].trim().to_string();
        }
    }
    t.trim().to_string()
}

/// 按考题编号的哈希排序后取前 n 条——同一份数据每次抽到同一批，换数据也不偏向旧的。
fn take_sample(mut cases: Vec<Case>, n: usize) -> Vec<Case> {
    if n == 0 || cases.len() <= n {
        return cases;
    }
    cases.sort_by_key(|c| blake3::hash(c.label.as_bytes()).to_hex().to_string());
    cases.truncate(n);
    cases
}

/// 目标在这串结果里排第几（取最靠前的那条答案）。
fn score(ranked: &[(i64, Vec<&'static str>)], targets: &[i64]) -> Outcome {
    let set: HashSet<i64> = targets.iter().copied().collect();
    let hit = ranked.iter().position(|(id, _)| set.contains(id));
    Outcome {
        pool_rank: hit.map(|i| i + 1),
        final_rank: None,
        routes: hit.map(|i| ranked[i].1.clone()).unwrap_or_default(),
        pool_size: ranked.len(),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Tally {
    n: usize,
    pool: usize,
    fin: usize,
    /// 命中的那条只靠这一路进来的
    only_vector: usize,
    only_brand: usize,
    only_fts: usize,
}

impl Tally {
    fn rate(hit: usize, n: usize) -> f64 {
        if n == 0 { 0.0 } else { hit as f64 / n as f64 }
    }
}

fn tally(cases: &[Case], results: &[Outcome], g: Group) -> Tally {
    let mut t = Tally::default();
    for (c, r) in cases.iter().zip(results) {
        if c.group != g {
            continue;
        }
        t.n += 1;
        if r.pool_rank.is_some() {
            t.pool += 1;
            match r.routes.as_slice() {
                ["向量"] => t.only_vector += 1,
                ["品牌"] => t.only_brand += 1,
                ["全文"] => t.only_fts += 1,
                _ => {}
            }
        }
        if r.final_rank.is_some() {
            t.fin += 1;
        }
    }
    t
}

fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

fn summary_lines(cases: &[Case], results: &[Outcome], reranked: bool) -> String {
    let mut s = String::new();
    for g in [Group::Decision, Group::Published] {
        let t = tally(cases, results, g);
        let pool = Tally::rate(t.pool, t.n);
        s.push_str(&format!(
            "{}：{} 条，召回命中 {}（{}）",
            g.name(),
            t.n,
            t.pool,
            pct(pool)
        ));
        if reranked {
            s.push_str(&format!(
                "，终选命中 {}（{}）",
                t.fin,
                pct(Tally::rate(t.fin, t.n))
            ));
        }
        s.push('\n');
    }
    let d = tally(cases, results, Group::Decision);
    let key = if reranked { d.fin } else { d.pool };
    let rate = Tally::rate(key, d.n);
    s.push_str(&format!(
        "M2 判定（{}）：{} {} {}\n",
        if reranked { "终选" } else { "召回" },
        pct(rate),
        if rate >= PASS_RATE { "≥" } else { "<" },
        pct(PASS_RATE)
    ));
    s.push_str(if d.n == 0 {
        "  → 没有考题，判不了\n"
    } else if rate >= PASS_RATE {
        "  → 过\n"
    } else {
        "  → 没过，看明细里的漏掉的那几条\n"
    });
    s
}

fn render(
    cases: &[Case],
    results: &[Outcome],
    no_source: &[String],
    reranked: bool,
    vector_ok: bool,
) -> String {
    let mut s = String::from("# M2 知识库检索命中率\n\n");
    s.push_str(&format!(
        "生成于 {}。向量路：{}；重排：{}。\n\n",
        jiff::Timestamp::now().strftime("%Y-%m-%d %H:%M UTC"),
        if vector_ok {
            "跑了（只用文字）"
        } else {
            "没跑"
        },
        if reranked {
            "跑了"
        } else {
            "没跑（不报终选）"
        }
    ));
    s.push_str("```\n");
    s.push_str(&summary_lines(cases, results, reranked));
    s.push_str("```\n\n");

    for g in [Group::Decision, Group::Published] {
        let t = tally(cases, results, g);
        s.push_str(&format!("## {}\n\n", g.name()));
        s.push_str(&format!(
            "命中的那条只靠一路进来的：向量 {}、品牌 {}、全文 {}。\n\n",
            t.only_vector, t.only_brand, t.only_fts
        ));
        s.push_str("| 结果 | 考题 | 品牌 | 标题 | 召回名次 / 池大小 | 终选名次 | 从哪路来 |\n");
        s.push_str("|---|---|---|---|---|---|---|\n");
        let mut rows: Vec<(&Case, &Outcome)> = cases
            .iter()
            .zip(results)
            .filter(|(c, _)| c.group == g)
            .collect();
        // 漏掉的排前面，要看的就是它们
        rows.sort_by_key(|(_, r)| (r.pool_rank.is_some(), r.pool_rank));
        for (c, r) in rows {
            s.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                if r.pool_rank.is_some() {
                    "中"
                } else {
                    "**漏**"
                },
                cell(&c.label),
                cell(&c.brand),
                cell(&c.title),
                r.pool_rank
                    .map(|k| format!("{k} / {}", r.pool_size))
                    .unwrap_or_else(|| format!("— / {}", r.pool_size)),
                r.final_rank
                    .map(|k| k.to_string())
                    .unwrap_or_else(|| "—".into()),
                r.routes.join("+"),
            ));
        }
        s.push('\n');
    }
    if !no_source.is_empty() {
        s.push_str("## 取不到原帖、没计入的决定\n\n");
        for u in no_source {
            s.push_str(&format!("- {}\n", cell(u)));
        }
    }
    s
}

/// 表格单元：竖线转义、换行压平、截长。
fn cell(s: &str) -> String {
    let s: String = s.replace('|', "\\|").replace(['\n', '\r'], " ");
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 短码三种链接都认() {
        assert_eq!(
            short_code("https://www.instagram.com/p/DdTvWwEGRt_/").as_deref(),
            Some("DdTvWwEGRt_")
        );
        assert_eq!(
            short_code("https://www.instagram.com/reel/DdTaFVyzaB0/?igsh=x").as_deref(),
            Some("DdTaFVyzaB0")
        );
        assert_eq!(short_code("https://www.instagram.com/someone/"), None);
        assert_eq!(short_code("https://prtimes.jp/main/html/rd/p/1.html"), None);
    }

    #[test]
    fn reel与p算同一条链接() {
        assert_eq!(
            norm_url("https://www.instagram.com/reel/DdTaFVyzaB0/"),
            norm_url("https://instagram.com/p/DdTaFVyzaB0?igsh=abc")
        );
        // 短码区分大小写：不能被小写化成同一条
        assert_ne!(
            norm_url("https://www.instagram.com/p/AbC/"),
            norm_url("https://www.instagram.com/p/abc/")
        );
    }

    #[test]
    fn 栏目后缀去掉_全角竖线留着() {
        assert_eq!(
            strip_column_suffix("Helinox 4P Dome：白天当凉亭的四人帐 | 营事"),
            "Helinox 4P Dome：白天当凉亭的四人帐"
        );
        assert_eq!(
            strip_column_suffix("DINEX：一只美国塑料杯|《CHECK"),
            "DINEX：一只美国塑料杯"
        );
        assert_eq!(
            strip_column_suffix("Snow Peak｜等了 9 个月的充气客厅"),
            "Snow Peak｜等了 9 个月的充气客厅"
        );
    }

    #[test]
    fn 名次取最靠前的那条答案() {
        let ranked = vec![
            (5, vec!["向量"]),
            (9, vec!["品牌", "全文"]),
            (7, vec!["向量"]),
        ];
        let o = score(&ranked, &[7, 9]);
        assert_eq!(o.pool_rank, Some(2));
        assert_eq!(o.routes, vec!["品牌", "全文"]);
        assert_eq!(score(&ranked, &[1]).pool_rank, None);
    }

    #[test]
    fn 抽样是确定的() {
        let mk = |l: &str| Case {
            group: Group::Decision,
            label: l.into(),
            brand: String::new(),
            title: String::new(),
            targets: vec![],
            query: String::new(),
        };
        let a: Vec<Case> = (0..20).map(|i| mk(&format!("u{i}"))).collect();
        let x: Vec<String> = take_sample(a.clone(), 5)
            .into_iter()
            .map(|c| c.label)
            .collect();
        let mut b = a;
        b.reverse();
        let y: Vec<String> = take_sample(b, 5).into_iter().map(|c| c.label).collect();
        assert_eq!(x, y);
    }

    #[test]
    fn 同一链接的几条决定并成一道题() {
        let dir = tempdir::TempDir::new("m2").unwrap();
        let conn = csw_collector_core::store::open(&dir.path().join("t.db")).unwrap();
        for (r, url, at) in [
            (
                "a",
                "https://www.instagram.com/reel/X1/",
                "2026-09-15T00:00:00Z",
            ),
            (
                "b",
                "https://www.instagram.com/p/X1",
                "2026-09-16T00:00:00Z",
            ),
            (
                "c",
                "https://www.instagram.com/p/X2/",
                "2026-09-16T00:00:00Z",
            ),
            (
                "d",
                "https://www.instagram.com/p/OLD/",
                "2026-07-01T00:00:00Z",
            ),
            ("e", "https://prtimes.jp/x", "2026-09-16T00:00:00Z"),
        ] {
            csw_collector_kb::docs::upsert(
                &conn,
                &csw_collector_kb::docs::KbDoc {
                    kind: "decision".into(),
                    ref_id: r.into(),
                    url: url.into(),
                    title: format!("t{r}"),
                    body: "结论：dropped".into(),
                    published_at: Some(at.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let cases = decision_cases(&conn, "2026-09-01").unwrap();
        assert_eq!(
            cases.len(),
            2,
            "X1 两条并一道，X2 一道；旧的与非 Instagram 的不出题"
        );
        assert_eq!(cases[0].targets.len(), 2);
    }

    #[test]
    fn 报告里不打查询正文() {
        // 决定的正文里有 Van 的原话；报告只该有编号、品牌、标题与名次
        let cases = vec![Case {
            group: Group::Decision,
            label: "https://www.instagram.com/p/X1".into(),
            brand: "B".into(),
            title: "B｜标题".into(),
            targets: vec![1],
            query: "原帖正文 SECRET_QUERY".into(),
        }];
        let r = vec![Outcome::default()];
        let md = render(&cases, &r, &[], false, true);
        assert!(!md.contains("SECRET_QUERY"));
        assert!(md.contains("**漏**"));
    }
}
