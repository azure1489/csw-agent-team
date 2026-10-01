//! 口径的代码兜底（09-22 判断台账反馈）。**口径不能靠模型自觉。**
//!
//! 提示词里写了「没有旧款证据不许写从……变成……」「没读到不等于没价值」等等，
//! 但写了不等于做到——第 3 轮台账里这几条全都犯过。所以每一条都配一道纯函数检查，
//! 违例时三种处理：
//!
//! - **改**：代码直接改到合规（如推荐却 gain 不成立 → 备选），并留痕；
//! - **退回重判一次**：模型要重写的（如编造的前后对比），附违例说明再问一次；
//! - **标红**：重判后仍违例的，就地删掉违例的句子并留痕给人看。
//!
//! 留痕一律以 [`NOTE`] 开头，进 `check_flags_json`，台账上与 Jev 核对的标记并排显示。
//! 代码改档直接写进结论的 `tier`，**不走人工改档表**——原档与原因都在留痕里。

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use csw_collector_core::types::{
    ComparisonHit, ComparisonVerdict, Dim, Gap, GapLevel, GapOwner, HitState, Judgement, Material,
    MaterialKind, NoveltyKind, Tier, Unanswered, Verdict,
};
use csw_collector_kb::brands::brand_key;

use crate::materials;

/// 留痕前缀。
pub const NOTE: &str = "【口径】";
/// headline 的字数上限。
pub const HEADLINE_MAX_CHARS: usize = 40;
/// 删掉违例句子后留下的占位。
pub const STRIPPED: &str = "（无旧款依据的前后对比，已删）";

/// 前后对比的句式。**只抓「从 A 变成 B」这类明确的对比结构**，
/// 单独的「新」「升级」不抓——那是正常的产品描述。
static CONTRAST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"从[^，。；！？\n]{1,30}?(?:变成|变为|改为|改成|升级为|升级成|转为|转成|进化为|演变为)|告别[^，。；！？\n]{1,20}|不再(?:只)?是",
    )
    .expect("正则")
});

/// 「没看全图」一类的缺口。**只在全部图片都已识别时**才拿来删——那时它说的不是事实。
static IMAGE_UNREAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"未(?:能)?(?:真正)?(?:读到|看到|查看|读取|实读)[^；。]{0,12}实图|(?:只|仅)(?:实际|实)?(?:查看|读取|读到|读|看|见)了?(?:前\s*(?:3|三)\s*张|第\s*1\s*(?:[—\-–~至到、]\s*)?3\s*张)|逐图核验|全图(?:核验|审核|判断)|(?:文字|画面)(?:描述|说明|摘要)(?:替代|代替)",
    )
    .expect("正则")
});

/// 写成缺口的「没有缺口」：模型偶尔把「没有影响结论的缺失材料」也填进 gaps。
static NOT_A_GAP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:没有|无|暂无|不存在)(?:任何)?(?:影响[^；。，]{0,10}的)?(?:缺口|缺失材料|缺失|缺少的?材料)")
        .expect("正则")
});

/// 句子切分：连同句末标点一起切。
/// 「五类材料齐备 / 均已提供」：查过五类不等于五类都拿到了正文（10-01 r59 主编退回）
static ALL_PROVIDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"五类(?:材料|对照)(?:均|都)?(?:已)?(?:提供|齐备|齐全|齐)(?:记录或查询结果)?")
        .expect("正则")
});

static SENTENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^。；！？\n]+[。；！？\n]?").expect("正则"));

pub struct Ctx<'a> {
    /// 送进模型的对照材料（含选题记忆），编号用 [`materials::numbered`]
    pub materials: &'a [Material],
    /// 这条候选命中的品牌键
    pub brand_keys: &'a [String],
    /// 还能不能退回重判。重判过一次的传 false，违例就地修正。
    pub allow_rejudge: bool,
    /// 这条的全部图片都已由识图步骤看过（代码算的，见 `pipeline::Item::image_seen`）
    pub images_all_read: bool,
    /// 深核过（带着深核条目卡判的）。深核过的不许再落待核
    pub deepchecked: bool,
    /// 这条候选的原文（正文 + 译文 + 补读正文）。**原文里本来就有的「从…改为…」不算编造**——
    /// 依据要引原话，品牌文案自己这么写，引用它不是模型虚构的对比。
    pub source_text: &'a str,
}

#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    /// 留痕，每条以 [`NOTE`] 开头
    pub notes: Vec<String>,
    /// 需要退回重判时的违例说明（给模型看的）。只在 `allow_rejudge` 时出现。
    pub rejudge: Option<String>,
}

/// 按 R1–R7 检查并修正一条结论。
pub fn apply(j: &mut Judgement, ctx: &Ctx<'_>) -> Applied {
    let mut out = Applied::default();
    let mut retry: Vec<String> = Vec::new();
    let numbered = materials::numbered(ctx.materials);
    let memory_nos: HashSet<String> = numbered
        .iter()
        .filter(|(_, m)| m.kind == MaterialKind::Memory)
        .map(|(n, _)| n.clone())
        .collect();

    // R8 图都看过了还说没看图。判断只随附前 3 张缩略、其余给看图结果，模型会把它当成
    // 「没读到第 4 张以后的实图」落待核——09-24 r53 的 61 条待核里 52 条是这个
    // 反过来：没读全图的条目，模型的输出转换时 image_seen 被写死为 true（verdict.rs），这里改回事实。
    // 平时没读到实图的不送模型，但深核带卡重判会送（10-01 r60 andwander：0 张图却 image_seen=true）
    if !ctx.images_all_read && j.image_seen {
        j.image_seen = false;
        if j.tier != Tier::PendingCheck {
            out.notes.push(format!(
                "{NOTE}这条没读全实图，档位由 {:?} 落待核（没读到实图只能待核）",
                j.tier
            ));
            j.tier = Tier::PendingCheck;
        }
        out.notes
            .push(format!("{NOTE}这条没读全实图，读到实图改为否"));
    }
    if ctx.images_all_read {
        j.image_seen = true;
        let before = j.gaps.len();
        j.gaps.retain(|g| !IMAGE_UNREAD.is_match(&g.what));
        let dropped = before - j.gaps.len();
        if dropped > 0 {
            out.notes.push(format!(
                "{NOTE}全部图片已由识图步骤看过，去掉 {dropped} 条「没看全图」的缺口"
            ));
            if j.tier == Tier::PendingCheck && !j.has_decision_gap() && ctx.allow_rejudge {
                retry.push(
                    "你因为「没看全图」判了待核。这条的全部图片都已由识图步骤逐张看过原图，\
                     「第 N 张」后面就是看图结果，只随附部分缩略不是没读到实图。\
                     请按已有的正文与看图结果重新定档；真有影响选题判断的缺口才判待核。"
                        .into(),
                );
            }
        }
    }

    // 「没有缺口」不是缺口
    let before = j.gaps.len();
    j.gaps.retain(|g| !NOT_A_GAP.is_match(&g.what));
    if j.gaps.len() < before {
        out.notes.push(format!(
            "{NOTE}去掉 {} 条写成缺口的「没有缺口」",
            before - j.gaps.len()
        ));
    }

    // R0 契约：没读到实图必须待核、每维要有依据。模型偶尔违反，不修的话这条落不了库，
    // 台账上就平白少一行（它也不在「未判」里）
    if !j.image_seen && j.tier != Tier::PendingCheck {
        j.tier = Tier::PendingCheck;
        if !j.has_decision_gap() {
            j.gaps.push(Gap {
                next: "下一轮重新下载、识别后重判".into(),
                ..Gap::decision("未读到实图")
            });
        }
        out.notes
            .push(format!("{NOTE}没读到实图却给了结论，改为待核"));
    }
    let mut empty_basis = 0;
    for (_, d) in &mut j.dims {
        if d.basis.trim().is_empty() {
            d.basis = "模型没给依据，按不明处理".into();
            d.verdict = Verdict::Unclear;
            empty_basis += 1;
        }
    }
    if empty_basis > 0 {
        out.notes
            .push(format!("{NOTE}{empty_basis} 个维度没给依据，按不明处理"));
    }

    // 查重命中的链接、状态、正文可得与否**按材料编号从我们自己的材料里抄**，不信模型写的：
    // 链接会被渲染成可点的地址（模型被正文诱导写一个 `javascript:` 就是一次存储型 XSS），
    // 状态决定 R5 认不认「只凭生成稿」（09-24 安全审查）。对不上编号的清空链接，留给 R5 处理。
    for h in &mut j.comparison.hits {
        match numbered.iter().find(|(n, _)| n == h.ref_no.trim()) {
            Some((_, m)) => {
                h.url = if m.source.starts_with("https://") || m.source.starts_with("http://") {
                    m.source.clone()
                } else {
                    String::new()
                };
                h.state = materials::hit_state(&m.publish_state);
                h.body_available = m.body_available;
            }
            None => h.url.clear(),
        }
    }

    // R2 编造的前后对比
    let evidenced = j.novelty.kind == NoveltyKind::EvidencedChange
        && !j.novelty.prior_evidence.trim().is_empty();
    if !evidenced {
        let found = contrast_phrases(j, ctx.source_text);
        if !found.is_empty() {
            if ctx.allow_rejudge {
                retry.push(format!(
                    "你写了「{}」这类前后对比，但 novelty 不是 evidenced_change 或 prior_evidence 为空。\
                     没有旧款或前代的证据，就写产品现在是什么样，不要写「从……变成……」；\
                     有证据就把 novelty.kind 填 evidenced_change，并在 prior_evidence 写明出处。",
                    found.join("」「")
                ));
            } else {
                strip_contrast(j, &found);
                out.notes.push(format!(
                    "{NOTE}无旧款依据却写了前后对比「{}」，重判后仍在，已删去这些句子",
                    found.join("」「")
                ));
            }
        }
    }

    // R5「同一事实无增量」要有站得住的命中：命中的材料得在我们给的材料里、不是生成稿、
    // **不是上一轮台账**、正文取得到。否则只能是查重未确认——生成稿不是近期已发的证据，
    // 上一轮台账是工作台自己之前的判断（待核也在里面），既不是发布也不是否决
    //（09-28 r56 v8 退回：SATISFY、asimocrafts 拿历史待核台账当「已被重复报道」淘汰）
    if j.comparison.verdict == ComparisonVerdict::SameFactNoGain {
        let solid = j.comparison.hits.iter().any(|h| {
            numbered.iter().any(|(n, m)| {
                n == h.ref_no.trim()
                    && !matches!(
                        m.kind,
                        MaterialKind::GeneratedPost | MaterialKind::PriorLedger
                    )
                    && m.body_available
            })
        });
        if !solid {
            j.comparison.verdict = ComparisonVerdict::Unconfirmed;
            out.notes.push(format!(
                "{NOTE}判「同一事实无增量」，但命中的只有生成稿、上一轮台账、正文不可得的材料或不存在的编号，改为查重未确认"
            ));
            if j.tier == Tier::NotRecommend {
                if ctx.allow_rejudge {
                    retry.push(
                        "你以「同一事实无增量」判了不推荐，但依据只有生成稿、上一轮台账、正文不可得的材料或不存在的编号。\
                         生成稿不是近期已发的证据，上一轮台账是工作台自己之前的判断、不是发布也不是否决；\
                         请只凭正文取得到的正式发布 / 已推草稿箱 / 03 决定查重，\
                         核不了就填 unconfirmed，并按内容本身的价值重新定档。"
                            .into(),
                    );
                } else {
                    // 不能再重判：不推荐的理由已经站不住，交给人核
                    j.tier = Tier::PendingCheck;
                    j.gaps.push(Gap {
                        level: GapLevel::Decision,
                        what: "查重未确认：不推荐的理由是「同一事实已发」，但没有站得住的已发证据"
                            .into(),
                        owner: GapOwner::Editor,
                        tried: "对照材料里只有生成稿或只有标题".into(),
                        next: "核对是否真的发过，再定档".into(),
                    });
                    out.notes.push(format!(
                        "{NOTE}不推荐只凭查重、而查重站不住，改为待核（原判不推荐）"
                    ));
                }
            }
        }
    }

    // R4 历史正文不可得却判「无关」
    if j.comparison.verdict == ComparisonVerdict::Unrelated {
        let missing: Vec<&Material> = ctx
            .materials
            .iter()
            .filter(|m| {
                matches!(m.kind, MaterialKind::Published | MaterialKind::Example)
                    && !m.body_available
                    && !m.brand.trim().is_empty()
                    && ctx.brand_keys.contains(&brand_key(&m.brand))
            })
            .collect();
        let model_admits = j.comparison.hits.iter().any(|h| !h.body_available);
        // 只对要进选题的（推荐 / 备选）要求人去核：不推荐的查重核不核不影响取舍
        let matters = matches!(j.tier, Tier::Recommend | Tier::Alternate);
        if (!missing.is_empty() || model_admits) && matters {
            j.comparison.verdict = ComparisonVerdict::Unconfirmed;
            let titles: Vec<&str> = missing.iter().map(|m| m.title.as_str()).collect();
            let what = if titles.is_empty() {
                "命中的历史文章正文缺失，无法确认有无重复".to_string()
            } else {
                format!(
                    "同品牌历史文章《{}》正文缺失，无法确认有无重复",
                    titles.join("》《")
                )
            };
            j.gaps.push(Gap {
                level: GapLevel::Decision,
                owner: GapOwner::Editor,
                tried: "对照材料里只有标题".into(),
                next: if titles.is_empty() {
                    "打开命中的历史文章核对正文".into()
                } else {
                    format!("核对《{}》正文", titles.join("》《"))
                },
                what,
            });
            out.notes.push(format!(
                "{NOTE}历史文章正文缺失却判「无关」，改为查重未确认"
            ));
        }
    }

    // R1 推荐必须核心维度成立
    if j.tier == Tier::Recommend && j.dim(Dim::Gain).map(|d| d.verdict) != Some(Verdict::Yes) {
        j.tier = Tier::Alternate;
        out.notes.push(format!(
            "{NOTE}核心维度「值得推荐的价值」未成立，推荐改备选（原判推荐）"
        ));
    }

    // R1b 推荐还要 CSW 适配度成立：推荐位是给 Van 选题的，适配说不清的最多备选
    // （09-25 M3：Van 否决的 8 条里 5 条被判推荐 / 备选，其中 3 条 csw 是「不明」）
    if j.tier == Tier::Recommend && j.dim(Dim::Csw).map(|d| d.verdict) != Some(Verdict::Yes) {
        j.tier = Tier::Alternate;
        out.notes.push(format!(
            "{NOTE}「CSW 适配度」未成立，推荐改备选（原判推荐）"
        ));
    }

    // R3 没读到不等于没价值。**只管「贴文指向别处的内容没取到」**：贴文本身就是全部、
    // 只是说得少，模型判了价值不足（low_value）的，不翻成待核（09-25：预取轮 17 条待核里
    // 约 8 条是这种）
    let points_elsewhere = j
        .gaps
        .iter()
        .any(|g| g.level == GapLevel::Decision && elsewhere(g));
    if j.tier == Tier::NotRecommend
        && (j.unanswered == Unanswered::MissingMaterial
            || (j.unanswered != Unanswered::LowValue && points_elsewhere))
    {
        j.tier = Tier::PendingCheck;
        out.notes.push(format!(
            "{NOTE}关键资料在别处尚未取得却判不推荐，改为待核（原判不推荐）"
        ));
    }

    // R9 深核过、仍缺核心证据的：**待核·已深核**，缺口交主编补证。
    // 原先定为「备选·待补证」，09-28 主编验收要求核心证据不足的一律落 pending_check 并同步统计
    //（r56 #614/#615 退回），改回待核；与没核过的待核靠「已深核」区分，补证由主编接手
    if ctx.deepchecked && j.tier == Tier::PendingCheck {
        if !j.has_decision_gap() {
            j.gaps.push(Gap::decision("深核后仍缺决定选题的关键资料"));
        }
        for g in j.gaps.iter_mut().filter(|g| g.level == GapLevel::Decision) {
            g.owner = GapOwner::Editor;
            if g.next.trim().is_empty() {
                g.next = "人工补证后再定是否推荐".into();
            }
        }
        out.notes
            .push(format!("{NOTE}已深核，仍缺核心证据：待核，缺口交主编补证"));
    }

    // R6 缺口
    if j.tier == Tier::PendingCheck && !j.has_decision_gap() {
        j.gaps.push(Gap {
            next: "补读或人工核对后重判".into(),
            ..Gap::decision(match j.unanswered {
                Unanswered::MissingMaterial => "关键资料尚未取得",
                Unanswered::AngleNotFormed => "报道角度尚未成立",
                Unanswered::LowValue => "价值是否成立尚待确认",
                Unanswered::None => "判断所需的关键信息尚待确认",
            })
        });
        out.notes.push(format!(
            "{NOTE}待核却没写影响判断的缺口，已按「答不清」的原因补一条"
        ));
    }
    let mut unplanned = 0;
    for g in &mut j.gaps {
        if g.owner == GapOwner::Collector
            && g.level != GapLevel::Boundary
            && g.next.trim().is_empty()
        {
            g.next = "补读或人工核对后重判".into();
            unplanned += 1;
        }
    }
    if unplanned > 0 {
        out.notes.push(format!(
            "{NOTE}{unplanned} 条由收集员处理的缺口没写下一步，已补默认做法，请确认"
        ));
    }

    // 选题记忆的引用只能是给出的编号
    let before = j.memory_refs.len();
    j.memory_refs.retain(|r| memory_nos.contains(r.trim()));
    if j.memory_refs.len() < before {
        out.notes.push(format!(
            "{NOTE}引用了 {} 个不是选题记忆的编号，已去掉",
            before - j.memory_refs.len()
        ));
    }

    // R7 headline
    if j.headline.trim().is_empty() {
        let what: String = j
            .three_sentences
            .what
            .trim()
            .chars()
            .take(HEADLINE_MAX_CHARS)
            .collect();
        j.headline = if what.is_empty() {
            j.candidate_key.clone()
        } else {
            what
        };
        out.notes
            .push(format!("{NOTE}缺标题，已用「是什么」那句顶上"));
    } else if j.headline.chars().count() > HEADLINE_MAX_CHARS {
        out.notes.push(format!(
            "{NOTE}标题超过 {HEADLINE_MAX_CHARS} 字，应写成「具体对象｜一句推荐理由」"
        ));
    }

    // R9 「正文可得」以代码为准：给模型看过正文的只有已发与范例、且确实带了正文摘要的那几条。
    // 生成稿、03 决定、上一轮台账只给了标题或决定摘要，模型照抄 true 会让导出自相矛盾
    let by_no: HashMap<&str, &Material> = numbered.iter().map(|(n, m)| (n.as_str(), *m)).collect();
    out.notes.extend(normalize_bodies(j, |h| {
        Some(by_no.get(h.ref_no.as_str()).is_some_and(|m| {
            matches!(m.kind, MaterialKind::Published | MaterialKind::Example)
                && m.body_available
                && !m.body_excerpt.trim().is_empty()
        }))
    }));

    if !retry.is_empty() {
        out.rejudge = Some(retry.join("\n"));
    }
    out
}

/// 把查重命中的「正文可得」改成事实，并把「五类材料齐备」改成「都查过」。返回留痕。
///
/// `given` 回答「这条命中给模型看过正文没有」；答不上（`None`，如返工时手上没有当时的材料）
/// 就按类型定：只有正式发布 / 已推草稿箱可能带正文，保留原值；生成稿、03 决定、来源不明的一律 false——
/// 判断时这几类从来只给标题或决定摘要（见 `materials::as_prompt_block`）。
pub fn normalize_bodies(
    j: &mut Judgement,
    given: impl Fn(&ComparisonHit) -> Option<bool>,
) -> Vec<String> {
    let mut notes = Vec::new();
    let mut fixed = Vec::new();
    for h in &mut j.comparison.hits {
        let by_state = matches!(h.state, HitState::Published | HitState::Draft) && h.body_available;
        let truth = given(h).unwrap_or(by_state);
        if h.body_available != truth {
            h.body_available = truth;
            fixed.push(if h.ref_no.is_empty() {
                h.title.clone()
            } else {
                h.ref_no.clone()
            });
        }
    }
    if !fixed.is_empty() {
        notes.push(format!(
            "{NOTE}查重命中 {} 判断时没有给出正文，「正文可得」改为否",
            fixed.join("、")
        ));
    }
    let missing: Vec<String> = j
        .comparison
        .hits
        .iter()
        .filter(|h| !h.body_available)
        .map(|h| {
            if h.ref_no.is_empty() {
                format!("《{}》", h.title)
            } else {
                h.ref_no.clone()
            }
        })
        .collect();
    if !missing.is_empty() && ALL_PROVIDED.is_match(&j.comparison.note) {
        j.comparison.note = ALL_PROVIDED
            .replace_all(&j.comparison.note, "五类材料都查过（查过不等于正文已得）")
            .into_owned();
        if !j.comparison.note.contains("正文未得：") {
            let sep = if j.comparison.note.ends_with(['。', '；', '.']) {
                ""
            } else {
                "；"
            };
            j.comparison.note.push_str(&format!(
                "{sep}正文未得：{}（只有标题或决定摘要），这几条的事实级查重只能是未确认。",
                missing.join("、")
            ));
        }
        notes.push(format!(
            "{NOTE}查重说明里的「五类齐备」改为「都查过」，并注明正文未得的编号"
        ));
    }
    notes
}

/// 结论里所有写了前后对比的片段。**原文里本来就有的不算**（见 [`Ctx::source_text`]）。
pub fn contrast_phrases(j: &Judgement, source: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for text in contrast_fields(j) {
        for m in CONTRAST.find_iter(text) {
            let s = m.as_str().to_string();
            if !source.contains(&s) && !found.contains(&s) {
                found.push(s);
            }
        }
    }
    found
}

fn contrast_fields(j: &Judgement) -> Vec<&str> {
    let mut v = vec![
        j.headline.as_str(),
        j.three_sentences.what.as_str(),
        j.three_sentences.why_worth.as_str(),
        j.three_sentences.grounds.as_str(),
        j.novelty.basis.as_str(),
    ];
    for (d, dj) in &j.dims {
        if matches!(d, Dim::Change | Dim::Explain | Dim::Gain) {
            v.push(dj.basis.as_str());
        }
    }
    v
}

/// 把含（编造的）前后对比的句子换成占位。每个字段至多留一个占位。
///
/// 标题特殊：它没有句号，整条会被当成一句删光、连产品名一起没了。所以标题只保留「｜」前的
/// 具体对象；对象本身也含对比就清空，交给 R7 用「是什么」那句重建。
fn strip_contrast(j: &mut Judgement, found: &[String]) {
    let hit = |x: &str| found.iter().any(|f| x.contains(f.as_str()));
    let fix = |s: &mut String| {
        if !hit(s) {
            return;
        }
        let mut out = String::new();
        let mut placed = false;
        for m in SENTENCE.find_iter(s) {
            if hit(m.as_str()) {
                if !placed {
                    out.push_str(STRIPPED);
                    placed = true;
                }
            } else {
                out.push_str(m.as_str());
            }
        }
        *s = out;
    };
    if hit(&j.headline) {
        let object = j
            .headline
            .split('｜')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        j.headline = if hit(&object) { String::new() } else { object };
    }
    fix(&mut j.three_sentences.what);
    fix(&mut j.three_sentences.why_worth);
    fix(&mut j.three_sentences.grounds);
    fix(&mut j.novelty.basis);
    for (d, dj) in &mut j.dims {
        if matches!(d, Dim::Change | Dim::Explain | Dim::Gain) {
            fix(&mut dj.basis);
        }
    }
}

/// 这条该不该去补读外链：关键内容没取到、且缺口里指向了外部内容。
pub fn wants_refetch(j: &Judgement) -> bool {
    j.tier == Tier::PendingCheck
        && j.image_seen
        && j.gaps
            .iter()
            .any(|g| g.level == GapLevel::Decision && elsewhere(g))
}

/// 这条缺口说的是「内容在别处」：外链、官网、主页、全文、快拍精选。
fn elsewhere(g: &Gap) -> bool {
    [
        "外链",
        "链接",
        "主页",
        "全文",
        "官网",
        "完整内容",
        "link",
        "详见",
        "快拍",
        "精选",
        "商品页",
        "产品页",
        "博客",
    ]
    .iter()
    .any(|k| format!("{}{}", g.what, g.next).contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{ComparisonHit, HitState, Novelty};

    fn ctx<'a>(ms: &'a [Material], brands: &'a [String], again: bool) -> Ctx<'a> {
        Ctx {
            materials: ms,
            brand_keys: brands,
            allow_rejudge: again,
            // 真实情况下送进模型的都是图全读到的；没读全的另有测试
            images_all_read: true,
            deepchecked: false,
            source_text: "",
        }
    }

    fn material(kind: MaterialKind, title: &str, brand: &str, body: bool) -> Material {
        Material {
            kind,
            ref_id: title.into(),
            title: title.into(),
            date: None,
            source: String::new(),
            quote: String::new(),
            publish_state: "published".into(),
            body_excerpt: if body { "正文".into() } else { String::new() },
            body_available: body,
            brand: brand.into(),
        }
    }

    #[test]
    fn 推荐却核心维度不成立改备选() {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        j.dims[2].1.verdict = Verdict::Unclear;
        let a = apply(&mut j, &ctx(&[], &[], true));
        assert_eq!(j.tier, Tier::Alternate);
        assert!(a.notes[0].contains("原判推荐"));
        // 反例：gain 成立的推荐不动
        let mut j = Judgement::fixture("k", Tier::Recommend);
        assert!(apply(&mut j, &ctx(&[], &[], true)).notes.is_empty());
        assert_eq!(j.tier, Tier::Recommend);
    }

    #[test]
    fn 没有旧款证据写前后对比先退回重判() {
        // 09-22 反馈原样：SML 磁吸包原文只说配了 Fidlock，摘要却写成「从普通开合变成磁吸」
        let mut j = Judgement::fixture("sml", Tier::Recommend);
        j.three_sentences.what = "SML 磁吸包从普通开合变成磁吸。配 Fidlock 部件。".into();
        let a = apply(&mut j, &ctx(&[], &[], true));
        let why = a.rejudge.expect("要退回重判");
        assert!(why.contains("从普通开合变成"), "{why}");
        // 退回时不就地删——等重判的结果
        assert!(j.three_sentences.what.contains("变成"));
    }

    #[test]
    fn 重判后仍编造对比就删掉那一句() {
        let mut j = Judgement::fixture("norbit", Tier::Recommend);
        j.three_sentences.what = "norbit 夹克从单一外观更新变成双用途。背面有收纳袋。".into();
        j.headline = "norbit 夹克｜告别单一用途".into();
        let a = apply(&mut j, &ctx(&[], &[], false));
        assert!(a.rejudge.is_none());
        assert_eq!(j.three_sentences.what, format!("{STRIPPED}背面有收纳袋。"));
        // 标题只删推荐理由里的对比，产品名留着
        assert_eq!(j.headline, "norbit 夹克");
        assert!(a.notes.iter().any(|n| n.contains("已删去")));
    }

    #[test]
    fn 有旧款证据的变化照写不动() {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        j.three_sentences.what = "背板从铝框改为碳纤维".into();
        j.novelty = Novelty {
            kind: NoveltyKind::EvidencedChange,
            basis: "正文第二句".into(),
            prior_evidence: "品牌 2024 年旧款页面写明铝框".into(),
        };
        let a = apply(&mut j, &ctx(&[], &[], true));
        assert!(a.rejudge.is_none() && a.notes.is_empty(), "{a:?}");
        // 「新」「升级」这类普通描述不抓
        let mut j = Judgement::fixture("k", Tier::Recommend);
        j.three_sentences.what = "全新升级的背包，新增侧袋".into();
        assert!(apply(&mut j, &ctx(&[], &[], true)).rejudge.is_none());
    }

    #[test]
    fn 没读到关键内容不许判不推荐() {
        // 09-22 反馈：Hyperlite 四款帐篷比较，完整内容在主页外链，没取到却判了不推荐
        let mut j = Judgement::fixture("hyperlite", Tier::NotRecommend);
        j.unanswered = Unanswered::MissingMaterial;
        let a = apply(&mut j, &ctx(&[], &[], true));
        assert_eq!(j.tier, Tier::PendingCheck);
        assert!(j.has_decision_gap(), "改成待核就要有影响判断的缺口");
        assert!(j.violations().is_empty(), "{:?}", j.violations());
        assert!(a.notes.iter().any(|n| n.contains("原判不推荐")));
        // 反例：读到了、确认价值不足的不推荐不动
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.unanswered = Unanswered::LowValue;
        apply(&mut j, &ctx(&[], &[], true));
        assert_eq!(j.tier, Tier::NotRecommend);
    }

    #[test]
    fn 历史正文缺失不许判无关() {
        // 09-22 反馈：Dapple Born 桌板同时写着「与已发的无关」和「历史文章正文缺失」
        let ms = [material(
            MaterialKind::Published,
            "Dapple Born 露营桌",
            "dappleborn",
            false,
        )];
        let brands = ["dappleborn".to_string()];
        let mut j = Judgement::fixture("dappleborn-b81b36", Tier::Recommend);
        apply(&mut j, &ctx(&ms, &brands, true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::Unconfirmed);
        let g = j.gaps.iter().find(|g| g.owner == GapOwner::Editor).unwrap();
        assert!(g.next.contains("Dapple Born 露营桌"));
        // 档位不因此动
        assert_eq!(j.tier, Tier::Recommend);
        // 不推荐的不因查重未确认而多一条缺口（否则 R3 会把它误改成待核）
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.unanswered = Unanswered::LowValue;
        apply(&mut j, &ctx(&ms, &brands, true));
        assert_eq!(j.tier, Tier::NotRecommend);
        // 反例：别的品牌的正文缺失不算
        let mut j = Judgement::fixture("k", Tier::Recommend);
        apply(&mut j, &ctx(&ms, &["norda".to_string()], true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::Unrelated);
    }

    #[test]
    fn 只凭生成稿不许判同一事实无增量() {
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M1".into(),
            state: HitState::Generated,
            ..Default::default()
        }];
        let ms = [material(MaterialKind::GeneratedPost, "生成稿", "", true)];
        let a = apply(&mut j, &ctx(&ms, &[], true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::Unconfirmed);
        assert!(a.rejudge.is_some(), "因生成稿判的不推荐要重判");
        // 反例：命中正式发布的照常
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M1".into(),
            state: HitState::Published,
            body_available: true,
            dup_fact: "同一款桌板的发售".into(),
            ..Default::default()
        }];
        let ms = [material(MaterialKind::Published, "已发", "", true)];
        let a = apply(&mut j, &ctx(&ms, &[], true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::SameFactNoGain);
        assert!(a.rejudge.is_none());
    }

    #[test]
    fn 引用不存在的编号改未确认() {
        let mut j = Judgement::fixture("k", Tier::Alternate);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M9".into(),
            state: HitState::Published,
            ..Default::default()
        }];
        j.memory_refs = vec!["M9".into()];
        let a = apply(&mut j, &ctx(&[], &[], true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::Unconfirmed);
        assert!(j.memory_refs.is_empty());
        assert!(a.notes.iter().any(|n| n.contains("不是选题记忆的编号")));
    }

    #[test]
    fn 待核补缺口与下一步() {
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.gaps = vec![Gap {
            level: GapLevel::Production,
            ..Gap::decision("缺价格")
        }];
        j.unanswered = Unanswered::AngleNotFormed;
        apply(&mut j, &ctx(&[], &[], true));
        assert!(j.has_decision_gap());
        assert!(j.gaps.iter().all(|g| !g.next.is_empty()));
        assert!(j.violations().is_empty(), "{:?}", j.violations());
    }

    #[test]
    fn 缺标题用是什么顶上() {
        let mut j = Judgement::fixture("k", Tier::Alternate);
        j.headline.clear();
        j.three_sentences.what = "山と道 Mini 2 背包".into();
        apply(&mut j, &ctx(&[], &[], true));
        assert_eq!(j.headline, "山と道 Mini 2 背包");
        let mut j = Judgement::fixture("k", Tier::Alternate);
        j.headline = "很".repeat(41);
        let a = apply(&mut j, &ctx(&[], &[], true));
        assert!(a.notes.iter().any(|n| n.contains("超过")));
    }

    #[test]
    fn 查重命中的链接与状态按材料抄不信模型() {
        let mut ms = vec![material(MaterialKind::GeneratedPost, "生成稿", "", true)];
        ms[0].source = "https://csw.example/p/1".into();
        ms[0].publish_state = "generated".into();
        let mut j = Judgement::fixture("k", Tier::Recommend);
        j.comparison.hits = vec![
            ComparisonHit {
                ref_no: "M1".into(),
                url: "javascript:alert(1)".into(),
                // 模型谎称这是正式发布
                state: HitState::Published,
                ..Default::default()
            },
            ComparisonHit {
                ref_no: "M9".into(),
                url: "javascript:alert(2)".into(),
                ..Default::default()
            },
        ];
        apply(&mut j, &ctx(&ms, &[], true));
        assert_eq!(j.comparison.hits[0].url, "https://csw.example/p/1");
        assert_eq!(j.comparison.hits[0].state, HitState::Generated);
        assert!(j.comparison.hits[1].url.is_empty());
    }

    #[test]
    fn 原文里本来就有的对比不算编造() {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        j.dims[0].1.basis = "正文「背板从铝框改为碳纤维」".into();
        let a = apply(
            &mut j,
            &Ctx {
                source_text: "新款背板从铝框改为碳纤维，更轻。",
                ..ctx(&[], &[], true)
            },
        );
        assert!(a.rejudge.is_none(), "{a:?}");
    }

    #[test]
    fn 上一轮台账不能当已被报道的证据() {
        // 09-28 r56 v8：SATISFY、asimocrafts 拿工作台自己之前的待核台账当「已被重复报道」淘汰
        let ms = [material(
            MaterialKind::PriorLedger,
            "SATISFY女装｜水瓶与杆具",
            "",
            true,
        )];
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M1".into(),
            ..Default::default()
        }];
        let a = apply(&mut j, &ctx(&ms, &[], true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::Unconfirmed);
        assert!(a.rejudge.is_some(), "能重判就退回重判：{a:?}");
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M1".into(),
            ..Default::default()
        }];
        apply(&mut j, &ctx(&ms, &[], false));
        assert_eq!(j.tier, Tier::PendingCheck, "不能重判就交人核，不留在不推荐");
    }

    #[test]
    fn 不能重判时只凭生成稿的不推荐改待核() {
        let ms = [material(MaterialKind::GeneratedPost, "生成稿", "", true)];
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M1".into(),
            ..Default::default()
        }];
        apply(&mut j, &ctx(&ms, &[], false));
        assert_eq!(j.tier, Tier::PendingCheck);
        assert!(j.has_decision_gap());
        assert!(j.violations().is_empty(), "{:?}", j.violations());
        // 只有标题（正文不可得）的已发也站不住
        let ms = [material(MaterialKind::Published, "已发", "", false)];
        let mut j = Judgement::fixture("k", Tier::Alternate);
        j.comparison.verdict = ComparisonVerdict::SameFactNoGain;
        j.comparison.hits = vec![ComparisonHit {
            ref_no: "M1".into(),
            ..Default::default()
        }];
        apply(&mut j, &ctx(&ms, &[], true));
        assert_eq!(j.comparison.verdict, ComparisonVerdict::Unconfirmed);
    }

    #[test]
    fn 契约违例就地修好不让这条落不了库() {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        j.image_seen = false;
        j.dims[3].1.basis = " ".into();
        let mut c = ctx(&[], &[], true);
        c.images_all_read = false;
        apply(&mut j, &c);
        assert_eq!(j.tier, Tier::PendingCheck);
        assert!(j.violations().is_empty(), "{:?}", j.violations());
    }

    #[test]
    fn 图都看过了就不许因为没看全图待核() {
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.image_seen = false;
        j.gaps = vec![
            Gap::decision("未读到实图：第4—8张；目前只读到第1—3张。"),
            Gap::decision(
                "未读到第4—6张实图；只实际查看了第1—3张，不能用画面文字说明替代逐图核验。",
            ),
            Gap {
                level: GapLevel::Production,
                ..Gap::decision("缺具体上市日期与价格")
            },
        ];
        let all = |again| Ctx {
            images_all_read: true,
            ..ctx(&[], &[], again)
        };
        let a = apply(&mut j, &all(true));
        assert!(j.image_seen);
        assert!(
            !j.gaps.iter().any(|g| g.what.contains("实图")),
            "{:?}",
            j.gaps
        );
        assert!(j.gaps.iter().any(|g| g.what.contains("上市日期")));
        assert!(a.rejudge.is_some(), "只因图待核的要退回重判");

        // 还有别的影响判断的缺口：不退回，只删图的那条
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.gaps = vec![
            Gap::decision("未读到第4—18张实图，仅实际查看第1—3张。"),
            Gap::decision("产品身份无法确认"),
        ];
        let a = apply(&mut j, &all(true));
        assert!(a.rejudge.is_none(), "{a:?}");
        assert_eq!(j.gaps.len(), 1);

        // 图没识别全的（代码没标全读）：缺口照留
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.image_seen = false;
        j.gaps = vec![Gap::decision("未读到实图")];
        let mut c = ctx(&[], &[], true);
        c.images_all_read = false;
        apply(&mut j, &c);
        assert_eq!(j.gaps.len(), 1);
        assert!(!j.image_seen);
    }

    /// 10-01 r60 andwander：0 张图，深核带卡重判后模型输出被写成 image_seen=true、不推荐
    #[test]
    fn 没读全图的模型输出改回没读到并落待核() {
        let mut j = Judgement::fixture("andwander-3bbddb", Tier::NotRecommend);
        assert!(j.image_seen, "模型输出转换时恒为 true");
        let mut c = ctx(&[], &[], false);
        c.images_all_read = false;
        let a = apply(&mut j, &c);
        assert!(!j.image_seen);
        assert_eq!(j.tier, Tier::PendingCheck);
        assert!(a.notes.iter().any(|n| n.contains("读到实图改为否")));
    }

    #[test]
    fn 推荐要csw也成立() {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        for (d, dj) in &mut j.dims {
            if *d == Dim::Csw {
                dj.verdict = Verdict::Unclear;
            }
        }
        apply(&mut j, &ctx(&[], &[], false));
        assert_eq!(j.tier, Tier::Alternate);
    }

    #[test]
    fn 价值不足的不推荐不翻成待核_指向外链的才翻() {
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.unanswered = Unanswered::LowValue;
        j.gaps = vec![Gap::decision("产品看点无法确认")];
        apply(&mut j, &ctx(&[], &[], false));
        assert_eq!(j.tier, Tier::NotRecommend);

        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.unanswered = Unanswered::None;
        j.gaps = vec![Gap::decision("完整内容在官网，尚未取得")];
        apply(&mut j, &ctx(&[], &[], false));
        assert_eq!(j.tier, Tier::PendingCheck);
    }

    #[test]
    fn 深核过仍缺核心证据的是待核且缺口交主编() {
        let deep = |again| Ctx {
            deepchecked: true,
            ..ctx(&[], &[], again)
        };
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.gaps = vec![Gap::decision("关键内容在官网")];
        let a = apply(&mut j, &deep(false));
        assert_eq!(j.tier, Tier::PendingCheck);
        assert!(j.gaps.iter().all(|g| g.owner == GapOwner::Editor));
        assert!(a.notes.iter().any(|n| n.contains("已深核")));
        assert!(j.violations().is_empty(), "{:?}", j.violations());
    }

    #[test]
    fn 写成缺口的没有缺口要去掉() {
        let mut j = Judgement::fixture("k", Tier::NotRecommend);
        j.gaps = vec![
            Gap::decision("没有影响结论的缺失材料；重复清仓事实已确认"),
            Gap::decision("无缺口"),
        ];
        apply(&mut j, &ctx(&[], &[], false));
        assert!(j.gaps.is_empty(), "{:?}", j.gaps);
        assert_eq!(j.tier, Tier::NotRecommend, "不该被 R3 误改成待核");
        // 正常的缺口不动
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.gaps = vec![Gap::decision("无法确认产品身份")];
        apply(&mut j, &ctx(&[], &[], false));
        assert_eq!(j.gaps.len(), 1);
    }

    #[test]
    fn 合规的结论一条留痕都没有() {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        assert_eq!(apply(&mut j, &ctx(&[], &[], true)), Applied::default());
    }

    #[test]
    fn 补读只针对指向外部内容的待核() {
        let mut j = Judgement::fixture("k", Tier::PendingCheck);
        j.gaps = vec![Gap::decision("完整内容在主页外链，未取得")];
        assert!(wants_refetch(&j));
        j.gaps = vec![Gap::decision("产品身份无法确认")];
        assert!(!wants_refetch(&j));
    }

    /// 10-01 r59 C65 e1cd9d：M3–M5 仅生成稿标题、M6 只有决定摘要，模型却写正文可得
    #[test]
    fn 正文可得以代码为准() {
        use csw_collector_core::types::{Comparison, ComparisonHit, HitState};
        let mut j = Judgement::fixture("cmf-e1cd9d", Tier::PendingCheck);
        let hit = |no: &str, state: HitState, body: bool| ComparisonHit {
            ref_no: no.into(),
            title: "New Release Info".into(),
            url: String::new(),
            state,
            published_at: String::new(),
            body_available: body,
            dup_fact: String::new(),
        };
        j.comparison = Comparison {
            verdict: ComparisonVerdict::Unconfirmed,
            against: String::new(),
            note: "五类对照均已提供记录或查询结果。M3—M5 有链接但无正文".into(),
            hits: vec![
                hit("M1", HitState::Published, true),
                hit("M3", HitState::Generated, true),
                hit("M6", HitState::Decision, true),
            ],
        };
        // 返工时没有材料：按类型定
        let notes = normalize_bodies(&mut j, |_| None);
        let b: Vec<bool> = j.comparison.hits.iter().map(|h| h.body_available).collect();
        assert_eq!(b, [true, false, false]);
        assert!(
            j.comparison
                .note
                .contains("五类材料都查过（查过不等于正文已得）")
        );
        assert!(j.comparison.note.contains("但无正文；正文未得：M3、M6"));
        for v in [
            "五类对照已提供",
            "五类材料已提供",
            "五类材料齐备",
            "五类对照均已提供记录或查询结果",
        ] {
            assert!(ALL_PROVIDED.is_match(v), "{v}");
        }
        assert!(!ALL_PROVIDED.is_match("五类材料都查过（查过不等于正文已得）"));
        assert_eq!(notes.len(), 2);
        // 幂等：再跑一次不再改
        assert!(normalize_bodies(&mut j, |_| None).is_empty());
        // 有材料时以材料为准：正式发布但没给正文摘要的也是否
        let notes = normalize_bodies(&mut j, |h| Some(h.ref_no != "M1"));
        assert!(!j.comparison.hits[0].body_available);
        assert!(notes[0].contains("M1"));
    }
}
