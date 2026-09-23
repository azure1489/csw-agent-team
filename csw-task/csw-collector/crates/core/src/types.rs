//! 跨 crate 的值类型。**改这里等于改契约**——阶段 1 冻结后，任何改动都要同步
//! OpenAPI（`crates/collector`）、前端 TS 类型与 `API.md`，并在实施进度里记一笔。
//!
//! 三条贯穿全文件的口径，代码里不再逐处重复：
//! - **时间一律 UTC**，只在界面与引擎交互的边界换算北京时间。
//! - **不打数字分、不设权重。** 判断的产物是档位 + 六维成立与否 + 依据，没有分。
//!   Jev 初评的概率只在内部决定「先判哪条」，不进台账、不显示。
//! - **每条都判、每张图都识别。** 没读到实图就是 `PendingCheck`，不是淘汰。

use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// UTC 时刻。库里存 RFC3339 字符串，比存整数好查、好人肉核对。
pub type Timestamp = jiff::Timestamp;

// utoipa 不认识 jiff 的类型，所以每个 `Timestamp` 字段都带
// `#[schema(value_type = Option<String>, format = DateTime)]`。
// 前端拿到的一律是 `"2026-09-22T01:40:00Z"` 这种形态。

// ─────────────────────────────── 轮次 ───────────────────────────────

/// 一轮是怎么起来的。
///
/// `Prefetch` 是 0.4 / 0.5 实测之后加的：网关侧一轮要约 95 分钟，
/// 05:30 接单再从零跑必然超时。预取只写本地缓存，不写引擎、不群播报。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoundKind {
    /// 引擎派单驱动
    Task,
    /// 人在工作台上手动开启（operator / superadmin）
    Manual,
    /// 凌晨预取，只写本地缓存
    Prefetch,
    /// 回放夹具，不出网
    Replay,
}

/// 这一轮由什么事件触发。与 `RoundKind::Task` 配合区分正常派单、退回返工与补件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoundTrigger {
    Dispatch,
    Returned,
    Supplement,
    Manual,
}

/// 十步流程。步与步之间靠 `input_hash` 判是否可复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepCode {
    /// 1 接单并建轮
    Intake,
    /// 2 采集媒体信息（取候选 → 下载 → 识别 → 向量化，四段连着做完才算这步完成）
    Harvest,
    /// 3 合并同一事件
    Merge,
    /// 4 取五类对照材料
    Materials,
    /// 5 逐条判断
    Judge,
    /// 6 深核首批
    Deepcheck,
    /// 7 生成交付物
    Build,
    /// 8 登记（items / sweeps / judgements）
    Register,
    /// 9 自查（intake-check）
    SelfCheck,
    /// 10 提交
    Submit,
}

impl StepCode {
    pub const ALL: [StepCode; 10] = [
        Self::Intake,
        Self::Harvest,
        Self::Merge,
        Self::Materials,
        Self::Judge,
        Self::Deepcheck,
        Self::Build,
        Self::Register,
        Self::SelfCheck,
        Self::Submit,
    ];

    /// 界面上的序号，1 起
    pub fn ordinal(self) -> u8 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8 + 1
    }
}

/// 一步的状态。
///
/// `Interrupted` 专给进程崩溃后重启用：启动时把还挂着 `Running` 的改成 `Interrupted`，
/// 再去续跑没做完的单元，而不是从头再来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    /// 部分单元失败但不阻塞下游（如单张图识别失败标「未识别」）
    Partial,
    Failed,
    Skipped,
    /// 上游重跑过，这一步的产物作废，需要重算
    Stale,
    Interrupted,
}

impl StepStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Skipped
        )
    }
}

// ─────────────────────────────── 候选 ───────────────────────────────

/// 候选来自哪个平台。采集器可以有很多个，平台就这几类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Instagram,
    Xiaohongshu,
    Web,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Instagram => "instagram",
            Self::Xiaohongshu => "xhs",
            Self::Web => "web",
        })
    }
}

/// 统一候选格式。不管从哪个采集器来，到这一层就长一个样，
/// 后面的合并、对照、判断都不必关心它从哪来。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Candidate {
    /// 条目键：品牌小写 + `-` + 链接 sha256 前 6 位。与交付物里的条目键同一个。
    pub candidate_key: String,
    pub platform: Platform,
    /// 平台侧的原始 id（Instagram 是短码 / postId）
    pub source_id: String,
    /// 哪个采集器取到的
    pub collector: String,
    pub account: String,
    pub url: String,
    /// 原文
    pub text: String,
    /// 平台给的译文，没有就空
    pub translated: String,
    /// 原始披露时间
    #[schema(value_type = Option<String>, format = DateTime)]
    pub posted_at: Option<Timestamp>,
    /// 首次进入来源库的时间。**窗口按这个算**——`posts/window` 只按发布时间过滤，
    /// 所以采集器取更宽的发布时间窗口，再在本地按这个字段收口。
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ingested_at: Option<Timestamp>,
    pub likes: Option<i64>,
    pub comments: Option<i64>,
    pub followers: Option<i64>,
    /// 相对该账号近 90 天中位点赞的倍数。热度是输入，不是维度。
    pub heat_ratio: Option<f64>,
    /// 来源平台的内容类型原值（Image / Carousel / Reel …），留原文便于追。
    pub content_type: String,
    pub media: Vec<MediaRef>,
    /// csw 后台的标签与话题。线上接口目前不返回，采集器要容忍缺失。
    pub tags: Vec<String>,
    pub hashtags: Vec<String>,
}

impl Candidate {
    /// 只取图文，判据是严格的：类型是图或轮播，**且**媒体里一个视频都没有。
    pub fn is_image_only(&self) -> bool {
        matches!(self.content_type.as_str(), "Image" | "Carousel")
            && !self.media.is_empty()
            && self.media.iter().all(|m| m.kind == MediaKind::Photo)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MediaRef {
    /// 来源侧的媒体哈希。**跨账号转载时对不上**（实测窗口内共享哈希的组为 0），
    /// 所以它只能去同一条内的重复，合并同一事件得靠向量 + Jev。
    pub source_hash: String,
    pub kind: MediaKind,
    pub url: String,
    /// 下载并按内容哈希存盘后的 blake3，未下载时为空
    pub blake3: Option<String>,
    pub ordinal: u16,
}

/// 一张图的识别结果（总方案 §5.5 的结构化输出）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MediaDescription {
    pub blake3: String,
    pub ordinal: u16,
    /// 对应正文哪一点；正文里没有就写「正文未提及」
    pub matches_text: String,
    /// 画面里实际有什么。只写看得见的。
    pub content: String,
    /// 正文提到但画面没有的
    pub missing_from_text: String,
    pub kind: ImageKind,
    pub usable_as_figure: bool,
    /// 模型与提示词版本，便于判「这条描述该不该重算」
    pub model: String,
    pub prompt_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    Product,
    Detail,
    Scene,
    Poster,
    Outfit,
    Screenshot,
    Unrelated,
}

// ─────────────────────────────── 对照材料 ───────────────────────────

/// 五类对照材料，**缺一不判**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    /// 正式已发布的条目
    Published,
    /// 知识库范例
    Example,
    /// 生成过文章的贴文
    GeneratedPost,
    /// 03 阶段 Van 的决定（含原话）
    Decision,
    /// 上一轮台账。留在本地库，**不进参考库**——否则系统自己的判断会被当成 Van 的口味证据。
    PriorLedger,
    /// 选题记忆（P5 回收的采用 / 否决案例）。**只带对象与结论，不带 Van 原话**。
    /// 不在 [`MaterialKind::REQUIRED`] 里：记忆空着不该让整轮判不了。
    Memory,
}

impl MaterialKind {
    pub const REQUIRED: [MaterialKind; 5] = [
        Self::Published,
        Self::Example,
        Self::GeneratedPost,
        Self::Decision,
        Self::PriorLedger,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Material {
    pub kind: MaterialKind,
    /// 来源标识（已发条目 id、范例编号、贴文短码、决定 id、上一轮的 candidate_key）
    pub ref_id: String,
    pub title: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub date: Option<Timestamp>,
    pub source: String,
    /// 原话。Van 的决定类必须带。
    pub quote: String,
    /// 发布状态，供「30 天查重」区分正式已发与未发
    pub publish_state: String,
    /// 正文节选（已发、范例两类才有）。**查重要看正文**——只给标题，模型拿不到
    /// 重复的事实，就会把「正文缺失」判成「无关」（09-22 反馈 Dapple Born）。
    #[serde(default)]
    pub body_excerpt: String,
    /// 这条材料的正文取没取到。取不到时查重只能是「未确认」。
    #[serde(default = "yes")]
    pub body_available: bool,
    #[serde(default)]
    pub brand: String,
}

fn yes() -> bool {
    true
}

// ─────────────────────────────── 判断 ───────────────────────────────

/// 结论档。**只有这四个，没有分数。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Recommend,
    Alternate,
    NotRecommend,
    /// 证据或图片不足。**不算判过，也不因此淘汰**，补齐后重评。
    PendingCheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Yes,
    No,
    Unclear,
}

/// Van 的六个维度。**键固定**（引擎按键名校验），含义以 `van-rubric/v2` 为准
/// （09-23 Van 本人认可的新定义）。改维度要她本人点头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    /// 具体看点：最值得报道的具体事实、变化或角度（不强求新品）
    Change,
    /// 与读者有关：与中国户外潮流爱好者的使用、穿搭、审美、兴趣、生活方式的联系
    Use,
    /// 值得推荐的价值——**核心维度**，其余为它提供支撑
    Gain,
    /// 差异与背景：同类、历史、品牌背景、既有做法
    Compare,
    /// 报道角度与依据：CSW 抓哪个角度、哪些事实撑得住
    Explain,
    /// CSW 适配度：结合实际采用 / 否决案例说明
    Csw,
}

impl Dim {
    pub const ALL: [Dim; 6] = [
        Self::Change,
        Self::Use,
        Self::Gain,
        Self::Compare,
        Self::Explain,
        Self::Csw,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DimJudgement {
    pub verdict: Verdict,
    /// 依据：引正文原话，或指明第几张图。**编不出来就判 Unclear，不许造。**
    /// Jev 的核对步骤专门抓这里的编造（实测张冠李戴 100% 抓出）。
    pub basis: String,
}

/// 三句话（v2）：是什么 / 为什么值得看 / 依据是什么。
///
/// v1 的键是 `what_changed` / `why_it_matters` / `how_different`——字段名本身就在逼
/// 模型找「变化」（09-22 反馈第二项）。旧行按别名读回。
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ThreeSentences {
    #[serde(alias = "what_changed")]
    pub what: String,
    #[serde(alias = "why_it_matters")]
    pub why_worth: String,
    #[serde(alias = "how_different")]
    pub grounds: String,
}

/// 看点属于哪一种。**没有旧款证据就不许写「从……变成……」。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoveltyKind {
    /// 产品现有特点
    #[default]
    ExistingFeature,
    /// 有证据的新变化——必须写明旧款 / 前代依据
    EvidencedChange,
    /// 值得解释的设计或文化内容
    ExplainableDesign,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Novelty {
    pub kind: NoveltyKind,
    pub basis: String,
    /// 旧款 / 前代依据出自哪里。`EvidencedChange` 时必填。
    pub prior_evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    /// 品牌或当事方原始发布
    Primary,
    /// 转载
    Reshared,
    /// 只有品牌自述，无旁证
    BrandClaimOnly,
    #[default]
    Unknown,
}

/// 制作条件。与选题价值**分开记**，不参与定档（09-22 反馈第一项）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Readiness {
    pub fact_source: FactSource,
    /// 可作配图的实图张数
    pub usable_images: u8,
    pub material_complete: bool,
    pub note: String,
}

/// 缺口级别（09-22 反馈第八项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GapLevel {
    /// 影响选题判断：产品身份、关键看点、报道价值无法确认。**只有这一级能让条目落待核。**
    #[default]
    Decision,
    /// 影响成稿：必要规格、关键图片、时间信息缺失。不影响档位。
    Production,
    /// 表达边界：如品牌声称的性能未经独立实测。只是写作提醒，不是补证任务。
    Boundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GapOwner {
    #[default]
    Collector,
    Editor,
    Van,
}

/// 一条缺口：缺什么、谁处理、已尝试什么、下一步。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ToSchema)]
pub struct Gap {
    pub level: GapLevel,
    pub what: String,
    pub owner: GapOwner,
    pub tried: String,
    pub next: String,
}

impl From<&str> for Gap {
    fn from(s: &str) -> Self {
        Gap::decision(s)
    }
}

impl Gap {
    pub fn decision(what: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            ..Default::default()
        }
    }
}

/// 旧行里的缺口是纯字符串——读回时按「影响判断、收集员处理」兜底。
impl<'de> Deserialize<'de> for Gap {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Text(String),
            Full {
                #[serde(default)]
                level: GapLevel,
                #[serde(default)]
                what: String,
                #[serde(default)]
                owner: GapOwner,
                #[serde(default)]
                tried: String,
                #[serde(default)]
                next: String,
            },
        }
        Ok(match Either::deserialize(d)? {
            Either::Text(what) => Gap::decision(what),
            Either::Full {
                level,
                what,
                owner,
                tried,
                next,
            } => Gap {
                level,
                what,
                owner,
                tried,
                next,
            },
        })
    }
}

/// 三句话答不清时，说明是哪一种答不清。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unanswered {
    None,
    MissingMaterial,
    AngleNotFormed,
    LowValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonVerdict {
    SameFactNoGain,
    SameBrandWithGain,
    Unrelated,
    /// 查重未确认：命中的历史文章正文缺失，或只有生成稿可比。**不能给确定的无重复结论。**
    Unconfirmed,
}

/// 命中的历史材料的发布状态。生成稿**不是**「近期已发」的证据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HitState {
    /// 正式发布
    Published,
    /// 已推草稿箱
    Draft,
    /// 仅生成稿
    Generated,
    /// 03 决定
    Decision,
    #[default]
    Unknown,
}

/// 查重命中的一条历史材料。
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ComparisonHit {
    /// 对照材料编号，如 `M3`
    pub ref_no: String,
    pub title: String,
    pub url: String,
    pub state: HitState,
    pub published_at: String,
    pub body_available: bool,
    /// 具体重复了哪条事实；无重复写空
    pub dup_fact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Comparison {
    pub verdict: ComparisonVerdict,
    /// 兼容旧行：v1 只写一句「对照的是哪篇」
    #[serde(default)]
    pub against: String,
    pub note: String,
    #[serde(default)]
    pub hits: Vec<ComparisonHit>,
}

/// 一条候选的判断结论。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Judgement {
    pub candidate_key: String,
    pub tier: Tier,
    /// 六维，键齐全
    pub dims: Vec<(Dim, DimJudgement)>,
    /// 「具体对象｜一句推荐理由」，40 字以内。列表与 Van 模式的主信息。
    #[serde(default)]
    pub headline: String,
    pub three_sentences: ThreeSentences,
    #[serde(default)]
    pub novelty: Novelty,
    #[serde(default)]
    pub readiness: Readiness,
    pub unanswered: Unanswered,
    pub comparison: Comparison,
    /// 热度说明。是输入的呈现，不是维度。
    pub heat_note: String,
    /// 实图里看到的外观与设计要点（辅助核查，界面上折叠）
    pub look: String,
    /// **没真正读到实图就是 false，且 tier 必须是 PendingCheck。**
    pub image_seen: bool,
    pub gaps: Vec<Gap>,
    /// 命中的七类优先关注 / 七类降低优先级。是倾向，不是黑名单。
    pub priority_hits: Vec<String>,
    pub lower_hits: Vec<String>,
    /// 与 Jev 初评不一致时写明分歧，否则空
    pub jev_disagreement: String,
    /// 引用到的参考库条目与选题记忆
    pub kb_refs: Vec<String>,
    pub memory_refs: Vec<String>,
    /// 决定这条结论能不能复用的输入指纹：正文、描述版本、对照材料、准则版本、
    /// 作业标准、模型与提示词版本。预取轮与正式轮靠它对账。
    pub inputs_hash: String,
}

impl Judgement {
    /// 契约自检：口径里写死的几条硬规则，落库前必须过。
    pub fn violations(&self) -> Vec<String> {
        let mut v = Vec::new();
        for d in Dim::ALL {
            match self.dims.iter().find(|(k, _)| *k == d) {
                None => v.push(format!("缺维度 {d:?}")),
                Some((_, j)) if j.basis.trim().is_empty() => v.push(format!("维度 {d:?} 没给依据")),
                _ => {}
            }
        }
        if !self.image_seen && self.tier != Tier::PendingCheck {
            v.push("没读到实图却不是 pending_check".into());
        }
        if self.inputs_hash.trim().is_empty() {
            v.push("缺 inputs_hash".into());
        }
        if self.headline.trim().is_empty() {
            v.push("缺 headline".into());
        }
        if self.tier == Tier::PendingCheck && !self.has_decision_gap() {
            v.push("待核却没有「影响判断」级的缺口".into());
        }
        v
    }

    pub fn has_decision_gap(&self) -> bool {
        self.gaps.iter().any(|g| g.level == GapLevel::Decision)
    }

    pub fn dim(&self, d: Dim) -> Option<&DimJudgement> {
        self.dims.iter().find(|(k, _)| *k == d).map(|(_, j)| j)
    }

    /// 测试与示例用的完整结论：六维成立、有图、有指纹。
    pub fn fixture(key: &str, tier: Tier) -> Self {
        Judgement {
            candidate_key: key.into(),
            tier,
            dims: Dim::ALL
                .into_iter()
                .map(|d| {
                    (
                        d,
                        DimJudgement {
                            verdict: Verdict::Yes,
                            basis: "正文第一句".into(),
                        },
                    )
                })
                .collect(),
            headline: format!("{key}｜示例"),
            three_sentences: ThreeSentences {
                what: "甲".into(),
                why_worth: "乙".into(),
                grounds: "丙".into(),
            },
            novelty: Novelty::default(),
            readiness: Readiness::default(),
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
                hits: vec![],
            },
            heat_note: String::new(),
            look: String::new(),
            image_seen: true,
            gaps: if tier == Tier::PendingCheck {
                vec![Gap::decision("示例缺口")]
            } else {
                vec![]
            },
            priority_hits: vec![],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "h".into(),
        }
    }
}

/// Jev 初评。**只在内部决定「先判哪条」**，不进台账、不显示成分数。
///
/// 0.7 实测：六维方向全对，`explain` / `csw` / `use` 分得最开；
/// 但总判 `worth` 当淘汰闸不行——阈值 0.30 会丢掉 29% 本该写的。所以它只排序。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Triage {
    pub candidate_key: String,
    /// 每维「成立」的概率
    pub dims: Vec<(Dim, f64)>,
    pub worth: f64,
    /// 七类降低优先级，一类一个概率（合成一个问法实测不分）
    pub lower_hits: Vec<(String, f64)>,
    pub model: String,
}

impl Triage {
    /// 流式推进的排序键。v2 以核心维度 gain 为主，不看 `worth`。
    /// 这个值只排序用，任何界面与交付物里都不出现。
    pub fn priority(&self) -> f64 {
        let get = |d: Dim| {
            self.dims
                .iter()
                .find(|(k, _)| *k == d)
                .map(|(_, v)| *v)
                .unwrap_or(0.0)
        };
        // v2：「值得推荐的价值」是核心，权重加倍
        get(Dim::Gain) * 2.0 + get(Dim::Csw) + get(Dim::Use)
    }
}

// ─────────────────────────────── 事件与轮次产物 ─────────────────────

/// 选题：同产品、同事件的多条贴文合成的一个报道对象（09-22 反馈第五项）。
///
/// 逐帖台账原样保留；选题是在它上面加的一层，推荐位按选题算，不按贴文算。
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Topic {
    /// = 主帖的 candidate_key
    pub topic_key: String,
    pub primary_key: String,
    /// 含主帖，主帖在前
    pub members: Vec<String>,
    pub merge_note: String,
    /// 组内最高的档（按代码给的档，不含人工改档——人工改档在页面上另算）
    pub tier: Option<Tier>,
    pub headline: String,
    pub synthesis: TopicSynthesis,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct TopicSynthesis {
    /// 各帖共有的事实
    pub shared_facts: Vec<String>,
    /// 每帖相对其余几帖新增了什么；只是重复的标 `is_duplicate`
    pub per_member: Vec<MemberNote>,
    /// 新增信息在该帖材料里核不到的（Jev 支持度低于阈值），给人看
    pub unsupported: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct MemberNote {
    pub candidate_key: String,
    pub new_info: String,
    pub is_duplicate: bool,
}

/// 合并后的一件事。同一件事可能被多个账号发成多条候选。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventGroup {
    pub event_key: String,
    /// 代表条目：正文最全、图最多的那条
    pub primary: String,
    pub members: Vec<String>,
    /// 合并依据：向量相似度 + Jev 的同一事件概率（阈值 0.5）
    pub merge_note: String,
}

/// 写引擎的动作。先落 outbox 再发，崩溃后可续、可幂等重放。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboxKind {
    Ack,
    Items,
    Sweeps,
    Judgements,
    Deliverable,
    Supplement,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    Pending,
    Sending,
    Confirmed,
    /// 引擎返回 409 之类的冲突，需要人核实。**不要换幂等键重试。**
    Conflict,
    Dead,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim_ok() -> DimJudgement {
        DimJudgement {
            verdict: Verdict::Yes,
            basis: "正文「改了结构」".into(),
        }
    }

    fn judgement(tier: Tier, image_seen: bool) -> Judgement {
        let mut j = Judgement::fixture("k", tier);
        j.dims = Dim::ALL.map(|d| (d, dim_ok())).to_vec();
        j.image_seen = image_seen;
        j
    }

    #[test]
    fn 旧行的缺口字符串与三句话旧键都读得回() {
        let gaps: Vec<Gap> = serde_json::from_str(
            r#"["价格未写",
            {"level":"boundary","what":"性能是品牌自述","owner":"editor","tried":"","next":""}]"#,
        )
        .unwrap();
        assert_eq!(gaps[0], Gap::decision("价格未写"));
        assert_eq!(gaps[1].level, GapLevel::Boundary);
        assert_eq!(gaps[1].owner, GapOwner::Editor);
        let t: ThreeSentences = serde_json::from_str(
            r#"{"what_changed":"甲","why_it_matters":"乙","how_different":"丙"}"#,
        )
        .unwrap();
        assert_eq!(
            (t.what.as_str(), t.why_worth.as_str(), t.grounds.as_str()),
            ("甲", "乙", "丙")
        );
        // 新键写出去就是新键
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["why_worth"], "乙");
        let c: Comparison =
            serde_json::from_str(r#"{"verdict":"unrelated","against":"x","note":""}"#).unwrap();
        assert!(c.hits.is_empty());
    }

    #[test]
    fn 待核必须有影响判断的缺口() {
        let mut j = judgement(Tier::PendingCheck, true);
        assert!(j.violations().is_empty());
        j.gaps = vec![Gap {
            level: GapLevel::Production,
            ..Gap::decision("缺价格")
        }];
        assert!(j.violations().iter().any(|v| v.contains("影响判断")));
        j.headline.clear();
        assert!(j.violations().iter().any(|v| v.contains("headline")));
    }

    #[test]
    fn 六维齐全且有依据才算合格() {
        assert!(judgement(Tier::Recommend, true).violations().is_empty());
        let mut j = judgement(Tier::Recommend, true);
        j.dims.retain(|(d, _)| *d != Dim::Csw);
        assert_eq!(j.violations().len(), 1);
        let mut j = judgement(Tier::Recommend, true);
        j.dims[0].1.basis = "  ".into();
        assert_eq!(j.violations().len(), 1);
    }

    #[test]
    fn 没读到实图必须落待核() {
        assert!(judgement(Tier::PendingCheck, false).violations().is_empty());
        assert_eq!(judgement(Tier::NotRecommend, false).violations().len(), 1);
    }

    #[test]
    fn 只取图文的判据要排掉轮播里夹的视频() {
        let mk = |ct: &str, kinds: &[MediaKind]| Candidate {
            candidate_key: "k".into(),
            platform: Platform::Instagram,
            source_id: "1".into(),
            collector: "csw_window".into(),
            account: "a".into(),
            url: String::new(),
            text: String::new(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: ct.into(),
            media: kinds
                .iter()
                .enumerate()
                .map(|(i, k)| MediaRef {
                    source_hash: format!("h{i}"),
                    kind: *k,
                    url: String::new(),
                    blake3: None,
                    ordinal: i as u16,
                })
                .collect(),
            tags: vec![],
            hashtags: vec![],
        };
        use MediaKind::{Photo, Video};
        assert!(mk("Image", &[Photo]).is_image_only());
        assert!(mk("Carousel", &[Photo, Photo]).is_image_only());
        assert!(!mk("Carousel", &[Photo, Video]).is_image_only());
        assert!(!mk("Reel", &[Photo]).is_image_only());
        assert!(!mk("Carousel", &[]).is_image_only());
    }

    #[test]
    fn 排序键只看分得最开的三维() {
        let t = |explain: f64, csw: f64, worth: f64| Triage {
            candidate_key: "k".into(),
            dims: vec![(Dim::Explain, explain), (Dim::Csw, csw), (Dim::Use, 0.0)],
            worth,
            lower_hits: vec![],
            model: "jev".into(),
        };
        // worth 高但两个分得开的维度低 → 排在后面
        assert!(t(0.9, 0.9, 0.1).priority() > t(0.1, 0.1, 0.9).priority());
    }

    #[test]
    fn 步骤序号从一起() {
        assert_eq!(StepCode::Intake.ordinal(), 1);
        assert_eq!(StepCode::Submit.ordinal(), 10);
    }
}
