//! 「采集媒体信息」这一步：**取候选 → 下载 → 识别 → 向量化，四段连着做完才算完成。**
//!
//! 这是总方案里刻意合并成一件事的那一步。分开会出一种很难查的状态：
//! 候选进了库、图还没下、判断却已经开始——于是「没读到实图」被当成了「这条没价值」。
//!
//! 计数全部由**代码**算，不由采集器自报：`found` / `fetched_unique` / `in_window` /
//! `reviewed` / `unreviewed` 是 `intake-check` 的对账依据，让实现方自己填等于自己给自己打分。
//!
//! 一条口径直接落在这里：**工作台每条都判，所以 `unreviewed` 必须是 0**。
//! 不是 0 就说明这一步没跑完——如实写上去，让自查判红，而不是凑成 0。

use std::collections::HashMap;

use csw_collector_core::model::ModelClient;
use csw_collector_core::types::{Candidate, MediaDescription, MediaKind, Timestamp};
use csw_collector_core::vector::{EmbedInput, VectorClient};

use crate::collector::{Collector, Outcome};
use crate::download::Downloader;
use crate::recognize;

/// 一个采集器跑完的账。字段名与引擎 `sweeps` 对齐。
#[derive(Debug, Clone, Default)]
pub struct SweepCount {
    pub sweep_key: String,
    pub platform: String,
    pub source_key: String,
    pub query: String,
    /// 接口返回条数，含重复
    pub found: i64,
    /// 去重后
    pub fetched_unique: i64,
    /// 其中落在窗口内的
    pub in_window: i64,
    /// 真读过正文或看过图并形成判断的。这一步只负责把材料备齐，
    /// 判断在后面——所以这里填的是「材料齐备、可以判」的条数。
    pub reviewed: i64,
    /// 只加载未展开的。工作台每条都判，这里应当是 0。
    pub unreviewed: i64,
    pub registered: i64,
    pub paged_to_end: bool,
    /// ok | failed | partial
    pub result: String,
    pub error: String,
    pub started_at: String,
    pub ended_at: String,
}

/// 一条候选在这一步的产物。
#[derive(Debug)]
pub struct Prepared {
    pub candidate: Candidate,
    pub descriptions: Vec<MediaDescription>,
    /// 图文融合向量（正文 + 代表图），一条候选一个
    pub fused: Option<Vec<f32>>,
    /// 每张图一个向量，与 `descriptions` 同序
    pub image_vectors: Vec<Vec<f32>>,
    /// 下载或识别失败的图
    pub failed_media: Vec<String>,
    /// 这条候选在模型网关上花的毫秒（识别）。**是占用时长不是墙钟**，
    /// 多条并发时各自的和会远大于整段墙钟——那正是要看的：
    /// 和 ÷ 墙钟 ≈ 网关的有效并发度，明显低于并发上限就说明重叠没做起来。
    pub recognize_ms: u128,
    /// 这条候选在 GPU 上花的毫秒（向量化）。GPU 是**全局串行**的，
    /// 所有候选的和就是整段里 GPU 的占用时长，直接和墙钟比就知道是不是卡在显卡上。
    pub embed_ms: u128,
}

impl Prepared {
    /// 真读到实图了吗。**只要有一张图没识别成功就是 false**——
    /// 判断那一步会据此落待核，待核不是淘汰。
    pub fn image_seen(&self) -> bool {
        !self.candidate.media.is_empty()
            && self.failed_media.is_empty()
            && self.descriptions.len()
                == self
                    .candidate
                    .media
                    .iter()
                    .filter(|m| m.kind == MediaKind::Photo)
                    .count()
    }
}

/// 一段 `prepare` 的耗时账。把「网关时间」与「GPU 时间」分开记——
/// 两者是不同的资源，混成一个总数就看不出该往哪儿加通道。
#[derive(Debug, Default, Clone, Copy)]
pub struct PrepareStats {
    /// 下载整段的墙钟（下载是统一做的，不分摊到候选头上）
    pub download_ms: u128,
    /// 识别 + 向量化整段的墙钟
    pub wall_ms: u128,
    /// 各候选在网关上的占用时长之和
    pub recognize_ms: u128,
    /// 各候选在 GPU 上的占用时长之和
    pub embed_ms: u128,
}

impl PrepareStats {
    /// 网关的有效并发度 = 占用之和 ÷ 墙钟。逼近并发上限才算把闸门用满了。
    pub fn gateway_concurrency(&self) -> f64 {
        ratio(self.recognize_ms, self.wall_ms)
    }
    /// GPU 的占用率。逼近 1 就说明瓶颈在显卡，再加模型通道也没用。
    pub fn gpu_busy(&self) -> f64 {
        ratio(self.embed_ms, self.wall_ms)
    }
}

fn ratio(a: u128, b: u128) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

#[derive(Debug, Default)]
pub struct HarvestResult {
    pub prepared: Vec<Prepared>,
    pub sweeps: Vec<SweepCount>,
    /// 必扫采集器失败了：这一步要停在这里告警，**不用旧接口兜底**
    pub blocked: Option<String>,
}

pub struct Deps<'a> {
    pub downloader: &'a Downloader,
    pub model: &'a ModelClient,
    pub vector: &'a VectorClient,
    /// 只取图文。总方案定死：类型是图或轮播，且媒体里一个视频都没有。
    pub image_only: bool,
    /// 同时处理几条候选。取模型网关的并发上限即可——真正的闸门在客户端里，
    /// 这里只是别让它们闲着。
    pub concurrency: usize,
}

/// 跑一遍取候选：每个采集器一条采集轮账。
///
/// 去重与窗口过滤在这里统一做，不交给采集器——它们各自去重的话，
/// 跨采集器的重复就没人管了。
pub async fn collect_all(
    collectors: &[&dyn CollectorDyn],
    from: Timestamp,
    to: Timestamp,
    image_only: bool,
) -> (Vec<Candidate>, Vec<SweepCount>, Option<String>) {
    let mut all = Vec::new();
    let mut sweeps = Vec::new();
    let mut blocked = None;

    for c in collectors {
        let started = now_str();
        let mut sc = SweepCount {
            sweep_key: c.sweep_key(),
            platform: c.platform().to_string(),
            source_key: c.source_key(),
            started_at: started,
            result: "ok".into(),
            ..Default::default()
        };
        match c.collect_dyn(from, to).await {
            Outcome::Ok(h) => {
                sc.found = h.found;
                sc.query = h.query;
                sc.paged_to_end = h.paged_to_end;
                // 采集器内部可能也有重复（翻页边界），先去一遍
                let uniq = crate::collector::dedup(h.candidates);
                sc.fetched_unique = uniq.len() as i64;
                let kept: Vec<_> = uniq
                    .into_iter()
                    .filter(|x| !image_only || x.is_image_only())
                    .filter(|x| in_window(x, from, to))
                    .collect();
                sc.in_window = kept.len() as i64;
                all.extend(kept);
            }
            Outcome::Failed(e) => {
                sc.result = "failed".into();
                sc.error = e.clone();
                // 必扫的失败要停：不用旧接口兜底，那会把「没去找」伪装成「没找到」
                if c.required() && blocked.is_none() {
                    blocked = Some(format!("必扫采集器 {} 失败：{e}", c.sweep_key()));
                }
            }
            Outcome::Disabled => {
                // 「没开」要与「没扫到」在台账上分得开
                sc.result = "partial".into();
                sc.error = "采集器未启用".into();
            }
        }
        sc.ended_at = now_str();
        sweeps.push(sc);
    }
    // 跨采集器再去一次重
    (crate::collector::dedup(all), sweeps, blocked)
}

/// 下载 → 识别 → 向量化。**一条候选的图全部处理完（成功或标未识别）才算采集完成。**
///
/// **候选之间并发**，并发度由 `Deps::concurrency` 给（取模型网关的并发上限即可）。
///
/// 这一条是实跑打出来的：最初写成逐条串行，M1 实测每条 71.5 秒、外推一期 7 小时——
/// 比 0.5 按并发 8 估的 42 分钟慢十倍。真正的闸门在两个客户端里
/// （模型的信号量 8、GPU 的互斥锁 1），外层再串行只是白白让它们闲着。
/// 用 `buffered` 而不是 `buffer_unordered`：保序才能让输出稳定、便于比对。
pub async fn prepare(candidates: Vec<Candidate>, deps: &Deps<'_>) -> (Vec<Prepared>, PrepareStats) {
    // 一次把全部图排进下载队列：并发受限在 Downloader 里，这里不必再切
    let urls: Vec<String> = candidates
        .iter()
        .flat_map(|c| c.media.iter().filter(|m| m.kind == MediaKind::Photo))
        .map(|m| m.url.clone())
        .collect();
    let t_dl = std::time::Instant::now();
    let report = deps.downloader.fetch_all(&urls).await;
    let download_ms = t_dl.elapsed().as_millis();
    let by_url: HashMap<String, _> = report
        .ok
        .iter()
        .map(|d| (d.url.clone(), d.clone()))
        .collect();
    let failed_urls: std::collections::HashSet<&str> =
        report.failed.iter().map(|f| f.url.as_str()).collect();

    let conc = deps.concurrency.max(1);
    let tasks = candidates
        .into_iter()
        .map(|c| prepare_one(c, &by_url, &failed_urls, deps));
    let t_prep = std::time::Instant::now();
    let prepared: Vec<Prepared> = futures::StreamExt::collect::<Vec<_>>(
        futures::StreamExt::buffered(futures::stream::iter(tasks), conc),
    )
    .await;
    let stats = PrepareStats {
        download_ms,
        wall_ms: t_prep.elapsed().as_millis(),
        recognize_ms: prepared.iter().map(|p| p.recognize_ms).sum(),
        embed_ms: prepared.iter().map(|p| p.embed_ms).sum(),
    };
    (prepared, stats)
}

/// 一条候选走完识别与向量化。下载在外面已经统一做过。
async fn prepare_one(
    mut c: Candidate,
    by_url: &HashMap<String, crate::download::Downloaded>,
    failed_urls: &std::collections::HashSet<&str>,
    deps: &Deps<'_>,
) -> Prepared {
    {
        let mut failed_media = Vec::new();
        let mut refs = Vec::new();
        for m in c.media.iter_mut().filter(|m| m.kind == MediaKind::Photo) {
            match by_url.get(&m.url) {
                Some(d) => {
                    m.blake3 = Some(d.blake3.clone());
                    match std::fs::read(&d.path) {
                        Ok(bytes) => refs.push(recognize::ImageRef {
                            blake3: d.blake3.clone(),
                            b64: b64(&bytes),
                            ordinal: m.ordinal,
                        }),
                        Err(e) => failed_media.push(format!("{}：读盘失败 {e}", m.url)),
                    }
                }
                None if failed_urls.contains(m.url.as_str()) => {
                    failed_media.push(format!("{}：下载失败", m.url))
                }
                None => failed_media.push(format!("{}：没有下载结果", m.url)),
            }
        }

        // 识别：按批走；某一批失败只影响那一批，不牵连整条候选
        let t_rec = std::time::Instant::now();
        let mut descriptions = Vec::new();
        for batch in refs.chunks(recognize::MAX_IMAGES_PER_CALL) {
            match recognize::recognize_batch(deps.model, &c.text, batch).await {
                Ok(mut d) => descriptions.append(&mut d),
                Err(e) => {
                    tracing::warn!(候选 = %c.candidate_key, 原因 = %format!("{e:#}"), "识别失败，这批图标未识别");
                    failed_media.extend(batch.iter().map(|b| format!("{}：识别失败", b.blake3)));
                }
            }
        }

        let recognize_ms = t_rec.elapsed().as_millis();

        // 向量化：图文融合一条 + 每张图一条
        let t_emb = std::time::Instant::now();
        let (fused, image_vectors) = embed_for(&c, &refs, &descriptions, deps.vector).await;
        Prepared {
            candidate: c,
            descriptions,
            fused,
            image_vectors,
            failed_media,
            recognize_ms,
            embed_ms: t_emb.elapsed().as_millis(),
        }
    }
}

async fn embed_for(
    c: &Candidate,
    refs: &[recognize::ImageRef],
    descriptions: &[MediaDescription],
    vector: &VectorClient,
) -> (Option<Vec<f32>>, Vec<Vec<f32>>) {
    if refs.is_empty() && c.text.trim().is_empty() {
        return (None, vec![]);
    }
    let mut inputs = Vec::with_capacity(refs.len() + 1);
    // 融合向量用正文 + 代表图（第一张）：整条候选的语义由它代表
    let fused_text = fused_text(c, descriptions);
    inputs.push(EmbedInput::Fused {
        text: fused_text,
        images_b64: refs
            .first()
            .map(|r| vec![r.b64.clone()])
            .unwrap_or_default(),
    });
    inputs.extend(refs.iter().map(|r| EmbedInput::Image(r.b64.clone())));

    match vector.embed(&inputs).await {
        Ok(mut vs) => {
            let fused = if vs.is_empty() {
                None
            } else {
                Some(vs.remove(0))
            };
            (fused, vs)
        }
        Err(e) => {
            // 向量化失败不阻塞判断：没有向量只是检索差一点，不是「这条不能判」
            tracing::warn!(候选 = %c.candidate_key, 原因 = %format!("{e:#}"), "向量化失败，本条无向量");
            (None, vec![])
        }
    }
}

/// 融合向量的入模文本：正文 + 译文 + 每张图的画面描述。
///
/// 把图片描述也拼进去，是因为很多贴文正文只有一句话加一串话题，
/// 真正的信息在图里——只用正文算向量，检索会找不着它。
fn fused_text(c: &Candidate, descriptions: &[MediaDescription]) -> String {
    let mut s = String::new();
    s.push_str(&c.text);
    if !c.translated.trim().is_empty() {
        s.push('\n');
        s.push_str(&c.translated);
    }
    for d in descriptions {
        s.push('\n');
        s.push_str(&d.content);
    }
    s.chars().take(1500).collect()
}

fn b64(bytes: &[u8]) -> String {
    use std::fmt::Write;
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let _ = write!(
            out,
            "{}{}",
            T[(n >> 18) as usize & 63] as char,
            T[(n >> 12) as usize & 63] as char
        );
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn in_window(c: &Candidate, from: Timestamp, to: Timestamp) -> bool {
    // 窗口按首次入库时间；取不到就退回发布时间，宁可多判一条也不漏
    let t = c.ingested_at.or(c.posted_at);
    t.map(|t| t >= from && t < to).unwrap_or(false)
}

fn now_str() -> String {
    jiff::Timestamp::now().to_string()
}

/// 让 `&dyn` 能用的对象安全壳。`Collector` 的 async 方法不是对象安全的，
/// 编排层又需要把一组不同类型的采集器放进同一个数组里。
#[allow(async_fn_in_trait)]
pub trait CollectorDyn {
    fn sweep_key(&self) -> String;
    fn platform(&self) -> &'static str;
    fn source_key(&self) -> String;
    fn required(&self) -> bool;
    fn collect_dyn(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + '_>>;
}

// 不要求 Send：采集器是**按顺序**跑的（一个失败要能立刻决定停不停），
// 要求 Send 会逼每个采集器实现都变成 Send，换不来任何好处。
impl<T: Collector> CollectorDyn for T {
    fn sweep_key(&self) -> String {
        Collector::sweep_key(self)
    }
    fn platform(&self) -> &'static str {
        Collector::platform(self)
    }
    fn source_key(&self) -> String {
        Collector::source_key(self)
    }
    fn required(&self) -> bool {
        Collector::required(self)
    }
    fn collect_dyn(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + '_>> {
        Box::pin(Collector::collect(self, from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{Disabled, Harvested};
    use csw_collector_core::types::{ImageKind, MediaRef, Platform};

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn cand(id: &str, ingested: Option<&str>, kinds: &[MediaKind]) -> Candidate {
        Candidate {
            candidate_key: format!("k-{id}"),
            platform: Platform::Instagram,
            source_id: id.into(),
            collector: "t".into(),
            account: "a".into(),
            url: String::new(),
            text: "正文".into(),
            translated: String::new(),
            posted_at: Some(ts("2026-09-01T00:00:00Z")),
            ingested_at: ingested.map(ts),
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: if kinds.contains(&MediaKind::Video) {
                "Carousel"
            } else {
                "Image"
            }
            .into(),
            media: kinds
                .iter()
                .enumerate()
                .map(|(i, k)| MediaRef {
                    source_hash: format!("s{i}"),
                    kind: *k,
                    url: format!("http://x/{id}-{i}.jpg"),
                    blake3: None,
                    ordinal: i as u16,
                })
                .collect(),
            tags: vec![],
            hashtags: vec![],
        }
    }

    struct Fake {
        key: &'static str,
        required: bool,
        out: std::sync::Mutex<Option<Outcome>>,
    }

    impl Collector for Fake {
        fn sweep_key(&self) -> String {
            self.key.into()
        }
        fn platform(&self) -> &'static str {
            "instagram"
        }
        fn required(&self) -> bool {
            self.required
        }
        async fn collect(&self, _: Timestamp, _: Timestamp) -> Outcome {
            self.out
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Outcome::Failed("用完了".into()))
        }
    }

    fn ok(cands: Vec<Candidate>, found: i64) -> Outcome {
        Outcome::Ok(Harvested {
            found,
            candidates: cands,
            paged_to_end: true,
            query: "q".into(),
        })
    }

    #[tokio::test]
    async fn 计数由代码算且只图文与窗口都收口() {
        let from = ts("2026-09-22T00:00:00Z");
        let to = ts("2026-09-23T00:00:00Z");
        let c = Fake {
            key: "csw-window",
            required: true,
            out: std::sync::Mutex::new(Some(ok(
                vec![
                    cand("1", Some("2026-09-22T01:00:00Z"), &[MediaKind::Photo]), // 收
                    cand("1", Some("2026-09-22T01:00:00Z"), &[MediaKind::Photo]), // 重复，去掉
                    cand(
                        "2",
                        Some("2026-09-22T02:00:00Z"),
                        &[MediaKind::Photo, MediaKind::Video],
                    ), // 夹视频，去掉
                    cand("3", Some("2026-09-20T02:00:00Z"), &[MediaKind::Photo]), // 窗口外，去掉
                ],
                99, // 采集器自报 found=99，代码不采信它去算别的
            ))),
        };
        let (cands, sweeps, blocked) = collect_all(&[&c], from, to, true).await;
        assert!(blocked.is_none());
        assert_eq!(cands.len(), 1, "只该留下窗口内的纯图文那条");
        let s = &sweeps[0];
        assert_eq!(s.found, 99, "found 用采集器报的接口返回数");
        assert_eq!(s.fetched_unique, 3, "去重后 3 条");
        assert_eq!(s.in_window, 1, "只图文 + 窗口内 = 1");
        assert_eq!(s.result, "ok");
    }

    #[tokio::test]
    async fn 必扫采集器失败要停下不用旧接口兜底() {
        let c = Fake {
            key: "csw-window",
            required: true,
            out: std::sync::Mutex::new(Some(Outcome::Failed("接口 500".into()))),
        };
        let (_, sweeps, blocked) = collect_all(
            &[&c],
            ts("2026-09-22T00:00:00Z"),
            ts("2026-09-23T00:00:00Z"),
            true,
        )
        .await;
        assert!(blocked.unwrap().contains("必扫采集器"));
        assert_eq!(sweeps[0].result, "failed");
        assert!(
            !sweeps[0].error.is_empty(),
            "失败必须写 error，否则分不清没找到与没去找"
        );
    }

    #[tokio::test]
    async fn 非必扫失败只记一笔继续() {
        let bad = Fake {
            key: "web",
            required: false,
            out: std::sync::Mutex::new(Some(Outcome::Failed("超时".into()))),
        };
        let good = Fake {
            key: "csw-window",
            required: true,
            out: std::sync::Mutex::new(Some(ok(
                vec![cand("1", Some("2026-09-22T01:00:00Z"), &[MediaKind::Photo])],
                1,
            ))),
        };
        let (cands, sweeps, blocked) = collect_all(
            &[&bad, &good],
            ts("2026-09-22T00:00:00Z"),
            ts("2026-09-23T00:00:00Z"),
            true,
        )
        .await;
        assert!(blocked.is_none(), "非必扫失败不该停整步");
        assert_eq!(cands.len(), 1);
        assert_eq!(sweeps.len(), 2);
    }

    #[tokio::test]
    async fn 关着的采集器在台账上是partial而不是ok() {
        let d = Disabled {
            key: "xhs",
            platform: "xhs",
            why: "默认关闭",
        };
        let (_, sweeps, _) = collect_all(
            &[&d],
            ts("2026-09-22T00:00:00Z"),
            ts("2026-09-23T00:00:00Z"),
            true,
        )
        .await;
        assert_eq!(sweeps[0].result, "partial", "「没开」要与「没扫到」分得开");
        assert!(sweeps[0].error.contains("未启用"));
    }

    #[test]
    fn 有一张图没识别成功就不算读到实图() {
        let c = cand("1", None, &[MediaKind::Photo, MediaKind::Photo]);
        let desc = |n: usize| -> Vec<MediaDescription> {
            (0..n)
                .map(|i| MediaDescription {
                    blake3: format!("h{i}"),
                    ordinal: i as u16,
                    matches_text: String::new(),
                    content: String::new(),
                    missing_from_text: String::new(),
                    kind: ImageKind::Product,
                    usable_as_figure: true,
                    model: "m".into(),
                    prompt_version: "v".into(),
                })
                .collect()
        };
        let full = Prepared {
            candidate: c.clone(),
            descriptions: desc(2),
            fused: None,
            image_vectors: vec![],
            failed_media: vec![],
            recognize_ms: 0,
            embed_ms: 0,
        };
        assert!(full.image_seen());
        let partial = Prepared {
            candidate: c.clone(),
            descriptions: desc(1),
            fused: None,
            image_vectors: vec![],
            failed_media: vec![],
            recognize_ms: 0,
            embed_ms: 0,
        };
        assert!(!partial.image_seen(), "少一张描述就不算读到实图");
        let failed = Prepared {
            candidate: c,
            descriptions: desc(2),
            fused: None,
            image_vectors: vec![],
            failed_media: vec!["x".into()],
            recognize_ms: 0,
            embed_ms: 0,
        };
        assert!(!failed.image_seen(), "有下载失败的就不算读到实图");
    }

    #[test]
    fn 融合向量的文本把图片描述也拼进去() {
        let c = cand("1", None, &[MediaKind::Photo]);
        let d = MediaDescription {
            blake3: "h".into(),
            ordinal: 0,
            matches_text: String::new(),
            content: "画面里是一顶轻量化帐篷".into(),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: "v".into(),
        };
        let t = fused_text(&c, std::slice::from_ref(&d));
        assert!(
            t.contains("正文") && t.contains("轻量化帐篷"),
            "正文只有一句话时信息全在图里：{t}"
        );
    }

    #[test]
    fn base64编码对得上() {
        assert_eq!(b64(b"a"), "YQ==");
        assert_eq!(b64(b"ab"), "YWI=");
        assert_eq!(b64(b"abc"), "YWJj");
        assert_eq!(b64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }
}
