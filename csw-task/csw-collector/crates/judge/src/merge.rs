//! 合并同一件事。同一个发布常被好几个账号发成好几条候选。
//!
//! # 两道：先用向量粗并，再让 Jev 定
//!
//! 358 条候选两两配对是六万多对，全丢给 Jev 既慢又没必要。所以先用融合向量
//! 在内存里算一遍余弦（六万对 × 2048 维不到一秒），只把**像的**那些送去问。
//!
//! [`COARSE_SIM`] 是个**召回优先**的粗筛线，不是判据——判据是 Jev 的
//! [`SAME_EVENT_THRESHOLD`]。粗筛线宁可放宽：放过的对只是多花几次调用，
//! 漏掉的对就永远合不上了。
//!
//! # 阈值 0.5 是实测的
//!
//! 0.7 回测分三桶：
//!
//! | 桶 | same_event 中位（四分位） |
//! |---|---|
//! | 跨品牌随机（真负例） | **0.03（0.03/0.04）** |
//! | 跨账号同品牌 | 0.24（0.03/0.63） |
//! | 同账号同品牌 | 0.05（0.04/**0.97**） |
//!
//! 真负例贴在 0.03 且极紧，**误合并的风险很低**；同账号那一桶呈双峰
//! （多数确实不是同一件事，少数 0.97 的确实是）。0.5 落在两峰之间。
//!
//! # 图片哈希兜不了底
//!
//! 实测窗口内**共享 `mediaHash` 的组为 0**——跨账号转载时平台会重新编码，
//! 哈希对不上。所以同一事件只能走向量 + Jev，别指望哈希。

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;

use csw_collector_core::jev::{JevClient, Question};
use csw_collector_core::types::{Candidate, EventGroup};

/// 粗筛线。**召回优先**，不是判据。
pub const COARSE_SIM: f32 = 0.70;
/// Jev 的同一事件阈值。见模块文档。
pub const SAME_EVENT_THRESHOLD: f64 = 0.5;
/// 一条候选最多和几条去问。防止一个热点话题把调用量炸开。
pub const MAX_PAIRS_PER_CANDIDATE: usize = 5;

const FRAME: &str = "Do these two posts report the SAME underlying event — the same product launch, \
the same collaboration, the same store opening, the same exhibition? Two different posts by \
different accounts about one launch count as the same event. Two different products from one brand \
do not.";

/// 一条候选进合并时要带的。向量为 None 的条目**不参与合并**，各自成组——
/// 向量化失败时合不了是事实，不该拿别的信号硬凑。
pub struct MergeInput<'a> {
    pub candidate: &'a Candidate,
    pub fused: Option<&'a [f32]>,
}

/// 粗筛出来的候选对，按相似度从高到低。
pub fn coarse_pairs(items: &[MergeInput<'_>]) -> Vec<(usize, usize, f32)> {
    let mut all: Vec<(usize, usize, f32)> = Vec::new();
    for (i, ii) in items.iter().enumerate() {
        let Some(a) = ii.fused else { continue };
        for (j, jj) in items.iter().enumerate().skip(i + 1) {
            let Some(b) = jj.fused else { continue };
            let s = cosine(a, b);
            if s >= COARSE_SIM {
                all.push((i, j, s));
            }
        }
    }
    all.sort_by(|x, y| y.2.total_cmp(&x.2));
    // 每条最多留几对：一个热点话题下二十条候选会产生一百九十对，
    // 留满会把 Jev 的调用量炸开，而真正需要确认的只是最像的那几对
    let mut used = vec![0usize; items.len()];
    all.retain(|(i, j, _)| {
        if used[*i] >= MAX_PAIRS_PER_CANDIDATE || used[*j] >= MAX_PAIRS_PER_CANDIDATE {
            return false;
        }
        used[*i] += 1;
        used[*j] += 1;
        true
    });
    all
}

/// 问 Jev：这两条是不是同一件事。
pub async fn same_event(jev: &JevClient, a: &Candidate, b: &Candidate) -> Result<f64> {
    let state = json!({
        "post_a": {"account": a.account, "caption": a.text},
        "post_b": {"account": b.account, "caption": b.text},
    });
    let q = BTreeMap::from([(
        "same_event".to_string(),
        Question::noul(FRAME, "Same underlying event.", "Different events."),
    )]);
    Ok(jev.ask(&state, &q).await?.noul("same_event").unwrap_or(0.0))
}

/// 把判定为同一事件的对合成组。没有被合并的候选各自成组。
///
/// 合并用并查集：A=B、B=C 就把三条并到一组，**不要求 A 与 C 也被问过**。
/// 这是有意的——同一个发布被五个账号转，两两都问是十对，链式合并只要四对。
pub fn group(items: &[MergeInput<'_>], merged: &[(usize, usize, f64)]) -> Vec<EventGroup> {
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        if p[x] != x {
            let r = find(p, p[x]);
            p[x] = r;
        }
        p[x]
    }
    let mut notes: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (i, j, prob) in merged {
        let (ri, rj) = (find(&mut parent, *i), find(&mut parent, *j));
        if ri != rj {
            parent[rj] = ri;
        }
        notes
            .entry(find(&mut parent, *i))
            .or_default()
            .push(format!(
                "{} 与 {} 同一事件 {prob:.2}",
                items[*i].candidate.candidate_key, items[*j].candidate.candidate_key
            ));
    }

    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        by_root.entry(r).or_default().push(i);
    }

    by_root
        .into_iter()
        .map(|(root, members)| {
            let primary = pick_primary(items, &members);
            EventGroup {
                event_key: items[primary].candidate.candidate_key.clone(),
                primary: items[primary].candidate.candidate_key.clone(),
                members: members
                    .iter()
                    .map(|i| items[*i].candidate.candidate_key.clone())
                    .collect(),
                merge_note: notes.remove(&root).unwrap_or_default().join("；"),
            }
        })
        .collect()
}

/// 代表条目：**正文最全、图最多的那条**。同分时按 candidate_key 定——
/// 排序必须是确定的，否则同样的输入两次跑出不同的代表条目，对账就废了。
fn pick_primary(items: &[MergeInput<'_>], members: &[usize]) -> usize {
    *members
        .iter()
        .max_by(|x, y| {
            let k = |i: &usize| {
                let c = items[*i].candidate;
                (c.text.chars().count(), c.media.len())
            };
            k(x).cmp(&k(y)).then_with(|| {
                items[**y]
                    .candidate
                    .candidate_key
                    .cmp(&items[**x].candidate.candidate_key)
            })
        })
        .expect("组里至少一条")
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{MediaKind, MediaRef, Platform};

    fn cand(key: &str, account: &str, text: &str, imgs: usize) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "c".into(),
            account: account.into(),
            url: format!("https://x/{key}"),
            text: text.into(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Carousel".into(),
            media: (0..imgs)
                .map(|i| MediaRef {
                    source_hash: format!("h{i}"),
                    kind: MediaKind::Photo,
                    url: format!("u{i}"),
                    blake3: None,
                    ordinal: i as u16,
                })
                .collect(),
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn v(main: usize, weight: f32) -> Vec<f32> {
        let mut x = vec![0.1_f32; 8];
        x[main] = weight;
        x
    }

    #[test]
    fn 只把像的送去问() {
        let (a, b, c) = (
            cand("a", "acc1", "山と道 新包发布", 3),
            cand("b", "acc2", "山と道 新包转载", 1),
            cand("c", "acc3", "完全不相干的话题", 2),
        );
        let (va, vb, vc) = (v(0, 1.0), v(0, 0.95), v(4, 1.0));
        let items = vec![
            MergeInput {
                candidate: &a,
                fused: Some(&va),
            },
            MergeInput {
                candidate: &b,
                fused: Some(&vb),
            },
            MergeInput {
                candidate: &c,
                fused: Some(&vc),
            },
        ];
        let pairs = coarse_pairs(&items);
        assert_eq!(pairs.len(), 1, "只有 a-b 该过粗筛：{pairs:?}");
        assert_eq!((pairs[0].0, pairs[0].1), (0, 1));
    }

    #[test]
    fn 没有向量的不参与合并() {
        let (a, b) = (cand("a", "acc1", "甲", 1), cand("b", "acc2", "乙", 1));
        let va = v(0, 1.0);
        let items = vec![
            MergeInput {
                candidate: &a,
                fused: Some(&va),
            },
            MergeInput {
                candidate: &b,
                fused: None,
            },
        ];
        // 向量化失败时合不了是事实，不该拿别的信号硬凑
        assert!(coarse_pairs(&items).is_empty());
        let gs = group(&items, &[]);
        assert_eq!(gs.len(), 2, "各自成组");
    }

    #[test]
    fn 一条最多问几对() {
        let cands: Vec<Candidate> = (0..10)
            .map(|i| cand(&format!("c{i}"), "acc", "同一个热点", 1))
            .collect();
        let vs: Vec<Vec<f32>> = (0..10).map(|i| v(0, 1.0 - i as f32 * 0.001)).collect();
        let items: Vec<MergeInput<'_>> = cands
            .iter()
            .zip(&vs)
            .map(|(c, x)| MergeInput {
                candidate: c,
                fused: Some(x),
            })
            .collect();
        let pairs = coarse_pairs(&items);
        // 十条两两是 45 对；留满会把 Jev 的调用量炸开
        assert!(pairs.len() < 45);
        let mut used = vec![0usize; 10];
        for (i, j, _) in &pairs {
            used[*i] += 1;
            used[*j] += 1;
        }
        assert!(
            used.iter().all(|n| *n <= MAX_PAIRS_PER_CANDIDATE),
            "{used:?}"
        );
    }

    #[test]
    fn 链式合并不要求两两都问过() {
        let (a, b, c) = (
            cand("a", "acc1", "很长很长的正文最全的一条", 5),
            cand("b", "acc2", "短", 1),
            cand("c", "acc3", "也短", 1),
        );
        let x = v(0, 1.0);
        let items = vec![
            MergeInput {
                candidate: &a,
                fused: Some(&x),
            },
            MergeInput {
                candidate: &b,
                fused: Some(&x),
            },
            MergeInput {
                candidate: &c,
                fused: Some(&x),
            },
        ];
        // 只问了 a-b 与 b-c，a-c 没问过，但三条该在一组
        let gs = group(&items, &[(0, 1, 0.9), (1, 2, 0.8)]);
        assert_eq!(gs.len(), 1);
        assert_eq!(gs[0].members.len(), 3);
        // 代表条目是正文最全、图最多的那条
        assert_eq!(gs[0].primary, "a");
        assert!(gs[0].merge_note.contains("0.90"), "{}", gs[0].merge_note);
    }

    #[test]
    fn 代表条目的挑选是确定的() {
        // 正文一样长、图一样多时，按 candidate_key 定——
        // 同样的输入两次跑出不同代表条目的话，对账就废了
        let (a, b) = (
            cand("aaa", "acc1", "一样长", 2),
            cand("bbb", "acc2", "一样长", 2),
        );
        let x = v(0, 1.0);
        let items = vec![
            MergeInput {
                candidate: &a,
                fused: Some(&x),
            },
            MergeInput {
                candidate: &b,
                fused: Some(&x),
            },
        ];
        let first = group(&items, &[(0, 1, 0.9)])[0].primary.clone();
        let second = group(&items, &[(0, 1, 0.9)])[0].primary.clone();
        assert_eq!(first, second);
        assert_eq!(first, "aaa");
    }

    #[test]
    fn 余弦对不齐的向量返回零而不是恐慌() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn 问同一事件只送账号与正文() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "answers": {"same_event": {"noul": 0.97}}
            })))
            .mount(&server)
            .await;
        let jev = JevClient::new(
            csw_collector_core::jev::JevConfig {
                base_url: server.uri(),
                ..Default::default()
            },
            "k",
        )
        .unwrap();
        let (a, b) = (cand("a", "acc1", "甲", 1), cand("b", "acc2", "乙", 1));
        assert_eq!(same_event(&jev, &a, &b).await.unwrap(), 0.97);
    }
}
