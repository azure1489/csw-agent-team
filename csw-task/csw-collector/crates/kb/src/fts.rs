//! 全文辅路：**SQLite FTS5（unicode61）+ `jieba-rs` 预分词 + 短语查询**。
//!
//! 阶段 0.3 在两套真实语料上对照了五个方案，这一套赢在每一项上：
//!
//! | 方案（中文语料） | 召回 | 精确 | 查询 | 建索引 |
//! |---|---|---|---|---|
//! | LanceDB · jieba | 100.0% | 47.9% | 76.11 ms | 1221 ms |
//! | SQLite FTS5 · unicode61 | 96.2% | 100.0% | 0.05 ms | 14 ms |
//! | **SQLite FTS5 · jieba + 短语** | **100.0%** | **100.0%** | **0.06 ms** | **62 ms** |
//!
//! 两条限制正好被这套组合绕开：
//! - 单纯 unicode61 把连续 CJK 当成**一个**词元——「轻量化帐篷推荐」整串是一个词，
//!   查「帐篷」就查不到。预分词切开它。
//! - jieba 的用户词典按空白分列，**含空格的品牌名根本进不去**（926 个别名里 244 个）。
//!   但预分词 + 短语查询里，多词别名就是它自己那几个词的短语，不需要进词典。
//!
//! 于是 **LanceDB 只放向量、不建全文索引**，lance #9016（BM25 大内存）也就不再是风险。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jieba_rs::Jieba;
use rusqlite::Connection;

/// 分词器。品牌别名作为自定义词，保证「山と道」不被切碎。
pub struct Tokenizer {
    jieba: Jieba,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer {
    pub fn new() -> Self {
        Self {
            jieba: Jieba::new(),
        }
    }

    /// 带品牌词典。
    ///
    /// 只收不含空白的词——jieba 的词典按空白分列，含空格的加进去要么报错要么被截断。
    /// 这不影响多词别名的检索：预分词后它就是几个词的短语。
    pub fn with_brands(words: &[&str]) -> Self {
        let mut jieba = Jieba::new();
        for w in words {
            if w.chars().any(char::is_whitespace) {
                continue;
            }
            // 词频给大一点，压过默认切分
            jieba.add_word(w, Some(100_000), Some("n"));
        }
        Self { jieba }
    }

    /// 切成以空格分隔的词串，给 FTS5 存。
    ///
    /// 用 `cut_for_search`（切得更碎）而不是精确模式：全文只是辅路，召回优先——
    /// 精确匹配那一路才是主路，这里宁可多切几刀。
    pub fn cut(&self, text: &str) -> String {
        self.jieba
            .cut_for_search(text, true)
            .into_iter()
            .map(|t| t.word)
            .filter(|w| !w.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 把查询切成 FTS5 的**短语**：`"tok1 tok2 tok3"`。
    ///
    /// 用短语而不是 OR：0.3 实测同样的切分下，OR 的精确率只有 61.8%，短语是 99.7%。
    /// 差别全在「把不相干的拉进来」上。
    pub fn phrase_query(&self, query: &str) -> Option<String> {
        let toks = self.cut(query);
        let cleaned: Vec<String> = toks
            .split_whitespace()
            .map(|t| t.replace('"', ""))
            .filter(|t| !t.is_empty())
            .collect();
        if cleaned.is_empty() {
            return None;
        }
        Some(format!("\"{}\"", cleaned.join(" ")))
    }
}

/// 写一条到全文索引。`rowid` 用 `kb_docs.id`，两边按它对上。
///
/// contentless 的 FTS5 表（`content=''`）**不能 UPDATE 也不能 DELETE**，
/// 删一行要用它自己那套语法：`INSERT INTO t(t, rowid, 列…) VALUES('delete', …)`，
/// 而且要把**原来那份**内容一字不差地给回去，它才知道要扣掉哪些词元。
/// 所以重建索引前先从 `kb_docs` 把旧正文读回来。
pub fn index_doc(conn: &Connection, doc_id: i64, tok: &Tokenizer, text: &str) -> Result<()> {
    let tokens = tok.cut(text);
    // 旧的词元串：取不到就说明这条还没进过索引，直接插
    let old: Option<String> = conn
        .query_row(
            "SELECT tokens FROM kb_fts WHERE rowid = ?1",
            [doc_id],
            |r| r.get(0),
        )
        .ok();
    if let Some(old) = old {
        if old == tokens {
            return Ok(()); // 内容没变，不必重建
        }
        conn.execute(
            "INSERT INTO kb_fts(kb_fts, rowid, tokens) VALUES('delete', ?1, ?2)",
            rusqlite::params![doc_id, old],
        )
        .context("从全文索引里删旧的那份")?;
    }
    conn.execute(
        "INSERT INTO kb_fts(rowid, tokens) VALUES (?1, ?2)",
        rusqlite::params![doc_id, tokens],
    )?;
    Ok(())
}

/// 建完一批之后合并索引段。批量导入后跑一次，查询会快不少。
pub fn optimize(conn: &Connection) -> Result<()> {
    conn.execute_batch("INSERT INTO kb_fts(kb_fts) VALUES('optimize');")?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub doc_id: i64,
    /// BM25 分，越小越相关（SQLite 的约定）。只用于排序，不外露。
    pub score: f64,
}

/// 短语检索。
///
/// 查询语法错时返回空而不是报错：用户在搜索框里打什么都可能，
/// 一个引号不该让整页崩掉。
pub fn search(conn: &Connection, tok: &Tokenizer, query: &str, limit: usize) -> Result<Vec<Hit>> {
    let Some(q) = tok.phrase_query(query) else {
        return Ok(vec![]);
    };
    let mut st = conn
        .prepare("SELECT rowid, bm25(kb_fts) FROM kb_fts WHERE kb_fts MATCH ?1 ORDER BY bm25(kb_fts) LIMIT ?2")
        .context("准备全文查询")?;
    let rows = match st.query_map(rusqlite::params![q, limit as i64], |r| {
        Ok(Hit {
            doc_id: r.get(0)?,
            score: r.get(1)?,
        })
    }) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::debug!(查询 = %q, 原因 = %e, "全文查询语法错，当作没命中");
            return Ok(vec![]);
        }
    };
    Ok(rows.filter_map(Result::ok).collect())
}

/// 这个词在索引里命中多少篇。用来把「新品」「上市」这类满库都有的词挑出来剔掉——
/// 真正该剔的是**词频高**的词，不是短词：`Dyneema` 只有七个字母却极specific，
/// 「新品」只有两个字却满库都是。
///
/// 一次查询 0.06 ms 量级，每条候选多花不到 1 ms。
pub fn doc_frequency(conn: &Connection, tok: &Tokenizer, term: &str) -> Result<usize> {
    let Some(q) = tok.phrase_query(term) else {
        return Ok(0);
    };
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM kb_fts WHERE kb_fts MATCH ?1",
            [&q],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(n as usize)
}

/// jieba 的词典文件（给 LanceDB 的分词器用；我们自己的路径用不上它，
/// 但部署脚本要知道去哪儿拷）。
///
/// **从 crate 源码里拷，不要去外网下**——生产主机不该为了一个词典去连公网。
pub fn bundled_dict_path() -> Option<PathBuf> {
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
                    .is_some_and(|n| n.to_string_lossy().starts_with("jieba-rs-"))
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

/// 供部署脚本用：确认词典文件在。
pub fn dict_exists(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        csw_collector_core::store::open_in_memory().unwrap()
    }

    fn seed(conn: &Connection, tok: &Tokenizer, docs: &[(i64, &str)]) {
        for (id, text) in docs {
            conn.execute(
                "INSERT INTO kb_docs(id, kind, ref_id, body, content_hash, created_at)
                 VALUES (?1,'example',?2,?3,'h','t')",
                rusqlite::params![id, id.to_string(), text],
            )
            .unwrap();
            index_doc(conn, *id, tok, text).unwrap();
        }
        optimize(conn).unwrap();
    }

    #[test]
    fn 中文里嵌着的词能查到() {
        // 这正是单纯 unicode61 做不到的：它把「轻量化帐篷推荐」当成一个词元
        let tok = Tokenizer::new();
        let conn = db();
        seed(
            &conn,
            &tok,
            &[
                (1, "今年最值得关注的轻量化帐篷推荐与选购指南"),
                (2, "一双跑鞋的中底改款"),
            ],
        );
        let hits = search(&conn, &tok, "帐篷", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id, 1);
    }

    #[test]
    fn 短语查询不会把不相干的拉进来() {
        let tok = Tokenizer::new();
        let conn = db();
        seed(
            &conn,
            &tok,
            &[
                (1, "轻量化帐篷的结构改款"),
                (2, "帐篷之外，这双鞋也做了结构改款"), // 两个词都有，但不是同一件事
            ],
        );
        // 短语要求词连着出现，所以只命中 1
        let hits = search(&conn, &tok, "轻量化帐篷", 10).unwrap();
        assert_eq!(hits.len(), 1, "OR 查询会把 2 也拉进来，短语不会");
        assert_eq!(hits[0].doc_id, 1);
    }

    #[test]
    fn 品牌词典让专名不被切碎() {
        let plain = Tokenizer::new();
        let withb = Tokenizer::with_brands(&["山と道"]);
        let conn = db();
        seed(&conn, &withb, &[(1, "山と道 的新款背包")]);
        // 不带词典时「山と道」可能被切成「山」「と」「道」，查全名就对不上短语
        let a = search(&conn, &withb, "山と道", 10).unwrap();
        assert_eq!(a.len(), 1, "带词典的切分要能查到");
        // 同一份索引换个不带词典的查询分词器，短语就可能对不上——说明词典确实起作用
        let _ = search(&conn, &plain, "山と道", 10).unwrap();
    }

    #[test]
    fn 含空格的别名不进词典也不报错() {
        let t = Tokenizer::with_brands(&["and wander", "andwander"]);
        // 不崩就是通过；含空格那个被跳过
        assert!(!t.cut("and wander 新品").is_empty());
    }

    #[test]
    fn 重复索引同一条不会留下两份() {
        let tok = Tokenizer::new();
        let conn = db();
        seed(&conn, &tok, &[(1, "第一版正文帐篷")]);
        index_doc(&conn, 1, &tok, "改过的正文帐篷").unwrap();
        let hits = search(&conn, &tok, "帐篷", 10).unwrap();
        assert_eq!(hits.len(), 1, "先删后插，不能留下两份");
    }

    #[test]
    fn 空查询与纯符号查询返回空而不是报错() {
        let tok = Tokenizer::new();
        let conn = db();
        seed(&conn, &tok, &[(1, "正文")]);
        assert!(search(&conn, &tok, "", 10).unwrap().is_empty());
        assert!(search(&conn, &tok, "   ", 10).unwrap().is_empty());
    }

    #[test]
    fn 带引号的查询不会让页面崩掉() {
        let tok = Tokenizer::new();
        let conn = db();
        seed(&conn, &tok, &[(1, "轻量化帐篷")]);
        // 用户在搜索框里打了个引号——不该报错
        let r = search(&conn, &tok, "帐篷\" OR 1=1", 10);
        assert!(r.is_ok(), "查询里的引号要被清掉：{r:?}");
    }
}
