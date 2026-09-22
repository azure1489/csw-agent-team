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
use csw_collector_deliver::material::{self, ItemShots, Shot};
use csw_collector_deliver::pack;
use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::TaskDetail;
use csw_collector_harvest::download::{DownloadConfig, Downloader};
use csw_collector_harvest::recognize;

/// 05 的阶段代号
pub const STAGE_MATERIAL: &str = "material";
/// 11 的阶段代号
pub const STAGE_XHS_PICK: &str = "xhs_pick";

/// 原图下载器。**与 01 那个分开建**：宽度不同，共用一个就得改全局配置。
fn original_downloader(cfg: &Config) -> Result<Downloader> {
    Downloader::new(DownloadConfig {
        dir: cfg.blob_dir(),
        // 0 = 原图。设计师要拿它做头图
        width: 0,
        concurrency: cfg.limits.download_concurrency,
        timeout: std::time::Duration::from_secs(120),
        max_attempts: 3,
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

    let body = material::material_body(&item, &picked);
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
            "配图与素材核_情报收集员_r{}_{}_v1",
            round.run_id.unwrap_or(0),
            file_part(item_key)
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
        &format!("小红书选图包_情报收集员_r{run_id}_v1"),
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

    Ok(ItemShots {
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
    })
}

/// 这条条目对应的贴文短码。
///
/// 先用本地台账里的 `source_id`（01 存过）；台账里没有就从链接里切。
/// **两条都不行就报错，不猜**——猜错会去取别人的贴文，配出一组完全无关的图。
fn shortcode(local: Option<&Candidate>, item_key: &str) -> Result<String> {
    if let Some(c) = local {
        if !c.source_id.trim().is_empty() {
            return Ok(c.source_id.clone());
        }
        if let Some(s) = from_url(&c.url) {
            return Ok(s);
        }
    }
    anyhow::bail!("台账里没有 {item_key} 这条，也切不出短码，不猜是哪一条贴文")
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
        format!("原图 {} 张，全部按原始尺寸下载（未缩放）", item.shots.len()),
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

fn checks_pick(items: &[ItemShots], failed: &[String]) -> Vec<String> {
    vec![
        format!(
            "{} 条资讯、{} 张原图，全部按原始尺寸下载（未缩放）",
            items.len(),
            items.iter().map(|i| i.shots.len()).sum::<usize>()
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
    fn 扩展名从链接猜猜不出来就jpg() {
        assert_eq!(ext_of("https://x/a.png?sig=1"), "png");
        assert_eq!(ext_of("https://x/a.JPEG"), "jpeg");
        assert_eq!(ext_of("https://x/a"), "jpg");
        // 一长串不是扩展名的东西不能当扩展名塞进文件名
        assert_eq!(ext_of("https://x/a.verylongthing"), "jpg");
    }
}
