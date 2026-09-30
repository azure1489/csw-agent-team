//! 宽取留痕：接口返回的每一条落到哪儿。
//!
//! 窗口按首次入库时间算，接口只按发布时间过滤，所以采集器宽取 7 天再本地收口。
//! 收口掉的那些以前只计了数（09-28 r56：1431 → 128），主编无从复算。这里逐条存下，
//! 交付包的 `trace/window_ids.jsonl` 全部列出。

use anyhow::Result;
use rusqlite::{Connection, params};

/// 一条宽取结果。`outcome` 取值见 `harvest::pipeline::TraceOutcome::as_str`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRow {
    pub sweep_key: String,
    pub source_id: String,
    pub candidate_key: String,
    pub account: String,
    pub url: String,
    pub posted_at: Option<String>,
    pub ingested_at: Option<String>,
    pub media: String,
    pub outcome: String,
    pub dup_of: Option<String>,
    /// live = 当轮宽取时记下；refetch = 事后按同一窗口重新取数复算
    pub source: String,
}

/// 整轮替换某个采集器的留痕：同一轮重跑采集时以最后一次为准，不叠加。
pub fn put(conn: &Connection, round_id: i64, sweep_key: &str, rows: &[TraceRow]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM window_trace WHERE round_id = ?1 AND sweep_key = ?2",
        params![round_id, sweep_key],
    )?;
    let now = jiff::Timestamp::now().to_string();
    {
        let mut st = tx.prepare(
            "INSERT INTO window_trace (round_id, sweep_key, seq, source_id, candidate_key, account, url,
                                       posted_at, ingested_at, media, outcome, dup_of, source, recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        )?;
        for (i, r) in rows.iter().enumerate() {
            st.execute(params![
                round_id,
                sweep_key,
                i as i64,
                r.source_id,
                r.candidate_key,
                r.account,
                r.url,
                r.posted_at,
                r.ingested_at,
                r.media,
                r.outcome,
                r.dup_of,
                r.source,
                now
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 某一轮的全部留痕，按采集器、接口返回顺序。
pub fn of_round(conn: &Connection, round_id: i64) -> Result<Vec<TraceRow>> {
    let mut st = conn.prepare(
        "SELECT sweep_key, source_id, candidate_key, account, url, posted_at, ingested_at,
                media, outcome, dup_of, source
         FROM window_trace WHERE round_id = ?1 ORDER BY sweep_key, seq",
    )?;
    let rows = st
        .query_map(params![round_id], |r| {
            Ok(TraceRow {
                sweep_key: r.get(0)?,
                source_id: r.get(1)?,
                candidate_key: r.get(2)?,
                account: r.get(3)?,
                url: r.get(4)?,
                posted_at: r.get(5)?,
                ingested_at: r.get(6)?,
                media: r.get(7)?,
                outcome: r.get(8)?,
                dup_of: r.get(9)?,
                source: r.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 一次补采：返工时补窗口缺的那一段。0 条也记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub window_from: String,
    pub window_to: String,
    pub fetched_at: String,
    pub found: i64,
    pub fetched_unique: i64,
    pub in_window: i64,
    pub query: String,
}

pub fn put_patch(conn: &Connection, round_id: i64, p: &Patch) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO window_patches (round_id, window_from, window_to, fetched_at, found, fetched_unique, in_window, query)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![round_id, p.window_from, p.window_to, p.fetched_at, p.found, p.fetched_unique, p.in_window, p.query],
    )?;
    Ok(())
}

pub fn patches_of(conn: &Connection, round_id: i64) -> Result<Vec<Patch>> {
    let mut st = conn.prepare(
        "SELECT window_from, window_to, fetched_at, found, fetched_unique, in_window, query
         FROM window_patches WHERE round_id = ?1 ORDER BY fetched_at",
    )?;
    let rows = st
        .query_map(params![round_id], |r| {
            Ok(Patch {
                window_from: r.get(0)?,
                window_to: r.get(1)?,
                fetched_at: r.get(2)?,
                found: r.get(3)?,
                fetched_unique: r.get(4)?,
                in_window: r.get(5)?,
                query: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, outcome: &str) -> TraceRow {
        TraceRow {
            sweep_key: "csw-window".into(),
            source_id: id.into(),
            candidate_key: format!("k-{id}"),
            account: "a".into(),
            url: String::new(),
            posted_at: Some("2026-09-20T00:00:00Z".into()),
            ingested_at: None,
            media: "Image：图 1".into(),
            outcome: outcome.into(),
            dup_of: None,
            source: "live".into(),
        }
    }

    #[test]
    fn 重跑采集以最后一次为准且保序() {
        let conn = crate::store::open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO rounds(id, kind, trigger, window_start, window_end, plan_version, rubric_version,
                                kb_snapshot, status, created_at)
             VALUES (1,'task','dispatch','a','b',1,'v','k','running','t')",
            [],
        )
        .unwrap();
        put(
            &conn,
            1,
            "csw-window",
            &[row("1", "in_window"), row("2", "before_window")],
        )
        .unwrap();
        put(
            &conn,
            1,
            "csw-window",
            &[
                row("3", "in_window"),
                row("1", "not_image_only"),
                row("2", "after_window"),
            ],
        )
        .unwrap();
        let got: Vec<_> = of_round(&conn, 1)
            .unwrap()
            .into_iter()
            .map(|r| (r.source_id, r.outcome))
            .collect();
        assert_eq!(
            got,
            vec![
                ("3".into(), "in_window".into()),
                ("1".into(), "not_image_only".into()),
                ("2".into(), "after_window".into()),
            ]
        );
    }
}
