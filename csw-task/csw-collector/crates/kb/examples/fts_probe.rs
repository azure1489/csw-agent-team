//! 阶段 0.3 的对照实验：全文辅路到底用 LanceDB 内置 jieba，还是 SQLite FTS5 + jieba-rs。
//!
//! 底料是真实数据——csw 的 9/17–9/18 窗口贴文（438 条）与 543 个在册账号名。
//! 金标不是人工标注，而是**主路自己**：别名在正文里精确出现（大小写不敏感）即算命中。
//! 辅路要回答的只有一个问题：主路漏掉的那些，全文检索能不能补上；以及全文检索
//! 会不会把不相干的塞进来。
//!
//! 五个变体：
//!   1. LanceDB FTS · simple 分词器（基线）
//!   2. LanceDB FTS · jieba/default（自带词典）
//!   3. LanceDB FTS · jieba/brands（自带词典 + 543 个品牌名作自定义词典）
//!   4. SQLite FTS5 · unicode61（原文直接入库）
//!   5. SQLite FTS5 · jieba-rs 预分词（品牌名进 jieba 词典后切好再入库）
//!
//! 跑法：
//!   cargo run -p csw-collector-kb --example fts_probe -- --data <装着 acc_*.json / w_*.json 的目录>
//!
//! `--data` 指向的目录不进仓库：里面是第三方贴文原文。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use futures::TryStreamExt;
use lancedb::arrow::arrow_array::{Int32Array, RecordBatch, RecordBatchIterator, StringArray};
use lancedb::arrow::arrow_schema::{DataType, Field, Schema};
use lancedb::index::Index;
use lancedb::index::scalar::{FtsIndexBuilder, FullTextSearchQuery};
use lancedb::query::{ExecutableQuery, QueryBase};

/// 取前 K 条判命中——工作台的知识库面板一屏也就这么多
const TOP_K: usize = 20;
/// 太短的别名（如 "GO"）在正文里到处都是，金标本身就不可信，剔掉
const MIN_ALIAS_LEN: usize = 4;

struct Doc {
    id: i32,
    text: String,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let data = arg(&args, "--data").context("必须给 --data <目录>")?;
    let workdir = std::env::temp_dir().join("csw-fts-probe");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir)?;

    let (docs, brands) = load(Path::new(&data))?;
    println!(
        "语料甲 · csw 贴文：{} 条，{} 个品牌别名",
        docs.len(),
        brands.len()
    );

    // 金标：别名在正文里精确出现
    let truth = ground_truth(&docs, &brands);
    let probed: Vec<&String> = truth.keys().collect();
    let hit_docs: HashSet<i32> = truth.values().flatten().copied().collect();
    println!(
        "金标：{} 个别名在正文里出现过，覆盖 {} 条贴文（{:.0}%）\n",
        probed.len(),
        hit_docs.len(),
        100.0 * hit_docs.len() as f64 / docs.len() as f64
    );

    let dict = prepare_jieba_home(&workdir, &brands)?;
    println!("jieba 词典目录 {}\n", dict.display());

    let rt = tokio::runtime::Runtime::new()?;
    let mut rows = Vec::new();
    for (label, tokenizer) in [
        ("LanceDB · simple", "simple"),
        ("LanceDB · jieba 自带词典", "jieba/default"),
        ("LanceDB · jieba + 品牌词典", "jieba/brands"),
    ] {
        rows.push(rt.block_on(lance_variant(&workdir, label, tokenizer, &docs, &truth))?);
    }
    rows.push(sqlite_variant(
        "SQLite FTS5 · unicode61",
        &docs,
        &truth,
        false,
        true,
    )?);
    rows.push(sqlite_variant(
        "SQLite FTS5 · jieba+OR",
        &docs,
        &truth,
        true,
        false,
    )?);
    rows.push(sqlite_variant(
        "SQLite FTS5 · jieba+短语",
        &docs,
        &truth,
        true,
        true,
    )?);

    print_table("语料甲（csw 贴文）", &rows);
    println!("\n只看含汉字/假名的查询（{} 个）", rows[0].cjk.2);
    println!("{:<28} {:>8} {:>8}", "变体", "召回", "精确");
    for r in &rows {
        println!(
            "{:<28} {:>7.1}% {:>7.1}%",
            r.label,
            r.cjk.0 * 100.0,
            r.cjk.1 * 100.0
        );
    }

    println!("\n#9016 内存探针（BM25 宽查询，每变体各 50 次高频词查询）");
    rt.block_on(bm25_memory_probe(&workdir, &docs))?;

    // 语料乙：中文长文。辅路真正要服务的是知识库里的已发文章，不是英文为主的贴文。
    if let Some(zh) = arg(&args, "--zh") {
        let (zdocs, zqueries) = load_zh(Path::new(&zh))?;
        println!(
            "\n\n语料乙 · 中文已发文章：{} 篇，{} 个查询词",
            zdocs.len(),
            zqueries.len()
        );
        let ztruth = ground_truth(&zdocs, &zqueries);
        println!("金标：{} 个查询词在正文里出现过", ztruth.len());
        let zdir = workdir.join("zh");
        std::fs::create_dir_all(&zdir)?;
        let mut zrows = Vec::new();
        for (label, tokenizer) in [
            ("LanceDB · simple", "simple"),
            ("LanceDB · jieba + 品牌词典", "jieba/brands"),
        ] {
            zrows.push(rt.block_on(lance_variant(&zdir, label, tokenizer, &zdocs, &ztruth))?);
        }
        zrows.push(sqlite_variant(
            "SQLite FTS5 · unicode61",
            &zdocs,
            &ztruth,
            false,
            true,
        )?);
        zrows.push(sqlite_variant(
            "SQLite FTS5 · jieba+短语",
            &zdocs,
            &ztruth,
            true,
            true,
        )?);
        print_table("语料乙（全中文）", &zrows);
    }

    let _ = std::fs::remove_dir_all(&workdir);
    Ok(())
}

/// lance #9016：BM25 查询在某些词形下会吃掉异常大的内存。
/// 这里拿真实语料里最高频的词反复查，看 RSS 会不会跑飞。
async fn bm25_memory_probe(workdir: &Path, docs: &[Doc]) -> Result<()> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for d in docs {
        for w in d.text.split_whitespace() {
            let w: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if w.chars().count() >= 3 {
                *freq.entry(w.to_lowercase()).or_default() += 1;
            }
        }
    }
    let mut top: Vec<(String, usize)> = freq.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let terms: Vec<String> = top.into_iter().take(10).map(|(w, _)| w).collect();
    println!("  高频词 {terms:?}");

    for tokenizer in ["simple", "jieba/brands"] {
        let uri = workdir.join(format!("lance-{}", tokenizer.replace('/', "-")));
        let db = lancedb::connect(&uri.to_string_lossy()).execute().await?;
        let tbl = db.open_table("docs").execute().await?;
        let before = rss_kb();
        let t0 = Instant::now();
        for _ in 0..5 {
            for term in &terms {
                let _ = tbl
                    .query()
                    .full_text_search(FullTextSearchQuery::new(term.clone()))
                    .limit(TOP_K)
                    .execute()
                    .await?
                    .try_collect::<Vec<_>>()
                    .await?;
            }
        }
        println!(
            "  {tokenizer:<14} 50 次查询 {:>5}ms，RSS {} → {}",
            t0.elapsed().as_millis(),
            fmt_kb(before),
            fmt_kb(rss_kb())
        );
    }
    Ok(())
}

fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn fmt_kb(v: Option<u64>) -> String {
    match v {
        Some(kb) => format!("{:.0} MB", kb as f64 / 1024.0),
        None => "不可用".into(),
    }
}

struct Row {
    label: String,
    recall: f64,
    precision: f64,
    empty: usize,
    index_ms: u128,
    avg_query_ms: f64,
    /// 只统计含汉字/假名的查询——辅路真正要解决的是这一类
    cjk: (f64, f64, usize),
}

/// 查询里带汉字或假名吗？unicode61 把连续的 CJK 当成一个词元，
/// 所以「一段中文里嵌着的品牌名」正是它会漏的那类。
fn is_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32, 0x3040..=0x30FF | 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
    })
}

fn print_table(title: &str, rows: &[Row]) {
    println!(
        "\n{title}\n{:<28} {:>8} {:>8} {:>8} {:>10} {:>10}",
        "变体", "召回", "精确", "零结果", "建索引", "查询均值"
    );
    for r in rows {
        println!(
            "{:<28} {:>7.1}% {:>7.1}% {:>7} {:>9}ms {:>9.2}ms",
            r.label,
            r.recall * 100.0,
            r.precision * 100.0,
            r.empty,
            r.index_ms,
            r.avg_query_ms
        );
    }
}

/// 中文语料：知识库范例的 kb_import.jsonl。文档 = 已发文章正文；
/// 查询词 = 每篇抽出的品牌与产品名（items[].brand / product），这正是
/// 「在一段中文里找一个嵌着的名字」——unicode61 单独最容易漏的那类。
fn load_zh(path: &Path) -> Result<(Vec<Doc>, Vec<String>)> {
    let mut docs = Vec::new();
    let mut queries = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)?;
        let body = [
            v.get("title").and_then(|s| s.as_str()).unwrap_or(""),
            v.get("body_text").and_then(|s| s.as_str()).unwrap_or(""),
        ]
        .join("\n");
        // 长文按段切成检索单元，和知识库里「条目」的粒度对齐
        for para in body.split("\n\n") {
            if para.chars().count() >= 30 {
                docs.push(Doc {
                    id: docs.len() as i32,
                    text: para.to_string(),
                });
            }
        }
        for it in v
            .get("items")
            .and_then(|i| i.as_array())
            .into_iter()
            .flatten()
        {
            for key in ["brand", "product"] {
                if let Some(s) = it.get(key).and_then(|s| s.as_str()) {
                    // 产品字段常是整句描述，只取前两个短语当查询词
                    for piece in s.split(['，', '。', '、', ',']).take(2) {
                        let piece = piece.trim();
                        if piece.chars().count() >= 3 && piece.chars().count() <= 20 {
                            queries.push(piece.to_string());
                        }
                    }
                }
            }
        }
    }
    queries.sort();
    queries.dedup();
    anyhow::ensure!(!docs.is_empty() && !queries.is_empty(), "中文语料为空");
    Ok((docs, queries))
}

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// 从 csw 的原始 JSON 转储里读贴文与账号名。信封是 `{success, data}`，
/// 账号接口的 data 里还套了第二层 success，`total` 是**页数**不是条数。
fn load(dir: &Path) -> Result<(Vec<Doc>, Vec<String>)> {
    let mut docs = Vec::new();
    let mut brands = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    for f in files {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let v: serde_json::Value = serde_json::from_reader(std::fs::File::open(&f)?)
            .with_context(|| format!("读 {name}"))?;
        let data = v.get("data").unwrap_or(&v);
        if name.starts_with("acc_") {
            for it in data
                .get("items")
                .and_then(|i| i.as_array())
                .into_iter()
                .flatten()
            {
                for key in ["accountName", "account"] {
                    if let Some(s) = it.get(key).and_then(|s| s.as_str()) {
                        brands.push(s.trim().to_string());
                    }
                }
            }
        } else if name.starts_with("w_") {
            for it in data
                .get("items")
                .and_then(|i| i.as_array())
                .into_iter()
                .flatten()
            {
                let text = ["description", "translatedText", "account"]
                    .iter()
                    .filter_map(|k| it.get(*k).and_then(|s| s.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                docs.push(Doc {
                    id: docs.len() as i32,
                    text,
                });
            }
        }
    }
    brands.sort();
    brands.dedup();
    brands.retain(|b| b.chars().count() >= MIN_ALIAS_LEN);
    anyhow::ensure!(
        !docs.is_empty() && !brands.is_empty(),
        "底料为空，检查 --data 目录"
    );
    Ok((docs, brands))
}

fn ground_truth(docs: &[Doc], brands: &[String]) -> HashMap<String, Vec<i32>> {
    let lowered: Vec<String> = docs.iter().map(|d| d.text.to_lowercase()).collect();
    let mut out = HashMap::new();
    for b in brands {
        let needle = b.to_lowercase();
        let hits: Vec<i32> = lowered
            .iter()
            .enumerate()
            .filter(|(_, t)| t.contains(&needle))
            .map(|(i, _)| docs[i].id)
            .collect();
        if !hits.is_empty() {
            out.insert(b.clone(), hits);
        }
    }
    out
}

/// 造 `$LANCE_LANGUAGE_MODEL_HOME`：jieba/default 用自带词典，jieba/brands 再挂一份品牌词典。
/// 词典从 jieba-rs crate 源码里拷——生产部署也照这条路，别去外网下。
fn prepare_jieba_home(workdir: &Path, brands: &[String]) -> Result<PathBuf> {
    let home = workdir.join("langmodels");
    let src = find_jieba_dict().context(
        "找不到 jieba 的 dict.txt。它在 jieba-rs crate 的 src/data/dict.txt，\
         先 cargo fetch 再跑，或用 --dict 指定",
    )?;
    for variant in ["default", "brands"] {
        let d = home.join("jieba").join(variant);
        std::fs::create_dir_all(&d)?;
        std::fs::copy(&src, d.join("dict.txt"))?;
    }
    // 品牌词典：jieba 的用户词典一行一个「词 词频 词性」，按空白分列——
    // 所以**含空格的品牌名根本进不去**（"adidas Outdoor" 会被读成 词=adidas 词频=Outdoor 然后报错）。
    // 这条限制本身就是结论的一部分：543 个品牌里多数是多词拉丁名，jieba 的词典救不了它们。
    let brands_dir = home.join("jieba").join("brands");
    let (single, multi): (Vec<&String>, Vec<&String>) = brands
        .iter()
        .partition(|b| !b.chars().any(char::is_whitespace));
    println!(
        "品牌词典：{} 个无空格别名可进 jieba 用户词典，{} 个含空格的进不去",
        single.len(),
        multi.len()
    );
    let user: String = single.iter().map(|b| format!("{b} 100000 n\n")).collect();
    std::fs::write(brands_dir.join("brands.txt"), user)?;
    std::fs::write(
        brands_dir.join("config.json"),
        r#"{"main":"dict.txt","users":["brands.txt"]}"#,
    )?;
    unsafe { std::env::set_var("LANCE_LANGUAGE_MODEL_HOME", &home) };
    Ok(home)
}

fn find_jieba_dict() -> Option<PathBuf> {
    let home = std::env::var("CARGO_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".cargo"))
        })?;
    let reg = home.join("registry").join("src");
    for entry in std::fs::read_dir(reg).ok()? {
        let dir = entry.ok()?.path();
        let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("jieba-rs-"))
                    .unwrap_or(false)
            })
            .collect();
        found.sort();
        if let Some(p) = found.last() {
            let dict = p.join("src").join("data").join("dict.txt");
            if dict.exists() {
                return Some(dict);
            }
        }
    }
    None
}

async fn lance_variant(
    workdir: &Path,
    label: &str,
    tokenizer: &str,
    docs: &[Doc],
    truth: &HashMap<String, Vec<i32>>,
) -> Result<Row> {
    let uri = workdir.join(format!("lance-{}", tokenizer.replace('/', "-")));
    let db = lancedb::connect(&uri.to_string_lossy()).execute().await?;
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("doc", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from_iter_values(docs.iter().map(|d| d.id))),
            Arc::new(StringArray::from_iter_values(
                docs.iter().map(|d| d.text.as_str()),
            )),
        ],
    )?;
    let tbl = db
        .create_table(
            "docs",
            Box::new(RecordBatchIterator::new(
                vec![Ok(batch)].into_iter(),
                schema,
            )) as Box<dyn lancedb::arrow::arrow_array::RecordBatchReader + Send>,
        )
        .execute()
        .await?;

    let t0 = Instant::now();
    tbl.create_index(
        &["doc"],
        Index::FTS(FtsIndexBuilder::default().base_tokenizer(tokenizer.to_string())),
    )
    .execute()
    .await
    .with_context(|| format!("建 FTS 索引（分词器 {tokenizer}）"))?;
    let index_ms = t0.elapsed().as_millis();

    let mut score = Score::default();
    for (brand, want) in truth {
        let t = Instant::now();
        let batches = tbl
            .query()
            .full_text_search(FullTextSearchQuery::new(brand.clone()))
            .limit(TOP_K)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        score.add(
            t.elapsed().as_secs_f64() * 1000.0,
            ids(&batches),
            want,
            is_cjk(brand),
        );
    }
    Ok(score.finish(label, index_ms))
}

fn ids(batches: &[RecordBatch]) -> Vec<i32> {
    batches
        .iter()
        .filter_map(|b| b.column_by_name("id").cloned())
        .filter_map(|c| {
            c.as_any()
                .downcast_ref::<Int32Array>()
                .map(|a| a.values().to_vec())
        })
        .flatten()
        .collect()
}

fn sqlite_variant(
    label: &str,
    docs: &[Doc],
    truth: &HashMap<String, Vec<i32>>,
    pre_tokenize: bool,
    phrase: bool,
) -> Result<Row> {
    let jieba = pre_tokenize.then(|| {
        let mut j = jieba_rs::Jieba::new();
        for b in truth.keys() {
            j.add_word(b, Some(100_000), Some("n"));
        }
        j
    });
    let cut = |s: &str| -> String {
        match &jieba {
            // search 模式切得更碎，召回优先——全文只是辅路，精确匹配才是主路
            Some(j) => j
                .cut_for_search(s, true)
                .into_iter()
                .map(|t| t.word)
                .collect::<Vec<_>>()
                .join(" "),
            None => s.to_string(),
        }
    };

    let conn = rusqlite::Connection::open_in_memory()?;
    let t0 = Instant::now();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE docs USING fts5(doc, tokenize='unicode61 remove_diacritics 2');",
    )?;
    {
        let mut ins = conn.prepare("INSERT INTO docs(rowid, doc) VALUES (?1, ?2)")?;
        for d in docs {
            ins.execute(rusqlite::params![d.id + 1, cut(&d.text)])?;
        }
    }
    conn.execute_batch("INSERT INTO docs(docs) VALUES('optimize');")?;
    let index_ms = t0.elapsed().as_millis();

    let mut score = Score::default();
    let mut stmt =
        conn.prepare("SELECT rowid FROM docs WHERE docs MATCH ?1 ORDER BY bm25(docs) LIMIT ?2")?;
    for (brand, want) in truth {
        // FTS5 的 MATCH 语法对标点敏感，整体当短语查；切过词的按 OR 查
        let q = if pre_tokenize {
            let terms = cut(brand);
            let toks: Vec<String> = terms
                .split_whitespace()
                .map(|t| t.replace('"', ""))
                .collect();
            if phrase {
                // 切好的词按原序当短语查：既吃到中文切分，又不像 OR 那样把不相干的拉进来
                format!("\"{}\"", toks.join(" "))
            } else {
                toks.iter()
                    .map(|t| format!("\"{t}\""))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            }
        } else {
            format!("\"{}\"", brand.replace('"', ""))
        };
        if q.trim().is_empty() {
            continue;
        }
        let t = Instant::now();
        let got: Vec<i32> = match stmt.query_map(rusqlite::params![q, TOP_K as i64], |r| {
            r.get::<_, i64>(0).map(|v| (v - 1) as i32)
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            // 语法错的查询算零结果，不算崩——真实系统里也得这么兜
            Err(_) => Vec::new(),
        };
        score.add(t.elapsed().as_secs_f64() * 1000.0, got, want, is_cjk(brand));
    }
    Ok(score.finish(label, index_ms))
}

#[derive(Default)]
struct Bucket {
    recall_sum: f64,
    precision_sum: f64,
    n: usize,
    empty: usize,
}

impl Bucket {
    fn add(&mut self, inter: f64, want: usize, got: usize) {
        self.recall_sum += inter / want as f64;
        // 精确率只在返回非空时计，否则零结果会被算成「很精确」
        if got == 0 {
            self.empty += 1;
        } else {
            self.precision_sum += inter / got as f64;
        }
        self.n += 1;
    }
    fn rates(&self) -> (f64, f64) {
        let n = self.n.max(1) as f64;
        let answered = (self.n - self.empty).max(1) as f64;
        (self.recall_sum / n, self.precision_sum / answered)
    }
}

#[derive(Default)]
struct Score {
    all: Bucket,
    cjk: Bucket,
    ms: f64,
}

impl Score {
    fn add(&mut self, ms: f64, got: Vec<i32>, want: &[i32], cjk: bool) {
        let want_set: HashSet<i32> = want.iter().copied().collect();
        let got_set: HashSet<i32> = got.into_iter().collect();
        let inter = got_set.intersection(&want_set).count() as f64;
        self.all.add(inter, want_set.len(), got_set.len());
        if cjk {
            self.cjk.add(inter, want_set.len(), got_set.len());
        }
        self.ms += ms;
    }

    fn finish(self, label: &str, index_ms: u128) -> Row {
        let (recall, precision) = self.all.rates();
        let (cr, cp) = self.cjk.rates();
        Row {
            label: label.to_string(),
            recall,
            precision,
            empty: self.all.empty,
            index_ms,
            avg_query_ms: self.ms / self.all.n.max(1) as f64,
            cjk: (cr, cp, self.cjk.n),
        }
    }
}
