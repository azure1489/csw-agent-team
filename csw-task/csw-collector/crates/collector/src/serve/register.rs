//! 把一轮的产物翻译成引擎的登记格式，排进 outbox。
//!
//! # 顺序是依赖：items → sweeps → judgements → intake-check → submit
//!
//! `depends_on` 把它钉死，前一条没确认后一条不出队。原因不是洁癖：
//! `intake-check` 要拿台账与采集轮对账，判断还没登记就查，查出来的是假的红灯。
//!
//! # 这一层只做翻译与排队，不发
//!
//! 发是 outbox 那条循环的事。分开的好处是崩溃后不必重建请求体——
//! `body_json` 已经在库里，重试发的就是同一份字节。

use anyhow::Result;
use rusqlite::Connection;

use csw_collector_core::outbox::{self, NewEntry};
use csw_collector_core::types::{Candidate, Judgement, OutboxKind, Tier};
use csw_collector_engineapi::types::{ItemInput, JudgementInput, SweepInput};
use csw_collector_harvest::pipeline::SweepCount;

/// 工作台直接调接口，不经 MCP。引擎的 `ValidSweepTool` 里加了这个值。
pub const TOOL: &str = "csw_api";

/// 采集轮账 → 引擎格式。
///
/// **`unreviewed` 必须是 0**：工作台每条都判。不是 0 就说明这一步没跑完，
/// 那就**如实写**——`intake-check` 靠它判「漏没漏」，糊过去等于把自查变成摆设。
pub fn sweep_inputs(
    sweeps: &[SweepCount],
    window: (&str, &str),
    judged: usize,
    unjudged: usize,
) -> Vec<SweepInput> {
    sweeps
        .iter()
        .map(|s| SweepInput {
            sweep_key: s.sweep_key.clone(),
            platform: s.platform.clone(),
            source_key: s.source_key.clone(),
            tool: TOOL.into(),
            query: s.query.clone(),
            started_at: s.started_at.clone(),
            ended_at: s.ended_at.clone(),
            window_from: window.0.into(),
            window_to: window.1.into(),
            found: s.found,
            fetched_unique: s.fetched_unique,
            // 判断跑完之后才知道真的「审阅」了多少，所以由调用方传进来
            reviewed: judged as i64,
            unreviewed: unjudged as i64,
            corroborated: 0,
            in_window: s.in_window,
            registered: s.registered,
            result: s.result.clone(),
            error: s.error.clone(),
            paged_to_end: s.paged_to_end,
        })
        .collect()
}

/// 判断 → 引擎格式。
pub fn judgement_inputs(
    js: &[Judgement],
    by_key: impl Fn(&str) -> Option<Candidate>,
    carried: &std::collections::HashSet<String>,
    rubric_version: &str,
) -> Result<Vec<JudgementInput>> {
    js.iter()
        .map(|j| {
            let c = by_key(&j.candidate_key);
            Ok(JudgementInput {
                candidate_key: j.candidate_key.clone(),
                item_key: String::new(),
                platform: c
                    .as_ref()
                    .map(|c| c.platform.to_string())
                    .unwrap_or_else(|| "instagram".into()),
                post_ref: c.as_ref().map(|c| c.source_id.clone()).unwrap_or_default(),
                source_url: c.as_ref().map(|c| c.url.clone()).unwrap_or_default(),
                tier: serde_json::to_value(j.tier)?
                    .as_str()
                    .unwrap_or("pending_check")
                    .to_string(),
                dims: serde_json::to_value(&j.dims)?,
                three_sentences: serde_json::to_value(&j.three_sentences)?,
                comparison: serde_json::to_value(&j.comparison)?,
                heat_note: j.heat_note.clone(),
                gaps: serde_json::to_value(&j.gaps)?,
                hits: serde_json::json!({
                    "priority": j.priority_hits,
                    "lower": j.lower_hits,
                }),
                jev: serde_json::json!({ "disagreement": j.jev_disagreement }),
                rubric_version: rubric_version.into(),
                image_seen: j.image_seen,
                carried: carried.contains(&j.candidate_key),
            })
        })
        .collect()
}

/// 条目登记。**只登记推荐与备选**——不推荐的进台账不进 `run_items`，
/// 它们是「看过并判了」，不是「要做的条目」。待核也不登记：
/// 它还没定论，登记了下游就会以为可以开工。
pub fn item_inputs(js: &[Judgement], by_key: impl Fn(&str) -> Option<Candidate>) -> Vec<ItemInput> {
    js.iter()
        .filter(|j| matches!(j.tier, Tier::Recommend | Tier::Alternate))
        .map(|j| {
            let c = by_key(&j.candidate_key);
            ItemInput {
                item_key: j.candidate_key.clone(),
                title: j.three_sentences.what_changed.clone(),
                brand: String::new(),
                product: String::new(),
                source_url: c.as_ref().map(|c| c.url.clone()).unwrap_or_default(),
                published_at: c
                    .as_ref()
                    .and_then(|c| c.posted_at)
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                status: if j.tier == Tier::Recommend {
                    "candidate".into()
                } else {
                    "alternate".into()
                },
                ..Default::default()
            }
        })
        .collect()
}

/// 按依赖顺序把这一轮要写的都排进 outbox。返回最后一条的 seq，
/// 交付物提交挂在它后面。
pub fn enqueue_registration(
    conn: &Connection,
    round_id: i64,
    run_id: i64,
    items: &[ItemInput],
    sweeps: &[SweepInput],
    judgements: &[JudgementInput],
) -> Result<i64> {
    let mut prev: Option<i64> = None;
    for (kind, body) in [
        (OutboxKind::Items, serde_json::json!({ "items": items })),
        (OutboxKind::Sweeps, serde_json::json!({ "sweeps": sweeps })),
        (
            OutboxKind::Judgements,
            serde_json::json!({ "judgements": judgements }),
        ),
    ] {
        let json = body.to_string();
        // 幂等键从内容派生：同样的内容重排队不会写重，内容变了就是新的一条
        let sha = blake3::hash(json.as_bytes()).to_hex().to_string();
        let e = outbox::enqueue(
            conn,
            &NewEntry {
                round_id,
                kind,
                idem_key: format!("r{run_id}-{}-{}", kind_slug(kind), &sha[..16]),
                body_path: String::new(),
                body_json: json,
                body_sha: sha,
                depends_on: prev,
            },
        )?;
        prev = Some(e.seq);
    }
    prev.ok_or_else(|| anyhow::anyhow!("一条都没排进去"))
}

fn kind_slug(k: OutboxKind) -> &'static str {
    match k {
        OutboxKind::Ack => "ack",
        OutboxKind::Items => "items",
        OutboxKind::Sweeps => "sweeps",
        OutboxKind::Judgements => "judgements",
        OutboxKind::Deliverable => "deliverable",
        OutboxKind::Supplement => "supplement",
        OutboxKind::Fail => "fail",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{
        Comparison, ComparisonVerdict, Dim, DimJudgement, MediaKind, MediaRef, Platform,
        ThreeSentences, Unanswered, Verdict,
    };

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: format!("sc-{key}"),
            collector: "csw-window".into(),
            account: "acc".into(),
            url: format!("https://x/{key}"),
            text: String::new(),
            translated: String::new(),
            posted_at: "2026-09-17T08:00:00Z".parse().ok(),
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![MediaRef {
                source_hash: "h".into(),
                kind: MediaKind::Photo,
                url: "u".into(),
                blake3: None,
                ordinal: 0,
            }],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn j(key: &str, tier: Tier) -> Judgement {
        Judgement {
            candidate_key: key.into(),
            tier,
            dims: Dim::ALL
                .into_iter()
                .map(|d| {
                    (
                        d,
                        DimJudgement {
                            verdict: Verdict::Yes,
                            basis: "b".into(),
                        },
                    )
                })
                .collect(),
            three_sentences: ThreeSentences {
                what_changed: "换了背板结构".into(),
                why_it_matters: "乙".into(),
                how_different: "丙".into(),
            },
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
            },
            heat_note: "369 赞".into(),
            look: String::new(),
            image_seen: tier != Tier::PendingCheck,
            gaps: vec![],
            priority_hits: vec!["老产品结构性改款".into()],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "ih".into(),
        }
    }

    fn sweep() -> SweepCount {
        SweepCount {
            sweep_key: "csw-window".into(),
            platform: "instagram".into(),
            query: "发布时间 …".into(),
            found: 1719,
            fetched_unique: 1719,
            in_window: 358,
            result: "ok".into(),
            ..Default::default()
        }
    }

    #[test]
    fn 采集轮账如实写没判完的条数() {
        // 糊过去等于把自查变成摆设——intake-check 靠 unreviewed 判「漏没漏」
        let s = sweep_inputs(&[sweep()], ("A", "B"), 355, 3);
        assert_eq!(s[0].tool, "csw_api");
        assert_eq!(s[0].reviewed, 355);
        assert_eq!(s[0].unreviewed, 3);
        assert_eq!(s[0].window_from, "A");
        assert_eq!(s[0].in_window, 358);
    }

    #[test]
    fn 只登记推荐与备选() {
        let js = [
            j("k1", Tier::Recommend),
            j("k2", Tier::Alternate),
            j("k3", Tier::NotRecommend),
            j("k4", Tier::PendingCheck),
        ];
        let items = item_inputs(&js, |k| Some(cand(k)));
        let keys: Vec<&str> = items.iter().map(|i| i.item_key.as_str()).collect();
        // 不推荐的是「看过并判了」，不是「要做的条目」；
        // 待核还没定论，登记了下游会以为可以开工
        assert_eq!(keys, ["k1", "k2"]);
        assert_eq!(items[0].status, "candidate");
        assert_eq!(items[1].status, "alternate");
        assert_eq!(items[0].title, "换了背板结构");
        assert_eq!(items[0].published_at, "2026-09-17T08:00:00Z");
    }

    #[test]
    fn 判断台账每条都登记包括不推荐的() {
        let js = [
            j("k1", Tier::Recommend),
            j("k3", Tier::NotRecommend),
            j("k4", Tier::PendingCheck),
        ];
        let carried = std::collections::HashSet::from(["k3".to_string()]);
        let out = judgement_inputs(&js, |k| Some(cand(k)), &carried, "van-rubric/v1").unwrap();
        // 台账是「都判过了」的证据，少一条这句话就不成立
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].platform, "instagram");
        assert_eq!(out[0].post_ref, "sc-k1");
        assert!(!out[0].carried);
        assert!(out[1].carried, "结转的要标出来");
        assert!(!out[2].image_seen);
        assert_eq!(out[2].tier, "pending_check");
        assert_eq!(out[0].rubric_version, "van-rubric/v1");
        assert_eq!(out[0].hits["priority"][0], "老产品结构性改款");
    }

    #[test]
    fn 取不到候选也要出这一条() {
        let js = [j("k1", Tier::Recommend)];
        let out = judgement_inputs(&js, |_| None, &Default::default(), "v1").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].platform, "instagram",
            "取不到就按默认平台填，不漏这一条"
        );
    }

    #[test]
    fn 排队顺序是依赖() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = csw_collector_core::rounds::open_round(
            &c,
            &csw_collector_core::rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                run_id: Some(48),
                task_id: Some(1),
                stage_code: Some("intake".into()),
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap();

        let js = [j("k1", Tier::Recommend)];
        let items = item_inputs(&js, |k| Some(cand(k)));
        let sweeps = sweep_inputs(&[sweep()], ("A", "B"), 1, 0);
        let jis = judgement_inputs(&js, |k| Some(cand(k)), &Default::default(), "v1").unwrap();
        let last = enqueue_registration(&c, r.id, 48, &items, &sweeps, &jis).unwrap();

        // intake-check 要拿台账与采集轮对账，判断没登记就查，查出来的是假的红灯
        let first = csw_collector_core::outbox::next_ready(&c).unwrap().unwrap();
        assert_eq!(first.kind, "items");
        let all = csw_collector_core::outbox::outstanding(&c, r.id).unwrap();
        assert_eq!(
            all.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            ["items", "sweeps", "judgements"]
        );
        assert_eq!(all[2].seq, last);
        assert_eq!(all[1].depends_on, Some(all[0].seq));
        assert_eq!(all[2].depends_on, Some(all[1].seq));
    }

    #[test]
    fn 幂等键从内容派生重排不会写重() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = csw_collector_core::rounds::open_round(
            &c,
            &csw_collector_core::rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Manual,
                trigger: csw_collector_core::types::RoundTrigger::Manual,
                run_id: None,
                task_id: None,
                stage_code: None,
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap();
        let js = [j("k1", Tier::Recommend)];
        let items = item_inputs(&js, |k| Some(cand(k)));
        let sweeps = sweep_inputs(&[sweep()], ("A", "B"), 1, 0);
        let jis = judgement_inputs(&js, |k| Some(cand(k)), &Default::default(), "v1").unwrap();
        enqueue_registration(&c, r.id, 48, &items, &sweeps, &jis).unwrap();
        enqueue_registration(&c, r.id, 48, &items, &sweeps, &jis).unwrap();
        assert_eq!(
            csw_collector_core::outbox::outstanding(&c, r.id)
                .unwrap()
                .len(),
            3
        );
    }
}
