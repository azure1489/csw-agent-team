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
/// 其中跨品牌的相似案例各至多几条：同品牌的优先，跨品牌的只补空位。
pub const CROSS_PER_DECISION: usize = 2;
/// 跨品牌案例的最低相似度（案例「品牌｜对象」的文本向量 与 候选融合向量 的余弦）。
/// 低于它的不送：拿不相干的案例比「调性」，模型只会顺着字面联想。
pub const MIN_SIMILARITY: f32 = 0.45;

/// 一条可送的案例（采用或否决）。**没有原话这一列**——根本不读。
#[derive(Debug, Clone)]
pub struct CaseRow {
    pub key: String,
    pub decision: String,
    pub brand: String,
    pub title: String,
    pub judged: String,
    pub decided_at: Option<String>,
    pub url: String,
}

impl CaseRow {
    /// 算向量用的文本：品牌与对象
    pub fn embed_text(&self) -> String {
        format!("{}｜{}", self.brand, self.title)
    }
}

/// 读全部采用 / 否决案例，按决定时间倒序。
pub fn load_cases(conn: &Connection) -> Result<Vec<CaseRow>> {
    let mut st = conn.prepare(
        "SELECT case_key, decision, brand, title, judged_tier, decided_at, source_url
         FROM memory_cases WHERE decision IN ('adopted','rejected')
         ORDER BY decided_at DESC, case_key",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(CaseRow {
                key: r.get(0)?,
                decision: r.get(1)?,
                brand: r.get(2)?,
                title: r.get(3)?,
                judged: r.get(4)?,
                decided_at: r.get(5)?,
                url: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 挑这条候选的选题记忆（只按同品牌），见 [`pick`]。
pub fn materials_for(
    conn: &Connection,
    brand_keys: &[String],
    self_url: &str,
) -> Result<Vec<Material>> {
    Ok(pick(&load_cases(conn)?, brand_keys, self_url, None))
}

/// 挑这条候选的选题记忆：**同品牌的先取**（按决定时间倒序），采用、否决各至多 [`PER_DECISION`] 条；
/// 空位再由**跨品牌、对象相近**的案例补（`sims` 与 `rows` 一一对应，相似度不低于
/// [`MIN_SIMILARITY`]，各至多 [`CROSS_PER_DECISION`] 条）。
///
/// 只按同品牌挑的时候，大多数候选一条案例都拿不到（09-25 M3：7 条推荐 / 备选的 csw 依据
/// 全是「选题记忆无相关」），Van 否决过的同类对象判断根本看不见。
///
/// **这条贴文自己的案例不送**（`self_url`）：判一条贴文时拿 Van 对它本身的决定当材料，
/// 等于把答案递给模型——重判、回测时尤其如此。
pub fn pick(
    rows: &[CaseRow],
    brand_keys: &[String],
    self_url: &str,
    sims: Option<&[f32]>,
) -> Vec<Material> {
    let own = csw_collector_kb::docs::norm_url(self_url);
    let not_self =
        |c: &CaseRow| self_url.is_empty() || csw_collector_kb::docs::norm_url(&c.url) != own;
    let same_brand =
        |c: &CaseRow| !c.brand.trim().is_empty() && brand_keys.contains(&brand_key(&c.brand));
    let mut out = Vec::new();
    let mut taken = [0usize; 2];
    let slot = |d: &str| usize::from(d != "adopted");
    for c in rows.iter().filter(|c| same_brand(c) && not_self(c)) {
        let n = &mut taken[slot(&c.decision)];
        if *n < PER_DECISION {
            *n += 1;
            out.push(material(c, false));
        }
    }
    if let Some(sims) = sims.filter(|s| s.len() == rows.len()) {
        let mut cross: Vec<(usize, f32)> = rows
            .iter()
            .enumerate()
            .filter(|(_, c)| !same_brand(c) && not_self(c))
            .map(|(i, _)| (i, sims[i]))
            .filter(|(_, s)| *s >= MIN_SIMILARITY)
            .collect();
        cross.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut added = [0usize; 2];
        for (i, _) in cross {
            let c = &rows[i];
            let k = slot(&c.decision);
            if taken[k] < PER_DECISION && added[k] < CROSS_PER_DECISION {
                taken[k] += 1;
                added[k] += 1;
                out.push(material(c, true));
            }
        }
    }
    out
}

fn material(c: &CaseRow, cross_brand: bool) -> Material {
    Material {
        kind: MaterialKind::Memory,
        ref_id: c.key.clone(),
        title: if cross_brand {
            format!("{}｜{}（跨品牌的相似对象）", c.brand, c.title)
        } else {
            format!("{}｜{}", c.brand, c.title)
        },
        date: c.decided_at.as_deref().and_then(|s| s.parse().ok()),
        source: String::new(),
        // **不送原话**：这一列根本没读
        quote: String::new(),
        publish_state: format!(
            "Van {}{}",
            if c.decision == "adopted" {
                "采用"
            } else {
                "否决"
            },
            if c.judged.trim().is_empty() {
                String::new()
            } else {
                format!("（当时系统判 {}）", c.judged)
            }
        ),
        body_excerpt: String::new(),
        body_available: true,
        brand: c.brand.clone(),
    }
}

/// 余弦相似度。长度不同或有零向量时给 0。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
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
        let ms = materials_for(&c, &["dappleborn".into()], "").unwrap();
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
    fn 跨品牌只补空位且要够相似() {
        let c = conn();
        let rows = load_cases(&c).unwrap();
        // 全部给高相似度：同品牌之外的也能补进来，但各至多两条
        let hi = vec![0.9; rows.len()];
        let ms = pick(&rows, &[], "", Some(&hi));
        assert!(!ms.is_empty());
        assert!(ms.iter().all(|m| m.title.contains("跨品牌")));
        assert!(ms.iter().all(|m| m.quote.is_empty()), "原话不送");
        let rej = ms
            .iter()
            .filter(|m| m.publish_state.contains("否决"))
            .count();
        let ado = ms
            .iter()
            .filter(|m| m.publish_state.contains("采用"))
            .count();
        assert!(rej <= CROSS_PER_DECISION && ado <= CROSS_PER_DECISION);
        // 不够相似的一条都不送
        let lo = vec![0.1; rows.len()];
        assert!(pick(&rows, &[], "", Some(&lo)).is_empty());
        // 长度对不上当没给
        assert!(pick(&rows, &[], "", Some(&[0.9])).is_empty());
    }

    #[test]
    fn 余弦() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

    #[test]
    fn 没有品牌命中就不送() {
        assert!(materials_for(&conn(), &[], "").unwrap().is_empty());
    }

    #[test]
    fn 这条贴文自己的案例不送() {
        let c = conn();
        c.execute(
            "UPDATE memory_cases SET source_url = 'https://www.instagram.com/p/ABC/' WHERE case_key = 'c1'",
            [],
        )
        .unwrap();
        let ms =
            materials_for(&c, &["dappleborn".into()], "https://instagram.com/reel/ABC").unwrap();
        assert!(!ms.iter().any(|m| m.ref_id == "c1"));
    }

    #[test]
    fn 准则卡只送确认过的() {
        let rules = confirmed_rules(&conn()).unwrap();
        assert_eq!(rules, ["（降低优先级）门店活动不当产品新闻"]);
    }
}
