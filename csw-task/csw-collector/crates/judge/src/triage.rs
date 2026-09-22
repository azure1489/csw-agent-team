//! Jev 初评：给每条候选一个**内部排序键**。
//!
//! # 它只排序，不淘汰
//!
//! 0.7 回测用真金标（正样本 = csw 里真被写成文章的贴文，负样本 = 同窗口没被写的）
//! 扫过阈值：想筛掉一半候选，就得丢掉三成本该写的。所以这一步**不设闸**，
//! 只决定「先判哪条」，好让首批能早点交出去。
//!
//! 排序键用 `explain + csw + use` 三维——回测里它们分得最开；`change` 与 `gain`
//! 几乎不分（Instagram 文案普遍都在宣布点什么、都带点事实），拿来排序没意义。
//! **这个数绝不进台账、绝不显示成分数**：「不打分、无权重」管的是判断输出，
//! 不是内部调度，但也正因如此，它不能泄漏到输出里去。
//!
//! # 问法保持英文原样
//!
//! 下面这些 instructions 是 0.7 回测里一字不差用过的那套，上面那些数字就是它们打出来的。
//! **翻译成中文等于换了一套问法，回测结论随之作废。** 要改先重测。
//!
//! 唯一有意改动的是 [`FRAME`] 结尾那句：回测时只给正文、没有图，所以要叮嘱模型
//! 「别为照片里才看得见的东西扣分」；正式运行时每张图都有识别描述，那句话就不对了。
//! 这个偏离是知情的——初评只用来排序，绝对阈值本来就不取用。
//!
//! # 送什么
//!
//! **只送贴文正文、译文、图片描述、账号、图数、热度。** Van 的原话、编辑部决定、
//! 对照材料一律不送——Jev 是第三方。state 在 [`state_of`] 里**逐字段列举**，
//! 不把结构体整个扔进去，就是为了让「多送了什么」在代码审查时一眼看得见。

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;

use csw_collector_core::jev::{JevClient, Question};
use csw_collector_core::types::{Candidate, Dim, MediaDescription, Triage};

use crate::rubric::LOWER;

/// 每个问题的 instructions 前都要拼上它——Jev 不支持请求级 instructions。
const FRAME_HEAD: &str = "You are screening one Instagram post for CSW, a Chinese-language magazine \
about outdoor, camping and urban-outdoor living. CSW's core question: does this contain a change \
concrete enough that CSW should explain it to readers once? A post that is not a new product can \
still qualify through a grounded design, culture, history or current-affairs angle. Brand fame is \
NOT a criterion. Judge only from the fields in state.";

/// 没有图片描述时的结尾：与 0.7 回测一字不差。
const FRAME_NO_IMAGES: &str = " You see the caption only — no images — so do not penalise a post \
for things that would be visible in photos.";

/// 有图片描述时的结尾。**这是对回测条件的知情偏离**，见模块文档。
const FRAME_WITH_IMAGES: &str = " The `photos` field describes what each photo shows; treat those \
descriptions as if you had seen the photos.";

fn frame(has_images: bool) -> &'static str {
    if has_images {
        FRAME_WITH_IMAGES
    } else {
        FRAME_NO_IMAGES
    }
}

/// 六维的问法。**一字不差来自 0.7 回测。**
fn dim_questions() -> [(Dim, &'static str, [&'static str; 3]); 6] {
    [
        (
            Dim::Change,
            "Does the caption state a concrete change — a new product, version, collaboration, material, structure, event or space — with specifics?",
            [
                "No concrete change: a reshown product, a greeting, a mood shot, a generic promotion.",
                "Something may have changed but the caption does not say what specifically.",
                "A specific, nameable change is stated.",
            ],
        ),
        (
            Dim::Use,
            "Does the caption connect to how a person would actually wear, carry, pitch, cook with or travel with the thing?",
            [
                "No usage relation; only appearance or an announcement.",
                "Usage is implied but not described.",
                "Usage, wearing, carrying or a concrete scenario is described.",
            ],
        ),
        (
            Dim::Gain,
            "Does the caption add information a reader could learn from — design intent, specifications, release date, price, channel, background?",
            [
                "Only a name, or nothing.",
                "One vague fact.",
                "Several concrete facts an editor could cite.",
            ],
        ),
        (
            Dim::Compare,
            "Does the caption give, or obviously allow, a comparison reference: a previous generation, the brand's earlier practice, or a similar product?",
            [
                "No reference and none implied.",
                "A reference is hinted but not stated.",
                "A comparison reference is stated or obviously available.",
            ],
        ),
        (
            Dim::Explain,
            "Is there an editorial point with factual support — something whose design choice can be tied to a use, a need or a piece of history? 'Looks good' or 'interesting' is not enough.",
            [
                "Nothing to explain.",
                "Possibly, but the caption gives too little.",
                "A clear point that rewards explanation.",
            ],
        ),
        (
            Dim::Csw,
            "Does the subject sit inside CSW's territory — outdoor, camping, urban-outdoor living, with a design or culture angle, for readers in China?",
            [
                "Off-topic, or a purely local shop notice.",
                "Related but marginal, or hard to relate to readers in China.",
                "Squarely in CSW's territory.",
            ],
        ),
    ]
}

/// 七类降低优先级的英文问法，与 [`LOWER`] 一一对应、同序。
///
/// 0.7 里把七类合成**一个** noul 问，已写过 0.39、未写过 0.46，几乎不分。
/// 按 TypeSafe 自己的指引，多标签共存时该一类一个 noul。这里就是那七个。
const LOWER_QUESTIONS: [&str; 7] = [
    "Is this post only a new colourway, or a plain logo-swap collaboration with nothing else changed?",
    "Does it swap a material or fabric without saying what difference that makes?",
    "Does it claim innovation or a breakthrough with nothing specific that could be verified?",
    "Is it purely a celebrity or influencer placement, with no information about the thing itself?",
    "Does it lack any reliable source — an unattributed rumour, a reposted screenshot, no original account?",
    "Is the only thing on offer that it looks good?",
    "Would calling this a trend require stretching a single instance into a general claim?",
];

/// 送给 Jev 的 state。**逐字段列举**，见模块文档。
pub fn state_of(c: &Candidate, descriptions: &[MediaDescription], heat: &str) -> serde_json::Value {
    let mut s = json!({
        "account": c.account,
        "caption": c.text,
        "content_type": c.content_type,
        "photo_count": c.media.len(),
        "heat": heat,
    });
    if !descriptions.is_empty() {
        s["photos"] = json!(
            descriptions
                .iter()
                .map(|d| json!({"n": d.ordinal + 1, "shows": d.content}))
                .collect::<Vec<_>>()
        );
    }
    s
}

/// 一条候选的全部初评问题。六维 + 总判 + 七类降低优先级，**一次问完**——
/// 它们互不依赖，分成几次请求只是更慢更贵。
fn questions(has_images: bool) -> BTreeMap<String, Question> {
    let f = |q: &str| format!("{FRAME_HEAD}{} {q}", frame(has_images));
    let mut m = BTreeMap::new();
    for (dim, instr, lv) in dim_questions() {
        m.insert(
            dim_key(dim),
            Question::score3(&f(instr), lv[0], lv[1], lv[2]),
        );
    }
    m.insert(
        "worth".to_string(),
        Question::noul(
            &f("Overall: is there a change concrete enough here that CSW should explain it to readers once?"),
            "Yes — an editor would take this into a selection meeting.",
            "No — there is nothing here CSW would explain.",
        ),
    );
    for (i, q) in LOWER_QUESTIONS.iter().enumerate() {
        m.insert(
            format!("lower{i}"),
            Question::noul(&f(q), "Yes, it is one of those.", "No, it is not."),
        );
    }
    m
}

fn dim_key(d: Dim) -> String {
    serde_json::to_value(d)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 命中「降低优先级」的概率到多少才记一笔。只是提示，不是闸。
pub const LOWER_HIT_THRESHOLD: f64 = 0.5;

/// 对一条候选做初评。
///
/// **失败不该让这条候选判不了**：调用方拿到 `Err` 就跳过初评，
/// 该条按默认顺序排，照常进入逐条判断。
pub async fn triage_one(
    jev: &JevClient,
    c: &Candidate,
    descriptions: &[MediaDescription],
    heat: &str,
) -> Result<Triage> {
    let has_images = !descriptions.is_empty();
    let a = jev
        .ask(&state_of(c, descriptions, heat), &questions(has_images))
        .await?;

    let dims: Vec<(Dim, f64)> = Dim::ALL
        .into_iter()
        .map(|d| (d, a.score_top(&dim_key(d)).unwrap_or(0.0)))
        .collect();
    let lower_hits: Vec<(String, f64)> = LOWER
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let p = a.noul(&format!("lower{i}"))?;
            (p >= LOWER_HIT_THRESHOLD).then(|| ((*name).to_string(), p))
        })
        .collect();

    Ok(Triage {
        candidate_key: c.candidate_key.clone(),
        dims,
        worth: a.noul("worth").unwrap_or(0.0),
        lower_hits,
        model: jev_model(jev),
    })
}

fn jev_model(_jev: &JevClient) -> String {
    // 客户端不外露配置；模型标识由调用方在落库时补全更合适，这里给个占位
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{ImageKind, MediaKind, MediaRef};

    fn cand() -> Candidate {
        Candidate {
            candidate_key: "yamatomichi-ab12cd".into(),
            platform: csw_collector_core::types::Platform::Instagram,
            source_id: "ab12cd".into(),
            collector: "csw-window".into(),
            account: "yamatomichi".into(),
            url: "https://instagram.com/p/ab12cd".into(),
            text: "新しいバックパックを発表しました".into(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: Some(369),
            comments: Some(12),
            followers: Some(90000),
            heat_ratio: Some(5.2),
            content_type: "Carousel".into(),
            media: vec![MediaRef {
                source_hash: "h1".into(),
                kind: MediaKind::Photo,
                url: "https://x/1.jpg".into(),
                ordinal: 0,
                blake3: None,
            }],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn desc(n: u16, content: &str) -> MediaDescription {
        MediaDescription {
            blake3: format!("b{n}"),
            ordinal: n,
            matches_text: String::new(),
            content: content.into(),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: "v".into(),
        }
    }

    #[test]
    fn state只带该带的字段() {
        let s = state_of(&cand(), &[], "369 赞");
        let keys: Vec<&String> = s.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["account", "caption", "content_type", "heat", "photo_count"]
        );
        // Jev 是第三方：Van 原话、对照材料、内部决定一概不送
        let text = s.to_string();
        assert!(!text.contains("van") && !text.contains("decision"));
    }

    #[test]
    fn 有图片描述就把图带上() {
        let s = state_of(&cand(), &[desc(0, "一只灰色背包的正面")], "369 赞");
        assert_eq!(s["photos"][0]["n"], 1, "给人看的序号从 1 起");
        assert_eq!(s["photos"][0]["shows"], "一只灰色背包的正面");
    }

    #[test]
    fn 有没有图用的是两套frame() {
        let no = questions(false);
        let yes = questions(true);
        let pick = |m: &BTreeMap<String, Question>| match m.get("change").unwrap() {
            Question::Score { instructions, .. } => instructions.clone(),
            _ => panic!("change 该是 score"),
        };
        // 回测时只给正文，所以叮嘱「别为照片里才看得见的东西扣分」；
        // 正式运行每张图都有描述，那句话就不对了
        assert!(pick(&no).contains("no images"));
        assert!(pick(&yes).contains("treat those descriptions"));
        assert!(
            pick(&yes).contains("Brand fame is NOT a criterion"),
            "框架头不能丢"
        );
    }

    #[test]
    fn 一次问完六维加总判加七类() {
        let q = questions(true);
        // 它们互不依赖，分成几次请求只是更慢更贵
        assert_eq!(q.len(), 6 + 1 + 7);
        for d in Dim::ALL {
            assert!(q.contains_key(&dim_key(d)), "缺 {d:?}");
        }
        assert!(matches!(q.get("worth"), Some(Question::Noul { .. })));
        // 七类要一类一个 noul：合成一个问法实测几乎不分（0.39 vs 0.46）
        for i in 0..7 {
            assert!(matches!(
                q.get(&format!("lower{i}")),
                Some(Question::Noul { .. })
            ));
        }
        assert_eq!(LOWER_QUESTIONS.len(), LOWER.len());
    }

    #[test]
    fn 排序键用分得开的三维() {
        let t = Triage {
            candidate_key: "k".into(),
            dims: vec![
                (Dim::Change, 0.99),
                (Dim::Use, 0.57),
                (Dim::Gain, 0.99),
                (Dim::Compare, 0.57),
                (Dim::Explain, 0.68),
                (Dim::Csw, 0.64),
            ],
            worth: 0.46,
            lower_hits: vec![],
            model: String::new(),
        };
        // explain + csw + use，不含 change 与 gain（回测里那两维几乎不分）
        assert!((t.priority() - (0.68 + 0.64 + 0.57)).abs() < 1e-9);
    }
}
