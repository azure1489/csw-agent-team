//! 引擎任务的本地镜像。**引擎是真相**，这里只是为了少打接口、能离线看，
//! 以及给预取轮一份「上一次的作业标准」。
//!
//! # 为什么存拼好的那一段，而不是三段分开存
//!
//! 作业标准是判断结论能不能复用的一部分（进 `inputs_hash`）。预取轮在
//! 01:40 还没有派单，只能拿上一次的标准去判；只有它与正式轮那天拼出来的
//! **一模一样**，结论才复用得上。分三段存就得在两处各拼一次，哪天拼法改了
//! 一处，复用就会静悄悄地永远落空——而那种落空没有任何报错，只是每天慢四十分钟。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// 一条任务在某一刻的样子。
#[derive(Debug, Clone, Default)]
pub struct TaskSnapshot {
    pub task_id: i64,
    pub run_id: i64,
    pub stage_code: String,
    pub status: String,
    pub item_key: String,
    pub editor_note: String,
    /// 拼好的三段作业标准，原样注入模型的那一份
    pub work_standard: String,
    pub upstreams_json: String,
    pub latest_review_json: String,
    pub deadline_at: String,
}

pub fn put(conn: &Connection, s: &TaskSnapshot) -> Result<()> {
    let instructions = serde_json::json!({ "work_standard": s.work_standard }).to_string();
    conn.execute(
        "INSERT INTO task_mirror(task_id, run_id, stage_code, status, item_key, editor_note,
                                 upstreams_json, instructions_json, latest_review_json,
                                 deadline_at, fetched_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(task_id) DO UPDATE SET
           status=excluded.status, item_key=excluded.item_key,
           editor_note=excluded.editor_note, upstreams_json=excluded.upstreams_json,
           instructions_json=excluded.instructions_json,
           latest_review_json=excluded.latest_review_json,
           deadline_at=excluded.deadline_at, fetched_at=excluded.fetched_at",
        params![
            s.task_id,
            s.run_id,
            s.stage_code,
            s.status,
            s.item_key,
            s.editor_note,
            if s.upstreams_json.is_empty() {
                "[]"
            } else {
                &s.upstreams_json
            },
            instructions,
            if s.latest_review_json.is_empty() {
                "{}"
            } else {
                &s.latest_review_json
            },
            s.deadline_at,
            jiff::Timestamp::now().to_string(),
        ],
    )
    .context("写任务镜像")?;
    Ok(())
}

/// 这个阶段最近一次派单用的作业标准。没派过就给 None。
///
/// **预取轮据此判断**：拿不到就用空标准跑一遍——那样结论复用不上，
/// 但描述与向量仍然省下来了，这是「缓存不是前置条件」的意思。
pub fn latest_work_standard(conn: &Connection, stage_code: &str) -> Result<Option<String>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT instructions_json FROM task_mirror
             WHERE stage_code = ?1 ORDER BY task_id DESC LIMIT 1",
            [stage_code],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|s| {
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()?
            .get("work_standard")?
            .as_str()
            .map(str::to_string)
    }))
}

/// 某一条任务当时下发的作业标准。
///
/// 重跑一轮要用**那一轮当时用的**标准，不是现在最新的：换一份标准重判，
/// 等于把结论换了个依据，而台账上看不出来换过。
pub fn work_standard_of(conn: &Connection, task_id: i64) -> Result<Option<String>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT instructions_json FROM task_mirror WHERE task_id = ?1",
            [task_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|s| {
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()?
            .get("work_standard")?
            .as_str()
            .map(str::to_string)
    }))
}

/// 这个阶段最近一次派单的任务号。给工作台与日志用。
pub fn latest_task_id(conn: &Connection, stage_code: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT task_id FROM task_mirror WHERE stage_code = ?1 ORDER BY task_id DESC LIMIT 1",
            [stage_code],
            |r| r.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(task_id: i64, stage: &str, standard: &str) -> TaskSnapshot {
        TaskSnapshot {
            task_id,
            run_id: 48,
            stage_code: stage.into(),
            status: "dispatched".into(),
            work_standard: standard.into(),
            ..Default::default()
        }
    }

    #[test]
    fn 作业标准原样存原样取() {
        let c = crate::store::open_in_memory().unwrap();
        let s = "【作业内容】\n每条都判\n\n【自检】\n窗口内条数与库内一致";
        put(&c, &snap(7, "intake", s)).unwrap();
        // 一个字都不能差：差一个字 inputs_hash 就变了，复用永远落空
        assert_eq!(
            latest_work_standard(&c, "intake").unwrap().as_deref(),
            Some(s)
        );
        assert_eq!(latest_task_id(&c, "intake").unwrap(), Some(7));
    }

    #[test]
    fn 取的是最近那一次() {
        let c = crate::store::open_in_memory().unwrap();
        put(&c, &snap(7, "intake", "旧标准")).unwrap();
        put(&c, &snap(19, "intake", "新标准")).unwrap();
        // 别的阶段不串台
        put(&c, &snap(20, "material", "配图标准")).unwrap();
        assert_eq!(
            latest_work_standard(&c, "intake").unwrap().as_deref(),
            Some("新标准")
        );
        assert_eq!(
            latest_work_standard(&c, "material").unwrap().as_deref(),
            Some("配图标准")
        );
    }

    #[test]
    fn 重跑要用那一轮当时的标准() {
        let c = crate::store::open_in_memory().unwrap();
        put(&c, &snap(7, "intake", "那天的标准")).unwrap();
        put(&c, &snap(19, "intake", "今天的标准")).unwrap();
        // 换一份标准重判，等于把结论换了个依据，而台账上看不出来换过
        assert_eq!(
            work_standard_of(&c, 7).unwrap().as_deref(),
            Some("那天的标准")
        );
        assert_eq!(work_standard_of(&c, 999).unwrap(), None);
    }

    #[test]
    fn 没派过就给none不给空串() {
        let c = crate::store::open_in_memory().unwrap();
        // None 与「标准是空的」要分得开：前者是没派过，后者是派了但没写
        assert_eq!(latest_work_standard(&c, "intake").unwrap(), None);
        assert_eq!(latest_task_id(&c, "intake").unwrap(), None);
    }

    #[test]
    fn 同一任务再取一次是覆盖不是新增() {
        let c = crate::store::open_in_memory().unwrap();
        put(&c, &snap(7, "intake", "一稿")).unwrap();
        let mut s = snap(7, "intake", "二稿");
        s.status = "returned".into();
        put(&c, &s).unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM task_mirror", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            latest_work_standard(&c, "intake").unwrap().as_deref(),
            Some("二稿")
        );
        let st: String = c
            .query_row("SELECT status FROM task_mirror WHERE task_id=7", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "returned");
    }
}
