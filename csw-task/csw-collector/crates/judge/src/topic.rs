//! 选题层：逐帖留痕之外，同产品、同事件的多条贴文合成一个选题（09-22 反馈第五项）。
//!
//! 第 3 轮里同一款 SOUTH2 WEST8 × BEN MILLER 夹克三条贴文两条推荐、一条备选，
//! 记录认出了同款，却各占一个推荐位——Van 要重复读三遍、自己拼资料。
//!
//! 这一层做三件事：
//! 1. [`build`]：按合并出的组建选题，**每条候选恰好属于一个选题**（没合并的自成一题），
//!    选题档 = 组内最高档，代表帖取最高档里的那条。
//! 2. [`synthesize`]：组里有两帖以上且进了推荐 / 备选的，问一次生成模型：
//!    共有的事实、每帖新增了什么（只是重复的标出来）、哪些其实是另一个角度要拆出去。
//! 3. 新增信息用 Jev 核一遍「这帖的材料里真有这句」，核不到的标出来给人看（不删）。
//!
//! 逐帖台账不动；推荐贴文数与独立选题数分开统计。

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use csw_collector_core::jev::{JevClient, Question};
use csw_collector_core::model::{ModelClient, Part};
use csw_collector_core::types::{
    Candidate, EventGroup, Judgement, MemberNote, Tier, Topic, TopicSynthesis,
};

/// 提示词版本。改了就要改它。
pub const PROMPT_VERSION: &str = "topic/v1";
/// 新增信息核不到的线。与依据核对同一个数（0.7 回测：编造的贴在 0.02）。
pub const SUPPORTED_THRESHOLD: f64 = crate::check::SUPPORTED_THRESHOLD;

fn rank(t: Tier) -> u8 {
    match t {
        Tier::Recommend => 0,
        Tier::Alternate => 1,
        Tier::PendingCheck => 2,
        Tier::NotRecommend => 3,
    }
}

/// 按合并组建选题。**每条有结论的候选恰好属于一个选题**；组里没有结论的成员略过。
pub fn build(groups: &[EventGroup], js: &[Judgement]) -> Vec<Topic> {
    let by_key: HashMap<&str, &Judgement> =
        js.iter().map(|j| (j.candidate_key.as_str(), j)).collect();
    let mut placed: std::collections::HashSet<&str> = Default::default();
    let mut out = Vec::new();
    for g in groups {
        let members: Vec<&Judgement> = g
            .members
            .iter()
            .filter_map(|k| by_key.get(k.as_str()).copied())
            .filter(|j| placed.insert(j.candidate_key.as_str()))
            .collect();
        if members.is_empty() {
            continue;
        }
        out.push(topic_of(&members, &g.primary, &g.merge_note));
    }
    // 合并那一步没见过的（如规则排除的）各自成题
    for j in js {
        if placed.insert(j.candidate_key.as_str()) {
            out.push(topic_of(&[j], &j.candidate_key, ""));
        }
    }
    out.sort_by(|a, b| {
        a.tier
            .map(rank)
            .cmp(&b.tier.map(rank))
            .then_with(|| a.topic_key.cmp(&b.topic_key))
    });
    out
}

fn topic_of(members: &[&Judgement], merge_primary: &str, note: &str) -> Topic {
    // 代表帖：档最高的；同档里合并时选的代表（正文最全）优先，再按键定死
    let best = members
        .iter()
        .min_by(|a, b| {
            rank(a.tier)
                .cmp(&rank(b.tier))
                .then_with(|| {
                    (a.candidate_key != merge_primary).cmp(&(b.candidate_key != merge_primary))
                })
                .then_with(|| a.candidate_key.cmp(&b.candidate_key))
        })
        .expect("至少一条");
    let mut keys: Vec<String> = members.iter().map(|j| j.candidate_key.clone()).collect();
    keys.sort_by_key(|k| (k != &best.candidate_key, k.clone()));
    Topic {
        topic_key: best.candidate_key.clone(),
        primary_key: best.candidate_key.clone(),
        members: keys,
        merge_note: note.to_string(),
        tier: Some(best.tier),
        headline: best.headline.clone(),
        synthesis: TopicSynthesis::default(),
    }
}

/// 值得综合的选题：两帖以上，且进了推荐 / 备选。
pub fn needs_synthesis(t: &Topic) -> bool {
    t.members.len() >= 2 && matches!(t.tier, Some(Tier::Recommend | Tier::Alternate))
}

/// 综合的结果。`split` 里的帖子其实是另一个报道角度，要拆成独立选题。
#[derive(Debug, Clone, Default)]
pub struct Synth {
    pub headline: String,
    pub synthesis: TopicSynthesis,
    pub split: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct Wire {
    headline: String,
    shared_facts: Vec<String>,
    per_member: Vec<MemberNote>,
    split: Vec<WireSplit>,
}

#[derive(Deserialize)]
struct WireSplit {
    candidate_key: String,
    reason: String,
}

/// 问一次生成模型：共有事实、各帖新增、要拆出去的。**第三方正文一律过不可信边界。**
pub async fn synthesize(
    model: &ModelClient,
    topic: &Topic,
    members: &[(&Candidate, &Judgement)],
) -> Result<Synth> {
    let fence = csw_collector_core::prompt::fence;
    let mut s = String::from(
        "下面几条 Instagram 贴文被认为在讲同一个产品或同一件事。你在为 CSW 把它们合成一个选题，\
         免得同一个对象占好几个推荐位、让编辑重复阅读。\n\
         - headline：「具体对象｜一句推荐理由」，40 字以内。\n\
         - shared_facts：几帖共有的事实，每条一句。\n\
         - per_member：每一帖相对其余几帖新增了什么（具体到事实或第几张图）；只是重复的，\
           new_info 写空、is_duplicate 填 true。**只写贴文里真有的，编不出来就留空。**\n\
         - split：其实是另一个报道角度或后续变化、应单列成另一个选题的帖子，写明理由；没有就空。\n\n",
    );
    s.push_str("【边界】\n");
    s.push_str(csw_collector_core::prompt::DATA_NOT_INSTRUCTIONS);
    s.push('\n');
    for (c, j) in members {
        s.push_str(&format!(
            "\n────────── 贴文 ──────────\ncandidate_key：{}\n账号：{}\n单帖结论：{:?}｜{}\n正文：\n{}\n",
            c.candidate_key,
            c.account,
            j.tier,
            fence(&j.headline),
            fence(c.text.trim())
        ));
        if !c.translated.trim().is_empty() {
            s.push_str(&format!("译文：\n{}\n", fence(c.translated.trim())));
        }
        if !j.look.trim().is_empty() {
            s.push_str(&format!("实图所见：{}\n", fence(&j.look)));
        }
    }
    s.push_str(&format!(
        "\n────────── 输出 ──────────\n选题代表帖是 {}。candidate_key 原样抄回去。",
        topic.primary_key
    ));
    let out = model
        .structured(&[Part::Text(s)], "csw_topic", &schema(), 4000)
        .await
        .context("选题综合")?;
    let w: Wire = out.parse()?;
    let keys: std::collections::HashSet<&str> = members
        .iter()
        .map(|(c, _)| c.candidate_key.as_str())
        .collect();
    Ok(Synth {
        headline: w.headline,
        synthesis: TopicSynthesis {
            shared_facts: w.shared_facts,
            // 对不上的键丢掉，不猜
            per_member: w
                .per_member
                .into_iter()
                .filter(|m| keys.contains(m.candidate_key.as_str()))
                .collect(),
            unsupported: vec![],
        },
        split: w
            .split
            .into_iter()
            // 代表帖不能被拆走，否则这个选题就没了主心骨
            .filter(|x| {
                keys.contains(x.candidate_key.as_str()) && x.candidate_key != topic.primary_key
            })
            .map(|x| (x.candidate_key, x.reason))
            .collect(),
    })
}

fn schema() -> Value {
    let strict = |props: Value| {
        let keys: Vec<String> = props
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        json!({"type": "object", "properties": props, "required": keys, "additionalProperties": false})
    };
    strict(json!({
        "headline": {"type": "string"},
        "shared_facts": {"type": "array", "items": {"type": "string"}},
        "per_member": {"type": "array", "items": strict(json!({
            "candidate_key": {"type": "string"},
            "new_info": {"type": "string"},
            "is_duplicate": {"type": "boolean"}
        }))},
        "split": {"type": "array", "items": strict(json!({
            "candidate_key": {"type": "string"},
            "reason": {"type": "string"}
        }))}
    }))
}

const FRAME_SUPPORTED: &str = "`statement` claims what one post adds beyond the others. `material` \
is that post's caption and photo descriptions. Does `material` actually support `statement`? Answer \
false if the statement asserts anything that is not in `material`.";

/// 用 Jev 核「新增信息」是不是真出自那一帖。核不到的返回 `帖子：那句话`。
pub async fn verify_new_info(
    jev: &JevClient,
    notes: &[MemberNote],
    material_of: impl Fn(&str) -> Option<String>,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for n in notes {
        if n.is_duplicate || n.new_info.trim().is_empty() {
            continue;
        }
        let Some(material) = material_of(&n.candidate_key) else {
            continue;
        };
        let q = BTreeMap::from([(
            "supported".to_string(),
            Question::noul(
                FRAME_SUPPORTED,
                "Everything the statement asserts is in the material.",
                "The statement asserts something the material does not contain.",
            ),
        )]);
        let state = json!({"material": material, "statement": n.new_info});
        let p = jev.ask(&state, &q).await?.noul("supported").unwrap_or(0.0);
        if p < SUPPORTED_THRESHOLD {
            out.push(format!("{}：{}", n.candidate_key, n.new_info));
        }
    }
    Ok(out)
}

/// 把综合结果并进选题，并把要拆出去的帖子各自成题。
pub fn apply(topics: &mut Vec<Topic>, topic_key: &str, s: Synth, js: &[Judgement]) {
    let Some(pos) = topics.iter().position(|t| t.topic_key == topic_key) else {
        return;
    };
    let split: Vec<String> = s.split.iter().map(|(k, _)| k.clone()).collect();
    {
        let t = &mut topics[pos];
        if !s.headline.trim().is_empty() {
            t.headline = s.headline;
        }
        t.synthesis = s.synthesis;
        t.members.retain(|m| !split.contains(m));
        if !s.split.is_empty() {
            let why: Vec<String> = s
                .split
                .iter()
                .map(|(k, r)| format!("{k} 拆出：{r}"))
                .collect();
            if !t.merge_note.is_empty() {
                t.merge_note.push('；');
            }
            t.merge_note.push_str(&why.join("；"));
        }
    }
    for k in split {
        if let Some(j) = js.iter().find(|j| j.candidate_key == k) {
            topics.push(topic_of(&[j], &k, "从同组选题拆出：另一个报道角度"));
        }
    }
}

/// 贴文数与选题数：推荐 / 备选各多少帖、多少题。
pub fn counts(topics: &[Topic], js: &[Judgement]) -> Value {
    let posts = |t: Tier| js.iter().filter(|j| j.tier == t).count();
    let tps = |t: Tier| topics.iter().filter(|x| x.tier == Some(t)).count();
    json!({
        "推荐贴文": posts(Tier::Recommend),
        "推荐选题": tps(Tier::Recommend),
        "备选贴文": posts(Tier::Alternate),
        "备选选题": tps(Tier::Alternate),
        "选题总数": topics.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(primary: &str, members: &[&str]) -> EventGroup {
        EventGroup {
            event_key: primary.into(),
            primary: primary.into(),
            members: members.iter().map(|s| s.to_string()).collect(),
            merge_note: "同一款".into(),
        }
    }

    #[test]
    fn 同一款的三帖合成一个选题且只占一个推荐位() {
        // 09-22 反馈：SOUTH2 WEST8 × BEN MILLER 夹克，两条推荐一条备选
        let js = vec![
            Judgement::fixture("s2w8-a", Tier::Alternate),
            Judgement::fixture("s2w8-b", Tier::Recommend),
            Judgement::fixture("s2w8-c", Tier::Recommend),
            Judgement::fixture("norda-x", Tier::Recommend),
            Judgement::fixture("other", Tier::NotRecommend),
        ];
        let groups = vec![
            g("s2w8-a", &["s2w8-a", "s2w8-b", "s2w8-c"]),
            g("norda-x", &["norda-x"]),
        ];
        let ts = build(&groups, &js);
        assert_eq!(ts.len(), 3, "没进合并的也要自成一题");
        let t = ts.iter().find(|t| t.members.len() == 3).unwrap();
        // 代表帖取档最高的，同档按键
        assert_eq!(t.topic_key, "s2w8-b");
        assert_eq!(t.members[0], "s2w8-b");
        assert_eq!(t.tier, Some(Tier::Recommend));
        let c = counts(&ts, &js);
        assert_eq!(c["推荐贴文"], 3);
        assert_eq!(c["推荐选题"], 2);
        assert!(needs_synthesis(t));
        assert!(!needs_synthesis(
            ts.iter().find(|t| t.topic_key == "norda-x").unwrap()
        ));
    }

    #[test]
    fn 每条候选只属于一个选题() {
        let js = vec![
            Judgement::fixture("a", Tier::Recommend),
            Judgement::fixture("b", Tier::Recommend),
        ];
        // 两组都声称有 b：只算第一次
        let ts = build(&[g("a", &["a", "b"]), g("b", &["b"])], &js);
        let all: Vec<&String> = ts.iter().flat_map(|t| &t.members).collect();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn 另一个角度的帖子拆成独立选题() {
        let js = vec![
            Judgement::fixture("a", Tier::Recommend),
            Judgement::fixture("b", Tier::Recommend),
            Judgement::fixture("c", Tier::Alternate),
        ];
        let mut ts = build(&[g("a", &["a", "b", "c"])], &js);
        apply(
            &mut ts,
            "a",
            Synth {
                headline: "夹克｜飞钓绘画延伸到睡衣".into(),
                synthesis: TopicSynthesis::default(),
                split: vec![("c".into(), "讲的是后续的睡衣系列".into())],
            },
            &js,
        );
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].members, ["a", "b"]);
        assert_eq!(ts[0].headline, "夹克｜飞钓绘画延伸到睡衣");
        assert!(ts[0].merge_note.contains("c 拆出"));
        assert_eq!(ts[1].topic_key, "c");
    }

    #[test]
    fn schema每一层都是严格的() {
        fn walk(v: &Value) {
            if let Some(o) = v.as_object() {
                if o.get("type").and_then(Value::as_str) == Some("object") {
                    assert_eq!(o["additionalProperties"], Value::Bool(false));
                    assert_eq!(
                        o["required"].as_array().unwrap().len(),
                        o["properties"].as_object().unwrap().len()
                    );
                }
                o.values().for_each(walk);
            }
        }
        walk(&schema());
    }
}
