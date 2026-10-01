//! 主编校准覆盖层：主编对具体条目定过的去向，工作台**直接采用**，不交给模型重判，
//! 登记时也不把工作台自己的状态写回去盖掉。
//!
//! 两个来源，后者覆盖前者：
//! 1. 历次退回意见里写明的「条目键 = 待核 / 备选 / 停止 / 继续」（按版本先后，后说的算）；
//! 2. 引擎里主编亲手改的条目状态（`intake-trace` 的 traces，actor_role = 中枢）。
//!
//! 09-30 r58 #614：主编 16:34 定了首批八条（四继续、一备选、两待核、一停止）并改了引擎条目，
//! 工作台返工时把点名条目交给模型重判、又把自己的状态登记回引擎，Coleman 冷藏箱从「继续」
//! 变成不推荐并撤下，drlv 从待核改回 shortlisted——主编连退十几版。

use std::sync::LazyLock;

use csw_collector_core::types::Tier;
use csw_collector_engineapi::types::TaskDetail;
use regex::Regex;

/// 主编对一个条目的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calibration {
    pub key: String,
    pub tier: Tier,
    /// 这个结论从哪来：「v6 退回意见」「引擎条目（主编 2026-09-30T08:34:02Z）」
    pub source: String,
}

impl Calibration {
    /// 给人看的去向
    pub fn label(&self) -> &'static str {
        match self.tier {
            Tier::Recommend => "继续",
            Tier::Alternate => "备选",
            Tier::PendingCheck => "待核",
            Tier::NotRecommend => "停止",
        }
    }
}

static KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z0-9_.]+-[0-9a-f]{6}").expect("正则"));

/// 状态词，长的在前（「不推荐」要先于「推荐」认）
const WORDS: [(&str, Tier); 7] = [
    ("不推荐", Tier::NotRecommend),
    ("停止", Tier::NotRecommend),
    ("淘汰", Tier::NotRecommend),
    ("待核", Tier::PendingCheck),
    ("备选", Tier::Alternate),
    ("继续", Tier::Recommend),
    ("推荐", Tier::Recommend),
];

/// 一段退回意见里写明去向的条目。按分句认：一个分句里的条目键，取它**后面最近的**状态词，
/// 后面没有就取前面最近的。没写条目键的（只写品牌）不认——宁可不认，也不认错。
pub fn parse_review(text: &str) -> Vec<(String, Tier)> {
    let mut out: Vec<(String, Tier)> = Vec::new();
    for clause in text.split(['；', ';', '。', '\n']) {
        let keys: Vec<(usize, &str)> = KEY
            .find_iter(clause)
            .map(|m| (m.start(), m.as_str()))
            .collect();
        if keys.is_empty() {
            continue;
        }
        // 状态词的位置（已被长词占住的位置不再让短词认）
        let mut marks: Vec<(usize, Tier)> = Vec::new();
        let mut taken: Vec<(usize, usize)> = Vec::new();
        for (w, t) in WORDS {
            for (i, _) in clause.match_indices(w) {
                let end = i + w.len();
                if taken.iter().any(|(a, b)| i < *b && end > *a) {
                    continue;
                }
                taken.push((i, end));
                marks.push((i, t));
            }
        }
        if marks.is_empty() {
            continue;
        }
        marks.sort_by_key(|m| m.0);
        for (pos, key) in keys {
            let after = marks.iter().find(|(p, _)| *p > pos);
            let before = marks.iter().rev().find(|(p, _)| *p < pos);
            if let Some((_, t)) = after.or(before) {
                out.retain(|(k, _)| k != key);
                out.push((key.to_string(), *t));
            }
        }
    }
    out
}

/// 主编常只写条目键末尾的 6 位（「e1cd9d.tier 由待核改推荐」）。在本轮条目里**唯一**对得上的，
/// 展开成完整条目键再认；对不上或对上多个的不动（10-01 r59 v9：只写短键，没认出来，沿用了 v8 的「继续」）。
pub fn expand_short_keys(text: &str, known: &[String]) -> String {
    static SHORT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(^|[^a-z0-9_.\-])([0-9a-f]{6})([^0-9a-z]|$)").expect("正则"));
    SHORT
        .replace_all(text, |c: &regex::Captures| {
            let hex = &c[2];
            let mut hits = known.iter().filter(|k| k.ends_with(&format!("-{hex}")));
            match (hits.next(), hits.next()) {
                (Some(full), None) => format!("{}{full}{}", &c[1], &c[3]),
                _ => c[0].to_string(),
            }
        })
        .into_owned()
}

/// 从任务详情里各版的审核意见收集校准（按版本先后，后说的覆盖先说的）；意见里只写了末 6 位的条目键按本轮条目展开。
pub fn from_reviews_with_keys(detail: &TaskDetail, known: &[String]) -> Vec<Calibration> {
    let mut ds: Vec<&serde_json::Value> = detail.deliverables.iter().collect();
    ds.sort_by_key(|d| d.get("version").and_then(|v| v.as_i64()).unwrap_or(0));
    let mut out: Vec<Calibration> = Vec::new();
    for d in ds {
        let Some(r) = d.get("latest_review") else {
            continue;
        };
        if r.get("verdict").and_then(|v| v.as_str()) != Some("reject") {
            continue;
        }
        let v = d.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
        let text = ["return_direction", "comment", "return_location"]
            .iter()
            .filter_map(|k| r.get(*k).and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        let text = expand_short_keys(&text, known);
        for (key, tier) in parse_review(&text) {
            upsert(&mut out, key, tier, format!("v{v} 退回意见"));
        }
    }
    out
}

/// 从引擎条目流转里取主编亲手改的状态（actor_role = 中枢），按时间先后覆盖。
pub fn from_traces(trace: &serde_json::Value, hub: &str, into: &mut Vec<Calibration>) {
    let mut rows: Vec<&serde_json::Value> = trace
        .get("traces")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|t| t.get("actor_role").and_then(|v| v.as_str()) == Some(hub))
        .collect();
    rows.sort_by_key(|t| {
        t.get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    });
    for t in rows {
        let (Some(key), Some(to)) = (
            t.get("item_key").and_then(|v| v.as_str()),
            t.get("to_status").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let tier = match to {
            "pending_check" => Tier::PendingCheck,
            "dropped" => Tier::NotRecommend,
            // 主编改成 shortlisted 的：档位按原来的推荐 / 备选，不在这里猜；已有校准就沿用
            "shortlisted" => match into.iter().find(|c| c.key == key) {
                Some(c) if matches!(c.tier, Tier::Recommend | Tier::Alternate) => c.tier,
                _ => Tier::Recommend,
            },
            _ => continue,
        };
        let at = t.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        upsert(
            into,
            key.to_string(),
            tier,
            format!("引擎条目（主编 {at}）"),
        );
    }
}

fn upsert(v: &mut Vec<Calibration>, key: String, tier: Tier, source: String) {
    match v.iter_mut().find(|c| c.key == key) {
        Some(c) => {
            c.tier = tier;
            c.source = source;
        }
        None => v.push(Calibration { key, tier, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 认出退回意见里写明去向的条目键() {
        // 09-30 r58 v6 退回意见原话片段
        let t = "对账文件已精确指出需覆盖的四个candidate_key：drlv-cf21b4、colemanjapan-3ca8da=待核；\
                 crossorange-0da84e=备选；colemanjapan-b12909=停止。首页恢复zaneartsoutdoor-fefb37、\
                 hillsfield-d90311、colemanjapan-e9591e、commanine-0b5b91四继续，另加上述四条，八键不换。";
        let got = parse_review(t);
        let tier = |k: &str| got.iter().find(|(x, _)| x == k).map(|(_, t)| *t);
        assert_eq!(tier("drlv-cf21b4"), Some(Tier::PendingCheck));
        assert_eq!(tier("colemanjapan-3ca8da"), Some(Tier::PendingCheck));
        assert_eq!(tier("crossorange-0da84e"), Some(Tier::Alternate));
        assert_eq!(tier("colemanjapan-b12909"), Some(Tier::NotRecommend));
        for k in [
            "zaneartsoutdoor-fefb37",
            "hillsfield-d90311",
            "colemanjapan-e9591e",
            "commanine-0b5b91",
        ] {
            assert_eq!(tier(k), Some(Tier::Recommend), "{k}");
        }
        assert_eq!(got.len(), 8);
    }

    #[test]
    fn 不推荐不被认成推荐_没写条目键的不认() {
        assert_eq!(parse_review("abc-123abc 不推荐")[0].1, Tier::NotRecommend);
        assert!(parse_review("ZANE ARTS YOMA 继续、HILLS FIELD 待核").is_empty());
    }

    #[test]
    fn 引擎里主编改的状态覆盖退回意见() {
        let mut v = vec![Calibration {
            key: "drlv-cf21b4".into(),
            tier: Tier::Recommend,
            source: "v5 退回意见".into(),
        }];
        let trace = serde_json::json!({"traces": [
            {"item_key": "drlv-cf21b4", "to_status": "pending_check", "actor_role": "editor", "created_at": "2026-09-30T08:34:02Z"},
            {"item_key": "x-aaaaaa", "to_status": "dropped", "actor_role": "collector", "created_at": "2026-09-30T08:40:00Z"}
        ]});
        from_traces(&trace, "editor", &mut v);
        assert_eq!(v.len(), 1, "执行方自己的流转不算校准");
        assert_eq!(v[0].tier, Tier::PendingCheck);
        assert!(v[0].source.contains("引擎条目"));
    }

    #[test]
    fn 只写末六位的条目键按本轮条目展开() {
        let known = vec![
            "cmfoutdoorgarmentofficial-e1cd9d".to_string(),
            "drlv-cf21b4".to_string(),
        ];
        let t = expand_short_keys("v9唯一变更为e1cd9d.tier由待核改推荐；恢复该条待核", &known);
        assert!(t.contains("cmfoutdoorgarmentofficial-e1cd9d.tier"));
        let got = parse_review(&t);
        assert_eq!(
            got,
            vec![(
                "cmfoutdoorgarmentofficial-e1cd9d".to_string(),
                Tier::PendingCheck
            )]
        );
        // 已是完整键的不重复展开；对不上的不动
        let t = expand_short_keys("drlv-cf21b4=待核；abcdef 不认", &known);
        assert_eq!(t, "drlv-cf21b4=待核；abcdef 不认");
    }
}
