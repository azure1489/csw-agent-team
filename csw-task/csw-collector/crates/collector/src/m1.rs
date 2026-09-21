//! M1 验收：对真数据跑一遍「采集媒体信息」，把代码算出来的数摆出来。
//!
//! 分两段跑，因为两段的成本差三个数量级：
//! - **取数**只调 csw 接口，几秒钟、不花钱 → 默认跑全窗口。
//! - **下载 + 识别 + 向量化**要过模型网关与 GPU，一期约 42 分钟
//!   → 默认只跑 `--prepare N` 条（N 默认 8）验证四段真能串起来；要全量得**显式加 `--full`**
//!   ——这么贵的动作不该靠一个默认值触发。
//!
//! 计划里写的「只图文 305」是更早一次测量的数，已经对不上了（9/22 实测 358）。
//! **这条命令的输出就是当日的验收基准**，不照抄旧数字。

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use csw_collector_core::model::{ModelClient, ModelConfig};
use csw_collector_core::types::{MediaKind, Timestamp};
use csw_collector_core::vector::{VectorClient, VectorConfig};
use csw_collector_core::{Config, Secrets};
use csw_collector_harvest::collector::{CswWindow, VanLinks};
use csw_collector_harvest::csw::{CswClient, CswConfig, WINDOW_LOOKBACK_DAYS};
use csw_collector_harvest::download::{DownloadConfig, Downloader};
use csw_collector_harvest::pipeline::{self, CollectorDyn, Deps};

pub struct Opts {
    pub from: String,
    pub to: String,
    /// 对前 N 条跑下载识别向量化。0 = 只取数，不往下走。
    pub prepare: usize,
    /// 全量跑下载识别向量化。**要花模型与 GPU，一期约 42 分钟**，所以要显式要。
    pub full: bool,
    /// Van 本期补的短码
    pub van_links: Vec<String>,
}

pub async fn run(cfg: &Config, secrets: &Secrets, o: Opts) -> Result<()> {
    let from: Timestamp = o
        .from
        .parse()
        .with_context(|| format!("窗口起点 {}", o.from))?;
    let to: Timestamp = o.to.parse().with_context(|| format!("窗口终点 {}", o.to))?;
    anyhow::ensure!(to > from, "窗口终点要晚于起点");

    let csw = Arc::new(CswClient::new(CswConfig {
        base_url: cfg.csw.base_url.clone(),
        api_key: secrets.csw_api_key.clone(),
        page_size: cfg.csw.window_page,
        timeout: Duration::from_secs(cfg.csw.timeout_secs),
    })?);

    println!("窗口（按首次入库时间，左闭右开）{from} ~ {to}");
    println!("取候选时的发布时间窗口往前放宽 {WINDOW_LOOKBACK_DAYS} 天——接口只按发布时间过滤\n");

    let window = CswWindow {
        client: csw.clone(),
        lookback_days: WINDOW_LOOKBACK_DAYS,
    };
    let van = VanLinks {
        client: csw.clone(),
        short_codes: o.van_links.clone(),
    };
    let collectors: Vec<&dyn CollectorDyn> = vec![&window, &van];

    let t0 = Instant::now();
    let (cands, sweeps, blocked) = // 只取图文是总方案定死的，不是开关
    pipeline::collect_all(&collectors, from, to, true).await;
    let take_ms = t0.elapsed().as_millis();

    // 表头直接写成一行：CJK 在等宽终端里本来也对不齐 {:<14} 那套宽度
    println!("采集器          接口返回    去重后  窗口内图文  结果      找了什么");
    for s in &sweeps {
        println!(
            "{:<14}{:>8}{:>10}{:>10}  {:<8}{}",
            s.sweep_key, s.found, s.fetched_unique, s.in_window, s.result, s.query
        );
        if !s.error.is_empty() {
            println!("               └ {}", s.error);
        }
    }
    if let Some(b) = blocked {
        anyhow::bail!("{b}");
    }
    let photos: usize = cands
        .iter()
        .map(|c| {
            c.media
                .iter()
                .filter(|m| m.kind == MediaKind::Photo)
                .count()
        })
        .sum();
    println!(
        "\n合并去重后 {} 条候选、{} 张图，取数用时 {} ms",
        cands.len(),
        photos,
        take_ms
    );

    // 按账号看看分布，主编要的是「来源有没有过于集中」
    let mut by_acct: std::collections::HashMap<&str, usize> = Default::default();
    for c in &cands {
        *by_acct.entry(c.account.as_str()).or_default() += 1;
    }
    let mut top: Vec<_> = by_acct.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!(
        "来源账号 {} 个，最多的五个：{}",
        top.len(),
        top.iter()
            .take(5)
            .map(|(a, n)| format!("{a} {n}"))
            .collect::<Vec<_>>()
            .join("、")
    );

    // 下面这段要花模型与 GPU 的钱，所以默认只抽几条。全量要显式加 --full。
    let n = if o.full {
        cands.len()
    } else {
        o.prepare.min(cands.len())
    };
    if n == 0 {
        println!(
            "\n（只跑了取数。加 --prepare N 抽 N 条走完四段；要全量加 --full，一期约 42 分钟）"
        );
        return Ok(());
    }
    println!("\n对前 {n} 条跑下载 → 识别 → 向量化 …");
    let downloader = Downloader::new(DownloadConfig {
        dir: cfg.blob_dir(),
        width: cfg.csw.thumb_width,
        concurrency: cfg.limits.download_concurrency,
        timeout: Duration::from_secs(60),
        max_attempts: 3,
    })?;
    let model = ModelClient::new(ModelConfig {
        base_url: cfg.model.base_url.clone(),
        api_key: secrets.sub2api_key.clone(),
        model: cfg.model.model.clone(),
        fallback_model: cfg.model.fallback_model.clone(),
        concurrency: cfg.model.concurrency,
        timeout: Duration::from_secs(cfg.model.timeout_secs),
        max_attempts: 3,
    })?;
    let vector = VectorClient::new(VectorConfig {
        base_url: cfg.vector.base_url.clone(),
        timeout: Duration::from_secs(cfg.vector.timeout_secs),
        ..Default::default()
    })?;
    if !vector.healthy().await {
        println!(
            "  提醒：向量服务 {} 不可达，这一段会全部失败",
            cfg.vector.base_url
        );
    }

    let t1 = Instant::now();
    let (prepared, stats) = pipeline::prepare(
        cands.into_iter().take(n).collect(),
        &Deps {
            downloader: &downloader,
            model: &model,
            vector: &vector,
            image_only: true,
            // 闸门在客户端里（模型信号量 8、GPU 互斥锁 1），这里只是别让它们闲着
            concurrency: cfg.model.concurrency,
        },
    )
    .await;
    let prep_s = t1.elapsed().as_secs_f64();

    let seen = prepared.iter().filter(|p| p.image_seen()).count();
    let descs: usize = prepared.iter().map(|p| p.descriptions.len()).sum();
    let vecs: usize = prepared
        .iter()
        .map(|p| p.image_vectors.len() + usize::from(p.fused.is_some()))
        .sum();
    let want_imgs: usize = prepared
        .iter()
        .map(|p| {
            p.candidate
                .media
                .iter()
                .filter(|m| m.kind == MediaKind::Photo)
                .count()
        })
        .sum();

    println!("\nM1 验收数字");
    println!("  候选              {n}");
    println!("  图片              {want_imgs}");
    println!(
        "  识别覆盖          {descs}/{want_imgs}（{:.0}%）",
        pct(descs, want_imgs)
    );
    println!("  读到实图的候选    {seen}/{n}（{:.0}%）", pct(seen, n));
    println!("  向量              {vecs}");
    println!(
        "  用时              {prep_s:.1}s，每条 {:.1}s、每张图 {:.1}s",
        prep_s / n.max(1) as f64,
        prep_s / want_imgs.max(1) as f64
    );
    println!(
        "  外推一期 {} 条    {:.1} 分钟",
        358,
        prep_s / n.max(1) as f64 * 358.0 / 60.0
    );

    // 把墙钟拆开：网关与 GPU 是两个资源，混成一个总数就看不出该往哪儿加通道
    println!("\n耗时都花在哪");
    println!(
        "  下载          {:.1}s（统一做，不分摊到候选）",
        stats.download_ms as f64 / 1000.0
    );
    println!("  识别+向量墙钟 {:.1}s", stats.wall_ms as f64 / 1000.0);
    println!(
        "  网关占用合计  {:.1}s → 有效并发 {:.1}（上限 {}）",
        stats.recognize_ms as f64 / 1000.0,
        stats.gateway_concurrency(),
        cfg.model.concurrency
    );
    println!(
        "  GPU 占用合计  {:.1}s → 占用率 {:.0}%（GPU 全局串行，逼近 100% 就是卡在显卡上）",
        stats.embed_ms as f64 / 1000.0,
        stats.gpu_busy() * 100.0
    );

    let bad: Vec<_> = prepared
        .iter()
        .filter(|p| !p.failed_media.is_empty())
        .collect();
    if !bad.is_empty() {
        println!(
            "\n有图没处理成功的 {} 条（它们会落待核，不算淘汰）：",
            bad.len()
        );
        for p in bad.iter().take(5) {
            println!(
                "  {} — {}",
                p.candidate.candidate_key,
                p.failed_media.join("；")
            );
        }
    }
    Ok(())
}

fn pct(a: usize, b: usize) -> f64 {
    if b == 0 {
        100.0
    } else {
        a as f64 * 100.0 / b as f64
    }
}
