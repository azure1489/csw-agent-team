//! 选题记忆接进判断（09-22 判断台账反馈第七项）。
//!
//! P5 回收的 Van 采用 / 否决案例与准则卡此前只在「选题记忆」页上给人看，
//! **判断那一步一条都没读**——模型只能拿抽象的六维去填理由。
//!
//! # 送什么、不送什么（09-23 用户拍板）
//!
//! - 案例：**只送「对象 + 结论」**（案例标题、采用 / 否决、当时系统给的档）。
//!   **Van 的原话一律不送**，`quote` 这一列在这里连读都不读。
//! - 准则卡：**只送 Van 确认过的**。归纳出来、她还没点头的一张都不送——
//!   没确认的归纳当依据，等于让猜测冒名顶替。
//!
//! 案例全是选题决定（引擎 `selection_cases.decision` 只有采用 / 否决 / 暂缓 / 待核），
//! 文风修改在 P5 里归进准则卡而不进案例，所以这里不必再按「选题 / 文风」过滤。

use anyhow::Result;
use rusqlite::{Connection, params};

use csw_collector_core::types::{Material, MaterialKind};
use csw_collector_kb::brands::brand_key;

/// 采用、否决各至多几条。
pub const PER_DECISION: usize = 3;

/// 挑这条候选的选题记忆：同品牌的先取，按决定时间倒序，采用、否决各至多 [`PER_DECISION`] 条。
///
/// 没有同品牌的案例就不送——跨品牌的案例拿来比「调性」，模型只会顺着字面联想。
pub fn materials_for(conn: &Connection, brand_keys: &[String]) -> Result<Vec<Material>> {
    if brand_keys.is_empty() {
        return Ok(Vec::new());
    }
    // 案例表几百行，全读出来按品牌键比，比在 SQL 里做归一化简单也可靠
    let mut st = conn.prepare(
        "SELECT case_key, decision, brand, title, judged_tier, decided_at
         FROM memory_cases WHERE decision IN ('adopted','rejected')
         ORDER BY decided_at DESC, case_key",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut adopted = 0;
    let mut rejected = 0;
    let mut out = Vec::new();
    for row in rows {
        let (key, decision, brand, title, judged, decided_at) = row?;
        if brand.trim().is_empty() || !brand_keys.contains(&brand_key(&brand)) {
            continue;
        }
        let n = if decision == "adopted" {
            &mut adopted
        } else {
            &mut rejected
        };
        if *n >= PER_DECISION {
            continue;
        }
        *n += 1;
        out.push(Material {
            kind: MaterialKind::Memory,
            ref_id: key,
            title: format!("{brand}｜{title}"),
            date: decided_at.as_deref().and_then(|s| s.parse().ok()),
            source: String::new(),
            // **不送原话**：这一列根本没读
            quote: String::new(),
            publish_state: format!(
                "Van {}{}",
                if decision == "adopted" {
                    "采用"
                } else {
                    "否决"
                },
                if judged.trim().is_empty() {
                    String::new()
                } else {
                    format!("（当时系统判 {judged}）")
                }
            ),
            body_excerpt: String::new(),
            body_available: true,
            brand,
        });
    }
    Ok(out)
}

/// Van **确认过**的准则卡正文，按类别与键排序。没确认的一张都不给。
pub fn confirmed_rules(conn: &Connection) -> Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT category, text FROM memory_rules
         WHERE confirmed_by_van = 1 AND trim(text) <> ''
         ORDER BY category, rule_key",
    )?;
    let rows = st.query_map(params![], |r| {
        let cat: String = r.get(0)?;
        let text: String = r.get(1)?;
        Ok(if cat.is_empty() {
            text
        } else {
            format!("（{}）{text}", category_name(&cat))
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn category_name(c: &str) -> &str {
    match c {
        "frame" => "判断框架",
        "prefer" => "优先关注",
        "lower" => "降低优先级",
        "dedup" => "查重口径",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let case = |k: &str, d: &str, brand: &str, t: &str, at: &str| {
            c.execute(
                "INSERT INTO memory_cases(case_key, decision, quote, source_url, decided_at,
                                          updated_at, brand, title, judged_tier)
                 VALUES (?1,?2,'这条角度太旧，不要','',?3,'x',?4,?5,'recommend')",
                params![k, d, at, brand, t],
            )
            .unwrap();
        };
        case("c1", "adopted", "Dapple Born", "折叠桌", "2026-09-10");
        case("c2", "rejected", "DAPPLE BORN", "新色", "2026-09-11");
        case("c3", "rejected", "norda", "跑鞋", "2026-09-12");
        case("c4", "deferred", "Dapple Born", "暂缓的", "2026-09-13");
        for i in 0..5 {
            case(
                &format!("r{i}"),
                "rejected",
                "dappleborn",
                &format!("否{i}"),
                &format!("2026-08-0{i}"),
            );
        }
        c.execute(
            "INSERT INTO memory_rules(rule_key, text, version, confirmed_by_van, updated_at, category)
             VALUES ('a','门店活动不当产品新闻','1',1,'x','lower'),
                    ('b','还没确认的归纳','1',0,'x','frame')",
            [],
        )
        .unwrap();
        c
    }

    #[test]
    fn 只送同品牌的采用与否决且不带原话() {
        let c = conn();
        let ms = materials_for(&c, &["dappleborn".into()]).unwrap();
        // 采用 1 条 + 否决至多 3 条；暂缓的不送；别的品牌不送
        assert_eq!(
            ms.iter()
                .filter(|m| m.publish_state.contains("采用"))
                .count(),
            1
        );
        assert_eq!(
            ms.iter()
                .filter(|m| m.publish_state.contains("否决"))
                .count(),
            3
        );
        assert!(ms.iter().all(|m| m.kind == MaterialKind::Memory));
        assert!(
            !ms.iter()
                .any(|m| m.title.contains("跑鞋") || m.title.contains("暂缓"))
        );
        // **原话一个字都不能出现**
        for m in &ms {
            assert!(m.quote.is_empty());
            assert!(!m.title.contains("角度太旧") && !m.publish_state.contains("角度太旧"));
        }
        let block = crate::materials::as_prompt_block(&ms);
        assert!(!block.contains("角度太旧"), "{block}");
        // 最新的否决在前
        assert!(ms.iter().any(|m| m.title.ends_with("新色")));
    }

    #[test]
    fn 没有品牌命中就不送() {
        assert!(materials_for(&conn(), &[]).unwrap().is_empty());
    }

    #[test]
    fn 准则卡只送确认过的() {
        let rules = confirmed_rules(&conn()).unwrap();
        assert_eq!(rules, ["（降低优先级）门店活动不当产品新闻"]);
    }
}
