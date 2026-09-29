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

use csw_collector_core::types::{HitState, Material, MaterialKind, Timestamp};
use csw_collector_kb::docs::KbKind;
use csw_collector_kb::search::Retrieved;

/// 每一类最多给模型几条。给多了会把正文挤掉，也会拖慢判断。
pub const PER_KIND: usize = 3;
/// 已发与范例带多少字正文。**查重要看正文**，只给标题模型就只能猜。
pub const BODY_EXCERPT_CHARS: usize = 400;

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
        // 这一类里也有推到草稿箱的，逐条标了状态；标题只写「正式已发布」，模型会拿草稿去质疑整类
        MaterialKind::Published => "已发布与已推草稿箱的条目（逐条标了状态）",
        MaterialKind::Example => "范例",
        MaterialKind::GeneratedPost => "生成过文章的贴文",
        MaterialKind::Decision => "03 决定",
        MaterialKind::PriorLedger => "上一轮台账",
        MaterialKind::Memory => "选题记忆（Van 的采用 / 否决案例）",
    }
}

/// 发布状态的中文名。**生成稿不是已发**——查重时它只能帮着复用资料。
pub fn state_name(publish_state: &str) -> &'static str {
    match hit_state(publish_state) {
        HitState::Published => "正式发布",
        HitState::Draft => "已推草稿箱",
        HitState::Generated => "仅生成稿",
        HitState::Decision => "03 决定",
        HitState::Unknown => "",
    }
}

pub fn hit_state(publish_state: &str) -> HitState {
    match publish_state {
        "published" => HitState::Published,
        "draft" => HitState::Draft,
        "generated" => HitState::Generated,
        "decision" => HitState::Decision,
        _ => HitState::Unknown,
    }
}

/// 给每条材料编号（`M1`、`M2`…），按 [`grouped`] 的顺序。
///
/// 提示词里的编号与代码核对 `comparison.hits` / `memory_refs` 用的是**同一个函数**，
/// 两边对不上的事不会发生。
pub fn numbered(materials: &[Material]) -> Vec<(String, &Material)> {
    grouped(materials)
        .into_iter()
        .flat_map(|(_, ms)| ms)
        .enumerate()
        .map(|(i, m)| (format!("M{}", i + 1), m))
        .collect()
}

/// 一条候选的对照材料。前四类来自检索，第五类来自上一轮台账。
///
/// 每一类都会出现在结果里，**哪怕是空的**——判断那一步要看到「这一类找过了、没有」，
/// 而不是看到一个缺口然后自己猜。空的那一类在 [`grouped`] 里是一个空数组。
pub fn assemble(retrieved: &Retrieved, prior: &[PriorLedgerItem]) -> Vec<Material> {
    let mut out = Vec::new();
    for (kind, hits) in retrieved.by_kind() {
        for s in hits.into_iter().take(PER_KIND) {
            let has_body = matches!(
                MaterialKind::from(kind),
                MaterialKind::Published | MaterialKind::Example
            );
            // 生成稿与决定按类型定死：生成稿**永远不是**「已发」，哪怕库里带着别的状态
            let publish_state = if s.doc.kind == KbKind::Decision.as_str() {
                "decision".into()
            } else if s.doc.kind == KbKind::GeneratedPost.as_str() {
                "generated".into()
            } else {
                s.doc.publish_state.clone()
            };
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
                publish_state,
                body_excerpt: if has_body {
                    s.doc.body.trim().chars().take(BODY_EXCERPT_CHARS).collect()
                } else {
                    String::new()
                },
                // 只有已发与范例的正文是查重要用的；取不到就如实说「正文不可得」
                body_available: !has_body || !s.doc.body.trim().is_empty(),
                brand: s.doc.brand.clone(),
            });
        }
    }
    for p in prior.iter().take(PER_KIND) {
        out.push(Material {
            kind: MaterialKind::PriorLedger,
            ref_id: p.candidate_key.clone(),
            title: p.title.clone(),
            date: parse_ts(&p.decided_at),
            // 写明它是什么：模型曾把它当「已被报道」（09-28 r56 v8 退回）
            source: "上一轮台账：工作台自己之前的判断，不是发布、不是否决，不能作为重复报道的依据"
                .into(),
            quote: p.note.clone(),
            publish_state: p.tier.clone(),
            body_excerpt: String::new(),
            body_available: true,
            brand: String::new(),
        });
    }
    out
}

/// 按五类分组，**每一类都在**，没有就是空数组；第六类「选题记忆」排最后。
pub fn grouped(materials: &[Material]) -> Vec<(MaterialKind, Vec<&Material>)> {
    MaterialKind::REQUIRED
        .into_iter()
        .chain([MaterialKind::Memory])
        .map(|k| (k, materials.iter().filter(|m| m.kind == k).collect()))
        .collect()
}

/// 拼成给模型看的一段。每条带类型、日期、出处——口径里点名要这三样。
/// 把决定类材料里 Van 的话去掉，**只留「结论」与「理由码」**。
///
/// 为什么不直接清空：决定类的正文里还有「结论：rejected」这种判断用得上的结构化信息，
/// 清空了模型就不知道「这件事被否过」。为什么连「理由」也去掉：真实数据里
/// `reason` 与 `decision_source` 存的是同一段话——就是她的原话或主编的批量说明。
///
/// 开关见 `Features::send_van_quotes_to_model`，默认关。
pub fn strip_van_quotes(materials: &mut [Material]) {
    for m in materials.iter_mut() {
        if m.kind != MaterialKind::Decision {
            continue;
        }
        m.quote = m
            .quote
            .lines()
            .filter(|l| l.starts_with("结论：") || l.starts_with("理由码："))
            // 每种只留**第一行**：决定正文由「结论 / 品牌 / 理由 / 理由码 / 原话」按行拼成，
            // 原话或理由若自己带换行、又恰好有一行以「结论：」开头，按行过滤会把那一行放出去。
            // 结构化的那两行永远排在原话前面，取第一行就不会取到她的话（09-24 安全审查）
            .fold(Vec::<&str>::new(), |mut acc, l| {
                let head = if l.starts_with("结论：") {
                    "结论："
                } else {
                    "理由码："
                };
                if !acc.iter().any(|x| x.starts_with(head)) {
                    acc.push(l);
                }
                acc
            })
            .join("\n");
    }
}

pub fn as_prompt_block(materials: &[Material]) -> String {
    let mut s = String::new();
    // 编号顺序与 [`numbered`] 相同：都是按 `grouped` 的顺序逐条数
    let mut n = 0;
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
            n += 1;
            s.push_str(&format!("- [M{n}] {}（{date}", m.title));
            if !m.source.is_empty() {
                s.push_str(&format!("，{}", m.source));
            }
            let state = match m.kind {
                // 上一轮台账与选题记忆的 publish_state 放的是档位 / 结论，原样给
                MaterialKind::PriorLedger | MaterialKind::Memory => m.publish_state.as_str(),
                _ => state_name(&m.publish_state),
            };
            if !state.is_empty() {
                s.push_str(&format!("，{state}"));
            }
            s.push_str(")\n");
            if matches!(m.kind, MaterialKind::Published | MaterialKind::Example) {
                if m.body_available && !m.body_excerpt.trim().is_empty() {
                    s.push_str(&format!("  正文：{}\n", one_line(&m.body_excerpt)));
                } else {
                    s.push_str("  正文不可得（查重只能是「未确认」）\n");
                }
            }
            if !m.quote.trim().is_empty() {
                // 决定类放的是一整段决定记录（结论、理由码，开关打开时才有原话），
                // 叫「原话」会让模型把「结论：rejected」也当成她说的话
                let label = if m.kind == MaterialKind::Decision {
                    "决定记录"
                } else {
                    "原话"
                };
                s.push_str(&format!("  {label}：{}\n", one_line(&m.quote)));
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
        assert_eq!(block.matches("查过，无相关").count(), 6);
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
        assert!(block.contains("正式发布"), "{block}");
        assert!(block.contains("[M1]"), "{block}");
        assert!(block.contains("正文："), "查重要看正文：{block}");
    }

    #[test]
    fn 已发正文缺失要明说不可得() {
        let ms = assemble(
            &retrieved(vec![
                scored(KbKind::PublishedItem, "k1", "Dapple Born 桌板", "  "),
                scored(KbKind::GeneratedPost, "g1", "生成稿", "x"),
            ]),
            &[],
        );
        let p = &ms[0];
        assert!(!p.body_available);
        let block = as_prompt_block(&ms);
        assert!(block.contains("正文不可得"), "{block}");
        // 生成稿没有发布状态时补成 generated，提示词里写「仅生成稿」
        assert!(block.contains("仅生成稿"), "{block}");
    }

    #[test]
    fn 编号与提示词里的一致() {
        let ms = assemble(
            &retrieved(vec![
                scored(KbKind::Decision, "48#k1", "d", "结论：rejected"),
                scored(KbKind::PublishedItem, "k1", "p", "正文"),
            ]),
            &[],
        );
        let nos = numbered(&ms);
        // 按五类的顺序编：已发在决定前面
        assert_eq!(nos[0].1.kind, MaterialKind::Published);
        assert_eq!(nos[0].0, "M1");
        assert_eq!(nos[1].0, "M2");
        let block = as_prompt_block(&ms);
        assert!(
            block.find("[M1] p").unwrap() < block.find("[M2] d").unwrap(),
            "{block}"
        );
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

    fn decision(quote: &str) -> Material {
        Material {
            kind: MaterialKind::Decision,
            ref_id: "50#norda-1f9378".into(),
            title: "某品牌腰包发布预告".into(),
            date: None,
            source: String::new(),
            quote: quote.into(),
            publish_state: String::new(),
            body_excerpt: String::new(),
            body_available: true,
            brand: String::new(),
        }
    }

    /// 开关关着时，决定类只留「结论」与「理由码」。
    ///
    /// 这两行是机器词表，模型要靠它知道「这件事被否过」；「理由」与「原话」在真数据里
    /// 都是她的话或主编的批量说明——Van 原话送不送第三方要单独拍板，没拍板前不送。
    #[test]
    fn 不送原话时只留结论与理由码() {
        let body = "结论：rejected\n品牌：norda\n理由：主选2备选1均批准继续推进，待核退回\n理由码：reject\n原话：这个不要";
        let mut ms = vec![
            decision(body),
            Material {
                kind: MaterialKind::Published,
                ref_id: "p1".into(),
                title: "已发".into(),
                date: None,
                source: String::new(),
                quote: "别的类不该被动".into(),
                publish_state: String::new(),
                body_excerpt: String::new(),
                body_available: true,
                brand: String::new(),
            },
        ];
        strip_van_quotes(&mut ms);
        assert_eq!(ms[0].quote, "结论：rejected\n理由码：reject");
        assert_eq!(ms[1].quote, "别的类不该被动");

        let block = as_prompt_block(&ms);
        assert!(!block.contains("这个不要"), "{block}");
        assert!(!block.contains("主选2备选1"), "{block}");
        // 去掉原话之后还叫「原话」就是错的——模型会把「结论：rejected」当成她说的
        assert!(block.contains("决定记录：结论：rejected"), "{block}");
    }
}
