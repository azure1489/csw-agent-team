//! 三个只读工具：`kb_search`、`kb_similar_selected`、`memory_lookup`。
//!
//! 它们是给深核线程和（可选）Hermes 用的**检索**入口。都只读——
//! 这个服务端里没有任何会改东西的工具。
//!
//! 返回的是**给模型看的文字**，不是 JSON：模型要的是「有哪几条、各是什么、
//! 什么时候的事」，一段排好的文字比一坨 JSON 更省 token 也更少歧义。

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use csw_collector_core::vector::VectorClient;
use csw_collector_kb::brands::BrandIndex;
use csw_collector_kb::docs::KbKind;
use csw_collector_kb::fts::Tokenizer;
use csw_collector_kb::search::{Query, Retriever};
use csw_collector_kb::vectors::VectorStore;

use crate::server::{BoxFut, Tool};

/// 一次检索最多回几条。给模型的上下文是有价的。
const DEFAULT_LIMIT: usize = 6;
const MAX_LIMIT: usize = 12;

/// 三个工具共用的家当。
///
/// `Connection` 不是 `Sync`，所以锁在这里：工具对象要能被多个线程同时持有。
pub struct KbContext {
    pub conn: Mutex<rusqlite::Connection>,
    pub store: VectorStore,
    pub brands: BrandIndex,
    pub tok: Tokenizer,
    /// 向量客户端。**这里只用它算查询向量与重排**，不写库。
    pub vector: Option<VectorClient>,
}

impl KbContext {
    /// 查询文本 → 向量。没有向量服务就返回 None，检索照样走品牌与全文两路。
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let v = self.vector.as_ref()?;
        let inputs = [csw_collector_core::vector::EmbedInput::Text(
            text.to_string(),
        )];
        match v.embed(&inputs).await {
            Ok(mut vs) if !vs.is_empty() => Some(vs.remove(0)),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(原因 = %format!("{e:#}"), "算查询向量失败，只走品牌与全文两路");
                None
            }
        }
    }

    async fn search(&self, q: &str, kinds: Vec<String>, limit: usize) -> Result<String> {
        anyhow::ensure!(!q.trim().is_empty(), "query 不能空");
        let vec = self.embed(q).await;
        let conn = self.conn.lock().await;
        let r = Retriever {
            store: &self.store,
            brands: &self.brands,
            tok: &self.tok,
            // 这条路上不重排：重排要占 GPU，而 GPU 是全局串行的，
            // 深核线程随手一查就把正式轮的向量化堵住不合适
            reranker: None,
        }
        .search(
            &conn,
            &Query {
                text: q,
                vector: vec.as_deref(),
                exclude_post_id: None,
                limit: limit.clamp(1, MAX_LIMIT),
                // 检索工具不补齐：被问「有没有关于 X 的」时，补齐会让答案永远是「有」
                backfill_kinds: false,
            },
        )
        .await?;

        let mut out = String::new();
        if !r.brands_hit.is_empty() {
            out.push_str(&format!("认出的品牌：{}\n\n", r.brands_hit.join("、")));
        }
        let wanted: Vec<KbKind> = if kinds.is_empty() {
            KbKind::ALL.to_vec()
        } else {
            KbKind::ALL
                .into_iter()
                .filter(|k| kinds.iter().any(|x| x == k.as_str()))
                .collect()
        };
        let mut any = false;
        for (kind, hits) in r.by_kind() {
            if !wanted.contains(&kind) {
                continue;
            }
            out.push_str(&format!("【{}】", kind_name(kind)));
            if hits.is_empty() {
                out.push_str("查过，无相关\n");
                continue;
            }
            any = true;
            out.push('\n');
            for s in hits {
                let date = s.doc.published_at.as_deref().unwrap_or("日期不详");
                let title = if s.doc.title.is_empty() {
                    first_line(&s.doc.body)
                } else {
                    s.doc.title.clone()
                };
                out.push_str(&format!("- {title}（{date}"));
                if !s.doc.brand.is_empty() {
                    out.push_str(&format!("，{}", s.doc.brand));
                }
                out.push_str(&format!("，命中路径：{}", s.routes.names().join("+")));
                if s.backfilled {
                    // 补齐来的说服力弱一档，要让模型知道
                    out.push_str("，补齐");
                }
                out.push_str(")\n");
                if !s.doc.url.is_empty() {
                    out.push_str(&format!("  {}\n", s.doc.url));
                }
            }
        }
        if !any {
            out.push_str("\n（三路都没有命中。这不等于「历史上没有发过」，只等于在已同步的参考库里没找到。）\n");
        }
        Ok(out)
    }
}

fn kind_name(k: KbKind) -> &'static str {
    match k {
        KbKind::PublishedItem => "正式已发布的条目",
        KbKind::Example => "范例",
        KbKind::GeneratedPost => "生成过文章的贴文",
        KbKind::Decision => "03 决定",
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(60).collect()
}

fn arg_str(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn arg_limit(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_LIMIT)
}

// ─────────────────────────── kb_search ───────────────────────────

pub struct KbSearch(pub Arc<KbContext>);

impl Tool for KbSearch {
    fn name(&self) -> &'static str {
        "kb_search"
    }
    fn description(&self) -> &'static str {
        "在编辑部参考库里检索：已发布条目、范例、生成过文章的贴文、03 决定。\
         向量、品牌精确匹配、全文三路一起走。只读。"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "要找什么。品牌名、产品名、一句话都行。"},
                "kinds": {
                    "type": "array",
                    "items": {"type": "string", "enum": ["published_item", "example", "generated_post", "decision"]},
                    "description": "只看这几类；不给就四类都看。"
                },
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT}
            },
            "required": ["query"]
        })
    }
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
        Box::pin(async move {
            let kinds = args
                .get("kinds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            self.0
                .search(&arg_str(args, "query"), kinds, arg_limit(args))
                .await
        })
    }
}

// ─────────────────── kb_similar_selected ───────────────────

pub struct KbSimilarSelected(pub Arc<KbContext>);

impl Tool for KbSimilarSelected {
    fn name(&self) -> &'static str {
        "kb_similar_selected"
    }
    fn description(&self) -> &'static str {
        "只在**编辑部真的采用过**的材料里找相似的：正式已发布的条目与范例。\
         用来回答「这类东西我们做过吗、怎么做的」。只读。"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT}
            },
            "required": ["query"]
        })
    }
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
        Box::pin(async move {
            // 「采用过」= 已发布 + 范例。生成过文章的贴文与 03 决定不算——
            // 前者只说明写过，后者可能是**否决**的记录
            let kinds = vec![
                KbKind::PublishedItem.as_str().to_string(),
                KbKind::Example.as_str().to_string(),
            ];
            self.0
                .search(&arg_str(args, "query"), kinds, arg_limit(args))
                .await
        })
    }
}

// ─────────────────────── memory_lookup ───────────────────────

pub struct MemoryLookup(pub Arc<KbContext>);

impl Tool for MemoryLookup {
    fn name(&self) -> &'static str {
        "memory_lookup"
    }
    fn description(&self) -> &'static str {
        "查选题记忆：编辑部的准则卡与案例。准则卡里标了哪些是 Van 本人确认过的。只读。"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "关键词；不给就列最近的"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT}
            },
            "required": []
        })
    }
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
        Box::pin(async move {
            let q = arg_str(args, "query");
            let limit = arg_limit(args).clamp(1, MAX_LIMIT) as i64;
            let conn = self.0.conn.lock().await;
            let like = format!("%{q}%");

            let mut out = String::from("【准则卡】\n");
            let mut st = conn.prepare(
                "SELECT text, version, confirmed_by_van FROM memory_rules
                 WHERE ?1 = '' OR text LIKE ?2
                 ORDER BY confirmed_by_van DESC, updated_at DESC LIMIT ?3",
            )?;
            let mut n = 0;
            for r in st
                .query_map(rusqlite::params![q, like, limit], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)? == 1,
                    ))
                })?
                .flatten()
            {
                n += 1;
                // Van 本人确认过的与系统自己沉淀的，分量不一样，要标出来
                out.push_str(&format!(
                    "- {}{}{}\n",
                    r.0,
                    if r.1.is_empty() {
                        String::new()
                    } else {
                        format!("（{}）", r.1)
                    },
                    if r.2 { "【Van 确认】" } else { "" }
                ));
            }
            if n == 0 {
                out.push_str("无\n");
            }

            out.push_str("\n【案例】\n");
            let mut st = conn.prepare(
                "SELECT case_key, decision, quote FROM memory_cases
                 WHERE ?1 = '' OR case_key LIKE ?2 OR quote LIKE ?2
                 ORDER BY decided_at IS NULL, decided_at DESC, id DESC LIMIT ?3",
            )?;
            let mut m = 0;
            for r in st
                .query_map(rusqlite::params![q, like, limit], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .flatten()
            {
                m += 1;
                out.push_str(&format!("- {}：{}", r.0, r.1));
                if !r.2.trim().is_empty() {
                    out.push_str(&format!("　原话：{}", r.2.replace('\n', " / ")));
                }
                out.push('\n');
            }
            if m == 0 {
                out.push_str("无\n");
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_kb::docs::{self, KbDoc};
    use csw_collector_kb::fts;

    async fn ctx(dir: &std::path::Path) -> Arc<KbContext> {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let brands = BrandIndex::new(
            [("山と道", "山と道"), ("and wander", "andwander")]
                .iter()
                .filter_map(|(b, a)| csw_collector_kb::brands::Alias::new(b, a))
                .collect(),
        );
        let tok = Tokenizer::with_brands(&brands.dict_words());
        for (kind, ref_id, title, body, brand) in [
            (
                KbKind::PublishedItem,
                "p1",
                "羽绒进城",
                "NANGA｜羽绒服",
                "山と道",
            ),
            (KbKind::Example, "e1", "范例一则", "这是范例正文", "山と道"),
            (
                KbKind::GeneratedPost,
                "g1",
                "",
                "写过文章的贴文正文",
                "山と道",
            ),
            (
                KbKind::Decision,
                "48#k1",
                "这条不做",
                "结论：rejected\n原话：角度太旧",
                "山と道",
            ),
        ] {
            let d = KbDoc {
                kind: kind.as_str().into(),
                ref_id: ref_id.into(),
                title: title.into(),
                body: body.into(),
                brand: brand.into(),
                url: format!("https://example.com/{ref_id}"),
                published_at: Some("2026-09-18T00:00:00Z".into()),
                ..Default::default()
            };
            let r = docs::upsert(&conn, &d).unwrap();
            fts::index_doc(&conn, r.id, &tok, &d.text()).unwrap();
        }
        conn.execute(
            "INSERT INTO memory_rules(rule_key, text, version, confirmed_by_van, updated_at)
             VALUES ('r1','只有新配色的不做','v1',1,'2026-09-18T00:00:00Z'),
                    ('r2','补证不足先待核','v1',0,'2026-09-17T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_cases(case_key, decision, quote, decided_at, updated_at)
             VALUES ('某品牌新色','rejected','就是个配色','2026-09-18T00:00:00Z','2026-09-18T00:00:00Z')",
            [],
        )
        .unwrap();

        Arc::new(KbContext {
            conn: Mutex::new(conn),
            store: VectorStore::open(dir, 8, "测试").await.unwrap(),
            brands,
            tok,
            // 不连向量服务：品牌与全文两路照样能检索
            vector: None,
        })
    }

    #[tokio::test]
    async fn 检索四类都出现哪怕没命中() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        let out = KbSearch(c)
            .call(&json!({"query": "山と道 的新包"}))
            .await
            .unwrap();
        for n in ["正式已发布的条目", "范例", "生成过文章的贴文", "03 决定"] {
            assert!(out.contains(n), "少了 {n}：\n{out}");
        }
        assert!(out.contains("认出的品牌：山と道"), "{out}");
        assert!(out.contains("命中路径："), "{out}");
    }

    #[tokio::test]
    async fn 只看采用过的那两类() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        let out = KbSimilarSelected(c)
            .call(&json!({"query": "山と道 的新包"}))
            .await
            .unwrap();
        assert!(out.contains("正式已发布的条目") && out.contains("范例"));
        // 生成过文章的只说明写过；03 决定可能是**否决**的记录。都不算「采用过」
        assert!(!out.contains("生成过文章的贴文"), "{out}");
        assert!(!out.contains("03 决定"), "{out}");
    }

    #[tokio::test]
    async fn 一条都没命中时说清楚是什么意思() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        let out = KbSearch(c)
            .call(&json!({"query": "完全不相干的外星话题"}))
            .await
            .unwrap();
        // 「没找到」不等于「历史上没发过」——这句话省不得
        assert!(out.contains("不等于"), "{out}");
    }

    #[tokio::test]
    async fn 空查询直接拒绝() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        assert!(KbSearch(c).call(&json!({"query": "  "})).await.is_err());
    }

    #[tokio::test]
    async fn 准则卡标出van确认过的() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        let out = MemoryLookup(c).call(&json!({})).await.unwrap();
        // Van 本人确认过的与系统自己沉淀的分量不一样
        assert!(out.contains("只有新配色的不做（v1）【Van 确认】"), "{out}");
        assert!(out.contains("补证不足先待核（v1）\n"), "{out}");
        assert!(!out.contains("补证不足先待核（v1）【Van 确认】"));
        assert!(out.contains("某品牌新色：rejected"), "{out}");
        assert!(out.contains("原话：就是个配色"), "{out}");
    }

    #[tokio::test]
    async fn 记忆能按关键词筛() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        let out = MemoryLookup(c)
            .call(&json!({"query": "配色"}))
            .await
            .unwrap();
        assert!(out.contains("只有新配色的不做"));
        assert!(!out.contains("补证不足"), "{out}");
    }

    #[tokio::test]
    async fn 工具的schema都写了必填项() {
        let dir = tempdir::TempDir::new("mcp").unwrap();
        let c = ctx(dir.path()).await;
        let s = KbSearch(c.clone()).input_schema();
        assert_eq!(s["required"][0], "query");
        assert!(s["properties"]["kinds"]["items"]["enum"].is_array());
        // memory_lookup 的 query 是可选的：不给就列最近的
        assert_eq!(
            MemoryLookup(c).input_schema()["required"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
