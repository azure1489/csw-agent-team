//! 硬性排除的判定：这条候选是不是 Van 否过的那件事、那个角度。
//!
//! 数据层与口径在 [`csw_collector_core::exclusion`]，这里只管判。
//!
//! # 两道，和合并同一个形状但严得多
//!
//! 粗筛用**品牌精确命中**，不用向量。原因是被否决条目留在台账上的是标题与理由，
//! 不是原贴正文——拿那段元信息的向量去跟贴文正文比余弦，两边根本不在一个语义
//! 空间里。品牌是这两边都可靠的东西。
//!
//! 判据问 Jev 两件事：
//!
//! | 问 | 为什么 |
//! |---|---|
//! | 同一事实 | 同一次发布、同一款产品、同一场活动 |
//! | 有没有新料 | 同一件事被否过，但后来放出了发售日与价格，那是该重新看的 |
//!
//! 排除 = 同一事实 **且** 没有新料。只看第一条会把「后来补上实质信息」的
//! 同一件事一起挡掉；只看第二条会把所有平淡贴文都挡掉，包括没人否过的品牌。
//!
//! # Van 的原话不出网
//!
//! 这两问都**不送 Van 的原话，也不送录入的理由**——`sync_decisions` 顶上那条
//! 硬约束（原话只进本地库、只送本机 GPU、不送第三方）在这里同样管用，而 Jev
//! 是第三方。送出去的只有被否条目的标题与品牌，那是贴文自己的公开信息。
//!
//! 换来的判据反而更实：「有没有新料」看两条内容就能答，
//! 比「Van 这次会不会还是否」可验证得多。原话留在本地——
//! 它是规则生效的前提（没原话不自动生效），也是台账上给人看的依据。

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;

use csw_collector_core::exclusion::{Exclusion, THRESHOLD};
use csw_collector_core::jev::{JevClient, Question};
use csw_collector_core::types::{
    Candidate, Comparison, ComparisonHit, ComparisonVerdict, Dim, DimJudgement, HitState,
    Judgement, Novelty, Readiness, ThreeSentences, Tier, Unanswered, Verdict,
};

/// 「确实没有新料」的线。**不是 `1 - THRESHOLD`**——它和同一事实那条线各管一头，
/// 两条线之间留出的空白就是「拿不准就别排」。
pub const NO_SUBSTANCE: f64 = 0.30;

/// 一条候选最多跟几条规则去问。同品牌规则攒多了会把调用量拖起来，
/// 按 `decided_at` 新的优先——近期的口味比一年前的准。
pub const MAX_RULES_PER_CANDIDATE: usize = 5;

const FACT_FRAME: &str = "A past editorial decision REJECTED an item. Does the new post report the \
SAME underlying fact as that rejected item — the same product release, the same collaboration, the \
same event? A different product from the same brand is NOT the same fact. A later restock or price \
change of the very same product IS the same fact.";

const SUBSTANCE_FRAME: &str = "Compared with the rejected item, does the new post add substantive \
information that the rejected item did not carry — a firm release date, a price, a named \
collaboration, a specification or design change, hands-on detail, or an official confirmation of \
something previously rumoured? Reposting the same visuals or restating the same announcement is \
NOT new substance.";

/// 判出来的一次命中。
#[derive(Debug, Clone)]
pub struct Match {
    pub exclusion_id: i64,
    pub same_fact: f64,
    /// 新料的概率。**越高越不该排**。
    pub new_substance: f64,
}

impl Match {
    /// 同一件事，而且确实没带来新东西。
    ///
    /// **两头都严，中间一律不排。** 同一事实要 ≥ [`THRESHOLD`]，新料要
    /// < [`NO_SUBSTANCE`]；两条线之间那一大片灰色地带（拿不准是不是同一件事、
    /// 拿不准有没有新料）统统交回模型正常判一遍。多花一次调用，
    /// 换掉一次「本该推荐的没进判断」。
    pub fn holds(&self) -> bool {
        self.same_fact >= THRESHOLD && self.new_substance < NO_SUBSTANCE
    }
}

/// 粗筛：这条候选要跟哪几条规则去问。
///
/// `hits` 是候选正文（含译文与图片描述）上命中的品牌集合。
///
/// **品牌为空的规则一律不参与**——没有品牌就没有可靠的粗筛依据，
/// 而把它放给 Jev 去跟每一条候选比，既贵又正是错排风险最高的那一档。
pub fn candidates_for<'a>(
    hits: &std::collections::HashMap<String, String>,
    rules: &'a [Exclusion],
) -> Vec<&'a Exclusion> {
    if hits.is_empty() {
        return Vec::new();
    }
    let norm: Vec<String> = hits.keys().map(|b| b.trim().to_lowercase()).collect();
    let mut out: Vec<&Exclusion> = rules
        .iter()
        .filter(|e| {
            let b = e.brand.trim().to_lowercase();
            !b.is_empty() && norm.contains(&b)
        })
        .collect();
    // 近期的决定排前面：口味会变，一年前否过的未必现在还否
    out.sort_by(|a, b| b.decided_at.cmp(&a.decided_at).then(b.id.cmp(&a.id)));
    out.truncate(MAX_RULES_PER_CANDIDATE);
    out
}

/// 问 Jev：这条候选是不是那次否决的同一事实、同一角度。
///
/// 两个问题一次问完——它们互相独立，分两次既慢又多花一次底座。
pub async fn ask(
    jev: &JevClient,
    cand: &Candidate,
    descriptions: &str,
    rule: &Exclusion,
) -> Result<Match> {
    let state = json!({
        "new_post": {
            "account": cand.account,
            "caption": cand.text,
            "translated": cand.translated,
            "what_the_images_show": descriptions,
        },
        // **只有条目自己的公开信息。** Van 的原话与录入的理由都留在本地库，
        // 见模块文档「Van 的原话不出网」。
        "rejected_item": {
            "title": rule.title,
            "brand": rule.brand,
            "published_at": rule.decided_at,
        },
    });
    let q = BTreeMap::from([
        (
            "same_fact".to_string(),
            Question::noul(FACT_FRAME, "Same underlying fact.", "A different fact."),
        ),
        (
            "new_substance".to_string(),
            Question::noul(
                SUBSTANCE_FRAME,
                "Adds substantive new information.",
                "Nothing substantive beyond the rejected item.",
            ),
        ),
    ]);
    let a = jev.ask(&state, &q).await?;
    Ok(Match {
        exclusion_id: rule.id,
        // 同一事实取不到当 0、新料取不到当 1：**两边都倒向「不排除」**。
        // Jev 挂了的时候整轮照常判，只是省不下那几次调用——正是想要的失败方向。
        same_fact: a.noul("same_fact").unwrap_or(0.0),
        new_substance: a.noul("new_substance").unwrap_or(1.0),
    })
}

/// 台账上写给人看的那句依据。
pub fn explain(rule: &Exclusion, m: &Match) -> String {
    let quote = rule.quote.trim();
    let head = format!(
        "与 {} 否过的《{}》是同一件事，且没有新料（同一事实 {:.2}、新料 {:.2}）",
        if rule.actor_role.trim().is_empty() {
            "编辑部"
        } else {
            rule.actor_role.trim()
        },
        rule.title,
        m.same_fact,
        m.new_substance
    );
    if quote.is_empty() {
        head
    } else {
        // 原话是从本地库读出来直接显示的，没有经过任何外部服务
        format!("{head}。原话：{quote}")
    }
}

/// 被排除的条目那一行台账。**它不是判断**——六维全是「不明」，
/// 依据如实写着「没送判断」和为什么。
///
/// 照 [`verdict::pending_for_missing_image`](crate::verdict::pending_for_missing_image)
/// 的做法：代码生成的结论就老实说自己是代码生成的，不编六维依据。台账与交付物上
/// 要一眼能看出「模型判的不推荐」与「规则排掉的」是两回事。
pub fn excluded_judgement(c: &Candidate, rule: &Exclusion, m: &Match) -> Judgement {
    let why = explain(rule, m);
    let basis = format!("未送判断：{why}");
    Judgement {
        candidate_key: c.candidate_key.clone(),
        tier: Tier::NotRecommend,
        dims: Dim::ALL
            .into_iter()
            .map(|d| {
                (
                    d,
                    DimJudgement {
                        verdict: Verdict::Unclear,
                        basis: basis.clone(),
                    },
                )
            })
            .collect(),
        headline: format!("{}｜同一事实已被 Van 否决", rule.title),
        three_sentences: ThreeSentences::default(),
        novelty: Novelty::default(),
        readiness: Readiness::default(),
        unanswered: Unanswered::None,
        // 排除本来就是一次对照的结论，写在它该在的地方
        comparison: Comparison {
            verdict: ComparisonVerdict::SameFactNoGain,
            against: rule.title.clone(),
            hits: vec![ComparisonHit {
                ref_no: rule.decision_ref.clone(),
                title: rule.title.clone(),
                url: rule.source_url.clone(),
                state: HitState::Decision,
                published_at: rule.decided_at.clone(),
                body_available: true,
                dup_fact: why.clone(),
            }],
            note: why,
        },
        heat_note: String::new(),
        look: String::new(),
        // 走到这儿的条目都读到过实图（`apply_exclusions` 的第 2 道闸），
        // 否则 `violations` 会要求它落 pending_check
        image_seen: true,
        gaps: vec![],
        priority_hits: vec![],
        lower_hits: vec![],
        jev_disagreement: String::new(),
        kb_refs: vec![],
        memory_refs: vec![],
        inputs_hash: fingerprint(c, rule),
    }
}

/// 排除行的指纹。**前缀和判断的指纹隔开**，这一条是要害：
///
/// 判断复用按 `inputs_hash` 查。要是排除行的指纹跟判断算得一样，那么规则一旦停用、
/// 这条候选本该重新判的时候，缓存会拿着同一个指纹把这条排除结论翻出来接着用——
/// 「停用了规则还在按规则排除」就这么悄悄发生了。加前缀让两边永不相撞。
fn fingerprint(c: &Candidate, rule: &Exclusion) -> String {
    let h = blake3::hash(format!("{}\u{1}{}", c.candidate_key, rule.decision_ref).as_bytes());
    format!("excluded:{}", h.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::Platform;
    use std::collections::HashMap;

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: "ab12cd".into(),
            collector: "csw-window".into(),
            account: "snowpeak".into(),
            url: "https://instagram.com/p/ab12cd".into(),
            text: "新しいテントを発表".into(),
            translated: "发布了新帐篷".into(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn rule(id: i64, brand: &str, decided: &str) -> Exclusion {
        Exclusion {
            id,
            decision_ref: format!("{id}#k"),
            item_key: "k".into(),
            title: format!("第 {id} 条"),
            brand: brand.into(),
            source_url: String::new(),
            quote: "这种普通上新没什么看点".into(),
            reason: "没有看点".into(),
            reason_code: String::new(),
            decided_at: decided.into(),
            actor_role: "Van".into(),
            active: true,
            inactive_reason: String::new(),
            changed_by: String::new(),
            changed_at: String::new(),
        }
    }

    fn hits(brands: &[&str]) -> HashMap<String, String> {
        brands
            .iter()
            .map(|b| (b.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn 只跟同品牌的规则去问() {
        let rules = [
            rule(1, "Snow Peak", "2026-09-01"),
            rule(2, "Nanga", "2026-09-02"),
            rule(3, "", "2026-09-03"),
        ];
        let got = candidates_for(&hits(&["snow peak"]), &rules);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, 1, "大小写不该影响粗筛");

        // 品牌为空的规则永远不参与：没有可靠粗筛依据，错排风险最高的就是这一档
        assert!(candidates_for(&hits(&["別的"]), &rules).is_empty());
        // 一个品牌都没抽出来的候选不会被排除——宁可漏排
        assert!(candidates_for(&HashMap::new(), &rules).is_empty());
    }

    #[test]
    fn 同品牌规则太多时留最近的() {
        let rules: Vec<Exclusion> = (1..=8)
            .map(|i| rule(i, "Snow Peak", &format!("2026-09-{i:02}")))
            .collect();
        let got = candidates_for(&hits(&["Snow Peak"]), &rules);
        assert_eq!(got.len(), MAX_RULES_PER_CANDIDATE);
        // 口味会变，近期的决定优先
        assert_eq!(got[0].id, 8);
        assert_eq!(got[4].id, 4);
    }

    #[test]
    fn 两头都严中间一律不排() {
        let m = |f: f64, n: f64| Match {
            exclusion_id: 1,
            same_fact: f,
            new_substance: n,
        };
        assert!(m(0.95, 0.05).holds());
        assert!(m(THRESHOLD, 0.0).holds());
        // 同一件事，但这次补上了发售日与价格——该重新看，不能排
        assert!(!m(0.99, 0.80).holds());
        // 同样平淡但不是同一件事：不能拿一次否决挡住整个品牌
        assert!(!m(0.40, 0.01).holds());
        // 灰色地带：两个判据都在中间，一律交回模型判
        assert!(!m(0.70, 0.50).holds());
        assert!(!m(0.90, 0.50).holds(), "拿不准有没有新料就别排");
    }

    #[test]
    fn 送出去的东西里没有原话也没有理由() {
        // 这条钉的是模块文档那条硬约束：Van 的话不出网。
        // ask() 组 state 的那段代码改动时，这里要跟着核一遍。
        let src = include_str!("exclude.rs");
        let body = &src[src.find("pub async fn ask(").unwrap()..];
        let state = &body[..body.find("let q =").unwrap()];
        assert!(!state.contains("rule.quote"), "原话不能进 state");
        assert!(!state.contains("rule.reason"), "录入的理由也不能进 state");
        assert!(state.contains("rule.title") && state.contains("rule.brand"));
    }

    #[test]
    fn 排除产出的台账行要过契约自检() {
        let c = cand("snowpeak-abc123");
        let r = rule(1, "Snow Peak", "2026-09-01");
        let m = Match {
            exclusion_id: 1,
            same_fact: 0.95,
            new_substance: 0.03,
        };
        let j = excluded_judgement(&c, &r, &m);
        // 六维齐全、每维有依据——依据是「没送判断」，如实
        assert!(j.violations().is_empty(), "{:?}", j.violations());
        assert!(j.dims.iter().all(|(_, d)| d.verdict == Verdict::Unclear));
        assert!(
            j.dims
                .iter()
                .all(|(_, d)| d.basis.starts_with("未送判断："))
        );
        // 靠对照结论与模型判的不推荐分开
        assert_eq!(j.comparison.verdict, ComparisonVerdict::SameFactNoGain);
        assert_eq!(j.comparison.against, r.title);
        // 没被判过，三句话编不出来
        assert!(j.three_sentences.what.is_empty());
        assert_eq!(j.comparison.hits[0].state, HitState::Decision);
    }

    #[test]
    fn 排除的指纹绝不会被判断复用捡走() {
        let c = cand("k1");
        let a = excluded_judgement(
            &c,
            &rule(1, "b", "2026-09-01"),
            &Match {
                exclusion_id: 1,
                same_fact: 0.9,
                new_substance: 0.0,
            },
        );
        // 前缀是这条的要害：判断的指纹是纯 hex，永远撞不上带前缀的
        assert!(a.inputs_hash.starts_with("excluded:"), "{}", a.inputs_hash);
        // 换一条规则就换一个指纹——规则换了，旧结论不该还算数
        let b = excluded_judgement(
            &c,
            &rule(2, "b", "2026-09-02"),
            &Match {
                exclusion_id: 2,
                same_fact: 0.9,
                new_substance: 0.0,
            },
        );
        assert_ne!(a.inputs_hash, b.inputs_hash);
    }

    #[test]
    fn 依据里带着原话() {
        let r = rule(1, "Snow Peak", "2026-09-01");
        let s = explain(
            &r,
            &Match {
                exclusion_id: 1,
                same_fact: 0.93,
                new_substance: 0.04,
            },
        );
        assert!(s.contains("Van"), "{s}");
        assert!(s.contains("这种普通上新没什么看点"), "{s}");
        assert!(s.contains("0.93"), "{s}");
        assert!(s.contains("没有新料"), "{s}");
    }
}
