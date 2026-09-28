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
use std::collections::{HashMap, HashSet};

use csw_collector_core::types::{Candidate, Dim, Judgement, OutboxKind, Tier, Topic};
use csw_collector_engineapi::types::{ItemInput, JudgementInput, SweepInput};
use csw_collector_harvest::pipeline::SweepCount;

/// 工作台直接调接口，不经 MCP。引擎的 `ValidSweepTool` 里加了这个值。
pub const TOOL: &str = "csw_api";

/// 采集轮键 → 候选上记的采集器名：`csw-window` → `csw_window`，`van-links` → `van_link`。
pub fn collector_of(sweep_key: &str) -> String {
    sweep_key
        .replace('-', "_")
        .trim_end_matches('s')
        .to_string()
}

/// 采集器 → 它那一路的采集轮键（[`collector_of`] 的反向）。条目的 `discovered_via` 要写它。
pub fn sweep_key_of(collector: &str) -> String {
    ["csw-window", "van-links"]
        .into_iter()
        .find(|k| collector_of(k) == collector)
        .map(str::to_string)
        .unwrap_or_else(|| collector.replace('_', "-"))
}

/// 采集轮账 → 引擎格式。
///
/// **`unreviewed` 必须是 0**：工作台每条都判。不是 0 就说明这一步没跑完，
/// 那就**如实写**——`intake-check` 靠它判「漏没漏」，糊过去等于把自查变成摆设。
///
/// 已审 / 未审**按采集轮各算各的**：`per(sweep_key)` 给出这一路候选里判了几条、没判几条。
/// 引擎把各轮加总：原先每一路都填整轮的数，两路就翻倍（Van 链接那一路取到 0 条也报「已审 124」），
/// 「产出落差」那条判据拿着翻倍的分母去算。
pub fn sweep_inputs(
    sweeps: &[SweepCount],
    window: (&str, &str),
    per: impl Fn(&str) -> (usize, usize),
) -> Vec<SweepInput> {
    sweeps
        .iter()
        .map(|s| (s, per(&s.sweep_key)))
        .map(|(s, (judged, unjudged))| SweepInput {
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

/// 六维 → 引擎要的**对象** `{"change": {"verdict", "basis"}, …}`。
///
/// `Judgement::dims` 是 `Vec<(Dim, DimJudgement)>`，直接序列化是**数组**，引擎按对象解析
/// 整批 400 `bad_dims_json`。09-23 演练时条目先挂了、台账没发出去，这一处没撞上；
/// 对着引擎源码核契约时才看出来。
///
/// 引擎还要求六维齐全、每维有依据，**一条不合就整批拒**。模型偶尔漏写依据——
/// 按引擎自己的口径「编不出来就判 unclear」补上并写明，不让一条拖垮整批。
fn dims_object(j: &Judgement) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for d in Dim::ALL {
        let key = serde_json::to_value(d)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let found = j.dims.iter().find(|(k, _)| *k == d).map(|(_, v)| v);
        let v = match found {
            Some(dj) if !dj.basis.trim().is_empty() => serde_json::json!({
                "verdict": dj.verdict,
                "basis": dj.basis,
            }),
            Some(_) => {
                serde_json::json!({"verdict": "unclear", "basis": "模型没给出依据，按不明处理"})
            }
            None => {
                serde_json::json!({"verdict": "unclear", "basis": "模型没判这一维，按不明处理"})
            }
        };
        m.insert(key, v);
    }
    serde_json::Value::Object(m)
}

/// 判断 → 引擎格式（不挂条目）。
#[cfg(test)]
pub fn judgement_inputs(
    js: &[Judgement],
    by_key: impl Fn(&str) -> Option<Candidate>,
    carried: &std::collections::HashSet<String>,
    rubric_version: &str,
) -> Result<Vec<JudgementInput>> {
    judgement_inputs_with_items(js, by_key, carried, rubric_version, |_| None)
}

/// 三句话连同标题与看点类型一起放进引擎的 `three_sentences` 列（引擎原样存 JSON，
/// 不改表）；查重连同制作条件放进 `comparison` 列。
fn engine_three(j: &Judgement) -> Result<serde_json::Value> {
    let mut v = serde_json::to_value(&j.three_sentences)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("headline".into(), serde_json::json!(j.headline));
        o.insert("novelty".into(), serde_json::to_value(&j.novelty)?);
    }
    Ok(v)
}

fn engine_comparison(j: &Judgement) -> Result<serde_json::Value> {
    let mut v = serde_json::to_value(&j.comparison)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("readiness".into(), serde_json::to_value(&j.readiness)?);
    }
    Ok(v)
}

/// 判断 → 引擎格式。`item_of` 给出这条候选挂在哪个已登记的条目（选题）上。
pub fn judgement_inputs_with_items(
    js: &[Judgement],
    by_key: impl Fn(&str) -> Option<Candidate>,
    carried: &std::collections::HashSet<String>,
    rubric_version: &str,
    item_of: impl Fn(&str) -> Option<String>,
) -> Result<Vec<JudgementInput>> {
    js.iter()
        .map(|j| {
            let c = by_key(&j.candidate_key);
            Ok(JudgementInput {
                candidate_key: j.candidate_key.clone(),
                item_key: item_of(&j.candidate_key).unwrap_or_default(),
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
                dims: dims_object(j),
                three_sentences: engine_three(j)?,
                comparison: engine_comparison(j)?,
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
#[cfg(test)]
pub fn item_inputs(js: &[Judgement], by_key: impl Fn(&str) -> Option<Candidate>) -> Vec<ItemInput> {
    item_inputs_by_topic(js, &csw_collector_judge::topic::build(&[], js), by_key)
}

/// 按**选题**登记（09-22 反馈第五项）：同产品、同事件的多帖只占一个条目，
/// 条目键取代表帖，其余帖子写进去重说明。逐帖判断照样全写进判断台账。
pub fn item_inputs_by_topic(
    js: &[Judgement],
    topics: &[Topic],
    by_key: impl Fn(&str) -> Option<Candidate>,
) -> Vec<ItemInput> {
    // 待核的选题也登记（pending_check）：只登推荐 / 备选的话，待核在引擎里被撤成 dropped，
    // 等于把缺证当淘汰（09-28 r56 #616：19 条待核在 intake-trace 里是 dropped）
    topics
        .iter()
        .filter(|t| {
            matches!(
                t.tier,
                Some(Tier::Recommend | Tier::Alternate | Tier::PendingCheck)
            )
        })
        .filter_map(|t| {
            let j = js.iter().find(|j| j.candidate_key == t.primary_key)?;
            let c = by_key(&j.candidate_key);
            let others: Vec<String> = t
                .members
                .iter()
                .filter(|m| **m != t.primary_key)
                .map(|m| by_key(m).map(|c| c.url).unwrap_or_else(|| m.clone()))
                .collect();
            Some(ItemInput {
                item_key: t.topic_key.clone(),
                title: if t.headline.trim().is_empty() {
                    j.headline.clone()
                } else {
                    t.headline.clone()
                },
                brand: c.as_ref().map(|c| c.account.clone()).unwrap_or_default(),
                evidence_url: c.as_ref().map(|c| c.url.clone()).unwrap_or_default(),
                // 去重说明：查重结论 + 同选题的其余帖子。引擎的去重判据看这一栏
                dedup_note: {
                    let mut n = format!(
                        "查重：{}",
                        csw_collector_deliver::intake::comparison_label(j.comparison.verdict)
                    );
                    if let Some(h) = j.comparison.hits.first() {
                        n.push_str(&format!("（对照《{}》）", h.title));
                    }
                    if !others.is_empty() {
                        n.push_str(&format!(
                            "；同选题另有 {} 帖：{}",
                            others.len(),
                            others.join(" ")
                        ));
                    }
                    n
                },
                product: String::new(),
                source_url: c.as_ref().map(|c| c.url.clone()).unwrap_or_default(),
                published_at: c
                    .as_ref()
                    .and_then(|c| c.posted_at)
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                // 从哪一路采集来的、什么时候进库：引擎按 discovered_via 把条目挂到采集轮上
                //（09-28 r56 退回：38 条条目缺 discovered_via）
                discovered_via: c
                    .as_ref()
                    .map(|c| sweep_key_of(&c.collector))
                    .unwrap_or_default(),
                fetched_at: c
                    .as_ref()
                    .and_then(|c| c.ingested_at)
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                first_seen_at: c
                    .as_ref()
                    .and_then(|c| c.ingested_at)
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                origin: if c
                    .as_ref()
                    .is_some_and(|c| c.collector == collector_of("van-links"))
                {
                    "van_link".into()
                } else {
                    "dispatch".into()
                },
                // 引擎的条目状态只有 candidate / pending_check / shortlisted / dropped，
                // 没有「备选」。v9 01 的口径是「判断过就不许留在 candidate」——推荐与备选
                // 都是判过、成形的，登记成 shortlisted（成熟）；两者的区别在判断台账的档位里。
                // 这里原先写的是 candidate / alternate，09-23 演练时引擎 400 bad_item_status
                //（mock 不校验这个字段）。
                // 备选里还挂着影响选题判断的缺口的（备选·待补证），**不算成熟**，登记成待核
                //（09-28 r56 退回：「这些不能算成熟备选」）
                status: if j.tier == Tier::PendingCheck || j.has_decision_gap() {
                    "pending_check".into()
                } else {
                    "shortlisted".into()
                },
                // 待核写明缺什么：主编看登记就知道为什么没成熟
                reason: j
                    .gaps
                    .iter()
                    .find(|g| g.level == csw_collector_core::types::GapLevel::Decision)
                    .map(|g| format!("待核：{}", g.what))
                    .unwrap_or_default(),
                ..Default::default()
            })
        })
        .collect()
}

/// 候选 → 它所属、且已登记的选题条目键（推荐 / 备选的选题）。
pub fn registered_item_of(topics: &[Topic]) -> HashMap<String, String> {
    topics
        .iter()
        .filter(|t| {
            matches!(
                t.tier,
                Some(Tier::Recommend | Tier::Alternate | Tier::PendingCheck)
            )
        })
        .flat_map(|t| t.members.iter().map(|m| (m.clone(), t.topic_key.clone())))
        .collect()
}

/// 候选 → 所属条目键，**连同这一轮登记过、后来撤掉的**。
///
/// 撤掉的条目在引擎里还在（状态 dropped），它名下的判断也还挂着这个条目键；
/// 本包若把它们写成「无所属条目」，逐键对账就对不上（09-28 r56 v7：12 条判断只差所属条目）。
pub fn item_of_with_history(
    conn: &Connection,
    round_id: i64,
    topics: &[Topic],
) -> HashMap<String, String> {
    let mut m = registered_item_of(topics);
    let history = registered_items(conn, round_id);
    for t in topics.iter().filter(|t| history.contains_key(&t.topic_key)) {
        for member in &t.members {
            m.entry(member.clone())
                .or_insert_with(|| t.topic_key.clone());
        }
    }
    m
}

/// 判断一次最多发多少条。引擎侧限 200，**在排队时就分好批**——
/// 一条 outbox 就是一次请求，发的时候原样发，不再切。
/// 切在发送侧的话「重试发同样的字节」就守不住了。
pub const JUDGEMENT_BATCH: usize = 200;

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
    let mut bodies: Vec<(OutboxKind, serde_json::Value)> = vec![
        (OutboxKind::Items, serde_json::json!({ "items": items })),
        (OutboxKind::Sweeps, serde_json::json!({ "sweeps": sweeps })),
    ];
    for chunk in judgements.chunks(JUDGEMENT_BATCH.max(1)) {
        bodies.push((
            OutboxKind::Judgements,
            serde_json::json!({ "judgements": chunk }),
        ));
    }
    let mut prev: Option<i64> = None;
    for (kind, body) in bodies {
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

/// 这一轮上次发给引擎的某类登记体（最新一份）。返工时要拿它对账。
fn last_body(conn: &Connection, round_id: i64, kind: &str) -> Option<serde_json::Value> {
    conn.query_row(
        "SELECT body_json FROM engine_outbox WHERE round_id = ?1 AND kind = ?2
         ORDER BY seq DESC LIMIT 1",
        rusqlite::params![round_id, kind],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|s| serde_json::from_str(&s).ok())
}

/// 这一轮登记过、还没撤、这次不再登记的条目：改登记为 `dropped` 并写明原因。
/// 返工在原轮次上收口窗口、改档后，引擎里旧的 shortlisted / pending_check 不撤掉，主编看到的就还是旧的。
///
/// **比的是这一轮历次登记的并集**（每个键取最后一次的状态），不只是上一次：09-28 r56 v5 只补登了
/// 一部分条目，v6 只跟 v5 比，v4 登记成待核、后来判成不推荐的 7 条就一直挂在引擎里（主编 v6 退回点名）。
pub fn dropped_since(conn: &Connection, round_id: i64, now: &[ItemInput]) -> Vec<ItemInput> {
    let keep: HashSet<&str> = now.iter().map(|i| i.item_key.as_str()).collect();
    registered_items(conn, round_id)
        .into_iter()
        .filter(|(k, (status, _))| !keep.contains(k.as_str()) && status != "dropped")
        .map(|(key, (_, title))| ItemInput {
            item_key: key,
            title,
            status: "dropped".into(),
            reason_code: "superseded".into(),
            reason: "返工后不再列入本期（移出窗口，或重判后不再是推荐 / 备选 / 待核）".into(),
            ..Default::default()
        })
        .collect()
}

/// 只补登条目（纠正引擎与本包不一致的）。与整套登记同一套幂等键规则。
pub fn enqueue_items(
    conn: &Connection,
    round_id: i64,
    run_id: i64,
    items: &[ItemInput],
) -> Result<i64> {
    let json = serde_json::json!({ "items": items }).to_string();
    let sha = blake3::hash(json.as_bytes()).to_hex().to_string();
    let e = outbox::enqueue(
        conn,
        &NewEntry {
            round_id,
            kind: OutboxKind::Items,
            idem_key: format!("r{run_id}-{}-{}", kind_slug(OutboxKind::Items), &sha[..16]),
            body_path: String::new(),
            body_json: json,
            body_sha: sha,
            depends_on: None,
        },
    )?;
    Ok(e.seq)
}

/// 这一轮历次登记过的条目，每个键取最后一次的（状态, 标题）。这就是引擎里应有的样子。
pub fn registered_items(
    conn: &Connection,
    round_id: i64,
) -> std::collections::BTreeMap<String, (String, String)> {
    // 按登记顺序覆盖
    let mut last: std::collections::BTreeMap<String, (String, String)> = Default::default();
    for body in all_bodies(conn, round_id, "items") {
        for it in body
            .get("items")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(key) = it.get("item_key").and_then(|v| v.as_str()) else {
                continue;
            };
            let status = it.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let title = it.get("title").and_then(|v| v.as_str()).unwrap_or("");
            last.insert(key.to_string(), (status.to_string(), title.to_string()));
        }
    }
    last
}

/// 这一轮某类登记历次排队的内容，按排队顺序。
fn all_bodies(conn: &Connection, round_id: i64, kind: &str) -> Vec<serde_json::Value> {
    let Ok(mut st) = conn.prepare(
        "SELECT body_json FROM engine_outbox WHERE round_id = ?1 AND kind = ?2 ORDER BY seq",
    ) else {
        return Vec::new();
    };
    st.query_map(rusqlite::params![round_id, kind], |r| r.get::<_, String>(0))
        .map(|it| {
            it.filter_map(Result::ok)
                .filter_map(|s| serde_json::from_str(&s).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// 这一轮上次上报的采集轮。返工不重新采集，采集轮沿用，只把窗口内条数按收口后的重算。
pub fn previous_sweeps(conn: &Connection, round_id: i64) -> Vec<SweepCount> {
    let Some(body) = last_body(conn, round_id, "sweeps") else {
        return Vec::new();
    };
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let n = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
    body.get("sweeps")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .map(|v| SweepCount {
            sweep_key: s(v, "sweep_key"),
            platform: s(v, "platform"),
            source_key: s(v, "source_key"),
            query: s(v, "query"),
            found: n(v, "found"),
            fetched_unique: n(v, "fetched_unique"),
            in_window: n(v, "in_window"),
            reviewed: n(v, "reviewed"),
            unreviewed: n(v, "unreviewed"),
            registered: n(v, "registered"),
            paged_to_end: v
                .get("paged_to_end")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            result: s(v, "result"),
            error: s(v, "error"),
            started_at: s(v, "started_at"),
            ended_at: s(v, "ended_at"),
            // 逐条留痕在本地 window_trace，不走引擎
            trace: Vec::new(),
        })
        .collect()
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
    use csw_collector_core::types::{Dim, MediaKind, MediaRef, Platform};

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
        let mut j = Judgement::fixture(key, tier);
        for (_, d) in &mut j.dims {
            d.basis = "b".into();
        }
        j.headline = format!("{key}｜背板结构值得解释");
        j.three_sentences.what = "换了背板结构".into();
        j.heat_note = "369 赞".into();
        j.image_seen = tier != Tier::PendingCheck;
        j.priority_hits = vec!["老产品结构性改款".into()];
        j.inputs_hash = "ih".into();
        j
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
        let s = sweep_inputs(&[sweep()], ("A", "B"), |_| (355, 3));
        assert_eq!(s[0].tool, "csw_api");
        assert_eq!(s[0].reviewed, 355);
        assert_eq!(s[0].unreviewed, 3);
        assert_eq!(s[0].window_from, "A");
        assert_eq!(s[0].in_window, 358);
    }

    #[test]
    fn 同一选题只登记一个条目其余帖子挂在它下面() {
        let js = vec![
            j("s2w8-a", Tier::Alternate),
            j("s2w8-b", Tier::Recommend),
            j("s2w8-c", Tier::Recommend),
            j("x", Tier::NotRecommend),
        ];
        let groups = vec![csw_collector_core::types::EventGroup {
            event_key: "s2w8-a".into(),
            primary: "s2w8-a".into(),
            members: vec!["s2w8-a".into(), "s2w8-b".into(), "s2w8-c".into()],
            merge_note: String::new(),
        }];
        let topics = csw_collector_judge::topic::build(&groups, &js);
        let items = item_inputs_by_topic(&js, &topics, |k| Some(cand(k)));
        assert_eq!(items.len(), 1, "三帖一个选题，只占一个条目");
        assert_eq!(items[0].item_key, "s2w8-b");
        assert!(
            items[0].dedup_note.contains("同选题另有 2 帖"),
            "{}",
            items[0].dedup_note
        );
        let item_of = registered_item_of(&topics);
        let jis = judgement_inputs_with_items(
            &js,
            |k| Some(cand(k)),
            &Default::default(),
            "v2",
            |k| item_of.get(k).cloned(),
        )
        .unwrap();
        // 逐帖判断全写，同题的都挂到那个条目上；不推荐的不挂
        assert_eq!(jis.len(), 4);
        for ji in &jis[..3] {
            assert_eq!(ji.item_key, "s2w8-b");
        }
        assert!(jis[3].item_key.is_empty());
        // 标题、看点类型与制作条件随判断一起进引擎（引擎原样存 JSON，不改表）
        assert!(jis[0].three_sentences.get("headline").is_some());
        assert!(jis[0].three_sentences.get("novelty").is_some());
        assert!(jis[0].comparison.get("readiness").is_some());
        assert!(jis[0].comparison.get("hits").is_some());
    }

    #[test]
    fn 登记推荐备选与待核_不登记不推荐() {
        let js = [
            j("k1", Tier::Recommend),
            j("k2", Tier::Alternate),
            j("k3", Tier::NotRecommend),
            j("k4", Tier::PendingCheck),
        ];
        let items = item_inputs(&js, |k| Some(cand(k)));
        let keys: Vec<&str> = items.iter().map(|i| i.item_key.as_str()).collect();
        // 不推荐的是「看过并判了」，不是「要做的条目」，不登记；
        // 待核登记成 pending_check——不登记的话引擎里就成了 dropped，缺证被当淘汰（09-28 r56 #616）
        assert_eq!(keys, ["k1", "k2", "k4"]);
        // 引擎只认四个值，推荐与备选都是 shortlisted
        assert_eq!(items[0].status, "shortlisted");
        assert_eq!(items[1].status, "shortlisted");
        assert_eq!(items[2].status, "pending_check");
        assert_eq!(items[0].title, "k1｜背板结构值得解释");
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
        let sweeps = sweep_inputs(&[sweep()], ("A", "B"), |_| (1, 0));
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
    fn 撤条目比这一轮历次登记的并集而不只是上一次() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = csw_collector_core::rounds::open_round(
            &c,
            &csw_collector_core::rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                run_id: Some(56),
                task_id: Some(592),
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
        let it = |k: &str, st: &str| ItemInput {
            item_key: k.into(),
            title: format!("{k} 标题"),
            status: st.into(),
            ..Default::default()
        };
        let put = |items: &[ItemInput]| {
            enqueue_registration(&c, r.id, 56, items, &[], &[]).unwrap();
        };
        // v4：a 待核、b 采用、x 已撤；v5 只补登了 b（09-28 r56 就是这样）
        put(&[
            it("a", "pending_check"),
            it("b", "shortlisted"),
            it("x", "dropped"),
        ]);
        put(&[it("b", "shortlisted")]);
        // v6：只列 b。a 在 v4 登记成待核、之后没撤，要撤；x 早撤过，不再发
        let out = dropped_since(&c, r.id, &[it("b", "shortlisted")]);
        assert_eq!(
            out.iter()
                .map(|i| (i.item_key.as_str(), i.status.as_str()))
                .collect::<Vec<_>>(),
            [("a", "dropped")]
        );
        assert_eq!(out[0].title, "a 标题");
        // 撤过之后再登记，就不再撤第二次
        put(&[it("b", "shortlisted"), it("a", "dropped")]);
        assert!(dropped_since(&c, r.id, &[it("b", "shortlisted")]).is_empty());

        // 撤掉的条目名下的判断仍挂着它：a 这个选题现在不推荐，成员 a、a2 仍归 a
        let topic = |k: &str, members: &[&str], tier| Topic {
            topic_key: k.into(),
            primary_key: k.into(),
            members: members.iter().map(|m| m.to_string()).collect(),
            merge_note: String::new(),
            tier: Some(tier),
            headline: String::new(),
            synthesis: Default::default(),
        };
        let topics = [
            topic("a", &["a", "a2"], Tier::NotRecommend),
            topic("b", &["b"], Tier::Recommend),
            topic("z", &["z"], Tier::NotRecommend), // 从没登记过
        ];
        let m = item_of_with_history(&c, r.id, &topics);
        assert_eq!(m.get("a2").map(String::as_str), Some("a"));
        assert_eq!(m.get("b").map(String::as_str), Some("b"));
        assert!(!m.contains_key("z"), "从没登记过的不挂条目");
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
        let sweeps = sweep_inputs(&[sweep()], ("A", "B"), |_| (1, 0));
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

    #[test]
    fn 六维送成对象且缺依据的补成不明() {
        use csw_collector_core::types::{DimJudgement, Verdict};
        let jj = Judgement {
            dims: vec![
                (
                    Dim::Change,
                    DimJudgement {
                        verdict: Verdict::Yes,
                        basis: "正文「换了背板」".into(),
                    },
                ),
                (
                    Dim::Use,
                    DimJudgement {
                        verdict: Verdict::No,
                        basis: "  ".into(),
                    },
                ),
            ],
            ..j("k", Tier::Recommend)
        };
        let v = dims_object(&jj);
        let o = v.as_object().expect("引擎按对象解析，不能是数组");
        for k in ["change", "use", "gain", "compare", "explain", "csw"] {
            let d = &o[k];
            assert!(
                !d["basis"].as_str().unwrap().trim().is_empty(),
                "{k} 没依据"
            );
            assert!(
                ["yes", "no", "unclear"].contains(&d["verdict"].as_str().unwrap()),
                "{k}"
            );
        }
        assert_eq!(o["change"]["verdict"], "yes");
        assert_eq!(o["use"]["verdict"], "unclear", "依据空着的按不明");
        assert_eq!(o["csw"]["verdict"], "unclear", "没判的维按不明");
    }

    #[test]
    fn 采集轮键对得上候选的采集器名() {
        assert_eq!(collector_of("csw-window"), "csw_window");
        assert_eq!(collector_of("van-links"), "van_link");
        // 两路各报各的，不再每路都填整轮的数
        let a = SweepCount {
            sweep_key: "csw-window".into(),
            ..sweep()
        };
        let b = SweepCount {
            sweep_key: "van-links".into(),
            ..sweep()
        };
        let s = sweep_inputs(&[a, b], ("A", "B"), |k| {
            if k == "csw-window" { (120, 4) } else { (0, 0) }
        });
        assert_eq!((s[0].reviewed, s[0].unreviewed), (120, 4));
        assert_eq!((s[1].reviewed, s[1].unreviewed), (0, 0));
    }

    /// **对真引擎把登记整段跑一遍**：条目、采集轮、判断台账都用这个模块真实的转换函数生成，
    /// 再让本机起的 `csw-task-svc/bin/server` 校验。引擎二进制不在就跳过。
    ///
    /// 09-23 演练连撞三处契约错（提交 kind、条目状态、六维是数组）——都是 mock 放过、
    /// 真引擎一上就挂的。这条测试就是为了让它们在本机就挂。
    #[tokio::test]
    async fn 登记三件套过得了真引擎的校验() {
        use std::process::{Command, Stdio};
        let bin =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../csw-task-svc/bin");
        let (adminctl, server) = (bin.join("adminctl"), bin.join("server"));
        if !adminctl.exists() || !server.exists() {
            println!("跳过：没有 {}（先 make build Go 那一侧）", bin.display());
            return;
        }
        csw_collector_core::ensure_crypto_provider();
        let dir = tempdir::TempDir::new("csw-reg-live").unwrap();
        let db = dir.path().join("e.db");
        let run = |args: &[&str]| {
            let o = Command::new(&adminctl)
                .args(args)
                .env("CSW_DB_PATH", &db)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{args:?}：{}",
                String::from_utf8_lossy(&o.stderr)
            );
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let tok = |s: String| {
            s.split_whitespace()
                .find(|w| w.starts_with("csw_live_"))
                .unwrap()
                .to_string()
        };
        run(&["migrate", "up"]);
        let collector = tok(run(&["token", "issue", "collector"]));
        let editor = tok(run(&["token", "issue", "editor"]));
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut child = Command::new(&server)
            .env("CSW_DB_PATH", &db)
            .env("CSW_ADDR", format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}/api/v1");
        let http = reqwest::Client::new();
        for _ in 0..80 {
            if http
                .get(format!("{base}/me/tasks"))
                .bearer_auth(&collector)
                .send()
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let body: serde_json::Value = http
            .post(format!("{base}/workflows/daily_news/runs"))
            .bearer_auth(&editor)
            .json(&serde_json::json!({"subject": "2026-09-24"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let run_id = body["run_id"].as_i64().expect("触发一期");
        let c = csw_collector_engineapi::EngineClient::new(
            &base,
            &collector,
            std::time::Duration::from_secs(10),
        )
        .unwrap();

        // 真实的转换：两条推荐 / 备选、一条待核（没读到图）、一条不推荐；其中一条六维缺依据
        let mut judgements = vec![
            j("a-000001", Tier::Recommend),
            j("b-000002", Tier::Alternate),
            j("c-000003", Tier::PendingCheck),
            j("d-000004", Tier::NotRecommend),
        ];
        judgements[2].image_seen = false;
        judgements[3].dims[0].1.basis = String::new();
        let by_key: std::collections::HashMap<String, Candidate> =
            ["a-000001", "b-000002", "c-000003", "d-000004"]
                .iter()
                .map(|k| (k.to_string(), cand(k)))
                .collect();
        let lookup = |k: &str| by_key.get(k).cloned();
        // a、b 同一选题：只登记一个条目，两帖的判断都挂在它上面
        let topics = csw_collector_judge::topic::build(
            &[csw_collector_core::types::EventGroup {
                event_key: "a-000001".into(),
                primary: "a-000001".into(),
                members: vec!["a-000001".into(), "b-000002".into()],
                merge_note: String::new(),
            }],
            &judgements,
        );
        let items = item_inputs_by_topic(&judgements, &topics, lookup);
        // a、b 一个选题 + c 待核（登记成 pending_check）
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].status, "pending_check");
        let item_of = registered_item_of(&topics);
        let sweeps = sweep_inputs(
            &[SweepCount {
                in_window: 4,
                fetched_unique: 4,
                found: 4,
                ..sweep()
            }],
            ("2026-09-23T00:00:00Z", "2026-09-24T00:00:00Z"),
            |_| (4, 0),
        );
        // v10 的形状（分级缺口、查重 hits、标题与看点类型）照样过引擎的校验
        judgements[2].gaps = vec![csw_collector_core::types::Gap::decision("未读到实图")];
        let jis = judgement_inputs_with_items(
            &judgements,
            lookup,
            &Default::default(),
            "van-rubric/v2",
            |k| item_of.get(k).cloned(),
        )
        .unwrap();
        assert_eq!(jis[1].item_key, "a-000001");

        let r1 = c.put_items(run_id, &items).await;
        let r2 = c.put_sweeps(run_id, &sweeps).await;
        let r3 = c.put_judgements(run_id, &jis).await;
        let check = c.intake_check(run_id).await;
        let _ = child.kill();
        let _ = child.wait();
        r1.expect("条目登记");
        r2.expect("采集轮登记");
        r3.expect("判断台账登记");
        let check = check.expect("自查");
        let red: Vec<_> = check
            .checks
            .iter()
            .filter(|x| !x.ok && !x.skipped)
            .map(|x| format!("{}：{}", x.name, x.detail))
            .collect();
        println!("自查：{} 条判据，红 {}", check.checks.len(), red.len());
        for r in &red {
            println!("  红：{r}");
        }
    }
}
