//! 写引擎的动作先进 outbox，再发。
//!
//! # 为什么不直接发
//!
//! 一轮要往引擎写五类东西（items、sweeps、judgements、intake-check、submit），
//! 中间任何一步崩掉，已经发出去的和没发的必须分得清。直接发的写法在崩溃后
//! 只剩一个问题：**刚才那条到底发出去没有？** 而这个问题没有答案——
//! 引擎的 `Idempotency-Key` 只对 POST 生效，且请求中途崩溃时同键会永久 409。
//!
//! 所以：**先落库，再发，发完改状态。** 崩溃后照 `pending` 与 `sending` 续，
//! 不重建请求体（`body_sha` 钉着它）。
//!
//! # 三条硬规矩
//!
//! **一、重试必须发同样的字节。** `body_sha` 就是为此存的：重试前核一遍，
//! 对不上说明有人重建了请求体，那会让幂等键指向另一份内容。
//!
//! **二、`conflict` 不许自动重试。** 引擎回 `idempotency_in_progress_or_uncertain`
//! 或 409 时，正确的动作是**先查任务状态对账**，不是换个幂等键再发一遍——
//! 换键等于重复提交，而重复提交在引擎那边是两份交付物、在人那边是两条一样的记录。
//!
//! **三、顺序是依赖，不是偏好。** items → sweeps → judgements → intake-check → submit。
//! `depends_on` 把它钉死：前一条没 confirmed，后一条不出队。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::types::{OutboxKind, OutboxStatus};

/// 一条最多试几次，超了就 `dead`。**不是无限重试**：
/// 一直失败的多半是请求本身有问题，重试只会把日志刷满。
pub const MAX_ATTEMPTS: i64 = 6;

#[derive(Debug, Clone)]
pub struct NewEntry {
    pub round_id: i64,
    pub kind: OutboxKind,
    pub idem_key: String,
    /// 大体积（zip）落盘，只存路径
    pub body_path: String,
    pub body_json: String,
    pub body_sha: String,
    /// 前一条的 seq。没有就 None。
    pub depends_on: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub seq: i64,
    pub round_id: i64,
    pub kind: String,
    pub idem_key: String,
    pub body_path: String,
    pub body_json: String,
    pub body_sha: String,
    pub depends_on: Option<i64>,
    pub status: String,
    pub attempts: i64,
    pub last_error: String,
}

const COLS: &str = "seq, round_id, kind, idem_key, body_path, body_json, body_sha,
                    depends_on, status, attempts, last_error";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        seq: r.get(0)?,
        round_id: r.get(1)?,
        kind: r.get(2)?,
        idem_key: r.get(3)?,
        body_path: r.get(4)?,
        body_json: r.get(5)?,
        body_sha: r.get(6)?,
        depends_on: r.get(7)?,
        status: r.get(8)?,
        attempts: r.get(9)?,
        last_error: r.get(10)?,
    })
}

/// 排一条进队。**同一轮同一类同一幂等键只排一条**，重复排队返回已有的那条。
pub fn enqueue(conn: &Connection, e: &NewEntry) -> Result<Entry> {
    let n = conn.execute(
        "INSERT INTO engine_outbox(round_id, kind, idem_key, body_path, body_json, body_sha,
                                   depends_on, status, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8) ON CONFLICT DO NOTHING",
        params![
            e.round_id,
            kind_str(e.kind),
            e.idem_key,
            e.body_path,
            e.body_json,
            e.body_sha,
            e.depends_on,
            jiff::Timestamp::now().to_string(),
        ],
    )?;
    if n == 1 {
        return get(conn, conn.last_insert_rowid())?.context("刚排的队读不回来");
    }
    conn.query_row(
        &format!("SELECT {COLS} FROM engine_outbox WHERE round_id=?1 AND kind=?2 AND idem_key=?3"),
        params![e.round_id, kind_str(e.kind), e.idem_key],
        row,
    )
    .optional()?
    .context("排队被唯一约束挡下，却又找不到已有的那条")
}

pub fn get(conn: &Connection, seq: i64) -> Result<Option<Entry>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM engine_outbox WHERE seq=?1"),
            [seq],
            row,
        )
        .optional()?)
}

/// 下一条该发的。
///
/// **依赖没 confirmed 就不出队**——顺序是依赖不是偏好。
/// `sending` 状态的也会被捡回来：那是上次崩溃时正在发的，
/// 要不要重发由调用方核对任务状态后决定。
pub fn next_ready(conn: &Connection) -> Result<Option<Entry>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM engine_outbox o
                 WHERE o.status IN ('pending','sending')
                   AND (o.depends_on IS NULL
                        OR EXISTS (SELECT 1 FROM engine_outbox d
                                   WHERE d.seq = o.depends_on AND d.status = 'confirmed'))
                 ORDER BY o.seq LIMIT 1"
            ),
            [],
            row,
        )
        .optional()?)
}

/// 标成正在发。**发之前核一遍 `body_sha`**：对不上说明有人重建了请求体。
pub fn begin_send(conn: &Connection, seq: i64, body_sha: &str) -> Result<Entry> {
    let e = get(conn, seq)?.context("这条不在队里")?;
    anyhow::ensure!(
        e.body_sha == body_sha,
        "第 {seq} 条的请求体变了（库里 {}，要发的 {body_sha}）。\
         重试必须发同样的字节，否则幂等键指向的就是另一份内容。",
        e.body_sha
    );
    conn.execute(
        "UPDATE engine_outbox SET status='sending', attempts=attempts+1 WHERE seq=?1",
        [seq],
    )?;
    get(conn, seq)?.context("读不回来")
}

pub fn confirm(conn: &Connection, seq: i64, response: &str) -> Result<()> {
    conn.execute(
        "UPDATE engine_outbox SET status='confirmed', response_json=?2, sent_at=?3, last_error=''
         WHERE seq=?1",
        params![seq, response, jiff::Timestamp::now().to_string()],
    )?;
    Ok(())
}

/// 这一次没发成。超过 [`MAX_ATTEMPTS`] 就 `dead`，否则回 `pending` 等下一轮。
pub fn fail(conn: &Connection, seq: i64, error: &str) -> Result<OutboxStatus> {
    let e = get(conn, seq)?.context("这条不在队里")?;
    let next = if e.attempts >= MAX_ATTEMPTS {
        OutboxStatus::Dead
    } else {
        OutboxStatus::Pending
    };
    conn.execute(
        "UPDATE engine_outbox SET status=?2, last_error=?3 WHERE seq=?1",
        params![seq, status_str(next), error],
    )?;
    Ok(next)
}

/// 冲突：**不许自动重试，交给人核实。**
///
/// 引擎回 `idempotency_in_progress_or_uncertain` 或 409 时走这条。
/// 正确的下一步是查任务状态对账，而不是换个幂等键再发一遍。
pub fn conflict(conn: &Connection, seq: i64, error: &str) -> Result<()> {
    conn.execute(
        "UPDATE engine_outbox SET status='conflict', last_error=?2 WHERE seq=?1",
        params![seq, error],
    )?;
    tracing::error!(
        第几条 = seq,
        原因 = error,
        "引擎返回冲突。**不要换幂等键重试**，先查任务状态对账。"
    );
    Ok(())
}

/// 这一轮还有没有没发完的。
pub fn outstanding(conn: &Connection, round_id: i64) -> Result<Vec<Entry>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM engine_outbox
         WHERE round_id=?1 AND status IN ('pending','sending','conflict') ORDER BY seq"
    ))?;
    Ok(st
        .query_map([round_id], row)?
        .filter_map(Result::ok)
        .collect())
}

fn kind_str(k: OutboxKind) -> &'static str {
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

fn status_str(s: OutboxStatus) -> &'static str {
    match s {
        OutboxStatus::Pending => "pending",
        OutboxStatus::Sending => "sending",
        OutboxStatus::Confirmed => "confirmed",
        OutboxStatus::Conflict => "conflict",
        OutboxStatus::Dead => "dead",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rounds::{NewRound, open_round};
    use crate::types::{RoundKind, RoundTrigger};

    fn setup() -> (Connection, i64) {
        let c = crate::store::open_in_memory().unwrap();
        let (r, _) = open_round(
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
        .unwrap();
        (c, r.id)
    }

    fn entry(round_id: i64, kind: OutboxKind, dep: Option<i64>) -> NewEntry {
        NewEntry {
            round_id,
            kind,
            idem_key: format!("{}-key", kind_str(kind)),
            body_path: String::new(),
            body_json: "{}".into(),
            body_sha: format!("sha-{}", kind_str(kind)),
            depends_on: dep,
        }
    }

    #[test]
    fn 顺序是依赖前一条没确认后一条不出队() {
        let (c, r) = setup();
        let items = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        let sweeps = enqueue(&c, &entry(r, OutboxKind::Sweeps, Some(items.seq))).unwrap();
        enqueue(&c, &entry(r, OutboxKind::Judgements, Some(sweeps.seq))).unwrap();

        assert_eq!(next_ready(&c).unwrap().unwrap().kind, "items");
        // items 没确认，sweeps 不该出来
        begin_send(&c, items.seq, "sha-items").unwrap();
        assert_eq!(next_ready(&c).unwrap().unwrap().seq, items.seq);
        confirm(&c, items.seq, "{}").unwrap();
        assert_eq!(next_ready(&c).unwrap().unwrap().kind, "sweeps");
    }

    #[test]
    fn 重试必须发同样的字节() {
        let (c, r) = setup();
        let e = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        // 有人重建了请求体 → 幂等键会指向另一份内容
        let err = begin_send(&c, e.seq, "别的 sha").unwrap_err().to_string();
        assert!(err.contains("请求体变了"), "{err}");
        assert!(begin_send(&c, e.seq, "sha-items").is_ok());
    }

    #[test]
    fn 重复排队返回已有的那条() {
        let (c, r) = setup();
        let a = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        let b = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        assert_eq!(a.seq, b.seq);
    }

    #[test]
    fn 失败会回到pending超次数才dead() {
        let (c, r) = setup();
        let e = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        for i in 1..MAX_ATTEMPTS {
            begin_send(&c, e.seq, "sha-items").unwrap();
            assert_eq!(
                fail(&c, e.seq, "网关超时").unwrap(),
                OutboxStatus::Pending,
                "第 {i} 次"
            );
        }
        begin_send(&c, e.seq, "sha-items").unwrap();
        // 一直失败的多半是请求本身有问题，重试只会把日志刷满
        assert_eq!(fail(&c, e.seq, "还是不行").unwrap(), OutboxStatus::Dead);
        assert!(next_ready(&c).unwrap().is_none(), "dead 的不该再出队");
    }

    #[test]
    fn 冲突要停下交给人不许自动重试() {
        let (c, r) = setup();
        let e = enqueue(&c, &entry(r, OutboxKind::Deliverable, None)).unwrap();
        begin_send(&c, e.seq, "sha-deliverable").unwrap();
        conflict(&c, e.seq, "idempotency_in_progress_or_uncertain").unwrap();
        // 换键等于重复提交：引擎那边两份交付物，人那边两条一样的记录
        assert!(next_ready(&c).unwrap().is_none());
        let got = get(&c, e.seq).unwrap().unwrap();
        assert_eq!(got.status, "conflict");
        assert!(got.last_error.contains("uncertain"));
        // 但它要出现在「这一轮还没发完」里，好让人看见
        assert_eq!(outstanding(&c, r).unwrap().len(), 1);
    }

    #[test]
    fn 崩溃时正在发的会被捡回来() {
        let (c, r) = setup();
        let e = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        begin_send(&c, e.seq, "sha-items").unwrap();
        // 这时候进程崩了。status 还是 sending
        let back = next_ready(&c).unwrap().unwrap();
        assert_eq!(back.seq, e.seq);
        assert_eq!(back.status, "sending");
        assert_eq!(back.attempts, 1, "attempts 不该被重置");
    }

    #[test]
    fn 确认过的不再出队也不算欠着() {
        let (c, r) = setup();
        let e = enqueue(&c, &entry(r, OutboxKind::Items, None)).unwrap();
        begin_send(&c, e.seq, "sha-items").unwrap();
        confirm(&c, e.seq, r#"{"ok":true}"#).unwrap();
        assert!(next_ready(&c).unwrap().is_none());
        assert!(outstanding(&c, r).unwrap().is_empty());
        let got = get(&c, e.seq).unwrap().unwrap();
        assert_eq!(got.status, "confirmed");
        assert!(got.last_error.is_empty(), "确认了就把上次的错清掉");
    }
}
