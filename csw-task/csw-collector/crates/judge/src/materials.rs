//! 对照材料：把检索结果与上一轮台账装成判断那一步的输入。
//!
//! # 「五类缺一不判」是**源级**的闸，不是条目级的
//!
//! 口径原文管的是第 4 步（对照材料）：正式已发布、范例、生成过文章的贴文、
//! 03 决定、上一轮台账，五个**来源**都得先备齐，才谈得上判。
//!
//! 按条目级读会把系统变成不可用：某条候选在 03 决定里找不到相似案例是家常便饭，
//! 一律落待核的话，第一期几乎每条都会卡住——而「上一轮台账」在第一期**必然**是空的，
//! 那就等于一条都判不了。
//!
//! 所以这里分两层：
//! - **源级**（[`SourceAvailability`]）：五类来源缺一，整轮停下告警，不判。
//!   第一期的「上一轮台账」按「已取，为空」处理——取过了就是备齐了。
//! - **条目级**（[`assemble`]）：某一类对这条候选没有命中是正常的，
//!   如实写成「无相关」交给模型，**不因此落待核**。
//!
//! 这个读法与口径的字面不完全重合，是有意的取舍，已记进进度文档待确认。

use anyhow::Result;
use rusqlite::Connection;

use csw_collector_core::types::{Material, MaterialKind, Timestamp};
use csw_collector_kb::docs::KbKind;
use csw_collector_kb::search::Retrieved;

/// 每一类最多给模型几条。给多了会把正文挤掉，也会拖慢判断。
pub const PER_KIND: usize = 3;

/// 上一轮台账里的一行。它留在本地库、**不进参考库**——
/// 否则系统自己的判断会被当成 Van 的口味证据。
#[derive(Debug, Clone, Default)]
pub struct PriorLedgerItem {
    pub candidate_key: String,
    pub title: String,
    /// 上一轮落的档
    pub tier: String,
    pub decided_at: String,
    pub note: String,
}

/// 五类来源备没备齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceAvailability {
    pub published: bool,
    pub example: bool,
    pub generated_post: bool,
    pub decision: bool,
    /// **取过了就算备齐，哪怕是空的**——第一期必然为空，不能因此判不了
    pub prior_ledger: bool,
}

impl SourceAvailability {
    pub fn missing(&self) -> Vec<MaterialKind> {
        let mut m = Vec::new();
        for (ok, kind) in [
            (self.published, MaterialKind::Published),
            (self.example, MaterialKind::Example),
            (self.generated_post, MaterialKind::GeneratedPost),
            (self.decision, MaterialKind::Decision),
            (self.prior_ledger, MaterialKind::PriorLedger),
        ] {
            if !ok {
                m.push(kind);
            }
        }
        m
    }

    pub fn ready(&self) -> bool {
        self.missing().is_empty()
    }

    /// 缺哪些，写成一句能直接进告警的话。
    pub fn blocked_reason(&self) -> Option<String> {
        let m = self.missing();
        (!m.is_empty()).then(|| {
            format!(
                "对照材料缺 {} 类：{}。五类缺一不判，这一轮停在这里。",
                m.len(),
                m.iter()
                    .map(|k| kind_name(*k))
                    .collect::<Vec<_>>()
                    .join("、")
            )
        })
    }
}

/// 从本地库看四类参考库材料在不在。第五类由调用方说（它不在参考库里）。
pub fn availability(conn: &Connection, prior_ledger_loaded: bool) -> Result<SourceAvailability> {
    let has = |kind: KbKind| -> Result<bool> {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM kb_docs WHERE kind = ?1",
            [kind.as_str()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    };
    Ok(SourceAvailability {
        published: has(KbKind::PublishedItem)?,
        example: has(KbKind::Example)?,
        generated_post: has(KbKind::GeneratedPost)?,
        decision: has(KbKind::Decision)?,
        prior_ledger: prior_ledger_loaded,
    })
}

pub fn kind_name(k: MaterialKind) -> &'static str {
    match k {
        MaterialKind::Published => "正式已发布的条目",
        MaterialKind::Example => "范例",
        MaterialKind::GeneratedPost => "生成过文章的贴文",
        MaterialKind::Decision => "03 决定",
        MaterialKind::PriorLedger => "上一轮台账",
    }
}

/// 一条候选的对照材料。前四类来自检索，第五类来自上一轮台账。
///
/// 每一类都会出现在结果里，**哪怕是空的**——判断那一步要看到「这一类找过了、没有」，
/// 而不是看到一个缺口然后自己猜。空的那一类在 [`grouped`] 里是一个空数组。
pub fn assemble(retrieved: &Retrieved, prior: &[PriorLedgerItem]) -> Vec<Material> {
    let mut out = Vec::new();
    for (kind, hits) in retrieved.by_kind() {
        for s in hits.into_iter().take(PER_KIND) {
            out.push(Material {
                kind: MaterialKind::from(kind),
                ref_id: s.doc.ref_id.clone(),
                title: if s.doc.title.is_empty() {
                    first_line(&s.doc.body)
                } else {
                    s.doc.title.clone()
                },
                date: s.doc.published_at.as_deref().and_then(parse_ts),
                source: source_of(s.doc.url.as_str(), &s.doc.platform),
                // 决定类的正文里带着原话，其余类没有原话就是没有
                quote: if s.doc.kind == KbKind::Decision.as_str() {
                    s.doc.body.clone()
                } else {
                    String::new()
                },
                publish_state: s.doc.publish_state.clone(),
            });
        }
    }
    for p in prior.iter().take(PER_KIND) {
        out.push(Material {
            kind: MaterialKind::PriorLedger,
            ref_id: p.candidate_key.clone(),
            title: p.title.clone(),
            date: parse_ts(&p.decided_at),
            source: "上一轮台账".into(),
            quote: p.note.clone(),
            publish_state: p.tier.clone(),
        });
    }
    out
}

/// 按五类分组，**每一类都在**，没有就是空数组。
pub fn grouped(materials: &[Material]) -> Vec<(MaterialKind, Vec<&Material>)> {
    MaterialKind::REQUIRED
        .into_iter()
        .map(|k| (k, materials.iter().filter(|m| m.kind == k).collect()))
        .collect()
}

/// 拼成给模型看的一段。每条带类型、日期、出处——口径里点名要这三样。
pub fn as_prompt_block(materials: &[Material]) -> String {
    let mut s = String::new();
    for (kind, ms) in grouped(materials) {
        s.push_str(&format!("【{}】", kind_name(kind)));
        if ms.is_empty() {
            // 「找过了没有」与「没找」是两回事，要说清楚
            s.push_str("查过，无相关\n");
            continue;
        }
        s.push('\n');
        for m in ms {
            let date = m
                .date
                .map(|d| d.to_string())
                .unwrap_or_else(|| "日期不详".into());
            s.push_str(&format!("- {}（{date}", m.title));
            if !m.source.is_empty() {
                s.push_str(&format!("，{}", m.source));
            }
            if !m.publish_state.is_empty() {
                s.push_str(&format!("，{}", m.publish_state));
            }
            s.push_str(")\n");
            if !m.quote.trim().is_empty() {
                s.push_str(&format!("  原话：{}\n", one_line(&m.quote)));
            }
        }
    }
    s
}

fn parse_ts(s: &str) -> Option<Timestamp> {
    s.trim().parse().ok()
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(60).collect()
}

fn one_line(s: &str) -> String {
    s.replace('\n', " / ").chars().take(300).collect()
}

fn source_of(url: &str, platform: &str) -> String {
    if !url.is_empty() {
        url.to_string()
    } else {
        platform.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_kb::docs::KbDoc;
    use csw_collector_kb::search::{Routes, Scored};

    fn scored(kind: KbKind, ref_id: &str, title: &str, body: &str) -> Scored {
        Scored {
            doc: KbDoc {
                kind: kind.as_str().into(),
                ref_id: ref_id.into(),
                title: title.into(),
                body: body.into(),
                url: format!("https://example.com/{ref_id}"),
                published_at: Some("2026-09-18T00:00:00Z".into()),
                publish_state: "published".into(),
                ..Default::default()
            },
            routes: Routes::default(),
            score: None,
            backfilled: false,
        }
    }

    fn retrieved(docs: Vec<Scored>) -> Retrieved {
        Retrieved {
            docs,
            ..Default::default()
        }
    }

    #[test]
    fn 缺一类就停下并说清缺的是哪一类() {
        let mut a = SourceAvailability {
            published: true,
            example: true,
            generated_post: true,
            decision: true,
            prior_ledger: true,
        };
        assert!(a.ready() && a.blocked_reason().is_none());

        a.decision = false;
        assert!(!a.ready());
        let why = a.blocked_reason().unwrap();
        assert!(why.contains("03 决定"), "{why}");
        assert!(why.contains("五类缺一不判"), "{why}");
    }

    #[test]
    fn 第一期的上一轮台账是空的但不该卡住() {
        // 「取过了」就算备齐——第一期必然为空，按条目级读会一条都判不了
        let a = SourceAvailability {
            published: true,
            example: true,
            generated_post: true,
            decision: true,
            prior_ledger: true,
        };
        assert!(a.ready());
        let ms = assemble(&retrieved(vec![]), &[]);
        assert!(ms.is_empty());
        // 但块里五类都要出现，让模型看到「查过了、没有」
        let block = as_prompt_block(&ms);
        for k in MaterialKind::REQUIRED {
            assert!(block.contains(kind_name(k)), "少了 {}", kind_name(k));
        }
        assert_eq!(block.matches("查过，无相关").count(), 5);
    }

    #[test]
    fn 四类从检索来第五类从台账来() {
        let r = retrieved(vec![
            scored(KbKind::PublishedItem, "k1", "羽绒进城", "品牌｜产品"),
            scored(KbKind::Example, "p2", "范例一则", "范例正文"),
            scored(
                KbKind::GeneratedPost,
                "g1",
                "",
                "已生成贴文的正文第一行\n第二行",
            ),
            scored(
                KbKind::Decision,
                "48#k1",
                "这条不做",
                "结论：rejected\n原话：角度太旧",
            ),
        ]);
        let prior = [PriorLedgerItem {
            candidate_key: "nanga-ab12cd".into(),
            title: "上一轮判过的".into(),
            tier: "alternate".into(),
            decided_at: "2026-09-17T00:00:00Z".into(),
            note: "证据不足".into(),
        }];
        let ms = assemble(&r, &prior);
        assert_eq!(ms.len(), 5);
        for k in MaterialKind::REQUIRED {
            assert!(ms.iter().any(|m| m.kind == k), "少了 {}", kind_name(k));
        }
        // 没有标题的用正文第一行顶上，别给模型一个空标题
        let g = ms
            .iter()
            .find(|m| m.kind == MaterialKind::GeneratedPost)
            .unwrap();
        assert_eq!(g.title, "已生成贴文的正文第一行");
        // 决定类的原话要带上：它是这一类材料的核心
        let d = ms
            .iter()
            .find(|m| m.kind == MaterialKind::Decision)
            .unwrap();
        assert!(d.quote.contains("角度太旧"));
        // 其余类没有原话就是没有，不许拿正文顶替
        let p = ms
            .iter()
            .find(|m| m.kind == MaterialKind::Published)
            .unwrap();
        assert!(p.quote.is_empty());
    }

    #[test]
    fn 每类最多给几条() {
        let many: Vec<Scored> = (0..10)
            .map(|i| scored(KbKind::PublishedItem, &format!("k{i}"), "标题", "正文"))
            .collect();
        let ms = assemble(&retrieved(many), &[]);
        assert_eq!(ms.len(), PER_KIND, "给多了会把正文挤掉，也拖慢判断");
    }

    #[test]
    fn 块里带类型日期与出处() {
        let ms = assemble(
            &retrieved(vec![scored(KbKind::PublishedItem, "k1", "羽绒进城", "b")]),
            &[],
        );
        let block = as_prompt_block(&ms);
        assert!(block.contains("羽绒进城"));
        assert!(block.contains("2026-09-18"), "{block}");
        assert!(block.contains("https://example.com/k1"), "{block}");
        assert!(block.contains("published"), "{block}");
    }

    #[test]
    fn 原话里的换行要压平() {
        // 多行原话会把「一条材料一行」的结构冲掉，模型容易把下一行当成新材料
        let ms = assemble(
            &retrieved(vec![scored(
                KbKind::Decision,
                "48#k",
                "决定",
                "第一行\n第二行",
            )]),
            &[],
        );
        let block = as_prompt_block(&ms);
        assert!(block.contains("第一行 / 第二行"), "{block}");
    }

    #[test]
    fn 参考库空的时候四类都算缺() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let a = availability(&c, false).unwrap();
        assert_eq!(a.missing().len(), 5);
        let a = availability(&c, true).unwrap();
        assert_eq!(a.missing().len(), 4, "台账取过了就不算缺");
    }
}
