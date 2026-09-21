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

/// Van 的六个维度。键固定，不增不减——改维度要她本人点头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    /// 变化必须具体
    Change,
    /// 与真实使用有关
    Use,
    /// 有 CSW 读者能理解的价值
    Gain,
    /// 最好有比较参照
    Compare,
    /// 能产生有事实支持的编辑判断
    Explain,
    /// CSW 视角（调性适配）
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ThreeSentences {
    pub what_changed: String,
    pub why_it_matters: String,
    pub how_different: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Comparison {
    pub verdict: ComparisonVerdict,
    pub against: String,
    pub note: String,
}

/// 一条候选的判断结论。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Judgement {
    pub candidate_key: String,
    pub tier: Tier,
    /// 六维，键齐全
    pub dims: Vec<(Dim, DimJudgement)>,
    pub three_sentences: ThreeSentences,
    pub unanswered: Unanswered,
    pub comparison: Comparison,
    /// 热度说明。是输入的呈现，不是维度。
    pub heat_note: String,
    /// 实图里看到的外观与设计要点
    pub look: String,
    /// **没真正读到实图就是 false，且 tier 必须是 PendingCheck。**
    pub image_seen: bool,
    pub gaps: Vec<String>,
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
        v
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
    /// 流式推进的排序键。用分得最开的三维，不看 `worth`。
    /// 这个值只排序用，任何界面与交付物里都不出现。
    pub fn priority(&self) -> f64 {
        let get = |d: Dim| {
            self.dims
                .iter()
                .find(|(k, _)| *k == d)
                .map(|(_, v)| *v)
                .unwrap_or(0.0)
        };
        get(Dim::Explain) + get(Dim::Csw) + get(Dim::Use)
    }
}

// ─────────────────────────────── 事件与轮次产物 ─────────────────────

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
        Judgement {
            candidate_key: "k".into(),
            tier,
            dims: Dim::ALL.map(|d| (d, dim_ok())).to_vec(),
            three_sentences: ThreeSentences {
                what_changed: "a".into(),
                why_it_matters: "b".into(),
                how_different: "c".into(),
            },
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
            },
            heat_note: String::new(),
            look: String::new(),
            image_seen,
            gaps: vec![],
            priority_hits: vec![],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "h".into(),
        }
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
