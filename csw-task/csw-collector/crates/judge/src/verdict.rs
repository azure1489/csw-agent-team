//! 逐条判断的第二小步：生成模型综合判断。
//!
//! # 参数是实测定的，不是估的
//!
//! 0.4 在真网关上对 104 条真实候选扫过配置：
//!
//! | 配置 | 每条耗时 | 每条入 token | 429 |
//! |---|---|---|---|
//! | 逐条 · 3 图 · 并发 8 | 9.7 s | 7292 | 1/8 |
//! | **批量 6 · 3 图 · 并发 8** | **8.5 s** | **2370** | 3/8 |
//! | 批量 6 · 0 图 · 并发 4 | 5.4 s | 1421 | 0/4 |
//!
//! 所以 [`BATCH`] = 6、[`IMAGES_PER_CANDIDATE`] = 3：**批量是为了成本不是速度**
//! （省三分之二输入 token，吞吐不变）。并发由模型客户端的全局信号量管，这里不再设。
//!
//! **不发图那一行看着最快，但它不是提速路径**：0.4 实测不发图时模型把 24 条
//! 全部落成 `pending_check`（依据：没读到实图）——这正是口径要求的行为，
//! 也说明纯文本判断按定义就判不出结论。
//!
//! 单批延迟方差极大（124–405 秒），所以客户端超时不能低于 420 秒。
//!
//! # 一批失败只重跑一批
//!
//! 批与批之间互不相关。某一批返回不合 schema、或网关一直失败，
//! **只有那一批的候选没有结论**，其余照常。仍失败的标「未判」并计数，
//! 不算判过、也不算淘汰。

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use csw_collector_core::model::{ModelClient, Part};
use csw_collector_core::types::{
    Candidate, Comparison, ComparisonVerdict, Dim, DimJudgement, Judgement, MediaDescription,
    ThreeSentences, Tier, Triage, Unanswered, Verdict,
};

use crate::materials;
use crate::rubric;

/// 一次请求里放几条候选。见模块文档。
pub const BATCH: usize = 6;
/// 每条候选送几张实图缩略。0.4 实测的配置就是 3。
pub const IMAGES_PER_CANDIDATE: usize = 3;
/// 提示词版本。**改提示词就要改它**，否则旧结论会被当成还能用。
pub const PROMPT_VERSION: &str = "judge/v1";
/// 一批的输出上限。实测每条出 780 token，六条留三倍余量。
const MAX_OUTPUT_TOKENS: u32 = 16000;

/// 一条候选进判断时要带的全部东西。
pub struct JudgeInput<'a> {
    pub candidate: &'a Candidate,
    pub descriptions: &'a [MediaDescription],
    /// 实图缩略的 base64，已按 [`IMAGES_PER_CANDIDATE`] 选好
    pub images_b64: Vec<String>,
    /// 五类对照材料，已装配
    pub materials: &'a [csw_collector_core::types::Material],
    /// Jev 初评；没有就 None（初评失败不该让这条判不了）
    pub triage: Option<&'a Triage>,
    /// 热度说明。是输入的呈现，不是维度。
    pub heat_note: String,
}

/// 挑送进模型的实图：优先能当配图的，其余按原序补足。
///
/// **保持原序**——「第 3 张图」这种依据要能对得上，乱序会让依据指错图。
pub fn pick_images(
    descriptions: &[MediaDescription],
    b64_by_hash: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut chosen: Vec<&MediaDescription> = descriptions
        .iter()
        .filter(|d| d.usable_as_figure)
        .take(IMAGES_PER_CANDIDATE)
        .collect();
    if chosen.len() < IMAGES_PER_CANDIDATE {
        for d in descriptions {
            if chosen.len() >= IMAGES_PER_CANDIDATE {
                break;
            }
            if !chosen.iter().any(|c| c.blake3 == d.blake3) {
                chosen.push(d);
            }
        }
    }
    chosen.sort_by_key(|d| d.ordinal);
    chosen
        .iter()
        .filter_map(|d| b64_by_hash(&d.blake3))
        .collect()
}

/// 判一批。**作业标准原样注入**，不改写、不摘要——它是任务下发的，不是我们定的。
pub async fn judge_batch(
    model: &ModelClient,
    batch: &[JudgeInput<'_>],
    work_standard: &str,
) -> Result<Vec<Judgement>> {
    anyhow::ensure!(!batch.is_empty(), "空批");
    let mut parts = vec![Part::Text(preamble(work_standard))];
    for (i, item) in batch.iter().enumerate() {
        parts.push(Part::Text(candidate_block(i + 1, item)));
        for b in &item.images_b64 {
            parts.push(Part::ImageB64(b.clone()));
        }
    }
    parts.push(Part::Text(closing(batch)));

    let out = model
        .structured(&parts, "csw_judgements", &schema(), MAX_OUTPUT_TOKENS)
        .await
        .context("判断请求")?;
    let wire: WireBatch = out.parse()?;
    anyhow::ensure!(
        wire.judgements.len() == batch.len(),
        "返回 {} 条，送进去 {} 条",
        wire.judgements.len(),
        batch.len()
    );
    Ok(wire.judgements.into_iter().map(Into::into).collect())
}

fn preamble(work_standard: &str) -> String {
    let mut s = String::from(
        "你在为 CSW（中文户外 / 露营 / 城市户外生活杂志）筛选 Instagram 贴文。\
         逐条判断，每条都要判，不要挑。\n\n",
    );
    s.push_str(&rubric::as_prompt_block());
    s.push_str(
        "\n【硬规则】\n\
         - 依据必须引正文原话或指明第几张图。**编不出来就判 unclear，不许造。**\n\
         - 没真正读到实图的，image_seen 填 false 且 tier 必须是 pending_check，\
           并在 gaps 里写明「未读到实图」。待核不是淘汰。\n\
         - 不打分、不排序、不设权重。结论只有四档。\n\
         - 点赞、评论、标签、话题是输入的呈现，写进 heat_note，不作维度。\n\
         - 与 Jev 初评不一致时，在 jev_disagreement 里写明分歧与理由；一致就留空。\n",
    );
    if !work_standard.trim().is_empty() {
        s.push_str("\n【本期作业标准（任务下发，原样照办）】\n");
        s.push_str(work_standard.trim());
        s.push('\n');
    }
    s
}

fn candidate_block(n: usize, item: &JudgeInput<'_>) -> String {
    let c = item.candidate;
    let mut s = format!(
        "\n────────── 候选 {n} ──────────\ncandidate_key：{}\n账号：{}\n链接：{}\n",
        c.candidate_key, c.account, c.url
    );
    if let Some(t) = c.posted_at {
        s.push_str(&format!("发布时间：{t}\n"));
    }
    s.push_str(&format!("热度：{}\n", item.heat_note));
    if !c.tags.is_empty() || !c.hashtags.is_empty() {
        s.push_str(&format!(
            "标签与话题：{}\n",
            [c.tags.join("、"), c.hashtags.join("、")]
                .iter()
                .filter(|x| !x.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("｜")
        ));
    }
    s.push_str(&format!("\n正文：\n{}\n", c.text.trim()));
    if !c.translated.trim().is_empty() {
        s.push_str(&format!("\n译文：\n{}\n", c.translated.trim()));
    }
    s.push_str(&format!("\n图片共 {} 张：\n", c.media.len()));
    if item.descriptions.is_empty() {
        s.push_str("（一张都没识别成功——这条按未读到实图处理）\n");
    }
    for d in item.descriptions {
        s.push_str(&format!(
            "- 第 {} 张（{}）：{}",
            d.ordinal + 1,
            image_kind(d),
            d.content
        ));
        if !d.matches_text.trim().is_empty() {
            s.push_str(&format!("；与正文对应：{}", d.matches_text));
        }
        if !d.missing_from_text.trim().is_empty() {
            s.push_str(&format!("；正文没提到：{}", d.missing_from_text));
        }
        s.push('\n');
    }
    if !item.images_b64.is_empty() {
        s.push_str(&format!(
            "（随附 {} 张实图缩略，紧跟在本段之后）\n",
            item.images_b64.len()
        ));
    }
    s.push_str("\n【对照材料】\n");
    s.push_str(&materials::as_prompt_block(item.materials));
    if let Some(t) = item.triage {
        s.push_str("\n【Jev 初评（六维「成立」概率，供参考；不一致就说明分歧）】\n");
        s.push_str(
            &t.dims
                .iter()
                .map(|(d, p)| format!("{}={p:.2}", dim_key(*d)))
                .collect::<Vec<_>>()
                .join("、"),
        );
        s.push('\n');
        if !t.lower_hits.is_empty() {
            s.push_str(&format!(
                "Jev 认为可能属于降低优先级的：{}\n",
                t.lower_hits
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
    }
    s
}

fn image_kind(d: &MediaDescription) -> String {
    serde_json::to_value(d.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn dim_key(d: Dim) -> String {
    serde_json::to_value(d)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn closing(batch: &[JudgeInput<'_>]) -> String {
    format!(
        "\n────────── 输出 ──────────\n\
         对上面 {} 条候选各给一条判断，**顺序与上面一致**，\
         candidate_key 原样抄回去（别改写、别缩写）。",
        batch.len()
    )
}

// ─────────────────────────── 线上格式 ───────────────────────────

#[derive(Debug, Deserialize)]
struct WireBatch {
    judgements: Vec<WireJudgement>,
}

#[derive(Debug, Deserialize)]
struct WireJudgement {
    candidate_key: String,
    tier: Tier,
    dims: WireDims,
    three_sentences: ThreeSentences,
    unanswered: Unanswered,
    comparison: WireComparison,
    heat_note: String,
    look: String,
    image_seen: bool,
    gaps: Vec<String>,
    priority_hits: Vec<String>,
    lower_hits: Vec<String>,
    jev_disagreement: String,
    kb_refs: Vec<String>,
    memory_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WireDims {
    change: DimJudgement,
    #[serde(rename = "use")]
    use_: DimJudgement,
    gain: DimJudgement,
    compare: DimJudgement,
    explain: DimJudgement,
    csw: DimJudgement,
}

#[derive(Debug, Deserialize)]
struct WireComparison {
    verdict: ComparisonVerdict,
    against: String,
    note: String,
}

impl From<WireJudgement> for Judgement {
    fn from(w: WireJudgement) -> Self {
        Self {
            candidate_key: w.candidate_key,
            tier: w.tier,
            dims: vec![
                (Dim::Change, w.dims.change),
                (Dim::Use, w.dims.use_),
                (Dim::Gain, w.dims.gain),
                (Dim::Compare, w.dims.compare),
                (Dim::Explain, w.dims.explain),
                (Dim::Csw, w.dims.csw),
            ],
            three_sentences: w.three_sentences,
            unanswered: w.unanswered,
            comparison: Comparison {
                verdict: w.comparison.verdict,
                against: w.comparison.against,
                note: w.comparison.note,
            },
            heat_note: w.heat_note,
            look: w.look,
            image_seen: w.image_seen,
            gaps: w.gaps,
            priority_hits: w.priority_hits,
            lower_hits: w.lower_hits,
            jev_disagreement: w.jev_disagreement,
            kb_refs: w.kb_refs,
            memory_refs: w.memory_refs,
            // 由调用方在落库前填：它要把对照材料与准则版本一起算进去
            inputs_hash: String::new(),
        }
    }
}

// ─────────────────────────── JSON Schema ───────────────────────────

/// 严格模式的 schema。**每一层都要写全 `required` 并关掉 `additionalProperties`**——
/// 留 `{}` 占位会被网关直接拒（总方案 §7.4 里的占位就是这么踩到的）。
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"judgements": {"type": "array", "items": judgement_schema()}},
        "required": ["judgements"],
        "additionalProperties": false
    })
}

fn judgement_schema() -> Value {
    let strs = || json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "candidate_key": {"type": "string"},
            "tier": {"type": "string", "enum": ["recommend", "alternate", "not_recommend", "pending_check"]},
            "dims": dims_schema(),
            "three_sentences": obj(&["what_changed", "why_it_matters", "how_different"]),
            "unanswered": {"type": "string", "enum": ["none", "missing_material", "angle_not_formed", "low_value"]},
            "comparison": json!({
                "type": "object",
                "properties": {
                    "verdict": {"type": "string", "enum": ["same_fact_no_gain", "same_brand_with_gain", "unrelated"]},
                    "against": {"type": "string"},
                    "note": {"type": "string"}
                },
                "required": ["verdict", "against", "note"],
                "additionalProperties": false
            }),
            "heat_note": {"type": "string"},
            "look": {"type": "string"},
            "image_seen": {"type": "boolean"},
            "gaps": strs(),
            "priority_hits": strs(),
            "lower_hits": strs(),
            "jev_disagreement": {"type": "string"},
            "kb_refs": strs(),
            "memory_refs": strs()
        },
        "required": [
            "candidate_key", "tier", "dims", "three_sentences", "unanswered", "comparison",
            "heat_note", "look", "image_seen", "gaps", "priority_hits", "lower_hits",
            "jev_disagreement", "kb_refs", "memory_refs"
        ],
        "additionalProperties": false
    })
}

fn dims_schema() -> Value {
    let one = json!({
        "type": "object",
        "properties": {
            "verdict": {"type": "string", "enum": ["yes", "no", "unclear"]},
            "basis": {"type": "string"}
        },
        "required": ["verdict", "basis"],
        "additionalProperties": false
    });
    let keys = ["change", "use", "gain", "compare", "explain", "csw"];
    let props: serde_json::Map<String, Value> = keys
        .iter()
        .map(|k| ((*k).to_string(), one.clone()))
        .collect();
    json!({
        "type": "object",
        "properties": props,
        "required": keys,
        "additionalProperties": false
    })
}

fn obj(keys: &[&str]) -> Value {
    let props: serde_json::Map<String, Value> = keys
        .iter()
        .map(|k| ((*k).to_string(), json!({"type": "string"})))
        .collect();
    json!({
        "type": "object",
        "properties": props,
        "required": keys,
        "additionalProperties": false
    })
}

/// 没读到实图时的兜底结论。模型判不出来时由代码给，**口径不能靠模型自觉**。
pub fn pending_for_missing_image(c: &Candidate, reason: &str) -> Judgement {
    let basis = format!("未读到实图：{reason}");
    Judgement {
        candidate_key: c.candidate_key.clone(),
        tier: Tier::PendingCheck,
        dims: Dim::ALL
            .into_iter()
            .map(|d| {
                (
                    d,
                    DimJudgement {
                        verdict: Verdict::Unclear,
                        basis: basis.clone(),
                    },
                )
            })
            .collect(),
        three_sentences: ThreeSentences {
            what_changed: String::new(),
            why_it_matters: String::new(),
            how_different: String::new(),
        },
        unanswered: Unanswered::MissingMaterial,
        comparison: Comparison {
            verdict: ComparisonVerdict::Unrelated,
            against: String::new(),
            note: basis.clone(),
        },
        heat_note: String::new(),
        look: String::new(),
        image_seen: false,
        gaps: vec![basis],
        priority_hits: vec![],
        lower_hits: vec![],
        jev_disagreement: String::new(),
        kb_refs: vec![],
        memory_refs: vec![],
        inputs_hash: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{ImageKind, MediaKind, MediaRef, Platform};

    fn cand() -> Candidate {
        Candidate {
            candidate_key: "yamatomichi-ab12cd".into(),
            platform: Platform::Instagram,
            source_id: "ab12cd".into(),
            collector: "csw-window".into(),
            account: "yamatomichi".into(),
            url: "https://instagram.com/p/ab12cd".into(),
            text: "新しいバックパックを発表".into(),
            translated: "发布了新背包".into(),
            posted_at: None,
            ingested_at: None,
            likes: Some(369),
            comments: Some(12),
            followers: Some(90000),
            heat_ratio: Some(5.2),
            content_type: "Carousel".into(),
            media: (0..4)
                .map(|i| MediaRef {
                    source_hash: format!("h{i}"),
                    kind: MediaKind::Photo,
                    url: format!("https://x/{i}.jpg"),
                    blake3: Some(format!("b{i}")),
                    ordinal: i,
                })
                .collect(),
            tags: vec!["新品".into()],
            hashtags: vec!["outdoor".into()],
        }
    }

    fn desc(n: u16, usable: bool) -> MediaDescription {
        MediaDescription {
            blake3: format!("b{n}"),
            ordinal: n,
            matches_text: String::new(),
            content: format!("画面{n}：一只背包"),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: usable,
            model: "m".into(),
            prompt_version: "v".into(),
        }
    }

    /// 递归检查：每个 object 节点都要有 additionalProperties:false，
    /// 且 required 恰好等于 properties 的键集。
    fn assert_strict(v: &Value, path: &str) {
        if let Some(o) = v.as_object() {
            if o.get("type").and_then(Value::as_str) == Some("object") {
                assert_eq!(
                    o.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "{path} 没关 additionalProperties"
                );
                let props: Vec<&String> = o["properties"].as_object().unwrap().keys().collect();
                let req: Vec<String> = o["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_string())
                    .collect();
                let mut a: Vec<String> = props.iter().map(|s| (*s).clone()).collect();
                let mut b = req.clone();
                a.sort();
                b.sort();
                assert_eq!(a, b, "{path} 的 required 与 properties 对不上");
            }
            for (k, x) in o {
                assert_strict(x, &format!("{path}.{k}"));
            }
        }
        if let Some(a) = v.as_array() {
            for (i, x) in a.iter().enumerate() {
                assert_strict(x, &format!("{path}[{i}]"));
            }
        }
    }

    #[test]
    fn schema每一层都是严格的() {
        // 留一个 {} 占位、或漏写一个 required，网关会直接拒——
        // 总方案 §7.4 里的占位就是这么踩到的
        assert_strict(&schema(), "root");
    }

    #[test]
    fn schema的六维与类型里的一致() {
        let s = schema();
        let dims = &s["properties"]["judgements"]["items"]["properties"]["dims"]["properties"];
        for d in Dim::ALL {
            assert!(dims.get(dim_key(d)).is_some(), "schema 少了 {d:?}");
        }
        assert_eq!(dims.as_object().unwrap().len(), 6);
    }

    #[test]
    fn 解析返回并还原成六维有序() {
        let one = |v: &str| json!({"verdict": v, "basis": "正文第一句"});
        let body = json!({"judgements": [{
            "candidate_key": "yamatomichi-ab12cd",
            "tier": "recommend",
            "dims": {"change": one("yes"), "use": one("unclear"), "gain": one("yes"),
                     "compare": one("no"), "explain": one("yes"), "csw": one("yes")},
            "three_sentences": {"what_changed": "甲", "why_it_matters": "乙", "how_different": "丙"},
            "unanswered": "none",
            "comparison": {"verdict": "unrelated", "against": "", "note": ""},
            "heat_note": "369 赞 · 常态的 5.2×",
            "look": "灰色主体",
            "image_seen": true,
            "gaps": [], "priority_hits": ["老产品结构性改款"], "lower_hits": [],
            "jev_disagreement": "", "kb_refs": [], "memory_refs": []
        }]});
        let w: WireBatch = serde_json::from_value(body).unwrap();
        let j: Judgement = w.judgements.into_iter().next().unwrap().into();
        // 顺序要与 Dim::ALL 一致：台账按这个顺序画六个小格
        assert_eq!(
            j.dims.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            Dim::ALL.to_vec()
        );
        // use 是 Rust 关键字，序列化名必须仍是 "use"
        assert_eq!(j.dims[1].1.verdict, Verdict::Unclear);
        // inputs_hash 由调用方在落库前填：它要把对照材料与准则版本一起算进去，
        // 所以刚解析出来时它必然缺，且这正是 violations 要挡的
        assert_eq!(j.violations(), ["缺 inputs_hash"]);
        let filled = Judgement {
            inputs_hash: "h".into(),
            ..j
        };
        assert!(filled.violations().is_empty(), "{:?}", filled.violations());
    }

    #[test]
    fn 挑图优先能当配图的且保持原序() {
        let ds = [desc(0, false), desc(1, true), desc(2, false), desc(3, true)];
        let got = pick_images(&ds, |h| Some(h.to_string()));
        // 先挑 usable 的（1、3），再按原序补一张（0），最后按 ordinal 排回去
        assert_eq!(got, ["b0", "b1", "b3"], "「第 3 张图」这种依据要对得上");
        assert!(got.len() <= IMAGES_PER_CANDIDATE);
    }

    #[test]
    fn 取不到缩略就少送不报错() {
        let ds = [desc(0, true), desc(1, true)];
        // 下载失败的图没有缩略：少送几张，不该让整条判不了
        let got = pick_images(&ds, |h| (h == "b0").then(|| h.to_string()));
        assert_eq!(got, ["b0"]);
    }

    #[test]
    fn 没读到实图的兜底结论自己就合规() {
        let j = pending_for_missing_image(&cand(), "3 张下载失败");
        // 口径不能靠模型自觉：代码给的兜底除了待补的 inputs_hash 之外必须自己合规
        assert_eq!(j.violations(), ["缺 inputs_hash"]);
        assert_eq!(j.tier, Tier::PendingCheck);
        assert!(!j.image_seen);
        assert!(j.gaps.iter().any(|g| g.contains("未读到实图")));
        assert_eq!(j.dims.len(), 6);
    }

    #[test]
    fn 提示词里有硬规则与原样注入的作业标准() {
        let p = preamble("本期只收 9/17 之后入库的；每条都要写原始披露时间。");
        assert!(
            p.contains("pending_check"),
            "没读到实图那条规则要写死在提示词里"
        );
        assert!(p.contains("不许造"));
        assert!(p.contains("不打分"));
        assert!(p.contains("每条都要写原始披露时间"), "作业标准要原样注入");
        assert!(p.contains("品牌知名度"), "「不是维度」那段不能漏");
        // 空的作业标准不该留一个空标题
        assert!(!preamble("   ").contains("作业标准"));
    }

    #[test]
    fn 候选段里图片序号从一起算() {
        let c = cand();
        let ds = vec![desc(0, true), desc(1, true)];
        let item = JudgeInput {
            candidate: &c,
            descriptions: &ds,
            images_b64: vec!["x".into()],
            materials: &[],
            triage: None,
            heat_note: "369 赞 · 常态的 5.2×".into(),
        };
        let s = candidate_block(1, &item);
        assert!(s.contains("第 1 张"), "{s}");
        assert!(s.contains("第 2 张"));
        assert!(!s.contains("第 0 张"), "序号从 0 起会让依据对不上图");
        assert!(s.contains("yamatomichi-ab12cd"));
        assert!(s.contains("译文"));
        assert!(s.contains("369 赞"));
        // 五类对照材料即使全空也要出现，让模型看到「查过了、没有」
        assert!(s.contains("上一轮台账"));
    }

    #[test]
    fn 一张都没识别成功要在段里说清楚() {
        let c = cand();
        let item = JudgeInput {
            candidate: &c,
            descriptions: &[],
            images_b64: vec![],
            materials: &[],
            triage: None,
            heat_note: String::new(),
        };
        let s = candidate_block(1, &item);
        assert!(s.contains("一张都没识别成功"), "{s}");
    }

    #[test]
    fn 初评概率进提示词但不出现分数字样() {
        let c = cand();
        let t = Triage {
            candidate_key: c.candidate_key.clone(),
            dims: Dim::ALL.into_iter().map(|d| (d, 0.66)).collect(),
            worth: 0.46,
            lower_hits: vec![("只有新配色或普通 logo 联名".into(), 0.8)],
            model: String::new(),
        };
        let item = JudgeInput {
            candidate: &c,
            descriptions: &[],
            images_b64: vec![],
            materials: &[],
            triage: Some(&t),
            heat_note: String::new(),
        };
        let s = candidate_block(1, &item);
        assert!(s.contains("change=0.66"));
        assert!(s.contains("只有新配色"));
        // worth 不进提示词：它是内部排序键，0.7 实测当闸不可用
        assert!(!s.contains("0.46"), "总判 worth 不该露给判断模型");
    }
}
