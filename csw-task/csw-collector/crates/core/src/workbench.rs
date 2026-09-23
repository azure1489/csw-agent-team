//! 工作台写进来的东西：改档、指定首批、Van 勾选、留痕、排队。
//!
//! # 一条贯穿全文的规矩：人的动作不覆盖模型的原判
//!
//! 改档另存一行（`judgement_overrides`），`judgements` 那一行一个字不动。
//! 台账上两者都要看得见——「模型判了不推荐、主编捞回成备选」与
//! 「模型判了备选」是两件不同的事，混成一个字段就再也分不出来了。
//!
//! # 排队：HTTP 不干活
//!
//! 手动开轮、重跑，都只往 `work_queue` 写一行就返回。真正干活的是常驻循环：
//! 它持有模型与向量客户端、持有那个唯一的写连接。一个 HTTP 请求里跑四十分钟
//! 的活，除了超时什么也得不到。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::types::Tier;

// ─────────────────────────────── 改档 ───────────────────────────────

/// 人工改档。**另存一行，原判不动。**
///
/// 返回这一行的 id。`from_tier` 由代码读**当前生效的那一档**，不信调用方传来的：
/// 页面上看到的可能已经是几分钟前的了，而改档是从「现在显示的那一档」往别处改。
pub fn put_override(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
    to: Tier,
    reason: &str,
    actor: &str,
) -> Result<i64> {
    anyhow::ensure!(!reason.trim().is_empty(), "改档必须写理由");
    let from =
        effective_tier(conn, round_id, candidate_key)?.context("这一轮没有这条的判断，改不了档")?;
    let to_s = tier_str(to);
    anyhow::ensure!(from != to_s, "本来就是{to_s}，不用改");
    conn.execute(
        "INSERT INTO judgement_overrides(round_id, candidate_key, from_tier, to_tier,
                                         reason, actor, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            round_id,
            candidate_key,
            from,
            to_s,
            reason.trim(),
            actor,
            jiff::Timestamp::now().to_string()
        ],
    )
    .context("写改档")?;
    Ok(conn.last_insert_rowid())
}

/// 这一条现在显示的是哪一档：有改档就是最后一次改到的那一档，否则是原判。
///
/// 台账、交付物、深核挑首批都该看它，而不是只看 `judgements.tier`——
/// 主编把一条捞回成备选之后，系统还按不推荐对待的话，那次捞回等于没发生。
pub fn effective_tier(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
) -> Result<Option<String>> {
    let judged: Option<String> = conn
        .query_row(
            "SELECT tier FROM judgements WHERE round_id=?1 AND candidate_key=?2",
            params![round_id, candidate_key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(judged) = judged else {
        return Ok(None);
    };
    let overridden: Option<String> = conn
        .query_row(
            "SELECT to_tier FROM judgement_overrides
             WHERE round_id=?1 AND candidate_key=?2 ORDER BY id DESC LIMIT 1",
            params![round_id, candidate_key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Some(overridden.unwrap_or(judged)))
}

/// 这一轮的全部改档，按条目键取最近一次。
///
/// 改了又改是常事（先捞回成备选、再压回不推荐），**只有最后一次算数**，
/// 但早先那几次留在表里，台账要能把整条来龙去脉摊开。
pub fn latest_overrides(conn: &Connection, round_id: i64) -> Result<Vec<Override>> {
    let mut st = conn.prepare(
        "SELECT candidate_key, from_tier, to_tier, reason, actor, created_at
         FROM judgement_overrides o
         WHERE round_id = ?1
           AND id = (SELECT MAX(id) FROM judgement_overrides
                     WHERE round_id = o.round_id AND candidate_key = o.candidate_key)
         ORDER BY candidate_key",
    )?;
    let rows = st.query_map([round_id], |r| {
        Ok(Override {
            candidate_key: r.get(0)?,
            from_tier: r.get(1)?,
            to_tier: r.get(2)?,
            reason: r.get(3)?,
            actor: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Override {
    pub candidate_key: String,
    pub from_tier: String,
    pub to_tier: String,
    pub reason: String,
    pub actor: String,
    pub created_at: String,
}

// ───────────────────────────── 指定首批 ─────────────────────────────

/// 主编指定的深核首批。传空清空指定，退回自动挑法。
///
/// 返回真的标上了几条：**不在这一轮里的条目键直接忽略**，
/// 而不是报错——页面上那份列表可能来自一轮已经翻篇的台账。
pub fn set_first_batch(conn: &Connection, round_id: i64, keys: &[String]) -> Result<usize> {
    conn.execute(
        "UPDATE round_candidates SET first_batch = 0 WHERE round_id = ?1",
        [round_id],
    )?;
    let mut n = 0;
    for k in keys {
        n += conn.execute(
            "UPDATE round_candidates SET first_batch = 1
             WHERE round_id = ?1 AND candidate_key = ?2",
            params![round_id, k],
        )?;
    }
    Ok(n)
}

/// 这一轮被指定进首批的条目键。
pub fn first_batch(conn: &Connection, round_id: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT candidate_key FROM round_candidates
         WHERE round_id = ?1 AND first_batch = 1 ORDER BY candidate_key",
    )?;
    Ok(st
        .query_map([round_id], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect())
}

// ──────────────────────────── Van 的勾选 ────────────────────────────

/// Van 在工作台上勾一下。**只写本地，不回写引擎**——
/// 进不进评选由主编代录，那是流程里人的决定，不是这里能替她做的。
pub fn put_van_mark(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
    mark: &str,
    note: &str,
    actor: &str,
) -> Result<()> {
    anyhow::ensure!(
        matches!(mark, "like" | "doubt" | "note"),
        "勾选只有 like / doubt / note 三种，收到 {mark}"
    );
    // note 这一种没有原话就等于什么也没说
    anyhow::ensure!(mark != "note" || !note.trim().is_empty(), "写备注要有内容");
    conn.execute(
        "INSERT INTO van_marks(round_id, candidate_key, mark, note, actor, created_at)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(round_id, candidate_key, mark, actor) DO UPDATE SET
           note = excluded.note, created_at = excluded.created_at",
        params![
            round_id,
            candidate_key,
            mark,
            note.trim(),
            actor,
            jiff::Timestamp::now().to_string()
        ],
    )
    .context("写勾选")?;
    Ok(())
}

/// 撤掉一个勾选。返回撤掉了几条。
pub fn drop_van_mark(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
    mark: &str,
    actor: &str,
) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM van_marks
         WHERE round_id=?1 AND candidate_key=?2 AND mark=?3 AND actor=?4",
        params![round_id, candidate_key, mark, actor],
    )?)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VanMark {
    pub candidate_key: String,
    pub mark: String,
    pub note: String,
    pub actor: String,
    pub created_at: String,
}

pub fn van_marks(conn: &Connection, round_id: i64) -> Result<Vec<VanMark>> {
    let mut st = conn.prepare(
        "SELECT candidate_key, mark, note, actor, created_at FROM van_marks
         WHERE round_id = ?1 ORDER BY candidate_key, mark",
    )?;
    let rows = st.query_map([round_id], |r| {
        Ok(VanMark {
            candidate_key: r.get(0)?,
            mark: r.get(1)?,
            note: r.get(2)?,
            actor: r.get(3)?,
            created_at: r.get(4)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

// ─────────────────────────────── 留痕 ───────────────────────────────

/// 谁在什么时候改了什么。**每一个写操作都要留一行。**
pub fn audit(
    conn: &Connection,
    actor: &str,
    action: &str,
    target: &str,
    detail: &serde_json::Value,
) -> Result<()> {
    conn.execute(
        "INSERT INTO audit(actor, action, target, detail_json, created_at)
         VALUES (?1,?2,?3,?4,?5)",
        params![
            actor,
            action,
            target,
            detail.to_string(),
            jiff::Timestamp::now().to_string()
        ],
    )
    .context("写留痕")?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail_json: String,
    pub created_at: String,
}

pub fn recent_audit(conn: &Connection, limit: usize) -> Result<Vec<AuditRow>> {
    let mut st = conn.prepare(
        "SELECT id, actor, action, target, detail_json, created_at
         FROM audit ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        Ok(AuditRow {
            id: r.get(0)?,
            actor: r.get(1)?,
            action: r.get(2)?,
            target: r.get(3)?,
            detail_json: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

// ─────────────────────────────── 排队 ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkItem {
    pub id: i64,
    pub kind: String,
    pub round_id: Option<i64>,
    pub payload_json: String,
    pub actor: String,
    pub status: String,
    pub note: String,
    pub created_at: String,
}

/// 排一件活。返回它的 id，页面拿着 id 就能看进度。
pub fn enqueue(
    conn: &Connection,
    kind: &str,
    round_id: Option<i64>,
    payload: &serde_json::Value,
    actor: &str,
) -> Result<i64> {
    anyhow::ensure!(
        matches!(kind, "manual_round" | "rerun"),
        "不认识的活：{kind}"
    );
    conn.execute(
        "INSERT INTO work_queue(kind, round_id, payload_json, actor, status, created_at)
         VALUES (?1,?2,?3,?4,'queued',?5)",
        params![
            kind,
            round_id,
            payload.to_string(),
            actor,
            jiff::Timestamp::now().to_string()
        ],
    )
    .context("排队")?;
    Ok(conn.last_insert_rowid())
}

const WORK_COLS: &str = "id, kind, round_id, payload_json, actor, status, note, created_at";

fn work_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    Ok(WorkItem {
        id: r.get(0)?,
        kind: r.get(1)?,
        round_id: r.get(2)?,
        payload_json: r.get(3)?,
        actor: r.get(4)?,
        status: r.get(5)?,
        note: r.get(6)?,
        created_at: r.get(7)?,
    })
}

/// 取下一件活并标成 running。**一次只取一件**：干活的循环是单线程的，
/// 取两件也只能一件一件做，而多出来的那件会在 running 上挂着假装在跑。
pub fn take_next(conn: &Connection) -> Result<Option<WorkItem>> {
    let item: Option<WorkItem> = conn
        .query_row(
            &format!(
                "SELECT {WORK_COLS} FROM work_queue WHERE status='queued' ORDER BY id LIMIT 1"
            ),
            [],
            work_row,
        )
        .optional()?;
    let Some(mut it) = item else { return Ok(None) };
    conn.execute(
        "UPDATE work_queue SET status='running', started_at=?2 WHERE id=?1",
        params![it.id, jiff::Timestamp::now().to_string()],
    )?;
    it.status = "running".into();
    Ok(Some(it))
}

/// 收一件活。`ok=false` 时 `note` 写失败原因——页面要看得见为什么没成。
pub fn finish_work(conn: &Connection, id: i64, ok: bool, note: &str) -> Result<()> {
    conn.execute(
        "UPDATE work_queue SET status=?2, note=?3, ended_at=?4 WHERE id=?1",
        params![
            id,
            if ok { "done" } else { "failed" },
            note,
            jiff::Timestamp::now().to_string()
        ],
    )?;
    Ok(())
}

/// 启动时调一次：上次崩的时候正在做的那件活，标成失败。
///
/// **不自动重排。** 手动的活多半有副作用（开了一轮、改了状态），
/// 悄悄再做一遍比不做更糟；页面上写明「上次没做完」，由人决定重不重排。
pub fn recover_running(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE work_queue SET status='failed', note='上次进程没有正常退出，这件活没做完',
                               ended_at=?1
         WHERE status='running'",
        [jiff::Timestamp::now().to_string()],
    )?)
}

pub fn recent_work(conn: &Connection, limit: usize) -> Result<Vec<WorkItem>> {
    let mut st = conn.prepare(&format!(
        "SELECT {WORK_COLS} FROM work_queue ORDER BY id DESC LIMIT ?1"
    ))?;
    Ok(st
        .query_map([limit as i64], work_row)?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger;
    use crate::rounds::{NewRound, open_round};
    use crate::types::{
        Candidate, Judgement, MediaKind, MediaRef, Platform, RoundKind, RoundTrigger,
    };

    fn setup() -> (Connection, i64) {
        let c = crate::store::open_in_memory().unwrap();
        let r = open_round(
            &c,
            &NewRound {
                kind: RoundKind::Task,
                trigger: RoundTrigger::Dispatch,
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
        .unwrap()
        .0
        .id;
        for k in ["k1", "k2"] {
            ledger::upsert_candidate(&c, &cand(k)).unwrap();
            ledger::attach_candidate(&c, r, k, "csw-window", false).unwrap();
        }
        (c, r)
    }

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw-window".into(),
            account: "acc".into(),
            url: format!("https://x/{key}"),
            text: "正文".into(),
            translated: String::new(),
            posted_at: None,
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

    fn judged(c: &Connection, round: i64, key: &str, tier: Tier) {
        let j = Judgement {
            inputs_hash: "ih".into(),
            ..Judgement::fixture(key, tier)
        };
        ledger::put_judgement(c, round, &j, &[], "m", "v1").unwrap();
    }

    #[test]
    fn 改档另存原判一个字不动() {
        let (c, r) = setup();
        judged(&c, r, "k1", Tier::NotRecommend);
        put_override(&c, r, "k1", Tier::Alternate, "同一事实但有新角度", "主编").unwrap();

        // 「模型判不推荐、主编捞回成备选」与「模型判备选」是两件事，不能混成一个字段
        let tier: String = c
            .query_row(
                "SELECT tier FROM judgements WHERE round_id=?1 AND candidate_key='k1'",
                [r],
                |x| x.get(0),
            )
            .unwrap();
        assert_eq!(tier, "not_recommend", "原判不该被改掉");

        let ov = latest_overrides(&c, r).unwrap();
        assert_eq!(ov.len(), 1);
        assert_eq!(
            (ov[0].from_tier.as_str(), ov[0].to_tier.as_str()),
            ("not_recommend", "alternate")
        );
        assert_eq!(ov[0].actor, "主编");
        assert_eq!(ov[0].reason, "同一事实但有新角度");
        // 但台账上生效的是改过的那一档——否则那次捞回等于没发生
        assert_eq!(
            effective_tier(&c, r, "k1").unwrap().as_deref(),
            Some("alternate")
        );
        assert_eq!(effective_tier(&c, r, "k9").unwrap(), None);
    }

    #[test]
    fn 改档必须写理由而且不能原地踏步() {
        let (c, r) = setup();
        judged(&c, r, "k1", Tier::NotRecommend);
        assert!(put_override(&c, r, "k1", Tier::Alternate, "   ", "主编").is_err());
        assert!(put_override(&c, r, "k1", Tier::NotRecommend, "理由", "主编").is_err());
        // 没判过的条目改不了档
        assert!(put_override(&c, r, "k9", Tier::Alternate, "理由", "主编").is_err());
    }

    #[test]
    fn 改了又改只有最后一次算数但历史留着() {
        let (c, r) = setup();
        judged(&c, r, "k1", Tier::NotRecommend);
        put_override(&c, r, "k1", Tier::Alternate, "先捞回", "主编").unwrap();
        put_override(&c, r, "k1", Tier::NotRecommend, "看错了，压回去", "主编").unwrap();

        let ov = latest_overrides(&c, r).unwrap();
        assert_eq!(ov.len(), 1);
        assert_eq!(ov[0].to_tier, "not_recommend");
        // 早先那几次留在表里，台账要能把来龙去脉摊开
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM judgement_overrides", [], |x| x.get(0))
            .unwrap();
        assert_eq!(n, 2);
        let mut st = c
            .prepare("SELECT from_tier FROM judgement_overrides ORDER BY id")
            .unwrap();
        let froms: Vec<String> = st
            .query_map([], |x| x.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        // 第二次是从「现在显示的那一档」改的，不是从原判改的
        assert_eq!(froms, ["not_recommend", "alternate"]);
        // 生效的是最后一次改到的那一档
        assert_eq!(
            effective_tier(&c, r, "k1").unwrap().as_deref(),
            Some("not_recommend")
        );
    }

    #[test]
    fn 指定首批只认这一轮里有的条目() {
        let (c, r) = setup();
        let n = set_first_batch(&c, r, &["k1".into(), "不在这一轮".into()]).unwrap();
        // 页面上那份列表可能来自一轮已经翻篇的台账，忽略而不是报错
        assert_eq!(n, 1);
        assert_eq!(first_batch(&c, r).unwrap(), ["k1"]);

        // 重设是覆盖，不是追加
        set_first_batch(&c, r, &["k2".into()]).unwrap();
        assert_eq!(first_batch(&c, r).unwrap(), ["k2"]);
        // 传空清空，退回自动挑法
        set_first_batch(&c, r, &[]).unwrap();
        assert!(first_batch(&c, r).unwrap().is_empty());
    }

    #[test]
    fn 勾选只认三种且备注不能是空的() {
        let (c, r) = setup();
        put_van_mark(&c, r, "k1", "like", "", "van").unwrap();
        put_van_mark(&c, r, "k1", "note", "这条我要", "van").unwrap();
        assert!(put_van_mark(&c, r, "k1", "推荐", "", "van").is_err());
        assert!(put_van_mark(&c, r, "k1", "note", "  ", "van").is_err());

        let ms = van_marks(&c, r).unwrap();
        assert_eq!(ms.len(), 2);
        // 同一个人同一条同一种再勾一次是改备注，不是又加一行
        put_van_mark(&c, r, "k1", "note", "改主意了", "van").unwrap();
        let ms = van_marks(&c, r).unwrap();
        assert_eq!(ms.len(), 2);
        assert!(ms.iter().any(|m| m.note == "改主意了"));

        assert_eq!(drop_van_mark(&c, r, "k1", "like", "van").unwrap(), 1);
        assert_eq!(van_marks(&c, r).unwrap().len(), 1);
    }

    #[test]
    fn 排队一次只取一件且取了就标running() {
        let c = crate::store::open_in_memory().unwrap();
        let a = enqueue(
            &c,
            "manual_round",
            None,
            &serde_json::json!({"days": 1}),
            "主编",
        )
        .unwrap();
        let b = enqueue(&c, "manual_round", None, &serde_json::json!({}), "主编").unwrap();
        assert!(enqueue(&c, "乱写", None, &serde_json::json!({}), "主编").is_err());

        // 干活的循环是单线程的，取两件也只能一件一件做
        let got = take_next(&c).unwrap().unwrap();
        assert_eq!(got.id, a);
        assert_eq!(got.status, "running");
        assert_eq!(got.payload_json, r#"{"days":1}"#);
        // 第一件还挂着 running，第二件才轮得到
        finish_work(&c, a, true, "开了一轮，取到 358 条").unwrap();
        assert_eq!(take_next(&c).unwrap().unwrap().id, b);
        finish_work(&c, b, false, "csw 取数超时").unwrap();
        assert!(take_next(&c).unwrap().is_none());

        let all = recent_work(&c, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            (all[0].status.as_str(), all[0].note.as_str()),
            ("failed", "csw 取数超时")
        );
    }

    #[test]
    fn 崩溃时在做的活标失败不自动重排() {
        let c = crate::store::open_in_memory().unwrap();
        enqueue(&c, "manual_round", None, &serde_json::json!({}), "主编").unwrap();
        take_next(&c).unwrap().unwrap();
        assert_eq!(recover_running(&c).unwrap(), 1);
        // 手动的活多半有副作用，悄悄再做一遍比不做更糟
        assert!(take_next(&c).unwrap().is_none());
        assert_eq!(recent_work(&c, 1).unwrap()[0].status, "failed");
    }

    #[test]
    fn 每个写操作都留一行痕() {
        let c = crate::store::open_in_memory().unwrap();
        audit(
            &c,
            "主编",
            "override",
            "r1/k1",
            &serde_json::json!({"to": "alternate"}),
        )
        .unwrap();
        audit(
            &c,
            "van",
            "van_mark",
            "r1/k1",
            &serde_json::json!({"mark": "like"}),
        )
        .unwrap();
        let rows = recent_audit(&c, 10).unwrap();
        assert_eq!(rows.len(), 2);
        // 最近的在前
        assert_eq!(rows[0].action, "van_mark");
        assert_eq!(rows[1].actor, "主编");
        assert!(rows[1].detail_json.contains("alternate"));
    }
}
