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
    /// 主编对这一条写的原话（条目键后面那一段），待核时作缺口的具体内容；引擎条目来源为空
    pub note: String,
    /// 第一次被校准的版本（退回意见的版本号；引擎条目流转记 [`SINCE_TRACE`]）。
    /// 首批卡按它先后占位：早定下的卡位不被后来的校准挤掉（10-09 r67 v5：v4 意见里描述误改的
    /// 四条 lesyndrome 被读成校准、按档位排到停止卡前面，四张停止卡与它们的 35 张原图被挤出包）
    pub since: i64,
}

/// 引擎条目流转来的校准排在所有退回意见之后
pub const SINCE_TRACE: i64 = 1_000_000;

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

/// 去向词（英文档位名也认），长的在前（「not_recommend」先于「recommend」、「不推荐」先于「推荐」）
const W: &str =
    "not_recommend|pending_check|alternate|recommend|不推荐|待核|备选|继续|停止|淘汰|推荐";

fn tier_of(w: &str) -> Option<Tier> {
    Some(match w {
        "not_recommend" | "不推荐" | "停止" | "淘汰" => Tier::NotRecommend,
        "pending_check" | "待核" => Tier::PendingCheck,
        "alternate" | "备选" => Tier::Alternate,
        "recommend" | "继续" | "推荐" => Tier::Recommend,
        _ => return None,
    })
}

/// 条目键前面是归属标签的，它是被引用的对象、不是这句的主语：
/// 「asimocrafts-e1a771：item_key=asimocrafts-628da9、tier=not_recommend」说的是 e1a771 挂在哪，
/// 不是把 628da9 改成不推荐（10-08 r66 v5：两个产品就这样被误降了档）
static REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:item_key|所属|归属|仍归|归到|归入|归|属于|并入|挂到|挂在|映射到?|指向)\s*[=＝:：]?\s*$")
        .expect("正则")
});

fn is_reference(before: &str) -> bool {
    REF.is_match(before)
}

/// 一段文字里写到的完整条目键（按出现顺序、去重）。
pub fn keys_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in KEY.find_iter(text) {
        if !out.iter().any(|k| k == m.as_str()) {
            out.push(m.as_str().to_string());
        }
    }
    out
}

static LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\s*[、,，]\s*[a-z0-9_.]+-[0-9a-f]{6})+").expect("正则"));

/// 条目键之后、紧跟着写明去向的几种写法。**去向词在条目键前面的不认**：
/// 「停止针对 andwanderofficial-3bbddb 再深核」说的是停止返工路线，不是把它定为停止
///（10-01 r60 v11：被认成「停止」，andwander 改成了不推荐）。
static FORMS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // key=待核 / key：备选 / key → 继续 / key 不推荐 / ……四继续（列举后带数量）
        r"^\s*[=＝:：→]?\s*[一二三四五六七八九十两0-9]*\s*(W)",
        // key：隔离待核 / key：暂备选 / key：降为不推荐——冒号后、去向词前只认这几个修饰词，
        // 不放任意前缀（「不是推荐」不能认成推荐）（10-08 r66 v1 退回：两条「隔离待核」没认出来）
        r"^\s*[=＝:：→]\s*(?:隔离|暂|先|仍|转为?|列为?|降为?|升为?|改列)\s*(W)",
        // key 的 tier=not_recommend / 档位：待核
        r"^[^，,。；;]{0,24}?(?:tier|status|档位|档)\s*[=＝:：]\s*(W)",
        // key 改为 / 改成 / 改回 / 恢复 / 定为 / 落 / 保持 待核
        r"^[^，,。；;]{0,12}?(?:改为|改成|改回|恢复为?|定为|落为?|保持为?)\s*(W)",
        // key …保留 pending_check / 保留官方正文与 recommend
        r"^[^。；;]{0,30}?保留[^，,。；;]{0,8}?(W)",
        // key …pending_check 不变 / 待核保留
        r"^[^。；;]{0,30}?(W)\s*(?:不变|保留)",
    ]
    .iter()
    .map(|f| Regex::new(&f.replace('W', W)).expect("正则"))
    .collect()
});

/// 一段退回意见里写明去向的条目。按分句认，只认条目键**之后**明确写出的去向（见 [`FORMS`]）。
/// 没写条目键的（只写品牌）不认——宁可不认，也不认错。
#[cfg(test)]
pub fn parse_review(text: &str) -> Vec<(String, Tier)> {
    parse_review_notes(text)
        .into_iter()
        .map(|(k, t, _)| (k, t))
        .collect()
}

/// 编号条目里后文才写的去向（「卡6 key：此前…无新增变化…。隔离待核不作成熟替补」）只认这几个明确说法，
/// 前面紧挨着否定词的不算（10-08 r66 v2 退回：卡 6、卡 8 的「隔离待核」写在第二句，没认出来）。
const ITEM_PHRASES: [(&str, Tier); 9] = [
    ("隔离待核", Tier::PendingCheck),
    ("降为待核", Tier::PendingCheck),
    ("降待核", Tier::PendingCheck),
    ("改为待核", Tier::PendingCheck),
    ("停止本期", Tier::NotRecommend),
    ("停止追加", Tier::NotRecommend),
    ("继续价值评估", Tier::Recommend),
    ("保留继续评估", Tier::Recommend),
    ("继续评估", Tier::Recommend),
];

fn item_phrase(after_key: &str) -> Option<Tier> {
    const NEG: [&str; 5] = ["不", "勿", "别", "禁止", "非"];
    let mut best: Option<(usize, Tier)> = None;
    for (p, t) in ITEM_PHRASES {
        for (at, _) in after_key.match_indices(p) {
            let before: String = after_key[..at].chars().rev().take(3).collect();
            if NEG.iter().any(|n| before.contains(n)) {
                continue;
            }
            if best.is_none_or(|(b, _)| at < b) {
                best = Some((at, t));
            }
            break;
        }
    }
    best.map(|(_, t)| t)
}

/// 同 [`parse_review`]，另带主编对这一条写的原话。
pub fn parse_review_notes(text: &str) -> Vec<(String, Tier, String)> {
    static ITEM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^\s*(?:\d+[\.、．]|卡\s*\d+)").expect("正则"));
    let note_of = |after: &str| -> String {
        after
            .trim_start_matches([':', '：', ' ', '='])
            .chars()
            .take(160)
            .collect::<String>()
            .trim()
            .to_string()
    };
    let mut out: Vec<(String, Tier, String)> = Vec::new();
    // 一、按分句认：条目键后紧跟去向
    for (k, t) in parse_review_clauses(text) {
        let note = text
            .find(&k)
            .map(|i| note_of(text[i + k.len()..].split(['\n']).next().unwrap_or("")))
            .unwrap_or_default();
        out.push((k, t, note));
    }
    // 二、编号条目里只有一个条目键、分句没认出的：在这一条里找明确说法
    let starts: Vec<usize> = ITEM.find_iter(text).map(|m| m.start()).collect();
    for (n, &a) in starts.iter().enumerate() {
        let b = starts.get(n + 1).copied().unwrap_or(text.len());
        // 一条到换行为止：最后一条后面常跟着「方向：」等别的段落，不能并进来
        let item = text[a..b]
            .split('\n')
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let keys: Vec<&str> = KEY
            .find_iter(item)
            .filter(|m| !is_reference(&item[..m.start()]))
            .map(|m| m.as_str())
            .collect();
        let [key] = keys.as_slice() else { continue };
        if out.iter().any(|(k, _, _)| k == key) {
            continue;
        }
        let after = &item[item.find(key).unwrap_or(0) + key.len()..];
        if let Some(t) = item_phrase(after) {
            out.push((key.to_string(), t, note_of(after)));
        }
    }
    out
}

fn parse_review_clauses(text: &str) -> Vec<(String, Tier)> {
    let mut out: Vec<(String, Tier)> = Vec::new();
    for clause in text.split(['；', ';', '。', '\n']) {
        for m in KEY.find_iter(clause) {
            if is_reference(&clause[..m.start()]) {
                continue;
            }
            // 列举的几个键共用后面的去向：「drlv-cf21b4、colemanjapan-3ca8da=待核」
            let tail = LIST
                .find(&clause[m.end()..])
                .map_or(&clause[m.end()..], |l| &clause[m.end() + l.end()..]);
            // 跨到别的条目键的不算：去向是那个键的
            let hit = FORMS.iter().find_map(|re| {
                let c = re.captures(tail)?;
                let w = c.get(1)?;
                if KEY.is_match(&tail[..w.start()]) {
                    return None;
                }
                tier_of(w.as_str())
            });
            if let Some(t) = hit {
                out.retain(|(k, _)| k != m.as_str());
                out.push((m.as_str().to_string(), t));
            }
        }
    }
    out
}

/// 首批卡按主编校准排的顺序：**先定下的先占位**（第一次校准的版本早的在前），同一版里继续 → 备选 →
/// 待核 → 停止（主编「首页恢复四继续，另加上述四条」）。后来的校准只补空位，不挤掉已定的卡
///（10-09 r67 v5：v4 意见里描述误改的四条被读成校准、按档位排到四张停止卡前面，停止卡与原图被挤出包）。
/// 主编要从池里另选来补首批时，定为停止的不再占卡位（10-08 r66）。
pub fn pin_order(calib: &[Calibration], replace: bool) -> Vec<String> {
    let rank = |t: Tier| match t {
        Tier::Recommend => 0,
        Tier::Alternate => 1,
        Tier::PendingCheck => 2,
        Tier::NotRecommend => 3,
    };
    let mut v: Vec<&Calibration> = calib
        .iter()
        .filter(|c| !(replace && c.tier == Tier::NotRecommend))
        .collect();
    v.sort_by_key(|c| (c.since, rank(c.tier)));
    v.into_iter().map(|c| c.key.clone()).collect()
}

/// 退回意见是不是要从池里另选条目补首批（停止的条目让出卡位）。
pub fn asks_replacement(review: &str) -> bool {
    static R: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"另选|另挑|补首批|补足首批|替换|换下|换上").expect("正则"));
    R.is_match(review)
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
        for (key, tier, note) in parse_review_notes(&text) {
            upsert(&mut out, key, tier, format!("v{v} 退回意见"), note, v);
        }
    }
    out
}

/// 主编要把一条并进另一个事件：「X 并入 Y」「X 退出独立事件位，关联 Y」。
/// X 的逐帖判断与图片不动，只是不再单占一个选题位，登记时撤出独立位并写明并入谁。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    pub subject: String,
    /// 完整条目键，或只写了账号（「关联fasunaa」）时的账号前缀，由选题层按唯一匹配解析
    pub target: MergeTarget,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeTarget {
    Key(String),
    Account(String),
}

/// 一段意见里的并入指示。按分句认；主语是这句里第一个不在归属标签后的条目键，
/// 目标是并入动词之后的另一个条目键，没有就取动词后紧跟的账号名。
pub fn parse_merges(text: &str) -> Vec<(String, MergeTarget)> {
    static VERB: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"并入|归入|归到|合并到|退出独立[^，,。；;]{0,4}位").expect("正则")
    });
    static POINT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:关联|并入|归入|归到|合并到)\s*[=＝:：]?\s*([a-z0-9_.]{3,})").expect("正则")
    });
    let mut out: Vec<(String, MergeTarget)> = Vec::new();
    for sent in text.split(['。', '；', ';', '\n']) {
        let Some(v) = VERB.find(sent) else { continue };
        if !super::designated::affirms(sent, v.as_str()) {
            continue;
        }
        let Some(subject) = KEY
            .find_iter(sent)
            .find(|m| !is_reference(&sent[..m.start()]))
            .map(|m| m.as_str().to_string())
        else {
            continue;
        };
        let after = &sent[v.start()..];
        let target = KEY
            .find_iter(after)
            .map(|m| m.as_str())
            .find(|k| *k != subject)
            .map(|k| MergeTarget::Key(k.to_string()))
            .or_else(|| {
                POINT
                    .captures_iter(sent)
                    .filter_map(|c| c.get(1))
                    .map(|m| m.as_str())
                    .find(|w| !subject.starts_with(&format!("{w}-")) && *w != subject)
                    .map(|w| MergeTarget::Account(w.trim_end_matches('.').to_string()))
            });
        if let Some(target) = target {
            out.retain(|(s, _)| *s != subject);
            out.push((subject, target));
        }
    }
    out
}

/// 主编指定的代表预览：「代表预览只放 hinataoutdoor-5caef3 第2张地钉包」。
/// 只认同一分句里写了完整条目键与「第 N 张」的；返回（条目键, 原帖第几张，从 1 起）。
pub fn parse_preview_picks(text: &str) -> Vec<(String, Vec<u32>)> {
    static NTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"第\s*(\d{1,2})\s*张").expect("正则"));
    let mut out: Vec<(String, Vec<u32>)> = Vec::new();
    for sent in text.split(['。', '；', ';', '\n']) {
        if !(sent.contains("代表预览") || sent.contains("代表图") || sent.contains("展示图"))
        {
            continue;
        }
        let keys = keys_in(sent);
        let [key] = keys.as_slice() else { continue };
        let ns: Vec<u32> = NTH
            .captures_iter(sent)
            .filter_map(|c| c.get(1)?.as_str().parse().ok())
            .filter(|n| *n >= 1)
            .collect();
        if ns.is_empty() {
            continue;
        }
        out.retain(|(k, _)| k != key);
        out.push((key.clone(), ns));
    }
    out
}

/// 主编要求原样加到卡面 / 台账旁的说明：「在 hakunokiroku-aa2a17 台账旁新增“主编本期不纳入：…”」。
/// 只认「在 条目键 …新增 / 加上 / 补上“原话”」这一种写法；返回（条目键, 原话）。
pub fn parse_side_notes(text: &str) -> Vec<(String, String)> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"在\s*([a-z0-9_.]+-[0-9a-f]{6})\s*(?:的)?\s*(?:首批卡|台账旁|台账|卡面|题卡)?\s*(?:旁)?\s*(?:新增|加上|补上|增加)\s*[“"「]([^”"」]{2,400})[”"」]"#,
        )
        .expect("正则")
    });
    let mut out: Vec<(String, String)> = Vec::new();
    for c in RE.captures_iter(text) {
        let (k, t) = (c[1].to_string(), c[2].trim().to_string());
        out.retain(|(x, _)| *x != k);
        out.push((k, t));
    }
    out
}

/// 各版退回意见里要加的卡面 / 台账说明，新的覆盖旧的；返回（条目键, 原话, 来源版本）。
pub fn side_notes_from_reviews(
    detail: &TaskDetail,
    known: &[String],
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for (text, source) in instruction_texts(detail, known) {
        for (k, t) in parse_side_notes(&text) {
            out.retain(|(x, _, _)| *x != k);
            out.push((k, t, source.clone()));
        }
    }
    out
}

/// 各版退回意见里指定的代表预览，新的覆盖旧的。
pub fn preview_picks_from_reviews(
    detail: &TaskDetail,
    known: &[String],
) -> Vec<(String, Vec<u32>)> {
    let mut out: Vec<(String, Vec<u32>)> = Vec::new();
    for (text, _) in instruction_texts(detail, known) {
        for (k, ns) in parse_preview_picks(&text) {
            out.retain(|(x, _)| *x != k);
            out.push((k, ns));
        }
    }
    out
}

/// 主编的指示全文，按时间先后：各版退回意见（意见 + 方向 + 位置）与本任务的主编反馈（returned 不能重开时，
/// 主编以反馈下达恢复交接，10-09 r67 #66）。短键已展开。返回（全文, 来源）。
pub fn instruction_texts(detail: &TaskDetail, known: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for d in &detail.deliverables {
        let Some(r) = d.get("latest_review") else {
            continue;
        };
        if r.get("verdict").and_then(|v| v.as_str()) != Some("reject") {
            continue;
        }
        let v = d.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
        let at = r
            .get("created_at")
            .and_then(|x| x.as_str())
            .map_or_else(|| format!("v{v:06}"), str::to_string);
        let text = ["return_direction", "comment", "return_location"]
            .iter()
            .filter_map(|k| r.get(*k).and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        out.push((
            at,
            expand_short_keys(&text, known),
            format!("v{v} 退回意见"),
        ));
    }
    for f in &detail.feedback {
        let Some(q) = f.get("quote").and_then(|x| x.as_str()) else {
            continue;
        };
        let at = f
            .get("created_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let id = f.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
        out.push((at, expand_short_keys(q, known), format!("反馈 #{id}")));
    }
    // 时间写法一致（RFC 3339 UTC），按字符串排就是按时间排；没有时间的退回意见按版本号排在前面
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, t, src)| (t, src)).collect()
}

/// 各版退回意见里的并入指示，新的覆盖旧的。
pub fn merges_from_reviews(detail: &TaskDetail, known: &[String]) -> Vec<Merge> {
    let mut out: Vec<Merge> = Vec::new();
    for (text, source) in instruction_texts(detail, known) {
        for (subject, target) in parse_merges(&text) {
            out.retain(|m| m.subject != subject);
            out.push(Merge {
                subject,
                target,
                source: source.clone(),
            });
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
            String::new(),
            SINCE_TRACE,
        );
    }
}

fn upsert(
    v: &mut Vec<Calibration>,
    key: String,
    tier: Tier,
    source: String,
    note: String,
    since: i64,
) {
    match v.iter_mut().find(|c| c.key == key) {
        Some(c) => {
            c.tier = tier;
            c.source = source;
            if !note.is_empty() {
                c.note = note;
            }
        }
        None => v.push(Calibration {
            key,
            tier,
            source,
            note,
            since,
        }),
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
            note: String::new(),
            since: 5,
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
        let t = expand_short_keys(
            "v9唯一变更为e1cd9d.tier由待核改推荐；恢复e1cd9d tier=pending_check",
            &known,
        );
        assert!(t.contains("cmfoutdoorgarmentofficial-e1cd9d.tier"));
        // 「由待核改推荐」是描述上一版的变化，不是去向；明确写出的 tier=pending_check 才认
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

    /// 10-01 r60 主编的真实写法：去向在键后才认；「停止针对 xx 再深核」不是校准
    #[test]
    fn 要另选补首批才让出停止的卡位() {
        assert!(asks_replacement(
            "现池另选实质使用/结构/工艺事件，不换措辞包装弱题"
        ));
        assert!(asks_replacement(
            "原全池中可另挑有明确使用变化的现成独立事件补首批"
        ));
        assert!(!asks_replacement("首页恢复四继续，另加上述四条"));
    }

    #[test]
    fn r66v2编号条目后文的去向也认出并带原话() {
        let t = "5. 卡6 campshoplantern-cc4260：此前有效反馈已停chair1987追加，当前材料与v1同核资料，无新增变化，不能以栗木+网布同义解释回流。隔离待核不作成熟替补，保留素材。\n\
6. 卡8 cargocontainer-fbe002：自己确认r62写成且同事实无增量，仍recommend且readiness称宜接续制作，与POPEYE同样隔离待核，旧批准不继承。\n\
方向：asimocrafts-628da9/bluelug-2a9c39保留继续评估；新增bushdebrunt-1b7eeb继续价值评估，画面可见三圈护框；cargocontainer-a3c6ce继续评估，图可见锅篮烤架";
        let got = parse_review_notes(t);
        let tier = |k: &str| got.iter().find(|(x, _, _)| x == k).map(|(_, t, _)| *t);
        assert_eq!(
            tier("campshoplantern-cc4260"),
            Some(Tier::PendingCheck),
            "{got:?}"
        );
        assert_eq!(
            tier("cargocontainer-fbe002"),
            Some(Tier::PendingCheck),
            "{got:?}"
        );
        assert_eq!(tier("bushdebrunt-1b7eeb"), Some(Tier::Recommend), "{got:?}");
        assert_eq!(
            tier("cargocontainer-a3c6ce"),
            Some(Tier::Recommend),
            "{got:?}"
        );
        let note = &got
            .iter()
            .find(|(k, _, _)| k == "cargocontainer-fbe002")
            .unwrap()
            .2;
        assert!(note.contains("r62写成"), "{note}");
        // 否定语境不认
        assert!(parse_review_notes("1. abc-123456：不继续评估，另议").is_empty());
    }

    #[test]
    fn 首批卡先定下的先占位后来的校准只补空位() {
        let c = |k: &str, tier, since| Calibration {
            key: k.into(),
            tier,
            source: String::new(),
            note: String::new(),
            since,
        };
        // r67：v1 定了四继续四停止，v4 意见里描述误改的 lesyndrome 被读成继续 / 备选
        let calib = [
            c("ankorau0-615184", Tier::Recommend, 1),
            c("betterweekend-7cc615", Tier::NotRecommend, 1),
            c("fasunaa-6ea867", Tier::Recommend, 1),
            c("lesyndrome-29913d", Tier::Recommend, 4),
            c("lesyndrome-4bd27e", Tier::Alternate, 4),
        ];
        assert_eq!(
            pin_order(&calib, false),
            [
                "ankorau0-615184",
                "fasunaa-6ea867",
                "betterweekend-7cc615",
                "lesyndrome-29913d",
                "lesyndrome-4bd27e"
            ]
        );
        // 要另选补首批时停止的让位
        assert!(!pin_order(&calib, true).contains(&"betterweekend-7cc615".to_string()));
    }

    #[test]
    fn 旁注认条目键和原话() {
        // 10-09 r67 v5 退回原文
        let t = "- 在hakunokiroku-aa2a17台账旁新增“主编本期不纳入：增量止于手工扎结视觉变化，缺染整方法或功能取舍实质信息；不写功能保持已验证，不换抽象文化角度补足。”\n- 在commanine-e30841首批卡新增“旧译文‘印度挂架’仅留溯源，不作为产品译名；采用独立置物架/挂架。”";
        let got = parse_side_notes(t);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "hakunokiroku-aa2a17");
        assert!(got[0].1.starts_with("主编本期不纳入：增量止于"));
        assert_eq!(got[1].0, "commanine-e30841");
        assert!(got[1].1.contains("印度挂架"));
    }

    #[test]
    fn 代表预览认条目键和第几张() {
        // 10-09 r67 v3 退回原文
        let t = "3. SotoLabo代表预览仍错：主编实际看v3联系图，第三行仍混有夹烤器与OFFLINE桌架，index卡3仍展示五件清单全部6图。首批代表预览只放hinataoutdoor-5caef3第2张地钉包，其他图可留原始素材目录，不作为该则配图。";
        assert_eq!(
            parse_preview_picks(t),
            vec![("hinataoutdoor-5caef3".to_string(), vec![2])]
        );
        // 只写品牌、没写条目键的不认
        assert!(parse_preview_picks("SotoLabo代表预览仅第2张地钉包").is_empty());
    }

    #[test]
    fn 并入指示认主语和目标() {
        // 10-09 r67 v3 退回原文（方向与意见各一句）
        let dir = "fasunaa-6ea867=继续；lesyndrome-53e957仅退出独立事件位(status=dropped,reason_code=superseded)，关联fasunaa，保留alternate逐帖判断与图片，不按低价值停止主事件。";
        assert_eq!(
            parse_merges(dir),
            vec![(
                "lesyndrome-53e957".to_string(),
                MergeTarget::Account("fasunaa".into())
            )]
        );
        let cmt = "仅将重复事件lesyndrome-53e957登记退出独立采用位：status=dropped、reason_code=superseded、reason=并入fasunaa-6ea867，仅作关联图文佐证";
        assert_eq!(
            parse_merges(cmt),
            vec![(
                "lesyndrome-53e957".to_string(),
                MergeTarget::Key("fasunaa-6ea867".into())
            )]
        );
        // 否定的、没有目标的都不认
        assert!(parse_merges("lesyndrome-53e957不并入任何事件").is_empty());
        assert!(parse_merges("lesyndrome-53e957退出独立事件位").is_empty());
    }

    #[test]
    fn 归属标签后的条目键是被引用的对象不是改档() {
        // 10-08 r66 #1132 v4、#1133 v5 退回意见原文节选：v4 那两行让 v5 把两个产品误降成不推荐
        let v4 = "- asimocrafts-e1a771：item_key=asimocrafts-628da9、tier=not_recommend；包内预期item_key空。\n\
- bushdebrunt-3cd9a3：item_key=bushdebrunt-1b7eeb、tier=not_recommend；包内预期item_key空。";
        let v5 = "主编独立GET intake-judgements1172条确认asimocrafts-e1a771仍归asimocrafts-628da9、bushdebrunt-3cd9a3仍归bushdebrunt-1b7eeb，两清单tier均not_recommend。\n\
新增误改：与1132 v4的1172判断逐candidate_key比较，只有两个产品判断的tier改变：asimocrafts-628da9和bushdebrunt-1b7eeb由recommend变为not_recommend，其他判断字段未变。\n\
- 清单candidate_key=asimocrafts-e1a771：无所属；维持not_recommend和低价值理由。\n\
- 清单candidate_key=bushdebrunt-3cd9a3：无所属；维持not_recommend和低价值理由。\n\
- 产品candidate_key/item_key=asimocrafts-628da9：恢复v4原recommend判断与非淘汰登记，主编方向继续评估、不代表成熟或Van批准。\n\
- 产品candidate_key/item_key=bushdebrunt-1b7eeb：恢复v4原recommend判断与非淘汰登记，主编方向继续评估、不代表成熟或Van批准。\n\
trace/judgements/items与引擎两产品键asimocrafts-628da9、bushdebrunt-1b7eeb；实际请求/响应/服务端日志和写后回读。";
        assert!(
            parse_review_notes(v4).is_empty(),
            "{:?}",
            parse_review_notes(v4)
        );
        assert!(
            parse_review_notes(v5).is_empty(),
            "{:?}",
            parse_review_notes(v5)
        );
        // 不是归属标签的照常认
        let mut got = parse_review("asimocrafts-628da9=不推荐；bushdebrunt-1b7eeb：隔离待核");
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("asimocrafts-628da9".to_string(), Tier::NotRecommend),
                ("bushdebrunt-1b7eeb".to_string(), Tier::PendingCheck),
            ]
        );
    }

    #[test]
    fn r66主编首批八卡全部认出() {
        // 10-08 r66 #1128 退回意见原文节选（八卡方向）
        let t = "1. 38explore-537a0b：停止本期追加。自选色组装和门店服务有事实。\n\
2. asimocrafts-628da9：保留继续评估。烤网、架锅与烤鱼有图文依据。\n\
3. bambooshootsshop-3f8b8f：停止本期追加。正文明确每年热门款到货。\n\
4. beamsmenscasual-41eb33：隔离待核，不直接回主选。透明杂志口袋具具体价值。\n\
5. bluelug-2a9c39：保留继续评估。缩小包体改善弯把空间。\n\
6. bluelug-c0e972：停止本期追加。卡面核心明确是采访与图案解释。\n\
7. bprbeams-dcba41：停止本期追加。头盔包缩成挂饰。\n\
8. brennholzkr-4bc4d9：隔离待核。本帖只有Comeback、铝铸、原装配件兼容。";
        let got = parse_review(t);
        let want = [
            ("38explore-537a0b", Tier::NotRecommend),
            ("asimocrafts-628da9", Tier::Recommend),
            ("bambooshootsshop-3f8b8f", Tier::NotRecommend),
            ("beamsmenscasual-41eb33", Tier::PendingCheck),
            ("bluelug-2a9c39", Tier::Recommend),
            ("bluelug-c0e972", Tier::NotRecommend),
            ("bprbeams-dcba41", Tier::NotRecommend),
            ("brennholzkr-4bc4d9", Tier::PendingCheck),
        ];
        for (k, tier) in want {
            assert!(
                got.contains(&(k.to_string(), tier)),
                "{k} 应为 {tier:?}，实得 {got:?}"
            );
        }
        assert_eq!(got.len(), 8);
        // 修饰词只认白名单：「不是推荐」不能认成推荐
        assert!(parse_review("abc-123456：不是推荐").is_empty());
    }

    #[test]
    fn 去向词在条目键前面的不认() {
        let t = "停止针对andwanderofficial-3bbddb再深核/重写/改档后整包升版的自动路径；\
                 固定预期1：andwanderofficial-3bbddb image_seen=false，tier及条目pending_check保留；\
                 预期3：eightoutdoor-2d8074 tier=not_recommend，条目status=dropped；\
                 cmfoutdoorgarmentofficial-8b6b68保留官方正文与recommend；\
                 cmfoutdoorgarmentofficial-e1cd9d仍recommend，decision缺口未解";
        let got = parse_review(t);
        let tier = |k: &str| got.iter().find(|(x, _)| x == k).map(|(_, t)| *t);
        assert_eq!(tier("andwanderofficial-3bbddb"), Some(Tier::PendingCheck));
        assert_eq!(tier("eightoutdoor-2d8074"), Some(Tier::NotRecommend));
        assert_eq!(
            tier("cmfoutdoorgarmentofficial-8b6b68"),
            Some(Tier::Recommend)
        );
        assert_eq!(
            tier("cmfoutdoorgarmentofficial-e1cd9d"),
            None,
            "「仍 recommend」是描述现状"
        );
        assert_eq!(
            parse_review("停止针对andwanderofficial-3bbddb再深核"),
            vec![],
            "停止的是返工路线"
        );
    }
}
