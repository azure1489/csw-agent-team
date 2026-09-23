//! 把三小步串成一轮：**初评 → 合并 → 逐条判断 → 核对**。
//!
//! # 「每条都判」按字面执行
//!
//! 合并只决定台账怎么分组、怎么排序，**不决定要不要判**。同一事件里的非代表条目
//! 照样各判各的：合并省下的模型调用最多一成，而「判过」这件事一旦打了折扣，
//! 「审阅覆盖 100%」这个指标就不再是真的了。
//!
//! # 流式推进
//!
//! 按 Jev 初评的排序键从高到低排，一批批判。每批判完就回调一次
//! （[`Deps::on_batch`]），好让调用方凑够首批就先去深核、先交货——
//! 整轮要四十多分钟，首批 ≤20 分钟的目标只能这么达成。
//!
//! # 失败的边界
//!
//! - **初评失败**：这条没有排序键，排到最后，照常判。
//! - **一批判断失败**：只有那一批没有结论，标进 [`Outcome::unjudged`]，
//!   **不算判过也不算淘汰**，其余批不受影响。
//! - **核对失败**：没有标记而已，判断结论照用。

use std::collections::HashMap;

use csw_collector_core::jev::JevClient;
use csw_collector_core::model::ModelClient;
use csw_collector_core::types::{
    Candidate, EventGroup, Judgement, Material, MediaDescription, Triage,
};

use crate::check::{self, Flag};
use crate::merge::{self, MergeInput};
use crate::rubric::RUBRIC_VERSION;
use crate::triage;
use crate::verdict::{self, JudgeInput, PROMPT_VERSION};

/// 一条候选进判断前备齐的东西。
pub struct Item<'a> {
    pub candidate: &'a Candidate,
    pub descriptions: &'a [MediaDescription],
    /// 已按 `verdict::IMAGES_PER_CANDIDATE` 选好的实图缩略
    pub images_b64: Vec<String>,
    pub materials: Vec<Material>,
    pub fused: Option<&'a [f32]>,
    /// 真读到实图了吗。false 的不进模型，直接由代码落待核。
    pub image_seen: bool,
    /// 没读到实图的原因，写进 gaps
    pub image_gap: String,
    pub heat_note: String,
}

/// 一批的下标与它的结果
type BatchResult = (Vec<usize>, anyhow::Result<Vec<Judgement>>);

/// 每判完一批就被叫一次的回调
pub type OnBatch<'a> = &'a (dyn Fn(&[Judgement]) + Sync);

pub struct Deps<'a> {
    pub model: &'a ModelClient,
    /// Jev 不可用时传 None：初评、合并、核对都跳过，判断照跑。
    pub jev: Option<&'a JevClient>,
    /// 任务下发的作业标准，**原样注入**
    pub work_standard: &'a str,
    /// 同时判几批。真正的闸在模型客户端的信号量里，这里只是别让它闲着。
    pub batch_concurrency: usize,
    /// 每判完一批调一次，给流式推进用。批内顺序与送进去的一致。
    pub on_batch: Option<OnBatch<'a>>,
    /// 指纹一致的旧结论从哪儿取。传 None 就是每条都重新问模型。
    pub cached: Option<&'a dyn Cached>,
}

/// 已经判过、且输入一点没变的结论从哪儿来。预取轮判过的，正式轮直接拿。
///
/// **判据全在指纹里**：正文、图片描述、对照材料、准则版本、提示词版本、
/// 作业标准，任何一样变了指纹就变，这里自然取不到。所以实现方**不要**
/// 自己再加宽松条件——那等于绕开唯一一道拦着「换了东西还在用旧结论」的闸。
pub trait Cached {
    fn get(&self, candidate_key: &str, inputs_hash: &str) -> Option<Judgement>;
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub judgements: Vec<Judgement>,
    /// 直接拿了旧结论、没问模型的那些条目键
    pub reused: Vec<String>,
    pub triages: Vec<Triage>,
    pub groups: Vec<EventGroup>,
    pub flags: Vec<Flag>,
    /// 判断失败、至今没有结论的候选。**不算判过、不算淘汰。**
    pub unjudged: Vec<String>,
}

pub async fn run(items: &[Item<'_>], deps: &Deps<'_>) -> Outcome {
    let mut out = Outcome::default();
    if items.is_empty() {
        return out;
    }

    // ── 一、初评 ──
    out.triages = match deps.jev {
        Some(jev) => triage_all(jev, items).await,
        None => Vec::new(),
    };
    let by_key: HashMap<&str, f64> = out
        .triages
        .iter()
        .map(|t| (t.candidate_key.as_str(), t.priority()))
        .collect();

    // ── 二、合并 ──
    if let Some(jev) = deps.jev {
        out.groups = merge_all(jev, items).await;
    } else {
        out.groups = merge::group(&merge_inputs(items), &[]);
    }

    // ── 三、逐条判断 ──
    // 排序键高的先判：整轮四十多分钟，首批要能早点交出去。
    // 没有初评的排最后，同分时按 candidate_key 定，保证顺序确定。
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| {
        let p = |i: &usize| {
            by_key
                .get(items[*i].candidate.candidate_key.as_str())
                .copied()
        };
        match (p(a), p(b)) {
            (Some(x), Some(y)) => y.total_cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| {
            items[*a]
                .candidate
                .candidate_key
                .cmp(&items[*b].candidate.candidate_key)
        })
    });

    // 没读到实图的不进模型：口径已经定死结论了，花钱问一遍没有意义
    let (seen, unseen): (Vec<usize>, Vec<usize>) =
        order.into_iter().partition(|i| items[*i].image_seen);
    for i in unseen {
        let it = &items[i];
        let mut j = verdict::pending_for_missing_image(it.candidate, &it.image_gap);
        j.inputs_hash = inputs_hash(it, deps.work_standard);
        out.judgements.push(j);
    }

    // 指纹一致的旧结论直接拿来用。预取轮 01:40 判过的，这里一次网关调用都不花，
    // 而且**结论立刻就在手上**——首批 ≤20 分钟主要靠这一段。
    let mut fresh = Vec::with_capacity(seen.len());
    let mut reused: Vec<Judgement> = Vec::new();
    for i in seen {
        let it = &items[i];
        let hash = inputs_hash(it, deps.work_standard);
        match deps
            .cached
            .and_then(|c| c.get(&it.candidate.candidate_key, &hash))
        {
            // 拿回来的也要过契约自检：库被手改过、或者旧版本写进去的结论
            // 不合现在的契约时，宁可重判一遍，也不要把它当成这一轮的结论
            Some(j) if j.violations().is_empty() => {
                out.reused.push(j.candidate_key.clone());
                reused.push(j);
            }
            Some(j) => {
                tracing::warn!(候选 = %j.candidate_key, 原因 = %j.violations().join("；"), "旧结论不合契约，重判");
                fresh.push(i);
            }
            None => fresh.push(i),
        }
    }
    if !reused.is_empty() {
        // 复用的这一批与判出来的那几批一样要回调：调用方凑首批时不该区别对待
        if let Some(cb) = deps.on_batch {
            cb(&reused);
        }
        out.judgements.extend(reused);
    }

    let batches: Vec<Vec<usize>> = fresh
        .chunks(verdict::BATCH)
        .map(<[usize]>::to_vec)
        .collect();
    let conc = deps.batch_concurrency.max(1);
    // 借一份给闭包用：闭包是 FnMut，直接引 out.triages 会被当成整体移动
    let triages = &out.triages;
    // 与采集那一步同理：不报进度的话，判断也是几十分钟的黑盒。
    // 按**批**报而不是按条——一批就是一次网关调用，它才是真正的时间单位。
    let total_batches = batches.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let done = &done;
    tracing::info!(
        待判 = fresh.len(),
        批 = total_batches,
        并发 = conc,
        "判断：开始"
    );
    let results: Vec<BatchResult> =
        futures::StreamExt::collect::<Vec<_>>(futures::StreamExt::buffered(
            futures::stream::iter(batches.into_iter().map(|b| async move {
                let t = std::time::Instant::now();
                let inputs: Vec<JudgeInput<'_>> = b
                    .iter()
                    .map(|i| {
                        let it = &items[*i];
                        JudgeInput {
                            candidate: it.candidate,
                            descriptions: it.descriptions,
                            images_b64: it.images_b64.clone(),
                            materials: &it.materials,
                            triage: triages
                                .iter()
                                .find(|t| t.candidate_key == it.candidate.candidate_key),
                            heat_note: it.heat_note.clone(),
                        }
                    })
                    .collect();
                let r = verdict::judge_batch(deps.model, &inputs, deps.work_standard).await;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                tracing::info!(
                    进度 = format!("{n}/{total_batches} 批"),
                    这批 = b.len(),
                    成 = r.is_ok(),
                    秒 = t.elapsed().as_secs(),
                    "判断：一批完成"
                );
                (b, r)
            })),
            conc,
        ))
        .await;

    let mut failed: Vec<Vec<usize>> = Vec::new();
    for (idx, r) in results {
        match r {
            Ok(js) => {
                let filled = attach(js, &idx, items, deps.work_standard);
                if let Some(cb) = deps.on_batch {
                    cb(&filled);
                }
                out.judgements.extend(filled);
            }
            Err(e) => {
                tracing::warn!(条数 = idx.len(), 原因 = %format!("{e:#}"), "这一批判断失败，最后再试一次");
                failed.push(idx);
            }
        }
    }

    // 失败的批**并发跑完之后按顺序再试一次**。失败多半是网关并发超限（429）——
    // 大家一起跑时撞上，等并发的那一波过去、一批一批地发，大多能过。
    // 09-23 演练时 P5 同时占着网关，4 批 24 条就是这么掉的，一次都没重试过。
    // 仍失败的标「未判」并计数，不算判过也不算淘汰。
    for idx in failed {
        let inputs: Vec<JudgeInput<'_>> = idx
            .iter()
            .map(|i| {
                let it = &items[*i];
                JudgeInput {
                    candidate: it.candidate,
                    descriptions: it.descriptions,
                    images_b64: it.images_b64.clone(),
                    materials: &it.materials,
                    triage: out
                        .triages
                        .iter()
                        .find(|t| t.candidate_key == it.candidate.candidate_key),
                    heat_note: it.heat_note.clone(),
                }
            })
            .collect();
        match verdict::judge_batch(deps.model, &inputs, deps.work_standard).await {
            Ok(js) => {
                let filled = attach(js, &idx, items, deps.work_standard);
                if let Some(cb) = deps.on_batch {
                    cb(&filled);
                }
                out.judgements.extend(filled);
            }
            Err(e) => {
                tracing::warn!(条数 = idx.len(), 原因 = %format!("{e:#}"), "重试后这一批仍判断失败");
                out.unjudged.extend(
                    idx.iter()
                        .map(|i| items[*i].candidate.candidate_key.clone()),
                );
            }
        }
    }

    // ── 四、核对依据 ──
    if let Some(jev) = deps.jev {
        out.flags = check_all(jev, &out.judgements, items).await;
    }
    out
}

/// 把模型返回的结论对回候选：**按 `candidate_key` 匹配，不按顺序**。
///
/// 模型多半会保序，但「多半」不够——顺序错一位，整批的依据就全指向了别人的正文，
/// 而那种错在台账上看不出来。对不上的按未判处理。
fn attach(
    js: Vec<Judgement>,
    idx: &[usize],
    items: &[Item<'_>],
    work_standard: &str,
) -> Vec<Judgement> {
    let pos: HashMap<&str, usize> = idx
        .iter()
        .map(|i| (items[*i].candidate.candidate_key.as_str(), *i))
        .collect();
    js.into_iter()
        .filter_map(|mut j| {
            let i = *pos.get(j.candidate_key.as_str())?;
            j.inputs_hash = inputs_hash(&items[i], work_standard);
            Some(j)
        })
        .collect()
}

async fn triage_all(jev: &JevClient, items: &[Item<'_>]) -> Vec<Triage> {
    let tasks = items.iter().map(|it| async move {
        triage::triage_one(jev, it.candidate, it.descriptions, &it.heat_note)
            .await
            .map_err(|e| {
                tracing::warn!(候选 = %it.candidate.candidate_key, 原因 = %format!("{e:#}"), "初评失败，这条排到最后照常判");
                e
            })
            .ok()
    });
    futures::StreamExt::collect::<Vec<_>>(futures::StreamExt::buffered(
        futures::stream::iter(tasks),
        8,
    ))
    .await
    .into_iter()
    .flatten()
    .collect()
}

fn merge_inputs<'a>(items: &'a [Item<'a>]) -> Vec<MergeInput<'a>> {
    items
        .iter()
        .map(|it| MergeInput {
            candidate: it.candidate,
            fused: it.fused,
        })
        .collect()
}

async fn merge_all(jev: &JevClient, items: &[Item<'_>]) -> Vec<EventGroup> {
    let inputs = merge_inputs(items);
    let pairs = merge::coarse_pairs(&inputs);
    let tasks = pairs.iter().map(|(i, j, _)| async move {
        let p = merge::same_event(jev, items[*i].candidate, items[*j].candidate)
            .await
            .unwrap_or(0.0);
        (*i, *j, p)
    });
    let asked: Vec<(usize, usize, f64)> = futures::StreamExt::collect::<Vec<_>>(
        futures::StreamExt::buffered(futures::stream::iter(tasks), 8),
    )
    .await;
    let merged: Vec<(usize, usize, f64)> = asked
        .into_iter()
        .filter(|(_, _, p)| *p >= merge::SAME_EVENT_THRESHOLD)
        .collect();
    merge::group(&inputs, &merged)
}

async fn check_all(jev: &JevClient, js: &[Judgement], items: &[Item<'_>]) -> Vec<Flag> {
    let by_key: HashMap<&str, &Item<'_>> = items
        .iter()
        .map(|it| (it.candidate.candidate_key.as_str(), it))
        .collect();
    let tasks = js.iter().filter_map(|j| {
        let it = by_key.get(j.candidate_key.as_str())?;
        Some(async move {
            check::check_one(jev, j, it.candidate, it.descriptions)
                .await
                .unwrap_or_default()
        })
    });
    futures::StreamExt::collect::<Vec<_>>(futures::StreamExt::buffered(
        futures::stream::iter(tasks),
        8,
    ))
    .await
    .into_iter()
    .flatten()
    .collect()
}

/// 决定这条结论能不能复用的输入指纹。
///
/// 覆盖：正文与译文、每张图的描述、五类对照材料、准则版本、提示词版本、作业标准。
/// **漏掉任何一项都会让「换了东西还在用旧结论」这件事悄悄发生**——
/// 预取轮与正式轮就是靠它对账的。
pub fn inputs_hash(item: &Item<'_>, work_standard: &str) -> String {
    let mut h = blake3::Hasher::new();
    let mut put = |s: &str| {
        h.update(s.as_bytes());
        h.update(b"\x1f");
    };
    put(RUBRIC_VERSION);
    put(PROMPT_VERSION);
    put(work_standard.trim());
    put(&item.candidate.candidate_key);
    put(&item.candidate.text);
    put(&item.candidate.translated);
    for d in item.descriptions {
        put(&d.blake3);
        put(&d.content);
        put(&d.prompt_version);
        put(&d.model);
    }
    // 对照材料按 (类型, ref_id) 排序后入哈希：检索顺序有随机性，
    // 不排序的话同样的材料会算出不同的指纹，复用就永远命中不了
    let mut refs: Vec<String> = item
        .materials
        .iter()
        .map(|m| format!("{:?}/{}", m.kind, m.ref_id))
        .collect();
    refs.sort();
    for r in &refs {
        put(r);
    }
    put(if item.image_seen { "seen" } else { "unseen" });
    h.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::model::ModelConfig;
    use csw_collector_core::types::{
        Dim, ImageKind, MaterialKind, MediaKind, MediaRef, Platform, Tier,
    };
    use serde_json::json;

    fn cand(key: &str, text: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "c".into(),
            account: "acc".into(),
            url: format!("https://x/{key}"),
            text: text.into(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![MediaRef {
                source_hash: "h".into(),
                kind: MediaKind::Photo,
                url: "u".into(),
                blake3: Some("b0".into()),
                ordinal: 0,
            }],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn desc() -> MediaDescription {
        MediaDescription {
            blake3: "b0".into(),
            ordinal: 0,
            matches_text: String::new(),
            content: "一只灰色背包".into(),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: "recognize/v1".into(),
        }
    }

    fn material(kind: MaterialKind, ref_id: &str) -> Material {
        Material {
            kind,
            ref_id: ref_id.into(),
            title: "t".into(),
            date: None,
            source: String::new(),
            quote: String::new(),
            publish_state: String::new(),
        }
    }

    fn item<'a>(c: &'a Candidate, ds: &'a [MediaDescription], ms: Vec<Material>) -> Item<'a> {
        Item {
            candidate: c,
            descriptions: ds,
            images_b64: vec!["img".into()],
            materials: ms,
            fused: None,
            image_seen: true,
            image_gap: String::new(),
            heat_note: "369 赞".into(),
        }
    }

    #[test]
    fn 指纹不受材料顺序影响但随内容变() {
        let c = cand("k", "正文");
        let ds = [desc()];
        let a = item(
            &c,
            &ds,
            vec![
                material(MaterialKind::Published, "p1"),
                material(MaterialKind::Example, "e1"),
            ],
        );
        let b = item(
            &c,
            &ds,
            vec![
                material(MaterialKind::Example, "e1"),
                material(MaterialKind::Published, "p1"),
            ],
        );
        // 检索顺序有随机性，不排序的话复用永远命中不了
        assert_eq!(inputs_hash(&a, "标准"), inputs_hash(&b, "标准"));
        // 换一条材料就该变
        let d = item(&c, &ds, vec![material(MaterialKind::Published, "p2")]);
        assert_ne!(inputs_hash(&a, "标准"), inputs_hash(&d, "标准"));
        // 作业标准变了也要变
        assert_ne!(inputs_hash(&a, "标准"), inputs_hash(&a, "别的标准"));
        // 正文变了也要变
        let c2 = cand("k", "改过的正文");
        assert_ne!(
            inputs_hash(&a, "标准"),
            inputs_hash(&item(&c2, &ds, vec![]), "标准")
        );
        // 读没读到实图是两份不同的输入
        let mut unseen = item(&c, &ds, vec![]);
        unseen.image_seen = false;
        assert_ne!(
            inputs_hash(&item(&c, &ds, vec![]), "标准"),
            inputs_hash(&unseen, "标准")
        );
    }

    #[test]
    fn 指纹覆盖图片描述的版本() {
        let c = cand("k", "正文");
        let mut d2 = desc();
        d2.prompt_version = "recognize/v2".into();
        let a = [desc()];
        let b = [d2];
        // 换了识别提示词，描述就不是同一份，旧结论不能再用
        assert_ne!(
            inputs_hash(&item(&c, &a, vec![]), "标准"),
            inputs_hash(&item(&c, &b, vec![]), "标准")
        );
    }

    #[test]
    fn 结论按key对回去不按顺序() {
        let (c1, c2) = (cand("k1", "甲"), cand("k2", "乙"));
        let ds = [desc()];
        let items = [item(&c1, &ds, vec![]), item(&c2, &ds, vec![])];
        let js = vec![
            verdict::pending_for_missing_image(&c2, "x"), // 模型把顺序调过来了
            verdict::pending_for_missing_image(&c1, "x"),
        ];
        let got = attach(js, &[0, 1], &items, "标准");
        // 顺序错一位，整批的依据就全指向别人的正文，而那种错在台账上看不出来
        assert_eq!(got[0].candidate_key, "k2");
        assert_eq!(got[1].candidate_key, "k1");
        assert_eq!(got[0].inputs_hash, inputs_hash(&items[1], "标准"));
        assert_eq!(got[1].inputs_hash, inputs_hash(&items[0], "标准"));
    }

    #[test]
    fn 对不上的结论丢掉不猜() {
        let c1 = cand("k1", "甲");
        let ds = [desc()];
        let items = [item(&c1, &ds, vec![])];
        let ghost = cand("模型编出来的键", "丙");
        let js = vec![verdict::pending_for_missing_image(&ghost, "x")];
        assert!(attach(js, &[0], &items, "标准").is_empty());
    }

    /// 模型会回的一条完整推荐结论
    fn one_recommend(k: &str) -> serde_json::Value {
        json!({
            "candidate_key": k, "tier": "recommend",
            "dims": {"change": {"verdict":"yes","basis":"b"}, "use": {"verdict":"yes","basis":"b"},
                     "gain": {"verdict":"yes","basis":"b"}, "compare": {"verdict":"yes","basis":"b"},
                     "explain": {"verdict":"yes","basis":"b"}, "csw": {"verdict":"yes","basis":"b"}},
            "three_sentences": {"what_changed":"a","why_it_matters":"b","how_different":"c"},
            "unanswered": "none",
            "comparison": {"verdict":"unrelated","against":"","note":""},
            "heat_note": "", "look": "", "image_seen": true,
            "gaps": [], "priority_hits": [], "lower_hits": [],
            "jev_disagreement": "", "kb_refs": [], "memory_refs": []
        })
    }

    async fn model_returning(body: serde_json::Value) -> (wiremock::MockServer, ModelClient) {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let m = ModelClient::new(ModelConfig {
            base_url: server.uri(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 2,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap();
        (server, m)
    }

    #[tokio::test]
    async fn 没读到实图的不进模型() {
        // 口径已经定死结论了，花钱问一遍没有意义
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let m = ModelClient::new(ModelConfig {
            base_url: server.uri(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 1,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap();

        let c = cand("k1", "甲");
        let ds = [desc()];
        let mut it = item(&c, &ds, vec![]);
        it.image_seen = false;
        it.image_gap = "3 张下载失败".into();
        let out = run(
            &[it],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "标准",
                batch_concurrency: 1,
                on_batch: None,
                cached: None,
            },
        )
        .await;
        assert_eq!(out.judgements.len(), 1);
        assert_eq!(out.judgements[0].tier, Tier::PendingCheck);
        assert!(out.judgements[0].gaps[0].contains("3 张下载失败"));
        // 代码给的兜底也要带指纹，不然对账时它永远是「变了」
        assert!(!out.judgements[0].inputs_hash.is_empty());
        assert!(out.unjudged.is_empty(), "待核不是未判");
    }

    #[tokio::test]
    async fn 判完一批就回调一次() {
        let payload = json!({"judgements": [one_recommend("k1")]});
        let (_s, m) = model_returning(json!({
            "output": [{"content": [{"type": "output_text", "text": payload.to_string()}]}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }))
        .await;

        let c = cand("k1", "甲");
        let ds = [desc()];
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let cb = |js: &[Judgement]| {
            seen.lock()
                .unwrap()
                .extend(js.iter().map(|j| j.candidate_key.clone()));
        };
        let out = run(
            &[item(&c, &ds, vec![])],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "标准",
                batch_concurrency: 1,
                on_batch: Some(&cb),
                cached: None,
            },
        )
        .await;
        assert_eq!(out.judgements.len(), 1);
        assert_eq!(out.judgements[0].tier, Tier::Recommend);
        assert!(!out.judgements[0].inputs_hash.is_empty());
        // 首批 ≤20 分钟只能靠流式推进达成，回调不能少
        assert_eq!(*seen.lock().unwrap(), ["k1"]);
    }

    struct Store(Vec<Judgement>);
    impl Cached for Store {
        fn get(&self, key: &str, hash: &str) -> Option<Judgement> {
            self.0
                .iter()
                .find(|j| j.candidate_key == key && j.inputs_hash == hash)
                .cloned()
        }
    }

    /// 一条合契约的完整结论，给复用那几个测试当料
    fn full_judgement(key: &str, hash: &str) -> Judgement {
        Judgement {
            candidate_key: key.into(),
            tier: Tier::Alternate,
            dims: Dim::ALL
                .into_iter()
                .map(|d| {
                    (
                        d,
                        csw_collector_core::types::DimJudgement {
                            verdict: csw_collector_core::types::Verdict::Yes,
                            basis: "正文第一句".into(),
                        },
                    )
                })
                .collect(),
            three_sentences: csw_collector_core::types::ThreeSentences {
                what_changed: "甲".into(),
                why_it_matters: "乙".into(),
                how_different: "丙".into(),
            },
            unanswered: csw_collector_core::types::Unanswered::None,
            comparison: csw_collector_core::types::Comparison {
                verdict: csw_collector_core::types::ComparisonVerdict::Unrelated,
                against: String::new(),
                note: String::new(),
            },
            heat_note: String::new(),
            look: String::new(),
            image_seen: true,
            gaps: vec![],
            priority_hits: vec![],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: hash.into(),
        }
    }

    #[tokio::test]
    async fn 指纹一致就不再问模型() {
        // 一次网关调用都不该发出去：预取轮已经判过了
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let m = ModelClient::new(ModelConfig {
            base_url: server.uri(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 1,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap();

        let c = cand("k1", "甲");
        let ds = [desc()];
        let it = item(&c, &ds, vec![]);
        let hash = inputs_hash(&it, "标准");
        let store = Store(vec![full_judgement("k1", &hash)]);

        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let cb = |js: &[Judgement]| {
            seen.lock()
                .unwrap()
                .extend(js.iter().map(|j| j.candidate_key.clone()));
        };
        let out = run(
            &[it],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "标准",
                batch_concurrency: 1,
                on_batch: Some(&cb),
                cached: Some(&store),
            },
        )
        .await;
        assert_eq!(out.judgements.len(), 1);
        assert_eq!(out.judgements[0].tier, Tier::Alternate);
        assert_eq!(out.reused, ["k1"]);
        // 复用的也要回调：调用方凑首批时不该区别对待
        assert_eq!(*seen.lock().unwrap(), ["k1"]);
    }

    #[tokio::test]
    async fn 作业标准变了就得重判() {
        let payload = json!({"judgements": [one_recommend("k1")]});
        let (_s, m) = model_returning(json!({
            "output": [{"content": [{"type": "output_text", "text": payload.to_string()}]}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }))
        .await;

        let c = cand("k1", "甲");
        let ds = [desc()];
        let it = item(&c, &ds, vec![]);
        // 预取轮用的是上一次的标准，这一轮主编改了派工单备注
        let store = Store(vec![full_judgement("k1", &inputs_hash(&it, "旧标准"))]);
        let out = run(
            &[it],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "新标准",
                batch_concurrency: 1,
                on_batch: None,
                cached: Some(&store),
            },
        )
        .await;
        assert!(out.reused.is_empty(), "指纹对不上就不许拿旧结论");
        assert_eq!(out.judgements.len(), 1);
        assert_eq!(out.judgements[0].tier, Tier::Recommend, "是模型这次判的");
    }

    #[tokio::test]
    async fn 旧结论不合契约就重判() {
        let payload = json!({"judgements": [one_recommend("k1")]});
        let (_s, m) = model_returning(json!({
            "output": [{"content": [{"type": "output_text", "text": payload.to_string()}]}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }))
        .await;

        let c = cand("k1", "甲");
        let ds = [desc()];
        let it = item(&c, &ds, vec![]);
        let hash = inputs_hash(&it, "标准");
        let mut broken = full_judgement("k1", &hash);
        broken.dims.truncate(2); // 六维缺了四维
        let store = Store(vec![broken]);
        let out = run(
            &[it],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "标准",
                batch_concurrency: 1,
                on_batch: None,
                cached: Some(&store),
            },
        )
        .await;
        // 宁可重判一遍，也不要把不合契约的结论当成这一轮的结论
        assert!(out.reused.is_empty());
        assert_eq!(out.judgements.len(), 1);
        assert!(out.judgements[0].violations().is_empty());
    }

    #[tokio::test]
    async fn 一批失败只影响一批() {
        let (_s, m) = model_returning(json!({"output": [], "usage": {}})).await;
        let c = cand("k1", "甲");
        let ds = [desc()];
        let out = run(
            &[item(&c, &ds, vec![])],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "标准",
                batch_concurrency: 1,
                on_batch: None,
                cached: None,
            },
        )
        .await;
        // 仍失败的标「未判」并计数——不算判过，也不算淘汰
        assert!(out.judgements.is_empty());
        assert_eq!(out.unjudged, ["k1"]);
    }

    #[tokio::test]
    async fn 撞上429的批最后再试一次() {
        let server = wiremock::MockServer::start().await;
        // 第一次 429（网关并发超限），之后正常
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(429).set_body_json(json!({
                "error": {"code": "gateway_concurrency_limit"}
            })))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        let payload = json!({"judgements": [one_recommend("k1")]});
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "output": [{"content": [{"type": "output_text", "text": payload.to_string()}]}],
                "usage": {"input_tokens": 10, "output_tokens": 5}
            })))
            .with_priority(2)
            .mount(&server)
            .await;
        let m = ModelClient::new(ModelConfig {
            base_url: server.uri(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 2,
            timeout: std::time::Duration::from_secs(5),
            // 客户端自己不重试：钉的是流水线那一次补跑
            max_attempts: 1,
        })
        .unwrap();
        let c = cand("k1", "甲");
        let ds = [desc()];
        let out = run(
            &[item(&c, &ds, vec![])],
            &Deps {
                model: &m,
                jev: None,
                work_standard: "标准",
                batch_concurrency: 1,
                on_batch: None,
                cached: None,
            },
        )
        .await;
        assert!(out.unjudged.is_empty(), "{:?}", out.unjudged);
        assert_eq!(out.judgements.len(), 1);
    }
}
