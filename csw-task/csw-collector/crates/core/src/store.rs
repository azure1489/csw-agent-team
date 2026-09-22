//! 本地 SQLite 状态库：开库、加 pragma、跑迁移。
//!
//! **单写者。** 所有写走同一个连接，读可以多开。这不是性能取舍——
//! LanceDB 也是单写者，两边保持同一个模型，崩溃后的恢复路径才只有一条。
//!
//! 所有「写引擎」的动作先落 `engine_outbox` 再发，保证崩溃后可续、可幂等重放。

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 迁移按顺序 apply，版本记在 `PRAGMA user_version`。
/// 加新迁移就在这里追一行，**不要改已发布的那几条**。
const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_workbench.sql")),
    (3, include_str!("../migrations/0003_exclusions.sql")),
];

/// 当前 schema 版本。与 `MIGRATIONS` 最后一项对齐。
pub const SCHEMA_VERSION: u32 = 3;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let conn = Connection::open(path).with_context(|| format!("打开 {}", path.display()))?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 只在测试里用
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // WAL：读不挡写。采集轮在写的时候工作台还要能刷页面。
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // 外键是真开着的——candidates 与 media 的引用完整性靠它，不靠代码自觉。
    conn.pragma_update(None, "foreign_keys", true)?;
    // 崩溃只会丢最后几个事务，换来快一个数量级的写入。本地状态丢一点可重算。
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32;
    for (v, sql) in MIGRATIONS {
        if *v > current {
            conn.execute_batch(sql)
                .with_context(|| format!("迁移 {v} 失败"))?;
            conn.pragma_update(None, "user_version", *v as i64)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(conn: &Connection) -> Vec<String> {
        let mut st = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        let rows = st.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.filter_map(Result::ok).collect()
    }

    #[test]
    fn 迁移建出全部表且版本落在最新() {
        let conn = open_in_memory().unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v as u32, SCHEMA_VERSION);
        let t = tables(&conn);
        for want in [
            "rounds",
            "round_steps",
            "candidates",
            "round_candidates",
            "media",
            "media_descriptions",
            "embeddings_log",
            "events",
            "materials",
            "triages",
            "judgements",
            "judgement_overrides",
            "deepchecks",
            "model_calls",
            "engine_outbox",
            "deliverables_local",
            "task_mirror",
            "sessions",
            "van_marks",
            "plan_versions",
            "audit",
            "kb_docs",
            "kb_fts",
            "brands",
            "brand_aliases",
            "kb_cursors",
            "memory_rules",
            "memory_cases",
            "work_queue",
        ] {
            assert!(t.iter().any(|x| x == want), "缺表 {want}；实有 {t:?}");
        }
    }

    #[test]
    fn 迁移可重复跑() {
        let dir = std::env::temp_dir().join(format!("csw-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("a.db");
        let n1 = tables(&open(&path).unwrap()).len();
        let n2 = tables(&open(&path).unwrap()).len();
        assert_eq!(n1, n2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 没读到实图的判断落不进非待核档() {
        let conn = open_in_memory().unwrap();
        seed_round(&conn);
        let ins = |tier: &str, seen: i64| {
            conn.execute(
                "INSERT INTO judgements(round_id,candidate_key,tier,dims_json,three_json,
                     comparison_json,image_seen,inputs_hash,model,rubric_version,created_at)
                 VALUES (1,?1,?2,'{}','{}','{}',?3,'h','m','v','t')",
                rusqlite::params![format!("k{tier}{seen}"), tier, seen],
            )
        };
        assert!(ins("recommend", 1).is_ok());
        assert!(ins("pending_check", 0).is_ok());
        // 这条是库层面的最后一道闸：代码漏判了，CHECK 也会拦住
        assert!(ins("not_recommend", 0).is_err());
    }

    #[test]
    fn 同一任务同一版本同一触发只许开一轮() {
        let conn = open_in_memory().unwrap();
        seed_round(&conn);
        let again = conn.execute(
            "INSERT INTO rounds(id,kind,trigger,task_id,target_version,window_start,window_end,
                 plan_version,rubric_version,kb_snapshot,status,created_at)
             VALUES (2,'task','dispatch',77,1,'a','b',1,'v','s','running','t')",
            [],
        );
        assert!(again.is_err(), "重复开轮没被挡住");
        // 换个触发原因（退回返工）可以另开一轮
        assert!(
            conn.execute(
                "INSERT INTO rounds(id,kind,trigger,task_id,target_version,window_start,window_end,
                     plan_version,rubric_version,kb_snapshot,status,created_at)
                 VALUES (3,'task','returned',77,1,'a','b',1,'v','s','running','t')",
                [],
            )
            .is_ok()
        );
    }

    #[test]
    fn 外键真的开着() {
        let conn = open_in_memory().unwrap();
        seed_round(&conn);
        let orphan = conn.execute(
            "INSERT INTO round_candidates(round_id,candidate_key,collector)
             VALUES (1,'不存在的候选','csw_window')",
            [],
        );
        assert!(orphan.is_err(), "外键没生效");
    }

    fn seed_round(conn: &Connection) {
        conn.execute(
            "INSERT INTO rounds(id,kind,trigger,task_id,target_version,window_start,window_end,
                 plan_version,rubric_version,kb_snapshot,status,created_at)
             VALUES (1,'task','dispatch',77,1,'2026-09-22T00:00:00Z','2026-09-22T12:00:00Z',
                 1,'v9','snap','running','2026-09-22T00:00:00Z')",
            [],
        )
        .unwrap();
    }
}
