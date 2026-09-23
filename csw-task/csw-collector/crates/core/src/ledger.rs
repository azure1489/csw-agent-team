//! 候选与判断的本地台账。
//!
//! # 候选是跨轮共享的
//!
//! 同一条贴文昨天判过、今天还在窗口里，`candidates` 只存一份；
//! 「这一轮取到了它」记在 `round_candidates`。这样「昨日已判结转」
//! 才有地方标（`carried`），也才能回答「这条是哪个采集器取到的」。
//!
//! # 判断一轮一份
//!
//! `UNIQUE(round_id, candidate_key)`：同一轮里一条候选只有一个结论。
//! 重判会覆盖——**但库上有个 CHECK 挡着**：`image_seen=0` 时 `tier` 必须是
//! `pending_check`。这条口径不靠代码自觉，靠库。人工改档走
//! `judgement_overrides`，原判不动。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::types::{
    Candidate, Comparison, ComparisonVerdict, Judgement, MediaKind, ThreeSentences, Tier,
    Unanswered,
};

/// 写候选。**按 `candidate_key` 覆盖元数据，但 `first_seen_at` 只写第一次**——
/// 它是「这条第一次进我们视野」的时间，被后来的同步改掉就没意义了。
pub fn upsert_candidate(conn: &Connection, c: &Candidate) -> Result<()> {
    let now = jiff::Timestamp::now().to_string();
    conn.execute(
        "INSERT INTO candidates(candidate_key, platform, source_id, account, url, text, translated,
                                posted_at, ingested_at, likes, comments, followers, heat_ratio,
                                content_type, image_only, tags_json, hashtags_json,
                                first_seen_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18)
         ON CONFLICT(candidate_key) DO UPDATE SET
           account=excluded.account, url=excluded.url, text=excluded.text,
           translated=excluded.translated, posted_at=excluded.posted_at,
           ingested_at=excluded.ingested_at, likes=excluded.likes, comments=excluded.comments,
           followers=excluded.followers, heat_ratio=excluded.heat_ratio,
           content_type=excluded.content_type, image_only=excluded.image_only,
           tags_json=excluded.tags_json, hashtags_json=excluded.hashtags_json,
           updated_at=excluded.updated_at",
        params![
            c.candidate_key,
            serde_json::to_value(c.platform)?.as_str().unwrap_or(""),
            c.source_id,
            c.account,
            c.url,
            c.text,
            c.translated,
            c.posted_at.map(|t| t.to_string()),
            c.ingested_at.map(|t| t.to_string()),
            c.likes,
            c.comments,
            c.followers,
            c.heat_ratio,
            c.content_type,
            i64::from(c.is_image_only()),
            serde_json::to_string(&c.tags)?,
            serde_json::to_string(&c.hashtags)?,
            now,
        ],
    )
    .context("写候选")?;
    Ok(())
}

/// 把候选挂到这一轮。`carried` = 昨天已判、今天仍在窗口内。
pub fn attach_candidate(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
    collector: &str,
    carried: bool,
) -> Result<()> {
    conn.execute(
        "INSERT INTO round_candidates(round_id, candidate_key, collector, in_window, carried)
         VALUES (?1,?2,?3,1,?4)
         ON CONFLICT(round_id, candidate_key) DO UPDATE SET
           collector=excluded.collector, carried=excluded.carried",
        params![round_id, candidate_key, collector, i64::from(carried)],
    )?;
    Ok(())
}

/// 这一轮取到的候选数（含结转）。**采集轮计数由代码算，不信接口给的数。**
pub fn round_candidate_count(conn: &Connection, round_id: i64) -> Result<(usize, usize)> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM round_candidates WHERE round_id=?1",
        [round_id],
        |r| r.get(0),
    )?;
    let carried: i64 = conn.query_row(
        "SELECT COUNT(*) FROM round_candidates WHERE round_id=?1 AND carried=1",
        [round_id],
        |r| r.get(0),
    )?;
    Ok((total as usize, carried as usize))
}

pub fn get_candidate(conn: &Connection, key: &str) -> Result<Option<Candidate>> {
    Ok(conn
        .query_row(
            "SELECT platform, source_id, account, url, text, translated, posted_at, ingested_at,
                    likes, comments, followers, heat_ratio, content_type, tags_json, hashtags_json
             FROM candidates WHERE candidate_key = ?1",
            [key],
            |r| {
                let tags: Vec<String> =
                    serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default();
                let hashtags: Vec<String> =
                    serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or_default();
                Ok(Candidate {
                    candidate_key: key.to_string(),
                    // 库里的值来自我们自己写的 Display，认不出来只可能是库被手改过
                    platform: serde_json::from_value(serde_json::Value::String(r.get(0)?))
                        .unwrap_or(crate::types::Platform::Web),
                    source_id: r.get(1)?,
                    collector: String::new(),
                    account: r.get(2)?,
                    url: r.get(3)?,
                    text: r.get(4)?,
                    translated: r.get(5)?,
                    posted_at: r.get::<_, Option<String>>(6)?.and_then(|s| s.parse().ok()),
                    ingested_at: r.get::<_, Option<String>>(7)?.and_then(|s| s.parse().ok()),
                    likes: r.get(8)?,
                    comments: r.get(9)?,
                    followers: r.get(10)?,
                    heat_ratio: r.get(11)?,
                    content_type: r.get(12)?,
                    media: vec![],
                    tags,
                    hashtags,
                })
            },
        )
        .optional()?)
}

// ─────────────────────────────── 判断 ───────────────────────────────

/// 写判断。**落库前过一遍契约自检**——六维齐全、每维有依据、
/// 没读到实图必须落待核、必须有 inputs_hash。
///
/// 库上还有一条 CHECK 兜底。两道都要：自检给出人能看懂的原因，
/// CHECK 挡住任何绕过自检的写法。
pub fn put_judgement(
    conn: &Connection,
    round_id: i64,
    j: &Judgement,
    check_flags: &[String],
    model: &str,
    rubric_version: &str,
) -> Result<i64> {
    let v = j.violations();
    anyhow::ensure!(
        v.is_empty(),
        "{} 的判断不合契约：{}",
        j.candidate_key,
        v.join("；")
    );
    conn.execute(
        "INSERT INTO judgements(round_id, candidate_key, tier, dims_json, three_json, unanswered,
                                comparison_json, heat_note, look, image_seen, gaps_json,
                                priority_hits_json, lower_hits_json, jev_disagreement,
                                check_flags_json, kb_refs_json, memory_refs_json,
                                inputs_hash, model, rubric_version, created_at,
                                headline, novelty_json, readiness_json, rejudged)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,
                 ?22,?23,?24,?25)
         ON CONFLICT(round_id, candidate_key) DO UPDATE SET
           tier=excluded.tier, dims_json=excluded.dims_json, three_json=excluded.three_json,
           unanswered=excluded.unanswered, comparison_json=excluded.comparison_json,
           heat_note=excluded.heat_note, look=excluded.look, image_seen=excluded.image_seen,
           gaps_json=excluded.gaps_json, priority_hits_json=excluded.priority_hits_json,
           lower_hits_json=excluded.lower_hits_json, jev_disagreement=excluded.jev_disagreement,
           check_flags_json=excluded.check_flags_json, kb_refs_json=excluded.kb_refs_json,
           memory_refs_json=excluded.memory_refs_json, inputs_hash=excluded.inputs_hash,
           model=excluded.model, rubric_version=excluded.rubric_version,
           headline=excluded.headline, novelty_json=excluded.novelty_json,
           readiness_json=excluded.readiness_json, rejudged=excluded.rejudged",
        params![
            round_id,
            j.candidate_key,
            tier_str(j.tier),
            serde_json::to_string(&j.dims)?,
            serde_json::to_string(&j.three_sentences)?,
            serde_json::to_value(j.unanswered)?
                .as_str()
                .unwrap_or("none"),
            serde_json::to_string(&j.comparison)?,
            j.heat_note,
            j.look,
            i64::from(j.image_seen),
            serde_json::to_string(&j.gaps)?,
            serde_json::to_string(&j.priority_hits)?,
            serde_json::to_string(&j.lower_hits)?,
            j.jev_disagreement,
            serde_json::to_string(check_flags)?,
            serde_json::to_string(&j.kb_refs)?,
            serde_json::to_string(&j.memory_refs)?,
            j.inputs_hash,
            model,
            rubric_version,
            jiff::Timestamp::now().to_string(),
            j.headline,
            serde_json::to_string(&j.novelty)?,
            serde_json::to_string(&j.readiness)?,
            i64::from(check_flags.iter().any(|f| f.starts_with(REJUDGED_FLAG))),
        ],
    )
    .context("写判断")?;
    Ok(conn.last_insert_rowid())
}

/// 口径违例后重判过的条目，在 check_flags 里以它开头留痕。
pub const REJUDGED_FLAG: &str = "【口径·已重判】";

/// 这一轮各档各多少条。给自查与总览用。
pub fn tier_counts(conn: &Connection, round_id: i64) -> Result<Vec<(String, usize)>> {
    let mut st = conn.prepare(
        "SELECT tier, COUNT(*) FROM judgements WHERE round_id=?1 GROUP BY tier ORDER BY tier",
    )?;
    Ok(st
        .query_map([round_id], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .filter_map(Result::ok)
        .collect())
}

/// 已判的条目键。用来算「窗口内去重候选数 = 台账行数」这条对账。
pub fn judged_keys(conn: &Connection, round_id: i64) -> Result<Vec<String>> {
    let mut st = conn
        .prepare("SELECT candidate_key FROM judgements WHERE round_id=?1 ORDER BY candidate_key")?;
    Ok(st
        .query_map([round_id], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect())
}

/// 这一轮里**没判**的候选。intake-check 的「每条都判」靠它。
pub fn unjudged_keys(conn: &Connection, round_id: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT rc.candidate_key FROM round_candidates rc
         LEFT JOIN judgements j ON j.round_id = rc.round_id AND j.candidate_key = rc.candidate_key
         WHERE rc.round_id = ?1 AND j.id IS NULL
         ORDER BY rc.candidate_key",
    )?;
    Ok(st
        .query_map([round_id], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect())
}

/// 按输入指纹找一条能直接拿来用的旧结论。**跨轮找**——预取轮判过的，
/// 正式轮就是靠这个把判断那一段省下来的。
///
/// 指纹一致意味着正文、图片描述、对照材料、准则版本、提示词版本、作业标准
/// 全都没变。任何一样变了指纹就变，这里自然就找不到——
/// **「换了东西还在用旧结论」这件事没有别的地方能挡住。**
///
/// 找不到不是错误，是「这条得重新判」。
pub fn judgement_by_hash(
    conn: &Connection,
    candidate_key: &str,
    inputs_hash: &str,
) -> Result<Option<Judgement>> {
    if inputs_hash.is_empty() {
        // 空指纹表示「这条没记输入」，不是「输入一样」
        return Ok(None);
    }
    Ok(conn
        .query_row(
            "SELECT tier, dims_json, three_json, unanswered, comparison_json, heat_note, look,
                    image_seen, gaps_json, priority_hits_json, lower_hits_json, jev_disagreement,
                    kb_refs_json, memory_refs_json, headline, novelty_json, readiness_json
             FROM judgements WHERE candidate_key = ?1 AND inputs_hash = ?2
             ORDER BY round_id DESC LIMIT 1",
            params![candidate_key, inputs_hash],
            |r| {
                let js = |i: usize| -> Vec<String> {
                    r.get::<_, String>(i)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default()
                };
                fn parse<T: serde::de::DeserializeOwned + Default>(s: &str) -> T {
                    serde_json::from_str(s).unwrap_or_default()
                }
                // 每个字段的目标类型不同，闭包只能定型一次，所以逐个写开
                let raw = |i: usize| r.get::<_, String>(i).unwrap_or_default();
                Ok(Judgement {
                    candidate_key: candidate_key.to_string(),
                    tier: serde_json::from_value(serde_json::Value::String(r.get(0)?))
                        .unwrap_or(Tier::PendingCheck),
                    dims: serde_json::from_str(&raw(1)).unwrap_or_default(),
                    headline: r.get(14)?,
                    three_sentences: parse::<ThreeSentences>(&raw(2)),
                    novelty: parse(&raw(15)),
                    readiness: parse(&raw(16)),
                    unanswered: serde_json::from_value(serde_json::Value::String(r.get(3)?))
                        .unwrap_or(Unanswered::None),
                    comparison: serde_json::from_str(&raw(4)).unwrap_or(Comparison {
                        verdict: ComparisonVerdict::Unrelated,
                        against: String::new(),
                        note: String::new(),
                        hits: vec![],
                    }),
                    heat_note: r.get(5)?,
                    look: r.get(6)?,
                    image_seen: r.get::<_, i64>(7)? != 0,
                    gaps: parse(&raw(8)),
                    priority_hits: js(9),
                    lower_hits: js(10),
                    jev_disagreement: r.get(11)?,
                    kb_refs: js(12),
                    memory_refs: js(13),
                    inputs_hash: inputs_hash.to_string(),
                })
            },
        )
        .optional()?)
}

/// 上一轮台账：第五类对照材料。**留在本地库，不进参考库**——
/// 否则系统自己的判断会被当成 Van 的口味证据。
///
/// **预取轮的结论不算数。** 它是同一天凌晨为这一轮预先算的，不是「上一轮」；
/// 当成对照材料等于让这一轮拿自己几小时前的判断给自己作证。
pub fn prior_ledger(
    conn: &Connection,
    before_round_id: i64,
    limit: usize,
) -> Result<Vec<(String, String, String)>> {
    let mut st = conn.prepare(
        "SELECT j.candidate_key, j.tier, j.created_at FROM judgements j
         JOIN rounds r ON r.id = j.round_id
         WHERE j.round_id < ?1 AND r.kind <> 'prefetch'
         ORDER BY j.round_id DESC, j.candidate_key LIMIT ?2",
    )?;
    Ok(st
        .query_map(params![before_round_id, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .filter_map(Result::ok)
        .collect())
}

/// 同一条候选在之前几轮（不含预取轮）的结论：第五类对照材料「上一轮台账」。
///
/// 返回 `(档, 判断时间, 标题)`，新的在前。
pub fn prior_for(
    conn: &Connection,
    before_round_id: i64,
    candidate_key: &str,
    limit: usize,
) -> Result<Vec<(String, String, String)>> {
    let mut st = conn.prepare(
        "SELECT j.tier, j.created_at, j.headline FROM judgements j
         JOIN rounds r ON r.id = j.round_id
         WHERE j.round_id < ?1 AND r.kind <> 'prefetch' AND j.candidate_key = ?2
         ORDER BY j.round_id DESC LIMIT ?3",
    )?;
    Ok(st
        .query_map(params![before_round_id, candidate_key, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .filter_map(Result::ok)
        .collect())
}

/// 一条判断的**有效档**：有人工改档就取最新一次改档，没有就取模型给的。
/// 各处统计与待核结转都要看它——只看原始档，改过档的还会挂在待核里。
pub const EFFECTIVE_TIER_SQL: &str = "COALESCE((SELECT o.to_tier FROM judgement_overrides o
     WHERE o.round_id = j.round_id AND o.candidate_key = j.candidate_key
     ORDER BY o.id DESC LIMIT 1), j.tier)";

/// 这一轮按**有效档**各多少条。
pub fn effective_tier_counts(conn: &Connection, round_id: i64) -> Result<Vec<(String, usize)>> {
    let sql = format!(
        "SELECT t, COUNT(*) FROM (SELECT {EFFECTIVE_TIER_SQL} AS t FROM judgements j
         WHERE j.round_id=?1) GROUP BY t ORDER BY t"
    );
    let mut st = conn.prepare(&sql)?;
    Ok(st
        .query_map([round_id], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .filter_map(Result::ok)
        .collect())
}

/// 待核且还没结清的条目，跨轮。01 的任务详情要附这个。
///
/// 看**最新一轮**的**有效档**：人工把待核改成别的档，就算结了。
pub fn open_pending_checks(conn: &Connection, limit: usize) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT j.candidate_key FROM judgements j
         WHERE j.round_id = (SELECT MAX(j2.round_id) FROM judgements j2
                             WHERE j2.candidate_key = j.candidate_key)
           AND {EFFECTIVE_TIER_SQL} = 'pending_check'
         ORDER BY j.round_id DESC, j.candidate_key LIMIT ?1"
    );
    let mut st = conn.prepare(&sql)?;
    Ok(st
        .query_map([limit as i64], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect())
}

fn tier_str(t: Tier) -> &'static str {
    match t {
        Tier::Recommend => "recommend",
        Tier::Alternate => "alternate",
        Tier::NotRecommend => "not_recommend",
        Tier::PendingCheck => "pending_check",
    }
}

/// 一条候选有几张图。写 `media` 表之后用它核对。
pub fn photo_count(c: &Candidate) -> usize {
    c.media
        .iter()
        .filter(|m| m.kind == MediaKind::Photo)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rounds::{NewRound, open_round};
    use crate::types::{Dim, Gap, MediaRef, Platform, RoundKind, RoundTrigger};

    fn setup(task_id: i64) -> (Connection, i64) {
        let c = crate::store::open_in_memory().unwrap();
        let id = open_one(&c, task_id);
        (c, id)
    }

    fn open_one(c: &Connection, task_id: i64) -> i64 {
        open_round(
            c,
            &NewRound {
                kind: RoundKind::Task,
                trigger: RoundTrigger::Dispatch,
                run_id: Some(48),
                task_id: Some(task_id),
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
        .unwrap()
        .0
        .id
    }

    fn cand(key: &str, imgs: usize) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw-window".into(),
            account: "yamatomichi".into(),
            url: format!("https://x/{key}"),
            text: "正文".into(),
            translated: String::new(),
            posted_at: "2026-09-17T08:00:00Z".parse().ok(),
            ingested_at: "2026-09-18T01:00:00Z".parse().ok(),
            likes: Some(369),
            comments: Some(12),
            followers: Some(90000),
            heat_ratio: Some(5.2),
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
            tags: vec!["新品".into()],
            hashtags: vec![],
        }
    }

    fn judgement(key: &str, tier: Tier, image_seen: bool) -> Judgement {
        let mut j = Judgement::fixture(key, tier);
        j.heat_note = "369 赞".into();
        j.image_seen = image_seen;
        j.inputs_hash = "ih".into();
        j
    }

    #[test]
    fn 合成的周一四百五十条落库与对账都还算得动() {
        const N: usize = 450;
        let (c, r) = setup(1);
        let t0 = std::time::Instant::now();
        for i in 0..N {
            let key = format!("k{i:03}");
            upsert_candidate(&c, &cand(&key, 6)).unwrap();
            attach_candidate(&c, r, &key, "csw-window", i % 9 == 0).unwrap();
            let tier = match i % 4 {
                0 => Tier::Recommend,
                1 => Tier::Alternate,
                2 => Tier::PendingCheck,
                _ => Tier::NotRecommend,
            };
            put_judgement(&c, r, &judgement(&key, tier, true), &[], "m", "v1").unwrap();
        }
        let write = t0.elapsed();

        let t1 = std::time::Instant::now();
        let (total, carried) = round_candidate_count(&c, r).unwrap();
        let judged = judged_keys(&c, r).unwrap().len();
        let unjudged = unjudged_keys(&c, r).unwrap().len();
        let tiers = tier_counts(&c, r).unwrap();
        let read = t1.elapsed();

        assert_eq!(
            (total, judged, unjudged),
            (N, N, 0),
            "每条都判，未判必须是 0"
        );
        assert_eq!(carried, 50);
        assert_eq!(tiers.len(), 4, "四档都要出现");
        // 对账那几条查询是自查每一轮都要跑的，不该随条数变慢到离谱。
        // 门槛放得很松（debug 构建、机器忙时也要过），它挡的是
        // 「不小心写出一条 N 次查询」那种数量级的退化
        assert!(write.as_secs() < 20, "写四百五十条用了 {write:?}");
        assert!(read.as_millis() < 2000, "对账用了 {read:?}");

        // 待核跨轮那条查询是整库扫的，也一起量一下
        let t2 = std::time::Instant::now();
        let open = open_pending_checks(&c, 100).unwrap();
        assert_eq!(open.len(), 100);
        assert!(
            t2.elapsed().as_millis() < 2000,
            "待核结转用了 {:?}",
            t2.elapsed()
        );
    }

    #[test]
    fn 指纹一致的旧结论跨轮拿得回来() {
        let (c, r1) = setup(1);
        upsert_candidate(&c, &cand("k1", 3)).unwrap();
        attach_candidate(&c, r1, "k1", "csw-window", false).unwrap();
        let mut j = judgement("k1", Tier::Recommend, true);
        j.gaps = vec!["价格未写".into()];
        j.priority_hits = vec!["新品发布".into()];
        j.kb_refs = vec!["kb:published_item:42".into()];
        put_judgement(
            &c,
            r1,
            &j,
            &["依据 3 材料不支持".into()],
            "m",
            "van-rubric/v1",
        )
        .unwrap();

        // 正式轮拿预取轮判过的：档、六维、三句话、缺口一样不少
        let got = judgement_by_hash(&c, "k1", "ih")
            .unwrap()
            .expect("该找得到");
        assert_eq!(got.tier, Tier::Recommend);
        assert_eq!(got.dims.len(), Dim::ALL.len());
        assert_eq!(got.three_sentences.what, "甲");
        assert_eq!(got.gaps, [Gap::decision("价格未写")]);
        assert_eq!(got.priority_hits, ["新品发布"]);
        assert_eq!(got.kb_refs, ["kb:published_item:42"]);
        assert_eq!(got.heat_note, "369 赞");
        assert!(got.image_seen);
        assert_eq!(got.inputs_hash, "ih");
        // 拿回来的必须还能过契约自检，否则等于把脏数据搬进新一轮
        assert!(got.violations().is_empty(), "{:?}", got.violations());
    }

    #[test]
    fn 指纹不一致就不给旧结论() {
        let (c, r1) = setup(1);
        upsert_candidate(&c, &cand("k1", 3)).unwrap();
        attach_candidate(&c, r1, "k1", "csw-window", false).unwrap();
        put_judgement(
            &c,
            r1,
            &judgement("k1", Tier::Recommend, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();
        // 换了正文、换了描述、换了作业标准，指纹都会变——旧结论不能再用
        assert!(judgement_by_hash(&c, "k1", "别的指纹").unwrap().is_none());
        // 空指纹表示「这条没记输入」，不是「输入一样」
        assert!(judgement_by_hash(&c, "k1", "").unwrap().is_none());
        // 没判过的当然没有
        assert!(judgement_by_hash(&c, "k2", "ih").unwrap().is_none());
    }

    #[test]
    fn 候选跨轮共享首次见到的时间只写一次() {
        let (c, r1) = setup(1);
        upsert_candidate(&c, &cand("k1", 3)).unwrap();
        let first: String = c
            .query_row(
                "SELECT first_seen_at FROM candidates WHERE candidate_key='k1'",
                [],
                |x| x.get(0),
            )
            .unwrap();

        // 第二轮又取到它：元数据更新，但「第一次进我们视野」的时间不动
        let mut again = cand("k1", 3);
        again.likes = Some(999);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        upsert_candidate(&c, &again).unwrap();
        let (now_first, likes): (String, i64) = c
            .query_row(
                "SELECT first_seen_at, likes FROM candidates WHERE candidate_key='k1'",
                [],
                |x| Ok((x.get(0)?, x.get(1)?)),
            )
            .unwrap();
        assert_eq!(now_first, first, "被后来的同步改掉就没意义了");
        assert_eq!(likes, 999);

        let r2 = open_one(&c, 2);
        attach_candidate(&c, r1, "k1", "csw-window", false).unwrap();
        attach_candidate(&c, r2, "k1", "csw-window", true).unwrap();
        // 一条候选，两轮各挂一次
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM candidates", [], |x| x.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(round_candidate_count(&c, r1).unwrap(), (1, 0));
        assert_eq!(
            round_candidate_count(&c, r2).unwrap(),
            (1, 1),
            "第二轮是结转"
        );
    }

    #[test]
    fn 只图文的判据存进库() {
        let (c, _) = setup(1);
        let mut mixed = cand("k2", 2);
        mixed.media[1].kind = MediaKind::Video;
        upsert_candidate(&c, &mixed).unwrap();
        let only: i64 = c
            .query_row(
                "SELECT image_only FROM candidates WHERE candidate_key='k2'",
                [],
                |x| x.get(0),
            )
            .unwrap();
        assert_eq!(only, 0, "媒体里有视频就不是只图文");
        assert_eq!(photo_count(&mixed), 1);
    }

    #[test]
    fn 判断落库前过契约自检() {
        let (c, r) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        let mut bad = judgement("k1", Tier::Recommend, true);
        bad.dims.pop(); // 少一维
        let err = put_judgement(&c, r, &bad, &[], "m", "v1")
            .unwrap_err()
            .to_string();
        assert!(err.contains("缺维度"), "{err}");

        let mut no_hash = judgement("k1", Tier::Recommend, true);
        no_hash.inputs_hash = String::new();
        assert!(put_judgement(&c, r, &no_hash, &[], "m", "v1").is_err());
    }

    #[test]
    fn 没读到实图却不是待核库会挡住() {
        let (c, r) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        // 自检先拦一道，给出人能看懂的原因
        let j = judgement("k1", Tier::Recommend, false);
        assert!(put_judgement(&c, r, &j, &[], "m", "v1").is_err());

        // 库上的 CHECK 是第二道：任何绕过自检的写法都过不去
        let e = c.execute(
            "INSERT INTO judgements(round_id, candidate_key, tier, dims_json, three_json,
                                    comparison_json, image_seen, inputs_hash, model,
                                    rubric_version, created_at)
             VALUES (?1,'k1','recommend','[]','{}','{}',0,'ih','m','v1','t')",
            [r],
        );
        assert!(e.is_err(), "库层没挡住");
    }

    #[test]
    fn 同一轮一条候选只有一个结论重判覆盖() {
        let (c, r) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        put_judgement(
            &c,
            r,
            &judgement("k1", Tier::Recommend, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();
        put_judgement(
            &c,
            r,
            &judgement("k1", Tier::Alternate, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();
        assert_eq!(tier_counts(&c, r).unwrap(), [("alternate".to_string(), 1)]);
    }

    #[test]
    fn 没判的挑出来() {
        let (c, r) = setup(1);
        for k in ["k1", "k2", "k3"] {
            upsert_candidate(&c, &cand(k, 1)).unwrap();
            attach_candidate(&c, r, k, "csw-window", false).unwrap();
        }
        put_judgement(
            &c,
            r,
            &judgement("k2", Tier::Recommend, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();
        // intake-check 的「每条都判」靠这个
        assert_eq!(unjudged_keys(&c, r).unwrap(), ["k1", "k3"]);
        assert_eq!(judged_keys(&c, r).unwrap(), ["k2"]);
    }

    #[test]
    fn 核对标记跟着判断一起存() {
        let (c, r) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        put_judgement(
            &c,
            r,
            &judgement("k1", Tier::Recommend, true),
            &["use 的依据在材料里核不到（0.02）".to_string()],
            "m",
            "v1",
        )
        .unwrap();
        let flags: String = c
            .query_row(
                "SELECT check_flags_json FROM judgements WHERE round_id=?1",
                [r],
                |x| x.get(0),
            )
            .unwrap();
        assert!(flags.contains("核不到"), "{flags}");
    }

    #[test]
    fn 上一轮台账只看更早的轮次() {
        let (c, r1) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        put_judgement(
            &c,
            r1,
            &judgement("k1", Tier::Recommend, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();

        let r2 = open_one(&c, 2);
        upsert_candidate(&c, &cand("k2", 1)).unwrap();
        put_judgement(
            &c,
            r2,
            &judgement("k2", Tier::Alternate, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();

        let prior = prior_ledger(&c, r2, 10).unwrap();
        assert_eq!(prior.len(), 1, "只看更早的轮次");
        assert_eq!(prior[0].0, "k1");
        assert_eq!(prior[0].1, "recommend");
    }

    #[test]
    fn 预取轮的结论不当对照材料() {
        let c = crate::store::open_in_memory().unwrap();
        let pre = open_round(
            &c,
            &NewRound {
                kind: RoundKind::Prefetch,
                trigger: RoundTrigger::Manual,
                run_id: None,
                task_id: None,
                stage_code: Some("intake".into()),
                target_version: 0,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: String::new(),
            },
        )
        .unwrap()
        .0
        .id;
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        put_judgement(
            &c,
            pre,
            &judgement("k1", Tier::Recommend, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();

        // 预取轮是同一天凌晨为这一轮预先算的，拿它作证等于自己给自己作证
        let formal = open_one(&c, 9);
        assert!(prior_ledger(&c, formal, 10).unwrap().is_empty());
    }

    #[test]
    fn 待核跨轮结清了就不再出现() {
        let (c, r1) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        put_judgement(
            &c,
            r1,
            &judgement("k1", Tier::PendingCheck, false),
            &[],
            "m",
            "v1",
        )
        .unwrap();
        assert_eq!(open_pending_checks(&c, 10).unwrap(), ["k1"]);

        // 下一轮补齐了图，判成推荐 → 不再是未结的待核
        let r2 = open_one(&c, 2);
        put_judgement(
            &c,
            r2,
            &judgement("k1", Tier::Recommend, true),
            &[],
            "m",
            "v1",
        )
        .unwrap();
        assert!(open_pending_checks(&c, 10).unwrap().is_empty());
    }

    #[test]
    fn 候选能原样读回来() {
        let (c, _) = setup(1);
        upsert_candidate(&c, &cand("k1", 2)).unwrap();
        let got = get_candidate(&c, "k1").unwrap().unwrap();
        assert_eq!(got.account, "yamatomichi");
        assert_eq!(got.heat_ratio, Some(5.2));
        assert_eq!(got.tags, ["新品"]);
        assert_eq!(
            got.posted_at.map(|t| t.to_string()).as_deref(),
            Some("2026-09-17T08:00:00Z")
        );
        assert!(get_candidate(&c, "没这条").unwrap().is_none());
    }

    #[test]
    fn 人工改档后不再挂在待核结转里() {
        let (c, r) = setup(1);
        upsert_candidate(&c, &cand("k1", 1)).unwrap();
        upsert_candidate(&c, &cand("k2", 1)).unwrap();
        put_judgement(
            &c,
            r,
            &judgement("k1", Tier::PendingCheck, true),
            &[],
            "m",
            "v",
        )
        .unwrap();
        put_judgement(
            &c,
            r,
            &judgement("k2", Tier::PendingCheck, true),
            &[],
            "m",
            "v",
        )
        .unwrap();
        crate::workbench::put_override(&c, r, "k1", Tier::Alternate, "正文已说明", "主编").unwrap();
        assert_eq!(open_pending_checks(&c, 10).unwrap(), ["k2"]);
        let counts = effective_tier_counts(&c, r).unwrap();
        assert!(counts.contains(&("alternate".into(), 1)));
        assert!(counts.contains(&("pending_check".into(), 1)));
    }
}
