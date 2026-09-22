//! 轮次与步骤的读写。**可重跑、可续跑**的那部分状态都在这里。
//!
//! # 三条规矩
//!
//! **一、同一任务的同一目标版本、同一触发原因只许开一轮。**
//! 引擎的轮询接口偶尔会把同一条派单读到两次；库上的唯一索引是唯一的防线，
//! 代码里再怎么判重都挡不住两个进程同时读到。[`open_round`] 撞到唯一约束时
//! **返回已有的那一轮**，不报错——重复派单不是错误，是常态。
//!
//! **二、重跑一步 = 开一个新 attempt，旧的留着。**
//! 不是把旧记录改掉。出问题时要能看见「第一次是怎么失败的、第二次改了什么」。
//! 重跑还会把**下游各步置 `stale`**：第 5 步的输入变了，第 6 步之后的产物就不能用了。
//!
//! **三、崩溃后不从头再来。** 启动时把还挂着 `Running` 的步改成 `Interrupted`
//! （见 [`recover_interrupted`]），再按 `input_hash` 去续跑没做完的单元。
//! 直接重跑一轮要花四十多分钟，而崩溃多半发生在最后那几步。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::types::{RoundKind, RoundTrigger, StepCode, StepStatus};

#[derive(Debug, Clone)]
pub struct NewRound {
    pub kind: RoundKind,
    pub trigger: RoundTrigger,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub stage_code: Option<String>,
    pub target_version: i64,
    pub parent_round_id: Option<i64>,
    pub window_start: String,
    pub window_end: String,
    pub plan_version: i64,
    pub rubric_version: String,
    pub kb_snapshot: String,
    pub instructions_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub id: i64,
    pub kind: String,
    pub trigger: String,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub target_version: i64,
    pub window_start: String,
    pub window_end: String,
    pub instructions_hash: String,
    pub status: String,
    pub note: String,
}

/// 开一轮。**撞到唯一约束就返回已有的那一轮**，`bool` 说明是不是新开的。
///
/// 重复派单不是错误：轮询读到两次、两个进程同时读到，都会走到这里。
pub fn open_round(conn: &Connection, r: &NewRound) -> Result<(Round, bool)> {
    let now = jiff::Timestamp::now().to_string();
    let n = conn.execute(
        "INSERT INTO rounds(kind, trigger, run_id, task_id, stage_code, target_version,
                            parent_round_id, window_start, window_end, plan_version,
                            rubric_version, kb_snapshot, instructions_hash, status, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'running',?14)
         ON CONFLICT DO NOTHING",
        params![
            kind_str(r.kind),
            trigger_str(r.trigger),
            r.run_id,
            r.task_id,
            r.stage_code,
            r.target_version,
            r.parent_round_id,
            r.window_start,
            r.window_end,
            r.plan_version,
            r.rubric_version,
            r.kb_snapshot,
            r.instructions_hash,
            now,
        ],
    )?;
    if n == 1 {
        let id = conn.last_insert_rowid();
        return Ok((get(conn, id)?.context("刚插的轮次读不回来")?, true));
    }
    // 撞了唯一索引：把已有的那一轮找回来
    let existing = conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM rounds
                 WHERE task_id = ?1 AND target_version = ?2 AND trigger = ?3"
            ),
            params![r.task_id, r.target_version, trigger_str(r.trigger)],
            row,
        )
        .optional()?
        .context("插入被唯一约束挡下，却又找不到已有的那一轮")?;
    Ok((existing, false))
}

const COLS: &str = "id, kind, trigger, run_id, task_id, target_version,
                    window_start, window_end, instructions_hash, status, note";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Round> {
    Ok(Round {
        id: r.get(0)?,
        kind: r.get(1)?,
        trigger: r.get(2)?,
        run_id: r.get(3)?,
        task_id: r.get(4)?,
        target_version: r.get(5)?,
        window_start: r.get(6)?,
        window_end: r.get(7)?,
        instructions_hash: r.get(8)?,
        status: r.get(9)?,
        note: r.get(10)?,
    })
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Round>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM rounds WHERE id = ?1"),
            [id],
            row,
        )
        .optional()?)
}

pub fn finish_round(conn: &Connection, id: i64, status: &str, note: &str) -> Result<()> {
    conn.execute(
        "UPDATE rounds SET status = ?2, note = ?3, ended_at = ?4 WHERE id = ?1",
        params![id, status, note, jiff::Timestamp::now().to_string()],
    )?;
    Ok(())
}

// ─────────────────────────────── 步骤 ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: i64,
    pub round_id: i64,
    pub step: String,
    pub attempt: i64,
    pub status: String,
    pub input_hash: String,
    pub counts_json: String,
    pub error: String,
}

impl Step {
    /// 这一步的产物还能用吗。
    pub fn reusable(&self, input_hash: &str) -> bool {
        // 空哈希不算命中：它表示「这一步没记输入」，而不是「输入一样」
        !input_hash.is_empty()
            && self.input_hash == input_hash
            && matches!(self.status.as_str(), "succeeded" | "partial")
    }
}

const STEP_COLS: &str = "id, round_id, step, attempt, status, input_hash, counts_json, error";

fn step_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Step> {
    Ok(Step {
        id: r.get(0)?,
        round_id: r.get(1)?,
        step: r.get(2)?,
        attempt: r.get(3)?,
        status: r.get(4)?,
        input_hash: r.get(5)?,
        counts_json: r.get(6)?,
        error: r.get(7)?,
    })
}

/// 开始一步。**每次调用都是一个新的 attempt**，旧的留着看。
///
/// 同时把下游各步置 `stale`：这一步的输入变了，后面的产物就不能用了。
pub fn begin_step(
    conn: &Connection,
    round_id: i64,
    step: StepCode,
    input_hash: &str,
) -> Result<Step> {
    let code = step_str(step);
    let attempt: i64 = conn.query_row(
        "SELECT COALESCE(MAX(attempt), 0) + 1 FROM round_steps WHERE round_id = ?1 AND step = ?2",
        params![round_id, code],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO round_steps(round_id, step, attempt, status, input_hash, started_at)
         VALUES (?1,?2,?3,'running',?4,?5)",
        params![
            round_id,
            code,
            attempt,
            input_hash,
            jiff::Timestamp::now().to_string()
        ],
    )?;
    if attempt > 1 {
        mark_downstream_stale(conn, round_id, step)?;
    }
    let id = conn.last_insert_rowid();
    conn.query_row(
        &format!("SELECT {STEP_COLS} FROM round_steps WHERE id = ?1"),
        [id],
        step_row,
    )
    .context("刚插的步读不回来")
}

/// 收一步。
pub fn end_step(
    conn: &Connection,
    step_id: i64,
    status: StepStatus,
    counts: &serde_json::Value,
    error: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE round_steps SET status = ?2, counts_json = ?3, error = ?4, ended_at = ?5
         WHERE id = ?1",
        params![
            step_id,
            status_str(status),
            counts.to_string(),
            error,
            jiff::Timestamp::now().to_string()
        ],
    )?;
    Ok(())
}

/// 某一步最近一次的记录（最大 attempt）。
pub fn latest(conn: &Connection, round_id: i64, step: StepCode) -> Result<Option<Step>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {STEP_COLS} FROM round_steps
                 WHERE round_id = ?1 AND step = ?2 ORDER BY attempt DESC LIMIT 1"
            ),
            params![round_id, step_str(step)],
            step_row,
        )
        .optional()?)
}

/// 这一步能不能复用上一轮的产物。给预取轮与正式轮对账用。
pub fn reusable_from(
    conn: &Connection,
    round_id: i64,
    step: StepCode,
    input_hash: &str,
) -> Result<bool> {
    Ok(latest(conn, round_id, step)?
        .map(|s| s.reusable(input_hash))
        .unwrap_or(false))
}

/// 把这一步之后的各步置 `stale`。
///
/// 只动**最新那个 attempt**：更早的 attempt 是历史，不该被改。
fn mark_downstream_stale(conn: &Connection, round_id: i64, step: StepCode) -> Result<()> {
    let after: Vec<&str> = StepCode::ALL
        .into_iter()
        .skip_while(|s| *s != step)
        .skip(1)
        .map(step_str)
        .collect();
    for code in after {
        conn.execute(
            "UPDATE round_steps SET status = 'stale'
             WHERE round_id = ?1 AND step = ?2
               AND attempt = (SELECT MAX(attempt) FROM round_steps
                              WHERE round_id = ?1 AND step = ?2)
               AND status IN ('succeeded','partial')",
            params![round_id, code],
        )?;
    }
    Ok(())
}

/// 启动时调一次：把还挂着 `running` 的步改成 `interrupted`。
///
/// 这些步是上次进程崩的时候正在跑的。改成 `interrupted` 而不是 `failed`，
/// 是因为**它们多半做了一半**——续跑要按 `input_hash` 去补没做完的单元，
/// 而不是当作从没跑过。
pub fn recover_interrupted(conn: &Connection) -> Result<usize> {
    let n = conn.execute(
        "UPDATE round_steps SET status = 'interrupted', ended_at = ?1 WHERE status = 'running'",
        [jiff::Timestamp::now().to_string()],
    )?;
    if n > 0 {
        tracing::warn!(
            步数 = n,
            "上次进程没有正常退出，这些步标成 interrupted，按输入指纹续跑"
        );
    }
    Ok(n)
}

/// 上一轮**派单轮**的窗口终点，就是这一轮的水位。
///
/// 只看派单轮：手动轮与预取轮的窗口是人随手指定或凌晨那一次的，
/// 拿它们当水位会让正式轮少扫一段——而少扫的那一段没有任何地方会报错。
pub fn last_task_window_end(conn: &Connection) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT window_end FROM rounds WHERE kind='task' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

/// 还在跑的轮次。重启后要把它们捡起来。
pub fn running_rounds(conn: &Connection) -> Result<Vec<Round>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM rounds WHERE status = 'running' ORDER BY id"
    ))?;
    Ok(st.query_map([], row)?.filter_map(Result::ok).collect())
}

fn kind_str(k: RoundKind) -> &'static str {
    match k {
        RoundKind::Task => "task",
        RoundKind::Manual => "manual",
        RoundKind::Prefetch => "prefetch",
        RoundKind::Replay => "replay",
    }
}

fn trigger_str(t: RoundTrigger) -> &'static str {
    match t {
        RoundTrigger::Dispatch => "dispatch",
        RoundTrigger::Returned => "returned",
        RoundTrigger::Supplement => "supplement",
        RoundTrigger::Manual => "manual",
    }
}

fn step_str(s: StepCode) -> &'static str {
    match s {
        StepCode::Intake => "intake",
        StepCode::Harvest => "harvest",
        StepCode::Merge => "merge",
        StepCode::Materials => "materials",
        StepCode::Judge => "judge",
        StepCode::Deepcheck => "deepcheck",
        StepCode::Build => "build",
        StepCode::Register => "register",
        StepCode::SelfCheck => "self_check",
        StepCode::Submit => "submit",
    }
}

fn status_str(s: StepStatus) -> &'static str {
    match s {
        StepStatus::Pending => "pending",
        StepStatus::Running => "running",
        StepStatus::Succeeded => "succeeded",
        StepStatus::Partial => "partial",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
        StepStatus::Stale => "stale",
        StepStatus::Interrupted => "interrupted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        crate::store::open_in_memory().unwrap()
    }

    fn task_round(task_id: i64, version: i64, trigger: RoundTrigger) -> NewRound {
        NewRound {
            kind: RoundKind::Task,
            trigger,
            run_id: Some(48),
            task_id: Some(task_id),
            stage_code: Some("intake".into()),
            target_version: version,
            parent_round_id: None,
            window_start: "2026-09-17T00:00:00Z".into(),
            window_end: "2026-09-19T00:00:00Z".into(),
            plan_version: 1,
            rubric_version: "van-rubric/v1".into(),
            kb_snapshot: "kb-2026-09-18".into(),
            instructions_hash: "abc".into(),
        }
    }

    #[test]
    fn 重复派单开不出第二轮() {
        let c = conn();
        let (a, new_a) = open_round(&c, &task_round(123, 1, RoundTrigger::Dispatch)).unwrap();
        assert!(new_a);
        // 轮询偶尔会把同一条派单读到两次——这不是错误，是常态
        let (b, new_b) = open_round(&c, &task_round(123, 1, RoundTrigger::Dispatch)).unwrap();
        assert!(!new_b);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn 退回返工是同一任务的新一轮() {
        let c = conn();
        let (a, _) = open_round(&c, &task_round(123, 1, RoundTrigger::Dispatch)).unwrap();
        // 目标版本不同 → 另一轮
        let (b, new_b) = open_round(&c, &task_round(123, 2, RoundTrigger::Returned)).unwrap();
        assert!(new_b && b.id != a.id);
        // 触发原因不同 → 也是另一轮（补件挂在已通过的任务上）
        let (d, new_d) = open_round(&c, &task_round(123, 1, RoundTrigger::Supplement)).unwrap();
        assert!(new_d && d.id != a.id);
    }

    #[test]
    fn 手动轮没有任务id不受唯一约束管() {
        let c = conn();
        let mut r = task_round(0, 1, RoundTrigger::Manual);
        r.kind = RoundKind::Manual;
        r.task_id = None;
        r.run_id = None;
        let (a, _) = open_round(&c, &r).unwrap();
        let (b, new_b) = open_round(&c, &r).unwrap();
        // 唯一索引带 WHERE task_id IS NOT NULL，手动轮想开几轮开几轮
        assert!(new_b && a.id != b.id);
    }

    #[test]
    fn 重跑一步是新attempt旧的留着() {
        let c = conn();
        let (r, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        let s1 = begin_step(&c, r.id, StepCode::Judge, "h1").unwrap();
        end_step(
            &c,
            s1.id,
            StepStatus::Failed,
            &serde_json::json!({}),
            "网关挂了",
        )
        .unwrap();
        let s2 = begin_step(&c, r.id, StepCode::Judge, "h2").unwrap();
        assert_eq!((s1.attempt, s2.attempt), (1, 2));

        // 第一次是怎么失败的要留着看
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM round_steps WHERE round_id=?1 AND step='judge'",
                [r.id],
                |x| x.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
        let old_err: String = c
            .query_row("SELECT error FROM round_steps WHERE id=?1", [s1.id], |x| {
                x.get(0)
            })
            .unwrap();
        assert_eq!(old_err, "网关挂了");
        assert_eq!(
            latest(&c, r.id, StepCode::Judge).unwrap().unwrap().attempt,
            2
        );
    }

    #[test]
    fn 重跑一步要把下游置stale() {
        let c = conn();
        let (r, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        // 先把判断到交付这几步做成功
        for step in [StepCode::Judge, StepCode::Deepcheck, StepCode::Build] {
            let s = begin_step(&c, r.id, step, "h1").unwrap();
            end_step(&c, s.id, StepStatus::Succeeded, &serde_json::json!({}), "").unwrap();
        }
        // 再重跑判断：第 5 步的输入变了，第 6 步之后的产物就不能用了
        begin_step(&c, r.id, StepCode::Judge, "h2").unwrap();
        for step in [StepCode::Deepcheck, StepCode::Build] {
            assert_eq!(
                latest(&c, r.id, step).unwrap().unwrap().status,
                "stale",
                "{step:?} 该被置 stale"
            );
        }
        // 上游不动
        let s = begin_step(&c, r.id, StepCode::Harvest, "hh").unwrap();
        end_step(&c, s.id, StepStatus::Succeeded, &serde_json::json!({}), "").unwrap();
        begin_step(&c, r.id, StepCode::Judge, "h3").unwrap();
        assert_eq!(
            latest(&c, r.id, StepCode::Harvest).unwrap().unwrap().status,
            "succeeded"
        );
    }

    #[test]
    fn 输入没变才算能复用() {
        let c = conn();
        let (r, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        let s = begin_step(&c, r.id, StepCode::Harvest, "h1").unwrap();
        end_step(
            &c,
            s.id,
            StepStatus::Succeeded,
            &serde_json::json!({"候选": 358}),
            "",
        )
        .unwrap();

        assert!(reusable_from(&c, r.id, StepCode::Harvest, "h1").unwrap());
        assert!(!reusable_from(&c, r.id, StepCode::Harvest, "h2").unwrap());
        // 空哈希表示「这一步没记输入」，不是「输入一样」
        assert!(!reusable_from(&c, r.id, StepCode::Harvest, "").unwrap());
        // 没跑过的步当然不能复用
        assert!(!reusable_from(&c, r.id, StepCode::Judge, "h1").unwrap());
    }

    #[test]
    fn 部分失败的产物仍可复用失败的不行() {
        let c = conn();
        let (r, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        // 单张图识别失败标「未识别」不阻塞下游，所以 partial 算可复用
        let s = begin_step(&c, r.id, StepCode::Harvest, "h1").unwrap();
        end_step(
            &c,
            s.id,
            StepStatus::Partial,
            &serde_json::json!({}),
            "3 张没识别",
        )
        .unwrap();
        assert!(reusable_from(&c, r.id, StepCode::Harvest, "h1").unwrap());

        let s = begin_step(&c, r.id, StepCode::Judge, "j1").unwrap();
        end_step(&c, s.id, StepStatus::Failed, &serde_json::json!({}), "").unwrap();
        assert!(!reusable_from(&c, r.id, StepCode::Judge, "j1").unwrap());
    }

    #[test]
    fn 崩溃后挂着的步改成interrupted而不是failed() {
        let c = conn();
        let (r, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        begin_step(&c, r.id, StepCode::Harvest, "h1").unwrap();
        let s = begin_step(&c, r.id, StepCode::Judge, "j1").unwrap();
        end_step(&c, s.id, StepStatus::Succeeded, &serde_json::json!({}), "").unwrap();

        assert_eq!(recover_interrupted(&c).unwrap(), 1);
        // 它多半做了一半：续跑要按输入指纹补没做完的单元，不是当作从没跑过
        assert_eq!(
            latest(&c, r.id, StepCode::Harvest).unwrap().unwrap().status,
            "interrupted"
        );
        assert_eq!(
            latest(&c, r.id, StepCode::Judge).unwrap().unwrap().status,
            "succeeded"
        );
        // 再调一次没活干
        assert_eq!(recover_interrupted(&c).unwrap(), 0);
    }

    #[test]
    fn 水位只看派单轮() {
        let c = conn();
        assert_eq!(last_task_window_end(&c).unwrap(), None);
        open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();

        // 手动轮的窗口是人随手指定的，拿它当水位会让正式轮少扫一段，
        // 而少扫的那一段没有任何地方会报错
        let mut manual = task_round(0, 1, RoundTrigger::Manual);
        manual.kind = RoundKind::Manual;
        manual.task_id = None;
        manual.run_id = None;
        manual.window_end = "2020-01-01T00:00:00Z".into();
        open_round(&c, &manual).unwrap();

        assert_eq!(
            last_task_window_end(&c).unwrap().as_deref(),
            Some("2026-09-19T00:00:00Z")
        );
    }

    #[test]
    fn 重启后要把还在跑的轮次捡起来() {
        let c = conn();
        let (a, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        let (b, _) = open_round(&c, &task_round(2, 1, RoundTrigger::Dispatch)).unwrap();
        finish_round(&c, b.id, "done", "").unwrap();
        let running = running_rounds(&c).unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, a.id);
        assert_eq!(running[0].instructions_hash, "abc");
    }

    #[test]
    fn 收轮要记下结论与时间() {
        let c = conn();
        let (r, _) = open_round(&c, &task_round(1, 1, RoundTrigger::Dispatch)).unwrap();
        finish_round(&c, r.id, "failed", "网关整体不可用").unwrap();
        let got = get(&c, r.id).unwrap().unwrap();
        assert_eq!(got.status, "failed");
        assert_eq!(got.note, "网关整体不可用");
        let ended: Option<String> = c
            .query_row("SELECT ended_at FROM rounds WHERE id=?1", [r.id], |x| {
                x.get(0)
            })
            .unwrap();
        assert!(ended.is_some());
    }
}
