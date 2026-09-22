//! 硬性排除：Van 在 03 否决过的东西，同事实同角度的不再端上来。
//!
//! # 它和「改档」不是一回事
//!
//! 改档是人看过模型的结论之后换一档；排除是**在模型看见之前**就把这一条挡下来。
//! 所以排除产出的不是判断——被排除的条目没有六维、没有三句话，硬给它编一份
//! 只会让台账分不清「模型判了不推荐」和「规则排掉的」。它单独成一组，
//! 依据写的是 Van 的原话。
//!
//! # 为什么要有它
//!
//! Van 否过的角度会一而再地被同一批账号发出来。每一轮都让模型重判一遍，
//! 既花钱又得到同一个答案，而且模型未必每次都想起 Van 说过什么。
//!
//! # 宁可漏排，不可错排
//!
//! 排错一条，一条本该推荐的就不进判断了；漏排一条，不过是多花一次模型调用。
//! 两边的代价差着量级，所以：
//!
//! - 粗筛只认**同品牌**。跨品牌不可能是同一件事实，也就不问。
//!   代价是品牌抽不出来的候选永远不会被排除——认了。
//! - 判据要**同一事实 _且_ 同一角度**，两个都过 [`THRESHOLD`] 才排。
//!   Van 否的常常是角度（「这种普通上新没看点」），不是这个品牌。
//! - 被排除的条目**照样完整登记在台账上**，只是单独一组。它不会消失，
//!   主编能看见「这三条被排掉了，因为 Van 说过 X」，也能当场捞回。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// 同一事实与同一角度都要过这条线才排除。
///
/// 比合并的 [`SAME_EVENT_THRESHOLD`](../../judge/merge) 高得多是故意的：
/// 合并错了两条并成一组、都还在台账上，排除错了那一条就不进判断了。
pub const THRESHOLD: f64 = 0.85;

/// 一条排除规则 = 一次 Van 的否决。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Exclusion {
    pub id: i64,
    /// `run_id#item_key`
    pub decision_ref: String,
    pub item_key: String,
    pub title: String,
    pub brand: String,
    pub source_url: String,
    /// Van 的原话。空的话 `active` 必为 0
    pub quote: String,
    pub reason: String,
    pub reason_code: String,
    pub decided_at: String,
    pub actor_role: String,
    pub active: bool,
    pub inactive_reason: String,
    pub changed_by: String,
    pub changed_at: String,
}

/// 没有原话时落的停用理由。**不是错误**——决定是真的，只是没留下话。
pub const NO_QUOTE: &str = "这次否决没有留下原话，需要人工确认后才参与排除";

/// 登记一条否决。已经在表里的按 `decision_ref` 更新内容，
/// **但不碰 `active`**——人工停用过的规则不该被一次同步又打开。
///
/// 返回是不是新增的。
pub fn upsert(conn: &Connection, e: &Exclusion) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM exclusions WHERE decision_ref = ?1",
            [&e.decision_ref],
            |r| r.get(0),
        )
        .optional()
        .context("查排除规则")?;
    if let Some(id) = existing {
        conn.execute(
            "UPDATE exclusions SET item_key=?2, title=?3, brand=?4, source_url=?5,
                    quote=?6, reason=?7, reason_code=?8, decided_at=?9, actor_role=?10
             WHERE id=?1",
            params![
                id,
                e.item_key,
                e.title,
                e.brand,
                e.source_url,
                e.quote,
                e.reason,
                e.reason_code,
                e.decided_at,
                e.actor_role
            ],
        )
        .context("更新排除规则")?;
        return Ok(false);
    }
    // 新登记：有原话才自动生效
    let has_quote = !e.quote.trim().is_empty();
    conn.execute(
        "INSERT INTO exclusions
           (decision_ref, item_key, title, brand, source_url, quote, reason, reason_code,
            decided_at, actor_role, active, inactive_reason, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
        params![
            e.decision_ref,
            e.item_key,
            e.title,
            e.brand,
            e.source_url,
            e.quote,
            e.reason,
            e.reason_code,
            e.decided_at,
            e.actor_role,
            has_quote as i64,
            if has_quote { "" } else { NO_QUOTE },
        ],
    )
    .context("登记排除规则")?;
    Ok(true)
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Exclusion> {
    Ok(Exclusion {
        id: r.get(0)?,
        decision_ref: r.get(1)?,
        item_key: r.get(2)?,
        title: r.get(3)?,
        brand: r.get(4)?,
        source_url: r.get(5)?,
        quote: r.get(6)?,
        reason: r.get(7)?,
        reason_code: r.get(8)?,
        decided_at: r.get(9)?,
        actor_role: r.get(10)?,
        active: r.get::<_, i64>(11)? != 0,
        inactive_reason: r.get(12)?,
        changed_by: r.get(13)?,
        changed_at: r.get(14)?,
    })
}

const COLS: &str = "id, decision_ref, item_key, title, brand, source_url, quote, reason,
                    reason_code, decided_at, actor_role, active, inactive_reason,
                    changed_by, changed_at";

/// 全部规则，停用的也在里面——工作台那一页要把它们都摆出来。
pub fn all(conn: &Connection) -> Result<Vec<Exclusion>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM exclusions ORDER BY active DESC, decided_at DESC, id DESC"
    ))?;
    let rows = st.query_map([], row)?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 生效中的规则。判断那一步只看这些。
pub fn active(conn: &Connection) -> Result<Vec<Exclusion>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM exclusions WHERE active = 1 ORDER BY id"
    ))?;
    let rows = st.query_map([], row)?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 开或关一条规则。关掉 = 这次否决过时了，别再拿它挡东西。
pub fn set_active(
    conn: &Connection,
    id: i64,
    on: bool,
    reason: &str,
    actor: &str,
) -> Result<usize> {
    conn.execute(
        "UPDATE exclusions
         SET active=?2, inactive_reason=?3, changed_by=?4,
             changed_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
         WHERE id=?1",
        params![id, on as i64, if on { "" } else { reason }, actor],
    )
    .context("改排除规则开关")
}

/// 一轮里的一次命中。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    pub candidate_key: String,
    pub exclusion_id: i64,
    pub same_fact: f64,
    /// 新料的概率。**越高越不该排**
    pub new_substance: f64,
    pub restored: bool,
    pub restored_by: String,
    pub restored_reason: String,
}

/// 记下这一轮排掉了哪条。
pub fn record_hit(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
    exclusion_id: i64,
    same_fact: f64,
    new_substance: f64,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO exclusion_hits
           (round_id, candidate_key, exclusion_id, same_fact, new_substance, created_at)
         VALUES (?1,?2,?3,?4,?5, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
        params![
            round_id,
            candidate_key,
            exclusion_id,
            same_fact,
            new_substance
        ],
    )
    .context("记排除命中")?;
    Ok(())
}

/// 这一轮的全部命中。
pub fn hits(conn: &Connection, round_id: i64) -> Result<Vec<Hit>> {
    let mut st = conn.prepare(
        "SELECT candidate_key, exclusion_id, same_fact, new_substance,
                restored, restored_by, restored_reason
         FROM exclusion_hits WHERE round_id = ?1 ORDER BY candidate_key",
    )?;
    let rows = st.query_map([round_id], |r| {
        Ok(Hit {
            candidate_key: r.get(0)?,
            exclusion_id: r.get(1)?,
            same_fact: r.get(2)?,
            new_substance: r.get(3)?,
            restored: r.get::<_, i64>(4)? != 0,
            restored_by: r.get(5)?,
            restored_reason: r.get(6)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 主编捞回一条：**只对这一轮这一条生效**，规则本身还在。
///
/// 捞回之后这一条要重新走判断——它当初根本没判过，没有结论可以拿来改档。
pub fn restore(
    conn: &Connection,
    round_id: i64,
    candidate_key: &str,
    actor: &str,
    reason: &str,
) -> Result<usize> {
    conn.execute(
        "UPDATE exclusion_hits SET restored=1, restored_by=?3, restored_reason=?4
         WHERE round_id=?1 AND candidate_key=?2",
        params![round_id, candidate_key, actor, reason],
    )
    .context("捞回被排除的条目")
}

/// 这一轮真正被挡下的条目键（捞回的不算）。
pub fn excluded_keys(conn: &Connection, round_id: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT candidate_key FROM exclusion_hits
         WHERE round_id = ?1 AND restored = 0 ORDER BY candidate_key",
    )?;
    let rows = st.query_map([round_id], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::open_in_memory;

    fn ex(r: &str, quote: &str) -> Exclusion {
        Exclusion {
            id: 0,
            decision_ref: r.into(),
            item_key: "snow-peak-a1b2c3".into(),
            title: "某品牌普通上新".into(),
            brand: "snow peak".into(),
            source_url: "https://x/1".into(),
            quote: quote.into(),
            reason: "没有看点".into(),
            reason_code: "no_angle".into(),
            decided_at: "2026-09-01T00:00:00Z".into(),
            actor_role: "van".into(),
            active: true,
            inactive_reason: String::new(),
            changed_by: String::new(),
            changed_at: String::new(),
        }
    }

    #[test]
    fn 没有原话的否决进表但不自动生效() {
        let c = open_in_memory().unwrap();
        assert!(upsert(&c, &ex("1#a", "这种普通上新不要")).unwrap());
        assert!(upsert(&c, &ex("2#b", "   ")).unwrap());

        // 两条都在表里——否决是真的，不能因为没原话就当没发生
        assert_eq!(all(&c).unwrap().len(), 2);
        // 但只有带原话的那条参与排除：拿不出 Van 的话就解释不了它为什么挡人
        let on = active(&c).unwrap();
        assert_eq!(on.len(), 1);
        assert_eq!(on[0].decision_ref, "1#a");
        let off = all(&c).unwrap().into_iter().find(|e| !e.active).unwrap();
        assert_eq!(off.inactive_reason, NO_QUOTE);
    }

    #[test]
    fn 再同步一次不会把人工停用的规则打开() {
        let c = open_in_memory().unwrap();
        upsert(&c, &ex("1#a", "不要")).unwrap();
        let id = all(&c).unwrap()[0].id;
        set_active(&c, id, false, "这个角度现在又想要了", "主编").unwrap();

        // 同一条决定再同步一遍：内容更新，开关不动
        let mut again = ex("1#a", "不要");
        again.title = "改过的标题".into();
        assert!(!upsert(&c, &again).unwrap());

        let got = &all(&c).unwrap()[0];
        assert_eq!(got.title, "改过的标题");
        assert!(!got.active, "人工停用过的规则不该被同步又打开");
        assert_eq!(got.changed_by, "主编");
    }

    #[test]
    fn 捞回只影响这一轮这一条() {
        let c = open_in_memory().unwrap();
        upsert(&c, &ex("1#a", "不要")).unwrap();
        let id = all(&c).unwrap()[0].id;
        record_hit(&c, 7, "k1", id, 0.95, 0.91).unwrap();
        record_hit(&c, 7, "k2", id, 0.88, 0.90).unwrap();
        record_hit(&c, 8, "k1", id, 0.93, 0.92).unwrap();

        restore(&c, 7, "k1", "主编", "这次确实有新料").unwrap();

        assert_eq!(excluded_keys(&c, 7).unwrap(), vec!["k2".to_string()]);
        // 另一轮的同一条不受影响，规则也还开着
        assert_eq!(excluded_keys(&c, 8).unwrap(), vec!["k1".to_string()]);
        assert!(active(&c).unwrap().len() == 1);

        let h = hits(&c, 7).unwrap();
        let k1 = h.iter().find(|x| x.candidate_key == "k1").unwrap();
        assert!(k1.restored && k1.restored_by == "主编");
        // 判据值留着：台账要能回答「它当初为什么被挡」
        assert!((k1.same_fact - 0.95).abs() < 1e-9);
    }
}
