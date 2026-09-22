//! 待补录清单：**出现在选题里、却不在 csw 在册名单里的品牌。**
//!
//! 来源不全，覆盖做到 100% 也命中不了 Van 的选择（总方案「来源不全」那条风险）。
//! 这张清单给人去 csw 后台补录账号用——补录由人操作，这里只列、只给依据。
//!
//! 「在册」的判据是别名表：它就是从 csw `/accounts` 建出来的。一个品牌算在册，要么
//! 它的品牌键（[`crate::brands::brand_key`]）与某个在册品牌或别名的键相同，要么
//! 它的标题里本来就提到了某个在册别名（`FUTURE FOX炉顶附件` 这种写法也认）。
//!
//! 「出现在选题里」看三类：03 的决定、已发条目、范例。已生成贴文不看——它本来就是
//! 从在册账号来的，拿它对账永远是齐的。
//!
//! **决定的正文里有 Van 的原话**，这里只取第一行的结论码（`结论：written`），
//! 不读别的，依据里也只放标题、日期、链接。

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::brands::{BrandIndex, brand_key};

/// 一条依据
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Evidence {
    pub kind: String,
    pub title: String,
    pub date: Option<String>,
    pub url: String,
    /// 决定的结论码；已发条目与范例是空
    pub conclusion: String,
}

/// 清单上的一行
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BackfillRow {
    /// 最常见的那种写法
    pub brand: String,
    /// 被选中过几次：写了、批准写、已发、范例
    pub adopted: usize,
    /// 一共出现几次
    pub mentions: usize,
    pub latest: Option<String>,
    /// 最近的几条依据
    pub evidence: Vec<Evidence>,
}

/// 依据最多给几条
const EVIDENCE_MAX: usize = 5;

/// 这几种结论算「被选中过」。`dropped`、`rejected` 也列出来——
/// 进过选题说明来源值得看，只是排在后面。
const ADOPTED: [&str; 3] = ["written", "approved_write", "published"];

pub fn backfill_list(conn: &Connection, brands: &BrandIndex) -> Result<Vec<BackfillRow>> {
    let known: HashSet<String> = brands.keys();

    let mut stmt = conn.prepare(
        "SELECT kind, brand, title, published_at, url,
                CASE WHEN kind = 'decision' THEN substr(body, 1, 40) ELSE '' END
           FROM kb_docs
          WHERE kind IN ('decision', 'published_item', 'example')
            AND brand <> ''",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // key → (写法计数, 行)
    let mut groups: BTreeMap<String, (BTreeMap<String, usize>, Vec<Evidence>)> = BTreeMap::new();
    for (kind, brand, title, date, url, head) in rows {
        // 冒烟测试留下的假条目
        if url.contains("example.com") {
            continue;
        }
        let key = brand_key(&brand);
        if key.is_empty() || known.contains(&key) {
            continue;
        }
        if !brands.hits(&format!("{brand} {title}")).is_empty() {
            continue;
        }
        let g = groups.entry(key).or_default();
        *g.0.entry(brand).or_default() += 1;
        g.1.push(Evidence {
            kind,
            title,
            date: date.filter(|d| looks_like_date(d)),
            url,
            conclusion: conclusion(&head),
        });
    }

    let mut out: Vec<BackfillRow> = groups
        .into_values()
        .map(|(spellings, mut ev)| {
            ev.sort_by(|a, b| b.date.cmp(&a.date));
            let brand = spellings
                .iter()
                .max_by_key(|(s, n)| (**n, std::cmp::Reverse((*s).clone())))
                .map(|(s, _)| s.clone())
                .unwrap_or_default();
            let adopted = ev
                .iter()
                .filter(|e| e.kind != "decision" || ADOPTED.contains(&e.conclusion.as_str()))
                .count();
            BackfillRow {
                brand,
                adopted,
                mentions: ev.len(),
                latest: ev.iter().find_map(|e| e.date.clone()),
                evidence: ev.into_iter().take(EVIDENCE_MAX).collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (b.adopted, b.mentions, &b.latest)
            .cmp(&(a.adopted, a.mentions, &a.latest))
            .then_with(|| a.brand.cmp(&b.brand))
    });
    Ok(out)
}

/// `结论：written\n品牌：…` → `written`。只看第一行。
fn conclusion(head: &str) -> String {
    head.lines()
        .next()
        .and_then(|l| l.strip_prefix("结论："))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn looks_like_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brands::Alias;
    use crate::docs::{KbDoc, upsert};

    fn put(conn: &Connection, kind: &str, r: &str, brand: &str, title: &str, body: &str) {
        upsert(
            conn,
            &KbDoc {
                kind: kind.into(),
                ref_id: r.into(),
                brand: brand.into(),
                title: title.into(),
                body: body.into(),
                url: format!("https://www.instagram.com/p/{r}/"),
                published_at: Some("2026-09-15T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn 在册的不列_没在册的按被选中排前() {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let idx = BrandIndex::new(
            [("SNOW PEAK", "snowpeak_official"), ("TRIPATH", "tripath")]
                .into_iter()
                .filter_map(|(b, a)| Alias::new(b, a))
                .collect(),
        );
        // 在册：键相同
        put(
            &conn,
            "decision",
            "a",
            "snowpeak",
            "Snow Peak新帐",
            "结论：written",
        );
        // 在册：标题里提到在册别名
        put(
            &conn,
            "decision",
            "b",
            "tp",
            "tripath燕子钩",
            "结论：dropped",
        );
        // 没在册：被写过一次、淘汰过一次
        put(
            &conn,
            "decision",
            "c",
            "futurefox",
            "FUTURE FOX炉顶附件",
            "结论：written\n原话：不该出现",
        );
        put(
            &conn,
            "decision",
            "d",
            "future-fox",
            "FUTURE FOX预告",
            "结论：dropped",
        );
        // 没在册：只淘汰过
        put(
            &conn,
            "decision",
            "e",
            "kaname",
            "KANAME东京出店",
            "结论：dropped",
        );
        // 已生成贴文不参与对账
        put(&conn, "generated_post", "f", "nobody", "x", "y");

        let rows = backfill_list(&conn, &idx).unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.brand.as_str()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert_eq!(rows[0].mentions, 2, "两种写法并成一行");
        assert_eq!(rows[0].adopted, 1);
        assert_eq!(rows[1].brand, "kaname");
        assert_eq!(rows[1].adopted, 0);
        // 依据里不带原话
        let json = serde_json::to_string(&rows).unwrap();
        assert!(!json.contains("不该出现"), "{json}");
    }

    #[test]
    fn 结论只取第一行() {
        assert_eq!(conclusion("结论：written\n品牌：x"), "written");
        assert_eq!(conclusion("品牌：x"), "");
    }
}
