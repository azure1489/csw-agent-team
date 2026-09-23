//! 选题层与定点补读的本地落库（迁移 0004）。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::types::{Tier, Topic};

fn tier_str(t: Option<Tier>) -> String {
    t.and_then(|t| serde_json::to_value(t).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 写这一轮的选题。**整轮替换**：重判一次选题就可能重组，旧的留着会重复。
/// 同时把成员的 `round_candidates.event_key` 指到所属选题。
pub fn put_topics(conn: &Connection, round_id: i64, topics: &[Topic]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM topics WHERE round_id = ?1", [round_id])?;
    let now = jiff::Timestamp::now().to_string();
    for t in topics {
        tx.execute(
            "INSERT INTO topics(round_id, topic_key, primary_key, members_json, merge_note, tier,
                                headline, synthesis_json, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                round_id,
                t.topic_key,
                t.primary_key,
                serde_json::to_string(&t.members)?,
                t.merge_note,
                tier_str(t.tier),
                t.headline,
                serde_json::to_string(&t.synthesis)?,
                now,
            ],
        )
        .with_context(|| format!("写选题 {}", t.topic_key))?;
        for m in &t.members {
            tx.execute(
                "UPDATE round_candidates SET event_key = ?3 WHERE round_id = ?1 AND candidate_key = ?2",
                params![round_id, m, t.topic_key],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 读这一轮的选题，按写入顺序。
pub fn topics(conn: &Connection, round_id: i64) -> Result<Vec<Topic>> {
    let mut st = conn.prepare(
        "SELECT topic_key, primary_key, members_json, merge_note, tier, headline, synthesis_json
         FROM topics WHERE round_id = ?1 ORDER BY id",
    )?;
    let rows = st.query_map([round_id], |r| {
        let members: String = r.get(2)?;
        let tier: String = r.get(4)?;
        let syn: String = r.get(6)?;
        Ok(Topic {
            topic_key: r.get(0)?,
            primary_key: r.get(1)?,
            members: serde_json::from_str(&members).unwrap_or_default(),
            merge_note: r.get(3)?,
            tier: serde_json::from_value(serde_json::Value::String(tier)).ok(),
            headline: r.get(5)?,
            synthesis: serde_json::from_str(&syn).unwrap_or_default(),
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 一次补读的记录。`text` 只留本机。
#[derive(Debug, Clone, Default)]
pub struct RefetchRow {
    pub candidate_key: String,
    pub url: String,
    pub status: String,
    pub http_status: Option<u16>,
    pub bytes: usize,
    pub text: String,
    pub error: String,
}

pub fn put_refetch(conn: &Connection, round_id: i64, r: &RefetchRow) -> Result<()> {
    conn.execute(
        "INSERT INTO refetches(round_id, candidate_key, url, status, http_status, bytes,
                               content_hash, text, error, attempted_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(round_id, candidate_key, url) DO UPDATE SET
           status=excluded.status, http_status=excluded.http_status, bytes=excluded.bytes,
           content_hash=excluded.content_hash, text=excluded.text, error=excluded.error,
           attempted_at=excluded.attempted_at",
        params![
            round_id,
            r.candidate_key,
            r.url,
            r.status,
            r.http_status.map(i64::from),
            r.bytes as i64,
            if r.text.is_empty() {
                String::new()
            } else {
                blake3::hash(r.text.as_bytes()).to_hex().to_string()
            },
            r.text,
            r.error,
            jiff::Timestamp::now().to_string(),
        ],
    )
    .context("写补读记录")?;
    Ok(())
}

/// 最近 `hours` 小时内这条候选这个地址补读成功过的正文。预取轮抓过的，正式轮直接用。
pub fn recent_refetch(
    conn: &Connection,
    candidate_key: &str,
    url: &str,
    hours: i64,
) -> Result<Option<String>> {
    let since = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_hours(hours))?
        .to_string();
    Ok(conn
        .query_row(
            "SELECT text FROM refetches WHERE candidate_key = ?1 AND url = ?2 AND status = 'ok'
               AND text <> '' AND attempted_at >= ?3
             ORDER BY id DESC LIMIT 1",
            params![candidate_key, url, since],
            |r| r.get(0),
        )
        .optional()?)
}

/// 这条候选最近一次补读了哪些地址、结果如何（给待核结转页看，不含正文）。
pub fn refetch_summary(conn: &Connection, candidate_key: &str) -> Result<Vec<(String, String)>> {
    let last: Option<i64> = conn
        .query_row(
            "SELECT MAX(f.round_id) FROM refetches f JOIN rounds r ON r.id = f.round_id
             WHERE f.candidate_key = ?1 AND r.kind NOT IN ('backtest', 'replay')",
            [candidate_key],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(round) = last else {
        return Ok(vec![]);
    };
    let mut st = conn.prepare(
        "SELECT url, status FROM refetches WHERE candidate_key = ?1 AND round_id = ?2 ORDER BY id",
    )?;
    let rows = st.query_map(params![candidate_key, round], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TopicSynthesis;

    fn round(c: &Connection) -> i64 {
        crate::rounds::open_round(
            c,
            &crate::rounds::NewRound {
                kind: crate::types::RoundKind::Manual,
                trigger: crate::types::RoundTrigger::Manual,
                run_id: None,
                task_id: None,
                stage_code: None,
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap()
        .0
        .id
    }

    #[test]
    fn 选题整轮替换且成员指回选题() {
        let c = crate::store::open_in_memory().unwrap();
        let r = round(&c);
        let t = Topic {
            topic_key: "a".into(),
            primary_key: "a".into(),
            members: vec!["a".into(), "b".into()],
            tier: Some(Tier::Recommend),
            headline: "夹克｜理由".into(),
            synthesis: TopicSynthesis {
                shared_facts: vec!["同一款".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        put_topics(&c, r, std::slice::from_ref(&t)).unwrap();
        put_topics(&c, r, &[t]).unwrap();
        let got = topics(&c, r).unwrap();
        assert_eq!(got.len(), 1, "整轮替换，不重复");
        assert_eq!(got[0].members, ["a", "b"]);
        assert_eq!(got[0].tier, Some(Tier::Recommend));
        assert_eq!(got[0].synthesis.shared_facts, ["同一款"]);
    }

    #[test]
    fn 补读记录只给地址与结果() {
        let c = crate::store::open_in_memory().unwrap();
        let r = round(&c);
        put_refetch(
            &c,
            r,
            &RefetchRow {
                candidate_key: "k".into(),
                url: "https://e.com/a".into(),
                status: "ok".into(),
                text: "全文".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            refetch_summary(&c, "k").unwrap(),
            [("https://e.com/a".to_string(), "ok".to_string())]
        );
        assert!(refetch_summary(&c, "x").unwrap().is_empty());
    }
}
