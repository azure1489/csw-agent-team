//! 05「配图与素材核」与 11「小红书选图包」：**条目取单条**。
//!
//! # 与 01 的分界
//!
//! 这两个阶段**不走合并、对照、判断、深核**。它们的活是「把这条资讯的原图备齐、
//! 说清每张图里有什么」，不是「这条值不值得写」——那件事 01 已经做过了，
//! 而且是 Van 拍的板。再判一遍只会给出第二个可能不一致的答案。
//!
//! # 为什么重新去取一次贴文
//!
//! 01 那一步下的是 w_768 缩略图。设计师拿 768 宽的图去做公众号头图，
//! 放大后糊得一眼就能看出来。所以这里**按 `source_id` 重新取一次单条**，
//! 拿到当下的 `mediaList` 再下原图（`width = 0`）。
//!
//! 顺带解决另一件事：贴文可能在这几小时里被编辑过（换图、加图）。
//! 拿当下的那一版，配图才跟得上正文。

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use csw_collector_core::rounds::{self, Round};
use csw_collector_core::types::{Candidate, MediaKind, StepCode, StepStatus};
use csw_collector_core::{Config, ledger, media};
use csw_collector_deliver::index::Meta;
use csw_collector_deliver::material::{self, HiresShot, HiresSummary, ItemShots, Shot};
use csw_collector_deliver::pack;
use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::TaskDetail;
use csw_collector_harvest::download::{DownloadConfig, Downloader};
use csw_collector_harvest::hires::{HiresClient, HiresItem};
use csw_collector_harvest::recognize;
use sha2::{Digest, Sha256};

use super::hires_match;

/// 05 的阶段代号
pub const STAGE_MATERIAL: &str = "material";
/// 11 的阶段代号
pub const STAGE_XHS_PICK: &str = "xhs_pick";

/// 原图下载器。**与 01 那个分开建**：宽度不同，共用一个就得改全局配置。
pub(crate) fn original_downloader(cfg: &Config) -> Result<Downloader> {
    Downloader::new(DownloadConfig {
        dir: cfg.blob_dir(),
        // 0 = 原图。设计师要拿它做头图
        width: 0,
        concurrency: cfg.limits.download_concurrency,
        timeout: std::time::Duration::from_secs(120),
        max_attempts: 3,
        max_bytes: csw_collector_harvest::download::DEFAULT_MAX_BYTES,
    })
}

/// 跑一轮 05：一个条目，三个图位 + 素材核对。
pub async fn run_material(
    conn: &Connection,
    round: &Round,
    detail: &TaskDetail,
    cfg: &Config,
    svc: &super::services::Services,
) -> Result<pack::Built> {
    let item_key = detail.task.item_key.trim();
    anyhow::ensure!(
        !item_key.is_empty(),
        "05 是逐条派工的，派工单上却没有条目键——不猜是哪一条"
    );
    let step = rounds::begin_step(conn, round.id, StepCode::Harvest, item_key)?;
    let item = match shots_for(conn, cfg, svc, item_key, &item_title(detail, item_key)).await {
        Ok(i) => i,
        Err(e) => {
            rounds::end_step(
                conn,
                step.id,
                StepStatus::Failed,
                &serde_json::json!({}),
                &format!("{e:#}"),
            )?;
            return Err(e);
        }
    };
    let mut item = item;
    // 重新派工（退修）时，派工单里写着这次要补什么图。工作台只会在原帖已有的图里重排图位，
    // 不会找新图源：要求原文进包，并记一条决定级缺口，交给人判断要求是否满足
    //（10-02 r60 Van 退修：LFE 要翻转后侧视、MINIMAL WORKS 要整顶全貌，v2 却写「缺口 0 条」直通）
    let note = dispatch_note(detail);
    let redo = round.target_version > 1 && !note.trim().is_empty();
    if redo {
        item.gaps.push(format!(
            "本次派工的修订要求工作台没有自动核对：只在原帖已有的 {} 张图里重排图位，没有新增图源。\
             要求里的功能图若不在下面「素材核对」的逐张说明里，就是原帖没有，需人工补采或换图源（不重绘、不拼图）",
            item.shots.len()
        ));
    }
    let picked = material::pick_figures(&item.shots, material::FIGURE_SLOTS);
    rounds::end_step(
        conn,
        step.id,
        StepStatus::Succeeded,
        &serde_json::json!({
            "图": item.shots.len(),
            "挑进图位": picked.len(),
            "缺口": item.gaps.len(),
        }),
        "",
    )?;

    let mut body = material::material_body(&item, &picked);
    if redo {
        body.push_str("\n## 本次派工要求（原文）\n\n");
        for l in note.trim().lines() {
            body.push_str(&format!("> {l}\n"));
        }
        body.push('\n');
    }
    let entries = material::assemble_material(
        meta_for(
            round,
            detail,
            "05-配图与素材核",
            item_key,
            checks_material(&item, picked.len()),
        ),
        body,
        &item,
    );
    build(
        conn,
        round,
        cfg,
        detail.task.id,
        item_key,
        &format!(
            "配图与素材核_情报收集员_r{}_{}_v{}",
            round.run_id.unwrap_or(0),
            file_part(item_key),
            // 按目标版本命名：写死 v1 时退修版会把本地的上一版覆盖掉
            round.target_version.max(1)
        ),
        &entries,
    )
}

/// 跑一轮 11：整期一包，按资讯分组的原图。
///
/// 条目清单从引擎来（`GET /runs/:id/items`），**只取 Van 批准可写的那几条**：
/// 把「研究员建议」当成批准，会多做一批没人要的图。
pub async fn run_xhs_pick(
    conn: &Connection,
    round: &Round,
    detail: &TaskDetail,
    cfg: &Config,
    svc: &super::services::Services,
    engine: &EngineClient,
) -> Result<pack::Built> {
    let run_id = round.run_id.context("11 要知道是哪一期")?;
    let step = rounds::begin_step(conn, round.id, StepCode::Harvest, &run_id.to_string())?;

    let wanted: Vec<(String, String)> = if detail.task.item_key.trim().is_empty() {
        let items = engine.run_items(run_id).await.context("取这一期的条目")?;
        items
            .items
            .iter()
            .filter(|i| i.approved())
            .map(|i| (i.item_key.clone(), i.title.clone()))
            .collect()
    } else {
        // 主编也可能只派一条过来补图
        let k = detail.task.item_key.trim().to_string();
        let t = item_title(detail, &k);
        vec![(k, t)]
    };
    if wanted.is_empty() {
        rounds::end_step(
            conn,
            step.id,
            StepStatus::Failed,
            &serde_json::json!({}),
            "这一期没有批准可写的条目",
        )?;
        anyhow::bail!("这一期一条批准可写的条目都没有，选图包无从做起");
    }

    let mut items = Vec::new();
    let mut failed = Vec::new();
    for (key, title) in &wanted {
        match shots_for(conn, cfg, svc, key, title).await {
            Ok(mut i) => {
                // 原图一张两三百 KB，不设上限会把 48 MiB 的包撑爆，
                // 而撑爆是在**提交那一刻**才发现的
                if i.shots.len() > material::XHS_MAX_PER_ITEM {
                    let dropped = i.shots.len() - material::XHS_MAX_PER_ITEM;
                    i.shots.truncate(material::XHS_MAX_PER_ITEM);
                    i.gaps.push(format!(
                        "这条共 {} 张图，只放了前 {} 张（包体上限）；其余 {dropped} 张要的话单独说",
                        dropped + material::XHS_MAX_PER_ITEM,
                        material::XHS_MAX_PER_ITEM
                    ));
                }
                items.push(i);
            }
            // 一条取不到不该让整包交不出来——写进缺口，其余照做
            Err(e) => failed.push(format!("{key}：{e:#}")),
        }
    }
    let status = if failed.is_empty() {
        StepStatus::Succeeded
    } else {
        StepStatus::Partial
    };
    rounds::end_step(
        conn,
        step.id,
        status,
        &serde_json::json!({
            "条目": items.len(),
            "图": items.iter().map(|i| i.shots.len()).sum::<usize>(),
            "取不到": failed.len(),
        }),
        &failed.join("；"),
    )?;
    anyhow::ensure!(!items.is_empty(), "一条都没取到：{}", failed.join("；"));

    let mut body = material::pick_body(&items);
    if !failed.is_empty() {
        // 缺口写在最前面：藏在末尾等于没写
        body = format!(
            "> **有 {} 条没取到图**\n>\n{}\n\n{body}",
            failed.len(),
            failed
                .iter()
                .map(|f| format!("> - {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let entries = material::assemble_pick(
        meta_for(
            round,
            detail,
            "11-小红书选图包",
            "",
            checks_pick(&items, &failed),
        ),
        body,
        &items,
    );
    build(
        conn,
        round,
        cfg,
        detail.task.id,
        "",
        &format!(
            "小红书选图包_情报收集员_r{run_id}_v{}",
            round.target_version.max(1)
        ),
        &entries,
    )
}

/// 取这一条的原图与识别结果。
async fn shots_for(
    conn: &Connection,
    cfg: &Config,
    svc: &super::services::Services,
    item_key: &str,
    title: &str,
) -> Result<ItemShots> {
    let local = ledger::get_candidate(conn, item_key)?;
    let short = shortcode(local.as_ref(), item_key)?;
    // 重新取一次单条：贴文这几小时里可能被编辑过（换图、加图），
    // 拿当下那一版，配图才跟得上正文
    let raw = svc
        .csw
        .post(&short)
        .await
        .with_context(|| format!("取单条 {short}"))?;
    let c = csw_collector_harvest::csw::to_candidate(&raw, "csw-post");
    ledger::upsert_candidate(conn, &c)?;

    let photos: Vec<_> = c
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Photo)
        .cloned()
        .collect();
    let mut gaps = Vec::new();
    if photos.is_empty() {
        gaps.push("这条贴文一张图都没有".to_string());
    }

    let dl = original_downloader(cfg)?;
    let urls: Vec<String> = photos.iter().map(|m| m.url.clone()).collect();
    let report = dl.fetch_all(&urls).await;
    let by_url: HashMap<&str, _> = report.ok.iter().map(|d| (d.url.as_str(), d)).collect();
    for f in &report.failed {
        gaps.push(format!("{}：下载失败 {}", f.url, f.error));
    }

    // 识别：按批走，与 01 同一套提示词——同一张图在两处给出不同的描述，
    // 主编对不上账
    let mut refs = Vec::new();
    let mut bytes_by_b3: HashMap<String, Vec<u8>> = HashMap::new();
    let mut shots = Vec::new();
    for m in &photos {
        let Some(d) = by_url.get(m.url.as_str()) else {
            continue;
        };
        match std::fs::read(&d.path) {
            Ok(bytes) => {
                refs.push(recognize::ImageRef {
                    blake3: d.blake3.clone(),
                    b64: recognize::b64(&bytes),
                    ordinal: m.ordinal,
                });
                bytes_by_b3.insert(d.blake3.clone(), bytes);
                shots.push(Shot {
                    blake3: d.blake3.clone(),
                    ordinal: m.ordinal,
                    url: m.url.clone(),
                    bytes: Vec::new(),
                    ext: ext_of(&m.url),
                    desc: None,
                    hires: None,
                });
            }
            Err(e) => gaps.push(format!("{}：读盘失败 {e}", m.url)),
        }
    }

    let mut descs = Vec::new();
    for batch in refs.chunks(recognize::MAX_IMAGES_PER_CALL) {
        match recognize::recognize_batch(&svc.model, &c.text, batch).await {
            Ok(mut d) => descs.append(&mut d),
            Err(e) => {
                tracing::warn!(条目 = %item_key, 原因 = %format!("{e:#}"), "这批图没识别成");
                gaps.extend(batch.iter().map(|b| format!("{}：识别失败", b.blake3)));
            }
        }
    }
    // 描述照样落库：05 识别过的图，明天 01 再遇到就不必重识别
    if let Err(e) = media::put_prepared(conn, &c.candidate_key, &c.media, &descs) {
        tracing::warn!(条目 = %item_key, 原因 = %format!("{e:#}"), "描述没落库");
    }

    for s in &mut shots {
        s.desc = descs.iter().find(|d| d.blake3 == s.blake3).cloned();
        // 取而不是拿走：同一张图在一条轮播里出现两次时（真有这种），
        // 拿走会让第二份变成一个零字节的文件塞进包里
        s.bytes = bytes_by_b3.get(&s.blake3).cloned().unwrap_or_default();
    }

    let mut item = ItemShots {
        item_key: item_key.to_string(),
        title: if title.trim().is_empty() {
            c.account.clone()
        } else {
            title.to_string()
        },
        source_url: c.url.clone(),
        account: c.account.clone(),
        posted_at: c.posted_at.map(|t| t.to_string()).unwrap_or_default(),
        ingested_at: c.ingested_at.map(|t| t.to_string()).unwrap_or_default(),
        shots,
        gaps,
        hires: None,
    };
    // 高清原图：来源库只有 640 档，Van 要的是原帖的图（10-02 r60）。按轮播序号对应、逐张画面校验
    if let Some(h) = svc.hires.as_ref() {
        let ordinals: Vec<u16> = photos.iter().map(|m| m.ordinal).collect();
        // `short` 是 csw 的贴文 id（数字），hires-service 要的是 Instagram 短码，从贴文链接里切
        //（10-05 r62 05 首跑：把数字 id 当短码送过去，服务按 base64 解成了一个 35 位的假 media_id，五条全 400）
        match ig_shortcode(&c) {
            Some(code) => attach_hires(cfg, h, &dl, &code, &ordinals, &mut item).await?,
            None => {
                let why = format!(
                    "贴文链接里切不出 Instagram 短码（{}），无法向 hires-service 取原图",
                    c.url
                );
                if cfg.hires.required {
                    anyhow::bail!("高清原图未取到：{why}");
                }
                item.gaps
                    .push(format!("高清原图未取到，包内是来源库 640 档：{why}"));
            }
        }
    }
    Ok(item)
}

/// 贴文的 Instagram 短码（`/p/<code>/`），给 hires-service 用。**不是** csw 的数字贴文 id。
fn ig_shortcode(c: &Candidate) -> Option<String> {
    from_url(&c.url)
        .and_then(|s| sane(&s))
        .filter(|s| !s.chars().all(|ch| ch.is_ascii_digit()))
}

/// 把 hires-service 取到的高清图换进 `item.shots`，每张记来历与校验结论。
///
/// `source_ordinals` 是来源库这条贴文**全部图片**的序号（按轮播顺序），不只是下载成功的那些：
/// 数量对不上说明帖子被编辑过，不按序号硬配。
///
/// 取不到时的处置看 `cfg.hires.required`：要求必有就报错（整条任务报失败，原因原样带出去，
/// 主编按它决定换 cookie 还是转人工）；不要求就交 640 图，自检与缺口明写。
async fn attach_hires(
    cfg: &Config,
    hires: &HiresClient,
    dl: &Downloader,
    shortcode: &str,
    source_ordinals: &[u16],
    item: &mut ItemShots,
) -> Result<()> {
    let total = source_ordinals.len();
    let mut summary = HiresSummary {
        service: hires.base_url().to_string(),
        shortcode: shortcode.to_string(),
        total,
        ..Default::default()
    };
    let bail_or_note = |item: &mut ItemShots,
                        mut summary: HiresSummary,
                        why: String|
     -> Result<()> {
        summary.note = why.clone();
        item.hires = Some(summary);
        if cfg.hires.required {
            anyhow::bail!(
                "高清原图未取到：{why}。工作台不交来源库 640 图冒充原图；请处理后重开派工（hires-service {}）",
                hires.base_url()
            );
        }
        item.gaps
            .push(format!("高清原图未取到，包内是来源库 640 档：{why}"));
        Ok(())
    };

    let res = match hires.media(shortcode).await {
        Ok(r) => r,
        Err(e) => return bail_or_note(item, summary, e.to_string()),
    };
    summary.pk = res.pk.clone();
    summary.cached = res.cached;
    let ph = res.photos();
    if let Err(why) = check_counts(source_ordinals, &ph) {
        return bail_or_note(item, summary, why);
    }

    // OSS 地址一次下完（原图下载器：不缩放、走守卫客户端）
    let urls: Vec<String> = ph
        .iter()
        .filter_map(|p| p.oss.as_ref().map(|o| o.url.clone()))
        .collect();
    let report = dl.fetch_all(&urls).await;
    let by_url = report.by_url();

    let mut problems = Vec::new();

    // 一、每张高清图读出来、算指纹（只解一次码）
    let mut his: Vec<Option<(Vec<u8>, hires_match::Prints)>> = Vec::with_capacity(ph.len());
    for (i, p) in ph.iter().enumerate() {
        let Some(oss) = p.oss.as_ref() else {
            problems.push(format!(
                "第 {} 张（{}）：hires-service 没转存成 OSS：{}",
                i + 1,
                p.file_key,
                p.error.clone().unwrap_or_else(|| "未说明原因".into())
            ));
            his.push(None);
            continue;
        };
        let Some(d) = by_url.get(oss.url.as_str()) else {
            let why = report
                .failed
                .iter()
                .find(|f| f.url == oss.url)
                .map(|f| f.error.clone())
                .unwrap_or_else(|| "未知".into());
            problems.push(format!(
                "第 {} 张（{}）：下载 OSS 高清图失败：{why}",
                i + 1,
                p.file_key
            ));
            his.push(None);
            continue;
        };
        let bytes = match std::fs::read(&d.path) {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("第 {} 张（{}）：读盘失败 {e}", i + 1, p.file_key));
                his.push(None);
                continue;
            }
        };
        match hires_match::prints(&bytes) {
            Ok(pr) => his.push(Some((bytes, pr))),
            Err(e) => {
                problems.push(format!("第 {} 张（{}）：{e}", i + 1, p.file_key));
                his.push(None);
            }
        }
    }
    // 二、来源库每张的指纹
    let los: Vec<Option<u64>> = item
        .shots
        .iter()
        .map(|s| hires_match::lo_print(&s.bytes).ok())
        .collect();

    // 三、按画面配对，不按列表位置。来源库返回的图片顺序可能与原帖轮播不同
    //（10-07 r65 circles_jp 16 张：来源库单条接口把原帖第 1 张排在第 5 位，按位置配 16 张全错）。
    // 同位置那张先试，不过再在没配上的里找最近的；都不过才算这张对不上
    let mut taken = vec![false; item.shots.len()];
    let mut pairs: Vec<(usize, usize, hires_match::Match)> = Vec::new();
    for (i, p) in ph.iter().enumerate() {
        let Some((_, pr)) = his[i].as_ref() else {
            continue;
        };
        let want = source_ordinals[i];
        let mut best: Option<(usize, hires_match::Match)> = None;
        for (j, s) in item.shots.iter().enumerate() {
            let (false, Some(lo)) = (taken[j], los[j]) else {
                continue;
            };
            let m = hires_match::compare(pr, lo);
            if !m.ok() {
                continue;
            }
            let better = match &best {
                None => true,
                Some((bj, bm)) => {
                    m.rank() < bm.rank()
                        || (m.rank() == bm.rank()
                            && s.ordinal == want
                            && item.shots[*bj].ordinal != want)
                }
            };
            if better {
                best = Some((j, m));
            }
        }
        match best {
            Some((j, m)) => {
                taken[j] = true;
                pairs.push((i, j, m));
            }
            None => {
                let same = item
                    .shots
                    .iter()
                    .position(|s| s.ordinal == want)
                    .and_then(|j| los[j].map(|lo| hires_match::compare(pr, lo).cn()))
                    .unwrap_or_else(|| "来源库同位置那张没下载成功或解不出".into());
                problems.push(format!(
                    "第 {} 张（{}）：来源库 {} 张里没有画面对得上的（同位置那张：{same}）",
                    i + 1,
                    p.file_key,
                    item.shots.len()
                ));
            }
        }
    }

    // 四、换进去，并把序号改成原帖轮播顺序（包里的图序以 Instagram 为准）
    let mut reordered = 0usize;
    for (i, j, m) in pairs {
        let p = ph[i];
        let Some((bytes, _)) = his[i].take() else {
            continue;
        };
        let Some(oss) = p.oss.as_ref() else { continue };
        let shot = &mut item.shots[j];
        let want = source_ordinals[i];
        let note = if shot.ordinal == want {
            m.cn()
        } else {
            reordered += 1;
            format!(
                "{}；来源库里排第 {} 张，与原帖顺序不同，按画面对上后改按原帖第 {} 张排",
                m.cn(),
                shot.ordinal + 1,
                i + 1
            )
        };
        let (w, h) = hires_match::dimensions(&bytes).unwrap_or((p.width, p.height));
        let (sw, sh) = hires_match::dimensions(&shot.bytes).unwrap_or((0, 0));
        let sha256: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        shot.hires = Some(HiresShot {
            file_key: p.file_key.clone(),
            source_url: p.source_url.clone(),
            oss_url: oss.url.clone(),
            width: w,
            height: h,
            bytes: bytes.len() as u64,
            sha256,
            mime_type: oss.mime_type.clone(),
            source_width: sw,
            source_height: sh,
            source_bytes: shot.bytes.len() as u64,
            matched: note,
        });
        if let Some(ext) = ext_of_mime(&oss.mime_type) {
            shot.ext = ext.to_string();
        }
        shot.ordinal = want;
        shot.bytes = bytes;
        summary.replaced += 1;
    }
    item.shots.sort_by_key(|s| s.ordinal);
    if reordered > 0 {
        tracing::info!(条目 = %item.item_key, 张数 = reordered, "来源库图片顺序与原帖不同，已按画面对上并改按原帖顺序");
    }
    if summary.replaced < total {
        let why = format!(
            "只换到 {}/{} 张：{}",
            summary.replaced,
            total,
            problems.join("；")
        );
        return bail_or_note(item, summary, why);
    }
    tracing::info!(
        条目 = %item.item_key,
        张数 = total,
        缓存 = res.cached,
        "高清原图已换进包"
    );
    item.hires = Some(summary);
    Ok(())
}

/// 来源库与 Instagram 原帖的图片张数必须一致，否则序号对应不可信。
fn check_counts(
    source_ordinals: &[u16],
    hires_photos: &[&HiresItem],
) -> std::result::Result<(), String> {
    if source_ordinals.is_empty() {
        return Err("来源库这条一张图都没有，没有可对应的".into());
    }
    if hires_photos.len() != source_ordinals.len() {
        return Err(format!(
            "Instagram 原帖图片 {} 张、来源库 {} 张，数量不一致（帖子可能被编辑过），不按序号硬配",
            hires_photos.len(),
            source_ordinals.len()
        ));
    }
    Ok(())
}

fn ext_of_mime(mime: &str) -> Option<&'static str> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// 这条条目对应的贴文短码。
///
/// 先用本地台账里的 `source_id`（01 存过）；台账里没有就从链接里切。
/// **两条都不行就报错，不猜**——猜错会去取别人的贴文，配出一组完全无关的图。
fn shortcode(local: Option<&Candidate>, item_key: &str) -> Result<String> {
    if let Some(c) = local {
        if let Some(s) = sane(c.source_id.trim()) {
            return Ok(s);
        }
        if let Some(s) = from_url(&c.url).and_then(|s| sane(&s)) {
            return Ok(s);
        }
    }
    anyhow::bail!("台账里没有 {item_key} 这条，也切不出短码，不猜是哪一条贴文")
}

/// 短码要拼进 csw 的接口路径，所以字符集收死。
///
/// 台账里的值来自 csw 自己，正常都是字母数字加下划线短横；但它经过我们的库
/// 转了一手，**不能假设它没被改过**——一个带 `../` 的值拼进路径就是另一个接口了。
fn sane(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| s.to_string())
}

/// 从 `https://www.instagram.com/p/ABC123/` 里切出 `ABC123`。
pub fn from_url(url: &str) -> Option<String> {
    let s = url.split(['?', '#']).next()?;
    let mut segs = s.split('/').filter(|x| !x.is_empty());
    while let Some(seg) = segs.next() {
        if matches!(seg, "p" | "reel" | "reels" | "tv") {
            let code = segs.next()?;
            return (!code.is_empty()).then(|| code.to_string());
        }
    }
    None
}

/// 条目键进文件名前过一遍。品牌名是从第三方正文来的，不能假设它干净。
fn file_part(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn ext_of(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .and_then(|s| s.rsplit('.').next())
        .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "jpg".into())
}

/// 最新派工单的备注（引擎放在 dispatch 里），没有就用顶层的。
fn dispatch_note(detail: &TaskDetail) -> String {
    detail
        .dispatch
        .as_ref()
        .and_then(|d| d.get("editor_note"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| detail.editor_note.clone())
}

fn item_title(detail: &TaskDetail, item_key: &str) -> String {
    detail
        .item
        .as_ref()
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(item_key)
        .to_string()
}

fn meta_for(
    round: &Round,
    detail: &TaskDetail,
    stage: &str,
    item_key: &str,
    checks: Vec<String>,
) -> Meta {
    let task = format!(
        "主编 · r{} 任务#{} 派工单",
        round.run_id.unwrap_or(0),
        detail.task.id
    );
    Meta {
        task: task.clone(),
        kind: "产出".into(),
        agent: "情报收集员".into(),
        stage: stage.into(),
        version: "v1".into(),
        // **不用当前时间**：同样的内容要打出同样的哈希
        at: round.window_end.clone(),
        upstreams: vec![task],
        status: "待审".into(),
        checks,
        item_key: item_key.to_string(),
    }
}

/// 05 的自检逐条。**如实写**——挑不出图位就说挑不出来。
fn checks_material(item: &ItemShots, picked: usize) -> Vec<String> {
    let recognized = item.shots.iter().filter(|s| s.desc.is_some()).count();
    vec![
        // 说清是哪一层的「原图」：来源库只有 640 档（10-02 r60 Van：预览里的图都比较模糊）
        format!(
            "来源库转存图 {} 张（Bright Data 640px 档，不是 Instagram 最大档）",
            item.shots.len()
        ),
        hires_check(item.hires.as_ref()),
        format!("识别 {recognized}/{} 张", item.shots.len()),
        format!(
            "图位 {picked}/{}{}",
            material::FIGURE_SLOTS,
            if picked < material::FIGURE_SLOTS {
                "，不足的原因见素材核对"
            } else {
                ""
            }
        ),
        format!("缺口 {} 条", item.gaps.len()),
    ]
}

/// 自检里关于高清原图的那一行。
fn hires_check(h: Option<&HiresSummary>) -> String {
    match h {
        Some(h) if h.total > 0 && h.replaced == h.total => format!(
            "高清原图 {}/{} 张取自 Instagram 原帖（hires-service{}），逐张溯源见「高清原图溯源」与 trace/hires.json",
            h.replaced,
            h.total,
            if h.cached { "，服务端缓存" } else { "" }
        ),
        Some(h) => format!(
            "高清原图未取全（{}/{} 张），包内其余是来源库 640 档：{}",
            h.replaced, h.total, h.note
        ),
        None => "高清原图未取：未配置 hires-service，包内是来源库 640 档".to_string(),
    }
}

fn checks_pick(items: &[ItemShots], failed: &[String]) -> Vec<String> {
    let shots: usize = items.iter().map(|i| i.shots.len()).sum();
    let hires: usize = items
        .iter()
        .filter_map(|i| i.hires.as_ref())
        .map(|h| h.replaced)
        .sum();
    vec![
        format!(
            "{} 条资讯、{} 张图：高清原图 {hires} 张取自 Instagram 原帖，其余 {} 张是来源库 640 档",
            items.len(),
            shots,
            shots - hires
        ),
        format!("取不到图的条目 {} 条", failed.len()),
        "未挑图位：图位由小红书图文作者定".to_string(),
    ]
}

/// 打包并落库。与 01 同一套确定性打包。
fn build(
    conn: &Connection,
    round: &Round,
    cfg: &Config,
    task_id: i64,
    item_key: &str,
    name: &str,
    entries: &[pack::Entry],
) -> Result<pack::Built> {
    let step = rounds::begin_step(conn, round.id, StepCode::Build, item_key)?;
    let out = cfg
        .data_dir
        .join("deliverables")
        .join(format!("{name}.zip"));
    let built = match pack::build(name, entries, &out) {
        Ok(b) => b,
        Err(e) => {
            rounds::end_step(
                conn,
                step.id,
                StepStatus::Failed,
                &serde_json::json!({}),
                &format!("{e:#}"),
            )?;
            return Err(e);
        }
    };
    conn.execute(
        "INSERT INTO deliverables_local(round_id, deliv_kind, item_key, zip_path, zip_sha256,
                                        zip_bytes, idem_key, created_at)
         VALUES (?1,'产出',?2,?3,?4,?5,?6,?7) ON CONFLICT DO NOTHING",
        params![
            round.id,
            item_key,
            built.path.to_string_lossy(),
            built.sha256,
            built.bytes as i64,
            built.idem_key(task_id),
            jiff::Timestamp::now().to_string(),
        ],
    )?;
    rounds::end_step(
        conn,
        step.id,
        StepStatus::Succeeded,
        &serde_json::json!({"字节": built.bytes, "sha": built.sha256}),
        "",
    )?;
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三张画面截然不同的图：横向渐变、纵向渐变、棋盘
    fn pic(kind: u8, w: u32, h: u32) -> Vec<u8> {
        let img = image::ImageBuffer::from_fn(w, h, |x, y| {
            let v = match kind {
                0 => (x * 255 / w) as u8,
                1 => (y * 255 / h) as u8,
                _ => {
                    if (x * 8 / w + y * 8 / h).is_multiple_of(2) {
                        230
                    } else {
                        20
                    }
                }
            };
            image::Rgb([v, v, v])
        });
        let mut b = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Jpeg)
            .unwrap();
        b
    }

    // r65 circles_jp：来源库给的顺序与原帖不同，按位置配会全错；按画面配应全部换上并改按原帖顺序
    #[tokio::test]
    async fn 来源库顺序乱了按画面对上() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        let base = srv.uri();
        // 原帖顺序 A(0) B(1) C(2)，高清 1080×1350
        for (i, k) in [(0u8, "a"), (1, "b"), (2, "c")] {
            Mock::given(method("GET"))
                .and(path(format!("/oss/{k}.jpg")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(pic(i, 1080, 1350)))
                .mount(&srv)
                .await;
        }
        let items: Vec<_> = ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(i, k)| {
                serde_json::json!({"index": i, "type": "Photo", "width": 1080, "height": 1350,
                    "file_key": format!("{k}.jpg"), "source_url": format!("https://cdn/{k}.jpg"),
                    "oss": {"url": format!("{base}/oss/{k}.jpg"), "size": 1, "mime_type": "image/jpeg", "file_hash": "x"}})
            })
            .collect();
        Mock::given(method("GET"))
            .and(path("/v1/media/CODE1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "shortcode": "CODE1", "pk": "1", "cached": false, "items": items})))
            .mount(&srv)
            .await;
        let hires = HiresClient::new(csw_collector_harvest::hires::HiresConfig {
            base_url: base.clone(),
            token: "t".into(),
            timeout: std::time::Duration::from_secs(10),
        })
        .unwrap();
        let dir = std::env::temp_dir().join(format!("hires-order-{}", std::process::id()));
        let dl = Downloader::new(DownloadConfig {
            dir: dir.clone(),
            width: 0,
            concurrency: 2,
            timeout: std::time::Duration::from_secs(10),
            max_attempts: 1,
            max_bytes: 10 << 20,
        })
        .unwrap()
        .allow_private_for_tests()
        .unwrap();
        // 来源库顺序 C(0) A(1) B(2)，640 档
        let shot = |ord: u16, kind: u8| Shot {
            blake3: format!("b{kind}"),
            ordinal: ord,
            url: format!("https://lib/{kind}.jpg"),
            bytes: pic(kind, 512, 640),
            ext: "jpg".into(),
            desc: None,
            hires: None,
        };
        let mut item = ItemShots {
            item_key: "circlesjp-x".into(),
            title: "t".into(),
            source_url: "https://www.instagram.com/p/CODE1/".into(),
            account: "a".into(),
            posted_at: String::new(),
            ingested_at: String::new(),
            shots: vec![shot(0, 2), shot(1, 0), shot(2, 1)],
            gaps: vec![],
            hires: None,
        };
        let cfg = Config::default();
        attach_hires(&cfg, &hires, &dl, "CODE1", &[0, 1, 2], &mut item)
            .await
            .unwrap();
        assert_eq!(item.hires.as_ref().unwrap().replaced, 3);
        let keys: Vec<_> = item
            .shots
            .iter()
            .map(|s| (s.ordinal, s.hires.as_ref().unwrap().file_key.clone()))
            .collect();
        assert_eq!(
            keys,
            vec![
                (0, "a.jpg".into()),
                (1, "b.jpg".into()),
                (2, "c.jpg".into())
            ]
        );
        assert!(
            item.shots[0]
                .hires
                .as_ref()
                .unwrap()
                .matched
                .contains("与原帖顺序不同")
        );
        assert!(item.shots.iter().all(|s| s.bytes.len() > 1000));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn 从链接里切短码() {
        for (url, want) in [
            ("https://www.instagram.com/p/ABC123/", "ABC123"),
            ("https://www.instagram.com/p/ABC123", "ABC123"),
            ("https://www.instagram.com/reel/XYZ/?utm=1", "XYZ"),
            ("https://www.instagram.com/tv/QQ/", "QQ"),
        ] {
            assert_eq!(from_url(url).as_deref(), Some(want), "{url}");
        }
        // 切不出来就是切不出来，不猜——猜错会去取别人的贴文，配出一组完全无关的图
        assert_eq!(from_url("https://www.instagram.com/and_wander/"), None);
        assert_eq!(from_url("https://example.com/"), None);
        assert_eq!(from_url(""), None);
    }

    #[test]
    fn 短码的字符集收死() {
        // 一个带 ../ 的值拼进路径就是另一个接口了
        assert_eq!(sane("ABC123"), Some("ABC123".into()));
        assert_eq!(sane("a_b-C9"), Some("a_b-C9".into()));
        assert_eq!(sane("../../admin"), None);
        assert_eq!(sane("ABC/DEF"), None);
        assert_eq!(sane("ABC?x=1"), None);
        assert_eq!(sane("ABC 123"), None);
        assert_eq!(sane(""), None);
        assert_eq!(sane(&"a".repeat(65)), None);

        // 台账里的短码不干净时，退回从链接切
        let mut c = cand();
        c.source_id = "../../admin".into();
        assert_eq!(shortcode(Some(&c), "k").unwrap(), "ABC123");
        // 两处都不干净就报错
        c.url = "https://www.instagram.com/p/..%2F..%2Fadmin/".into();
        assert!(shortcode(Some(&c), "k").is_err());
    }

    #[test]
    fn 台账里没有就报错不猜() {
        assert!(shortcode(None, "and-wander-a1b2c3").is_err());
    }

    #[test]
    fn 台账有短码就用台账的() {
        let mut c = cand();
        c.source_id = "SHORT1".into();
        assert_eq!(shortcode(Some(&c), "k").unwrap(), "SHORT1");
        // 没有短码时退回从链接切
        c.source_id = String::new();
        assert_eq!(shortcode(Some(&c), "k").unwrap(), "ABC123");
        // 两条都不行就报错
        c.url = "https://www.instagram.com/and_wander/".into();
        assert!(shortcode(Some(&c), "k").is_err());
    }

    fn cand() -> Candidate {
        Candidate {
            candidate_key: "k".into(),
            platform: csw_collector_core::types::Platform::Instagram,
            source_id: String::new(),
            collector: "c".into(),
            account: "and_wander".into(),
            url: "https://www.instagram.com/p/ABC123/".into(),
            text: String::new(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![],
            tags: vec![],
            hashtags: vec![],
        }
    }

    #[test]
    fn 张数对不上就不按序号硬配() {
        let mk = |i: usize| HiresItem {
            index: i,
            kind: "Photo".into(),
            width: 1440,
            height: 1800,
            file_key: format!("{i}.jpg"),
            source_url: String::new(),
            oss: None,
            error: None,
        };
        let a = mk(0);
        let b = mk(1);
        assert!(check_counts(&[0, 1], &[&a, &b]).is_ok());
        let e = check_counts(&[0, 1, 2], &[&a, &b]).unwrap_err();
        assert!(
            e.contains("2 张") && e.contains("3 张") && e.contains("不按序号硬配"),
            "{e}"
        );
        assert!(check_counts(&[], &[&a]).is_err());
        assert_eq!(ext_of_mime("image/jpeg"), Some("jpg"));
        assert_eq!(ext_of_mime("Image/PNG"), Some("png"));
        assert_eq!(ext_of_mime("application/octet-stream"), None);
    }

    #[test]
    fn 自检说清哪一层的原图() {
        let full = HiresSummary {
            replaced: 3,
            total: 3,
            cached: true,
            ..Default::default()
        };
        assert!(hires_check(Some(&full)).contains("3/3 张取自 Instagram 原帖"));
        assert!(hires_check(Some(&full)).contains("服务端缓存"));
        let part = HiresSummary {
            replaced: 1,
            total: 3,
            note: "第 2 张：x".into(),
            ..Default::default()
        };
        let s = hires_check(Some(&part));
        assert!(s.contains("1/3") && s.contains("第 2 张：x"), "{s}");
        assert!(hires_check(None).contains("未配置 hires-service"));
    }

    /// 10-05 r62 05 首跑：csw 的数字贴文 id 被当成短码送给 hires-service
    #[test]
    fn 送给hires的是链接里的短码不是csw数字id() {
        let mut c = cand();
        c.source_id = "3998767843600818383".into();
        c.url = "https://www.instagram.com/p/Dd-eirZFODP/".into();
        assert_eq!(ig_shortcode(&c).as_deref(), Some("Dd-eirZFODP"));
        c.url = "https://www.instagram.com/p/3998767843600818383/".into();
        assert!(ig_shortcode(&c).is_none(), "全数字的不是短码");
        c.url = "https://example.com/x".into();
        assert!(ig_shortcode(&c).is_none());
    }

    #[test]
    fn 扩展名从链接猜猜不出来就jpg() {
        assert_eq!(ext_of("https://x/a.png?sig=1"), "png");
        assert_eq!(ext_of("https://x/a.JPEG"), "jpeg");
        assert_eq!(ext_of("https://x/a"), "jpg");
        // 一长串不是扩展名的东西不能当扩展名塞进文件名
        assert_eq!(ext_of("https://x/a.verylongthing"), "jpg");
    }
}
