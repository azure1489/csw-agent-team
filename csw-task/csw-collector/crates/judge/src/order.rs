//! 台账排序与热度说明。
//!
//! # 排序由代码算，不由模型给
//!
//! 顺序是：**先按档，再按成立的维度数，再按相对账号基线的热度，最后按发布时间。**
//! 全是能复算的量，所以同样的输入永远排出同样的顺序——这一点对对账是必需的。
//!
//! **不打数字分、不设权重、没有综合分。** 打分本来想解决的两件事在这里各有别的解法：
//! 「几百条要有个顺序」由四档加成立维度数解决；「判断要可核」由逐维依据解决，
//! 一句指向正文或某张图的依据比一个 0.73 好核得多。
//!
//! # 热度是输入的呈现，不是维度
//!
//! [`heat_note`] 把点赞、评论、相对该账号近 90 天中位点赞的倍数、标签与话题
//! 拼成一句话，原样交给模型与台账。它**不参与**六维判断，也不参与档位。
//! 它只在同档内部排序时起作用——「这一档里哪条更值得先看」。

use csw_collector_core::types::{Candidate, Judgement, Tier, Verdict};

/// 档位的先后。**只有这四档，没有分数。**
fn tier_rank(t: Tier) -> u8 {
    match t {
        Tier::Recommend => 0,
        Tier::Alternate => 1,
        // 待核排在不推荐前面：它是「还没判完」，不是「判过了不要」
        Tier::PendingCheck => 2,
        Tier::NotRecommend => 3,
    }
}

/// 成立的维度数。`unclear` 不算成立，也不算不成立。
pub fn yes_count(j: &Judgement) -> usize {
    j.dims
        .iter()
        .filter(|(_, d)| d.verdict == Verdict::Yes)
        .count()
}

/// 台账顺序的排序键。
///
/// 返回的元组直接 `sort_by_key` 就行——**每一项都是能复算的量**，
/// 同样的输入永远排出同样的顺序。
pub fn sort_key(
    j: &Judgement,
    c: Option<&Candidate>,
) -> (
    u8,
    std::cmp::Reverse<usize>,
    std::cmp::Reverse<i64>,
    std::cmp::Reverse<i64>,
    String,
) {
    let heat = c
        .and_then(|c| c.heat_ratio)
        // 浮点不能直接当排序键（NaN 没有全序）；乘 1000 取整，千分位足够分辨
        .map(|r| (r * 1000.0) as i64)
        .unwrap_or(0);
    let posted = c
        .and_then(|c| c.posted_at)
        .map(|t| t.as_second())
        .unwrap_or(0);
    (
        tier_rank(j.tier),
        std::cmp::Reverse(yes_count(j)),
        std::cmp::Reverse(heat),
        std::cmp::Reverse(posted),
        // 最后按键定死，保证完全确定
        j.candidate_key.clone(),
    )
}

/// 按台账顺序排。
pub fn sort_ledger(js: &mut [Judgement], by_key: impl Fn(&str) -> Option<Candidate>) {
    let cands: std::collections::HashMap<String, Option<Candidate>> = js
        .iter()
        .map(|j| (j.candidate_key.clone(), by_key(&j.candidate_key)))
        .collect();
    js.sort_by_key(|j| sort_key(j, cands.get(&j.candidate_key).and_then(Option::as_ref)));
}

/// 热度说明。**是输入的呈现，不是维度。**
///
/// 「相对该账号近 90 天中位点赞的倍数」才是有意义的量——绝对点赞数在
/// 九万粉的账号和九百粉的账号之间没有可比性，拿它排序就是按粉丝数排序。
pub fn heat_note(c: &Candidate, baseline_likes: Option<i64>) -> String {
    let mut parts = Vec::new();
    if let Some(l) = c.likes {
        let mut s = format!("{l} 赞");
        match (baseline_likes, c.heat_ratio) {
            (Some(b), Some(r)) if b > 0 => s.push_str(&format!(" · 常态 {b} 的 {r:.1}×")),
            (_, Some(r)) => s.push_str(&format!(" · 常态的 {r:.1}×")),
            _ => {}
        }
        parts.push(s);
    }
    if let Some(n) = c.comments {
        parts.push(format!("{n} 评论"));
    }
    if let Some(f) = c.followers {
        parts.push(format!("账号 {f} 粉丝"));
    }
    let mut s = parts.join(" · ");
    let labels: Vec<&str> = c
        .tags
        .iter()
        .chain(c.hashtags.iter())
        .map(String::as_str)
        .filter(|x| !x.trim().is_empty())
        .collect();
    if !labels.is_empty() {
        if !s.is_empty() {
            s.push('；');
        }
        s.push_str(&format!("标签：{}", labels.join("、")));
    }
    if s.is_empty() {
        // 取不到就说取不到，别拿 0 冒充
        s.push_str("热度不详");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{
        Comparison, ComparisonVerdict, Dim, DimJudgement, Platform, ThreeSentences, Unanswered,
    };

    fn j(key: &str, tier: Tier, yes: usize) -> Judgement {
        Judgement {
            candidate_key: key.into(),
            tier,
            dims: Dim::ALL
                .into_iter()
                .enumerate()
                .map(|(i, d)| {
                    (
                        d,
                        DimJudgement {
                            verdict: if i < yes {
                                Verdict::Yes
                            } else {
                                Verdict::Unclear
                            },
                            basis: "b".into(),
                        },
                    )
                })
                .collect(),
            three_sentences: ThreeSentences {
                what_changed: String::new(),
                why_it_matters: String::new(),
                how_different: String::new(),
            },
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
            },
            heat_note: String::new(),
            look: String::new(),
            image_seen: true,
            gaps: vec![],
            priority_hits: vec![],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "h".into(),
        }
    }

    fn cand(key: &str, likes: Option<i64>, ratio: Option<f64>) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "c".into(),
            account: "acc".into(),
            url: "u".into(),
            text: String::new(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes,
            comments: Some(12),
            followers: Some(90000),
            heat_ratio: ratio,
            content_type: "Image".into(),
            media: vec![],
            tags: vec!["新品".into()],
            hashtags: vec!["outdoor".into()],
        }
    }

    #[test]
    fn 先按档再按成立维度数() {
        let mut js = vec![
            j("c", Tier::Recommend, 3),
            j("a", Tier::NotRecommend, 6),
            j("b", Tier::Recommend, 5),
        ];
        sort_ledger(&mut js, |_| None);
        assert_eq!(
            js.iter()
                .map(|x| x.candidate_key.as_str())
                .collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
    }

    #[test]
    fn 待核排在不推荐前面() {
        // 待核是「还没判完」，不是「判过了不要」
        let mut js = vec![j("a", Tier::NotRecommend, 6), j("b", Tier::PendingCheck, 0)];
        sort_ledger(&mut js, |_| None);
        assert_eq!(js[0].candidate_key, "b");
    }

    #[test]
    fn 同档同维度数时按相对热度() {
        let mut js = vec![j("a", Tier::Recommend, 3), j("b", Tier::Recommend, 3)];
        sort_ledger(&mut js, |k| {
            Some(cand(k, Some(100), Some(if k == "b" { 5.2 } else { 1.1 })))
        });
        assert_eq!(js[0].candidate_key, "b");
    }

    #[test]
    fn 排序是完全确定的() {
        // 每一项都是能复算的量；最后按键定死
        let mut a = vec![j("x", Tier::Recommend, 3), j("y", Tier::Recommend, 3)];
        let mut b = vec![j("y", Tier::Recommend, 3), j("x", Tier::Recommend, 3)];
        sort_ledger(&mut a, |_| None);
        sort_ledger(&mut b, |_| None);
        assert_eq!(
            a.iter().map(|x| &x.candidate_key).collect::<Vec<_>>(),
            b.iter().map(|x| &x.candidate_key).collect::<Vec<_>>()
        );
        assert_eq!(a[0].candidate_key, "x");
    }

    #[test]
    fn 热度说明带倍数而不是只有绝对数() {
        // 绝对点赞数在九万粉和九百粉的账号之间没有可比性
        let s = heat_note(&cand("k", Some(369), Some(5.2)), Some(71));
        assert!(s.contains("369 赞"), "{s}");
        assert!(s.contains("常态 71 的 5.2×"), "{s}");
        assert!(s.contains("12 评论"));
        assert!(s.contains("标签：新品、outdoor"), "{s}");
    }

    #[test]
    fn 没有基线时只说倍数() {
        let s = heat_note(&cand("k", Some(369), Some(5.2)), None);
        assert!(s.contains("常态的 5.2×"), "{s}");
    }

    #[test]
    fn 取不到就说取不到不拿零冒充() {
        let mut c = cand("k", None, None);
        c.comments = None;
        c.followers = None;
        c.tags.clear();
        c.hashtags.clear();
        assert_eq!(heat_note(&c, None), "热度不详");
    }

    #[test]
    fn 浮点热度不当排序键() {
        // NaN 没有全序，直接拿 f64 排会 panic 或给出不确定顺序
        let mut c = cand("k", Some(1), Some(f64::NAN));
        c.heat_ratio = Some(f64::NAN);
        let key = sort_key(&j("k", Tier::Recommend, 1), Some(&c));
        // 只要不恐慌、能得出一个确定的键就行
        assert_eq!(key.0, 0);
    }
}
