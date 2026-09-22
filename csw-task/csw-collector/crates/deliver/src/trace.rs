//! `trace/` 里的溯源文件：`sweeps.jsonl`、`items.jsonl`。
//!
//! 这两份是给**核对**用的，不是给人读的。台账正文说「这 358 条都判过了」，
//! 而这两份文件让这句话可以被逐行验证：采集轮各自取到多少、每条候选的结论是什么、
//! 哪些没读到实图。
//!
//! **键序固定**：用结构体而不是 `serde_json::Value` 拼，字段顺序就是声明顺序。
//! 键序一变，确定性打包就白做了。

use anyhow::Result;
use serde::Serialize;

use csw_collector_core::types::{Candidate, Judgement, Tier};

/// `items.jsonl` 的一行。字段顺序即声明顺序。
#[derive(Debug, Serialize)]
pub struct ItemLine<'a> {
    pub candidate_key: &'a str,
    pub account: &'a str,
    pub url: &'a str,
    /// 原始披露时间
    pub posted_at: Option<String>,
    /// 首次入库时间（窗口按它收口）
    pub ingested_at: Option<String>,
    pub tier: Tier,
    /// 成立的维度数。排序用的量，写出来便于复算。
    pub yes_count: usize,
    pub image_seen: bool,
    pub photos: usize,
    pub gaps: &'a [String],
    /// 结论能不能复用的指纹
    pub inputs_hash: &'a str,
}

/// `sweeps.jsonl` 的一行。字段名与引擎的 `sweeps` 对齐。
#[derive(Debug, Serialize, Clone)]
pub struct SweepLine {
    pub sweep_key: String,
    pub query: String,
    pub found: usize,
    pub fetched_unique: usize,
    pub in_window: usize,
    pub result: String,
    pub error: String,
}

/// 一行一个 JSON，末尾带换行。
pub fn to_jsonl<T: Serialize>(rows: &[T]) -> Result<String> {
    let mut s = String::new();
    for r in rows {
        s.push_str(&serde_json::to_string(r)?);
        s.push('\n');
    }
    Ok(s)
}

/// 把判断与候选拼成 `items.jsonl`。
///
/// **顺序就是台账顺序**，不另外排——两份对不上会让核对的人无所适从。
pub fn items_jsonl(
    judgements: &[Judgement],
    by_key: impl Fn(&str) -> Option<Candidate>,
) -> Result<String> {
    let cands: Vec<Option<Candidate>> = judgements
        .iter()
        .map(|j| by_key(&j.candidate_key))
        .collect();
    let rows: Vec<ItemLine<'_>> = judgements
        .iter()
        .zip(&cands)
        .map(|(j, c)| ItemLine {
            candidate_key: &j.candidate_key,
            account: c.as_ref().map(|c| c.account.as_str()).unwrap_or(""),
            url: c.as_ref().map(|c| c.url.as_str()).unwrap_or(""),
            posted_at: c.as_ref().and_then(|c| c.posted_at).map(|t| t.to_string()),
            ingested_at: c
                .as_ref()
                .and_then(|c| c.ingested_at)
                .map(|t| t.to_string()),
            tier: j.tier,
            yes_count: j
                .dims
                .iter()
                .filter(|(_, d)| d.verdict == csw_collector_core::types::Verdict::Yes)
                .count(),
            image_seen: j.image_seen,
            photos: c.as_ref().map(|c| c.media.len()).unwrap_or(0),
            gaps: &j.gaps,
            inputs_hash: &j.inputs_hash,
        })
        .collect();
    to_jsonl(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{
        Comparison, ComparisonVerdict, Dim, DimJudgement, Platform, ThreeSentences, Unanswered,
        Verdict,
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
                            verdict: if i < yes { Verdict::Yes } else { Verdict::No },
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
            image_seen: tier != Tier::PendingCheck,
            gaps: vec![],
            priority_hits: vec![],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "abc".into(),
        }
    }

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw-window".into(),
            account: "yamatomichi".into(),
            url: format!("https://x/{key}"),
            text: String::new(),
            translated: String::new(),
            posted_at: "2026-09-17T08:00:00Z".parse().ok(),
            ingested_at: "2026-09-18T01:00:00Z".parse().ok(),
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

    #[test]
    fn 一行一个json末尾带换行() {
        let out = items_jsonl(
            &[j("k1", Tier::Recommend, 4), j("k2", Tier::NotRecommend, 1)],
            |k| Some(cand(k)),
        )
        .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(out.ends_with('\n'));
        for l in lines {
            serde_json::from_str::<serde_json::Value>(l).expect("每行都要是合法 JSON");
        }
    }

    #[test]
    fn 键序固定() {
        // 键序一变，确定性打包就白做了
        let out = items_jsonl(&[j("k1", Tier::Recommend, 4)], |k| Some(cand(k))).unwrap();
        let keys: Vec<&str> = out
            .lines()
            .next()
            .unwrap()
            .split("\":")
            .filter_map(|x| x.rsplit('"').next())
            .collect();
        assert!(
            out.starts_with(r#"{"candidate_key":"k1","account":"yamatomichi","url":"#),
            "{out}"
        );
        assert!(keys.contains(&"inputs_hash"));
    }

    #[test]
    fn 顺序就是台账顺序不另排() {
        // 两份对不上会让核对的人无所适从
        let js = [j("zzz", Tier::Recommend, 4), j("aaa", Tier::Recommend, 4)];
        let out = items_jsonl(&js, |k| Some(cand(k))).unwrap();
        let first = out.lines().next().unwrap();
        assert!(first.contains("zzz"), "{out}");
    }

    #[test]
    fn 成立维度数写出来便于复算() {
        let out = items_jsonl(&[j("k1", Tier::Recommend, 4)], |k| Some(cand(k))).unwrap();
        assert!(out.contains(r#""yes_count":4"#), "{out}");
    }

    #[test]
    fn 取不到候选也要出这一行() {
        let out = items_jsonl(&[j("k1", Tier::Recommend, 4)], |_| None).unwrap();
        assert!(out.contains(r#""account":"""#), "{out}");
        assert!(out.contains(r#""posted_at":null"#), "{out}");
        assert_eq!(out.lines().count(), 1, "候选取不到不是漏写这条的理由");
    }

    #[test]
    fn 采集轮账也能写成jsonl() {
        let s = to_jsonl(&[SweepLine {
            sweep_key: "csw-window".into(),
            query: "发布时间 2026-09-10~2026-09-20".into(),
            found: 1719,
            fetched_unique: 1719,
            in_window: 358,
            result: "ok".into(),
            error: String::new(),
        }])
        .unwrap();
        assert!(s.starts_with(r#"{"sweep_key":"csw-window""#), "{s}");
        assert!(s.contains(r#""in_window":358"#));
    }
}
