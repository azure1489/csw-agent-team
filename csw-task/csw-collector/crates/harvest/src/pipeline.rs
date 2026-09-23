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
    /// 描述是从缓存里拿的，这一条没走网关
    pub reused: bool,
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
    /// 识别段平均有几条候选在飞 = 占用之和 ÷ 墙钟。
    ///
    /// **注意它不等于网关的并发请求数**：一条候选的识别段里包含它在信号量上
    /// 排队的时间，而且批级并发之后一条候选可能同时压着好几个请求。
    /// 这个数只回答「流水线有没有把槽位填满」，不回答「网关跑了几路」。
    pub fn recognize_in_flight(&self) -> f64 {
        ratio(self.recognize_ms, self.wall_ms)
    }
    /// GPU 的占用率。逼近 1 就说明瓶颈在显卡，再加模型通道也没用。
    pub fn gpu_busy(&self) -> f64 {
        ratio(self.embed_ms, self.wall_ms)
    }

    /// 外推到 `total` 条要多久（秒）。**按资源占用算，不按每条墙钟算。**
    ///
    /// 抽几条试跑时，最慢那一条的尾巴占墙钟的比重极大（实测 16 条里最慢一条
    /// 116 秒、墙钟 176 秒）；用「每条墙钟 × 总条数」外推会把这段尾巴按条数
    /// 复制 N 遍，得出的数远大于真实值。大批量里尾巴只有一条，会被摊薄。
    ///
    /// 按资源算就没这个问题：网关的总占用除以并发上限、GPU 的总占用除以 1
    /// （它是串行的），两者可以重叠，所以取较大的那个。
    pub fn project_secs(&self, sampled: usize, total: usize, gateway_concurrency: usize) -> f64 {
        if sampled == 0 {
            return 0.0;
        }
        let scale = total as f64 / sampled as f64;
        let gw = self.recognize_ms as f64 * scale / gateway_concurrency.max(1) as f64 / 1000.0;
        let gpu = self.embed_ms as f64 * scale / 1000.0;
        gw.max(gpu) + self.download_ms as f64 * scale / 1000.0
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
    /// 已经识别过的描述从哪儿来。传 None 就是每张图都重新识别一遍。
    pub cache: Option<&'a dyn Descriptions>,
    /// 要不要**逐张**算图片向量。默认不要。
    ///
    /// 一期 1871 张图对 358 条候选，逐图算占掉向量化 GPU 时间的八成五，
    /// 而它现在**没有任何读取路径**：合并用的是融合向量（跨账号转载时平台会
    /// 重新编码，图哈希对不上，所以合并本来就只能走融合向量 + Jev），
    /// 知识库检索用的也是融合向量，`kb::vectors` 那张 `images` 表全仓没人写也没人查。
    ///
    /// 开关留着不是为了以后「可能有用」——是为了做以图搜图那天，
    /// 打开它就有数据，不用再改这一段。
    pub image_vectors: bool,
    /// 每条做完报一次（已完成, 总数）。编排层拿它写进度给页面看
    pub progress: Option<&'a dyn Fn(usize, usize)>,
}

/// 已经识别过的图从哪儿取。预取轮把描述写进库，正式轮靠它把识别整段跳过。
///
/// **命中的判据是「这条候选的每一张图都有描述」**，不是「有几条描述」：
/// 缺一张就得重识别，否则判断那一步会拿着一份不全的画面去下结论，
/// 而台账上看不出来少了什么。
pub trait Descriptions {
    /// 这条候选在当前提示词版本下的全部描述。没有就给空。
    fn get(&self, candidate_key: &str) -> Vec<MediaDescription>;
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
                    .filter(|x| c.ignore_window() || in_window(x, from, to))
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
    tracing::info!(候选 = candidates.len(), 图 = urls.len(), "采集：开始下载");
    let report = deps.downloader.fetch_all(&urls).await;
    let download_ms = t_dl.elapsed().as_millis();
    tracing::info!(
        成功 = report.ok.len(),
        失败 = report.failed.len(),
        秒 = download_ms / 1000,
        "采集：下载完成，开始逐条识别"
    );
    let by_url: HashMap<String, _> = report
        .ok
        .iter()
        .map(|d| (d.url.clone(), d.clone()))
        .collect();
    let failed_urls: std::collections::HashSet<&str> =
        report.failed.iter().map(|f| f.url.as_str()).collect();

    let conc = deps.concurrency.max(1);
    let total = candidates.len();
    // **每条完成时报一次数。**
    //
    // 不报的话这一步就是个四十分钟的黑盒：线上第一次真跑时，我只能靠数 blobs
    // 文件、翻 TCP 连接、看 CPU 时间去猜它在下图、在识别、还是已经挂了，
    // 猜了半小时也没敢下结论。主编早上看着进度条不动时，要能一眼分得出
    // 「在走但慢」和「卡死了」——这两件事的处置完全不同。
    let done = std::sync::atomic::AtomicUsize::new(0);
    // 下面那个 async move 块会把捕获的东西移进去，所以先各取一份引用
    let (done, by_url, failed_urls) = (&done, &by_url, &failed_urls);
    let tasks = candidates.into_iter().map(|c| async move {
        let key = c.candidate_key.clone();
        let t = std::time::Instant::now();
        let p = prepare_one(c, by_url, failed_urls, deps).await;
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if let Some(f) = deps.progress {
            f(n, total);
        }
        tracing::info!(
            进度 = format!("{n}/{total}"),
            候选 = %key,
            图 = p.descriptions.len(),
            复用 = p.reused,
            秒 = t.elapsed().as_secs(),
            "采集：一条完成"
        );
        p
    });
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

/// 一条候选自己最多同时打几个识别请求。全局信号量是 8，这里只占一半——
/// 一个二十图的轮播不该把闸门占满让别的候选全排队。
const MAX_PARALLEL_BATCHES: usize = 4;

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
                            b64: recognize::b64(&bytes),
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

        // 先问缓存：预取轮 01:40 已经识别过的，正式轮不必再走一趟网关。
        // 这是「整轮 ≤45 分钟」的主要来源——识别是四十分钟里最大的一块。
        let hit = deps
            .cache
            .and_then(|cache| covered(cache, &c.candidate_key, &refs));
        if let Some(cached) = hit {
            let t_emb = std::time::Instant::now();
            // 只算融合向量，不再逐图算：这条候选的图上一轮已经算过了。
            // （等 11 选图包要用图向量时，这里要改成**从库里读回来**，
            // 而不是改回重算——重算的那三秒多是整轮 GPU 时间的大头。）
            let (fused, _) = embed_for(&c, &refs, &cached, deps.vector, false).await;
            return Prepared {
                candidate: c,
                descriptions: cached,
                fused,
                image_vectors: vec![],
                failed_media,
                reused: true,
                // 没走网关，占用就是 0：把它算进去会让「网关有效并发」虚高
                recognize_ms: 0,
                embed_ms: t_emb.elapsed().as_millis(),
            };
        }

        // 识别：按批走；某一批失败只影响那一批，不牵连整条候选。
        //
        // **批与批之间也要并发。** 这里原本是串行的，M1 实测暴露出来：网关的有效
        // 并发只有 3.5（上限 8），GPU 占用率才 21%——两边都闲着，墙钟却下不来。
        // 原因是候选之间的图数很不均匀，一个二十图的轮播要连打四次，到了尾部
        // 它独占墙钟，其他槽位全空着。批级并发让这条候选自己就能把闸门填上。
        //
        // 每条候选自己最多占 `MAX_PARALLEL_BATCHES` 个，不用满全局的 8：
        // 留一半给别的候选，免得一个大轮播把闸门占满、其他候选全卡在信号量上。
        let t_rec = std::time::Instant::now();
        let batches: Vec<&[recognize::ImageRef]> =
            refs.chunks(recognize::MAX_IMAGES_PER_CALL).collect();
        let done: Vec<_> = futures::StreamExt::collect::<Vec<_>>(futures::StreamExt::buffered(
            futures::stream::iter(
                batches
                    .iter()
                    .map(|b| recognize::recognize_batch(deps.model, &c.text, b)),
            ),
            MAX_PARALLEL_BATCHES,
        ))
        .await;
        let mut descriptions = Vec::new();
        for (batch, r) in batches.iter().zip(done) {
            match r {
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
        let (fused, image_vectors) =
            embed_for(&c, &refs, &descriptions, deps.vector, deps.image_vectors).await;
        Prepared {
            candidate: c,
            descriptions,
            fused,
            image_vectors,
            failed_media,
            reused: false,
            recognize_ms,
            embed_ms: t_emb.elapsed().as_millis(),
        }
    }
}

/// 缓存里这条候选的描述够不够用。**每一张图都要有**，少一张就算没命中。
///
/// 按 blake3 比而不是按数量比：贴文被编辑过、换了图的时候数量可能还一样，
/// 而那正是最该重识别的情形。
fn covered(
    cache: &dyn Descriptions,
    candidate_key: &str,
    refs: &[recognize::ImageRef],
) -> Option<Vec<MediaDescription>> {
    if refs.is_empty() {
        // 没有图就没什么可省的，照常走（纯文本那条路也在下面）
        return None;
    }
    let cached = cache.get(candidate_key);
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let d = cached.iter().find(|d| d.blake3 == r.blake3)?;
        // 描述里的「第几张图」要与这一轮的图序一致：判断的依据指的就是它
        let mut d = d.clone();
        d.ordinal = r.ordinal;
        out.push(d);
    }
    Some(out)
}

/// 算向量。`with_images` 为 false 时只算融合那一条。
///
/// GPU 是全进程串行的，逐图那几条是整轮 GPU 时间的大头；命中缓存时
/// 那些图上一轮已经算过，再算一遍纯属白占卡。
async fn embed_for(
    c: &Candidate,
    refs: &[recognize::ImageRef],
    descriptions: &[MediaDescription],
    vector: &VectorClient,
    with_images: bool,
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
    if with_images {
        inputs.extend(refs.iter().map(|r| EmbedInput::Image(r.b64.clone())));
    }

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
    fn ignore_window(&self) -> bool;
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
    fn ignore_window(&self) -> bool {
        Collector::ignore_window(self)
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

    /// 点名要看的链接：取到的都要
    struct Named(Fake);

    impl Collector for Named {
        fn sweep_key(&self) -> String {
            self.0.key.into()
        }
        fn platform(&self) -> &'static str {
            "instagram"
        }
        fn ignore_window(&self) -> bool {
            true
        }
        async fn collect(&self, a: Timestamp, b: Timestamp) -> Outcome {
            self.0.collect(a, b).await
        }
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
    async fn 点名的链接不按窗口筛_但照样只取图文() {
        let from = ts("2026-09-22T00:00:00Z");
        let to = ts("2026-09-23T00:00:00Z");
        let c = Named(Fake {
            key: "van-links",
            required: false,
            out: std::sync::Mutex::new(Some(ok(
                vec![
                    cand("old", Some("2026-09-15T02:00:00Z"), &[MediaKind::Photo]), // 窗口外，照收
                    cand(
                        "vid",
                        Some("2026-09-15T02:00:00Z"),
                        &[MediaKind::Photo, MediaKind::Video],
                    ), // 夹视频，照样去掉
                ],
                2,
            ))),
        });
        let (cands, sweeps, _) = collect_all(&[&c], from, to, true).await;
        assert_eq!(cands.len(), 1, "{cands:?}");
        assert_eq!(sweeps[0].in_window, 1);
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
            reused: false,
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
            reused: false,
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
            reused: false,
            recognize_ms: 0,
            embed_ms: 0,
        };
        assert!(!failed.image_seen(), "有下载失败的就不算读到实图");
    }

    struct Cache(Vec<MediaDescription>);
    impl Descriptions for Cache {
        fn get(&self, _: &str) -> Vec<MediaDescription> {
            self.0.clone()
        }
    }

    fn img(b3: &str, ordinal: u16) -> recognize::ImageRef {
        recognize::ImageRef {
            blake3: b3.into(),
            b64: "x".into(),
            ordinal,
        }
    }

    fn cached(b3: &str, ordinal: u16) -> MediaDescription {
        MediaDescription {
            blake3: b3.into(),
            ordinal,
            matches_text: String::new(),
            content: format!("{b3} 的画面"),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: "recognize/v1".into(),
        }
    }

    #[test]
    fn 每张图都有描述才算命中缓存() {
        let refs = [img("b0", 0), img("b1", 1)];
        let full = Cache(vec![cached("b0", 0), cached("b1", 1)]);
        let got = covered(&full, "k1", &refs).expect("两张都有就该命中");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].content, "b0 的画面");

        // 少一张就得重识别：拿一份不全的画面去判断，台账上看不出来少了什么
        let half = Cache(vec![cached("b0", 0)]);
        assert!(covered(&half, "k1", &refs).is_none());
        assert!(covered(&Cache(vec![]), "k1", &refs).is_none());
    }

    #[test]
    fn 换了图就不算命中() {
        // 贴文被编辑过、换掉一张图时数量可能还一样，而那正是最该重识别的情形
        let refs = [img("b0", 0), img("新图", 1)];
        let old = Cache(vec![cached("b0", 0), cached("b1", 1)]);
        assert!(covered(&old, "k1", &refs).is_none());
    }

    #[test]
    fn 缓存里的图序按这一轮的来() {
        // 描述里的「第几张图」是判断的依据，指错了依据就指向别人的画面
        let refs = [img("b1", 0), img("b0", 1)];
        let c = Cache(vec![cached("b0", 7), cached("b1", 9)]);
        let got = covered(&c, "k1", &refs).unwrap();
        assert_eq!((got[0].blake3.as_str(), got[0].ordinal), ("b1", 0));
        assert_eq!((got[1].blake3.as_str(), got[1].ordinal), ("b0", 1));
    }

    #[test]
    fn 没有图就没什么可省的() {
        let c = Cache(vec![cached("b0", 0)]);
        assert!(covered(&c, "k1", &[]).is_none());
    }

    /// 照着请求里的条数回同样多条向量，顺带把每次的条数记下来
    struct CountEcho(std::sync::Arc<std::sync::Mutex<Vec<usize>>>);

    impl wiremock::Respond for CountEcho {
        fn respond(&self, req: &wiremock::Request) -> wiremock::ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            let n = body
                .get("input")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            self.0.lock().unwrap().push(n);
            let data: Vec<_> = (0..n)
                .map(|_| serde_json::json!({"embedding": [0.1, 0.2]}))
                .collect();
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data}))
        }
    }

    #[tokio::test]
    async fn 命中缓存时既不识别也不逐图算向量() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // 图片服务：两张不同内容的图
        let imgs = MockServer::start().await;
        for (p, body) in [("/a.jpg", b"IMG-A".to_vec()), ("/b.jpg", b"IMG-B".to_vec())] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                .mount(&imgs)
                .await;
        }
        // 模型网关：**一次都不该被调到**
        let model_srv = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&model_srv)
            .await;
        // 向量服务：记下每次送了几条
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let vec_srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(CountEcho(seen.clone()))
            .mount(&vec_srv)
            .await;

        let dir = std::env::temp_dir().join(format!("csw-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let downloader = crate::download::Downloader::new(crate::download::DownloadConfig {
            dir,
            width: 768,
            concurrency: 2,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
            max_bytes: crate::download::DEFAULT_MAX_BYTES,
        })
        .unwrap();
        let model = ModelClient::new(csw_collector_core::model::ModelConfig {
            base_url: model_srv.uri(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 1,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap();
        let vector = VectorClient::new(csw_collector_core::vector::VectorConfig {
            base_url: vec_srv.uri(),
            ..Default::default()
        })
        .unwrap();

        let mut c = cand("p1", Some("2026-09-18T01:00:00Z"), &[]);
        c.media = vec![
            MediaRef {
                source_hash: "s0".into(),
                kind: MediaKind::Photo,
                url: format!("{}/a.jpg", imgs.uri()),
                blake3: None,
                ordinal: 0,
            },
            MediaRef {
                source_hash: "s1".into(),
                kind: MediaKind::Photo,
                url: format!("{}/b.jpg", imgs.uri()),
                blake3: None,
                ordinal: 1,
            },
        ];
        // 缓存里放的就是这两张图上一轮的描述（哈希按内容算，与下载器一致）
        let cache = Cache(vec![
            cached(&blake3::hash(b"IMG-A").to_hex(), 0),
            cached(&blake3::hash(b"IMG-B").to_hex(), 1),
        ]);

        let (prepared, stats) = prepare(
            vec![c],
            &Deps {
                downloader: &downloader,
                model: &model,
                vector: &vector,
                image_only: true,
                concurrency: 2,
                cache: Some(&cache),
                image_vectors: false,
                progress: None,
            },
        )
        .await;

        assert_eq!(prepared.len(), 1);
        let p = &prepared[0];
        assert!(p.reused, "两张图都有描述就该命中");
        assert_eq!(p.descriptions.len(), 2);
        assert!(p.image_seen(), "复用回来的也算读到了实图");
        assert!(p.failed_media.is_empty());
        // 没走网关，占用就该是 0：算进去会让「网关有效并发」虚高
        assert_eq!(p.recognize_ms, 0);
        assert_eq!(stats.recognize_ms, 0);
        // 只算了融合那一条。逐图那几条是整轮 GPU 时间的大头，上一轮已经算过了
        assert_eq!(*seen.lock().unwrap(), vec![1]);
        assert!(p.fused.is_some());
        assert!(p.image_vectors.is_empty());
    }

    /// 没命中缓存、开关关着时，**也只算融合那一条**。
    ///
    /// 逐图向量占向量化 GPU 时间的八成五，而它现在没有任何读取路径
    /// （合并与检索用的都是融合向量）。开关开着才逐图算，
    /// 这条把「默认不算」钉死——它是一期省下十几分钟 GPU 的地方。
    #[tokio::test]
    async fn 没命中缓存时逐图向量也要看开关() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let imgs = MockServer::start().await;
        for (p, body) in [("/a.jpg", b"IMG-A".to_vec()), ("/b.jpg", b"IMG-B".to_vec())] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                .mount(&imgs)
                .await;
        }
        // 识别照常走（没有缓存），回一份两条描述的结果
        let model_srv = MockServer::start().await;
        Mock::given(method("POST"))
            // Responses 接口的形状：output[].content[].output_text
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "output": [{"content": [{"type": "output_text", "text":
                    serde_json::to_string(&serde_json::json!({
                        "images": [
                            {"index": 1, "kind": "product", "content": "图一",
                             "matches_text": "对得上", "missing_from_text": "",
                             "usable_as_figure": true},
                            {"index": 2, "kind": "detail", "content": "图二",
                             "matches_text": "对得上", "missing_from_text": "",
                             "usable_as_figure": true}
                        ]
                    })).unwrap()
                }]}],
                "usage": {"input_tokens": 10, "output_tokens": 20}
            })))
            .mount(&model_srv)
            .await;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let vec_srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(CountEcho(seen.clone()))
            .mount(&vec_srv)
            .await;

        let dir = std::env::temp_dir().join(format!("csw-iv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let downloader = crate::download::Downloader::new(crate::download::DownloadConfig {
            dir,
            width: 768,
            concurrency: 2,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
            max_bytes: crate::download::DEFAULT_MAX_BYTES,
        })
        .unwrap();
        let model = ModelClient::new(csw_collector_core::model::ModelConfig {
            base_url: model_srv.uri(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 1,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap();
        let vector = VectorClient::new(csw_collector_core::vector::VectorConfig {
            base_url: vec_srv.uri(),
            ..Default::default()
        })
        .unwrap();

        let mut c = cand("p1", Some("2026-09-18T01:00:00Z"), &[]);
        c.media = vec![
            MediaRef {
                source_hash: "s0".into(),
                kind: MediaKind::Photo,
                url: format!("{}/a.jpg", imgs.uri()),
                blake3: None,
                ordinal: 0,
            },
            MediaRef {
                source_hash: "s1".into(),
                kind: MediaKind::Photo,
                url: format!("{}/b.jpg", imgs.uri()),
                blake3: None,
                ordinal: 1,
            },
        ];

        let deps = |image_vectors| Deps {
            downloader: &downloader,
            model: &model,
            vector: &vector,
            image_only: true,
            concurrency: 2,
            cache: None,
            image_vectors,
            progress: None,
        };

        // 数的是**总条数**不是批次数：客户端会按 batch_weight 自己分批，
        // 钉批次数等于把它的分批策略也钉死了，那不是这条测试要管的事
        let total = || -> usize { seen.lock().unwrap().iter().sum() };

        // 关着：只送融合那一条
        let (prepared, _) = prepare(vec![c.clone()], &deps(false)).await;
        assert_eq!(prepared[0].descriptions.len(), 2, "识别照常走");
        assert!(prepared[0].image_vectors.is_empty());
        assert_eq!(total(), 1, "只该送融合那一条");

        // 开着：融合 + 两张图 = 3 条
        seen.lock().unwrap().clear();
        let (prepared, _) = prepare(vec![c], &deps(true)).await;
        assert_eq!(prepared[0].image_vectors.len(), 2);
        assert_eq!(total(), 3, "融合一条 + 每图一条");
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
}
