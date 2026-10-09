//! 常驻服务：任务驱动 + outbox 排空 + 进程内定时。
//!
//! # 启动顺序是有讲究的
//!
//! 1. **先把上次没退干净的收拾了**（`recover_interrupted`）。不先做这一步，
//!    续跑逻辑会看到一堆挂着 `running` 的步，以为别人正在跑。
//! 2. **再排空 outbox**。上次崩在「已经登记完、还没提交」的可能性很大，
//!    那时候最该做的是把欠的发完，而不是去接新单。
//! 3. **最后才开始轮询派单。**
//!
//! # 轮询是唯一的入口
//!
//! 不接飞书、不接 webhook。引擎是唯一真相，派单从 `GET /me/tasks` 来。
//! 群消息只是提示。

pub mod calibration;
pub mod designated;
pub mod finish;
pub mod hires_match;
pub mod http;
pub mod item;
pub mod kb_magazine;
pub mod outbox_sender;
pub mod register;
pub mod round;
pub mod schedule;
pub mod services;
pub mod slots;
pub mod tasks;
pub mod write;

use std::time::Duration;

use anyhow::{Context, Result};

use csw_collector_core::rounds;
use csw_collector_core::{Config, Secrets};
use csw_collector_engineapi::client::EngineClient;
use std::collections::HashMap;

use tasks::Action;

/// 现在真正跑得起来的那一个阶段。`tasks::OUR_STAGES` 里的另外两个
/// （`material`、`xhs_pick`）会被接下来的 `start_round` 挡在门外并如实报失败。
const STAGE_INTAKE: &str = "intake";

pub async fn run(cfg: &Config, secrets: &Secrets) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let engine = EngineClient::new(
        &cfg.engine.base_url,
        &secrets.engine_token,
        Duration::from_secs(120),
    )?;
    // 一、先把上次没退干净的收拾了。
    // 排在建客户端之前是有意的：这一步只动本地库，而建客户端要开向量库、
    // 探网络，任何一样卡住都会让「上次崩在哪」一直没人标记。
    let n = rounds::recover_interrupted(&conn)?;
    let running = rounds::running_rounds(&conn)?;
    // 手动的活不自动重排：它多半有副作用，悄悄再做一遍比不做更糟
    let dropped = csw_collector_core::workbench::recover_running(&conn)?;
    tracing::info!(
        中断的步 = n,
        还在跑的轮 = running.len(),
        没做完的手动活 = dropped,
        磁盘 = %csw_collector_core::disk::note(
            csw_collector_core::disk::used_pct(&cfg.data_dir),
            cfg.limits.disk_warn_pct,
            cfg.limits.disk_block_pct
        ),
        "启动自检"
    );

    // 客户端一次建好整个进程复用：闸门藏在里面，每处各建一个等于把闸门复制几份
    // Arc 是为了让工作台的知识库页也能用同一套客户端：闸门（网关信号量、GPU 锁）
    // 必须全进程只有一份，页面上搜一下与正式轮抢的是同一块卡
    let svc = std::sync::Arc::new(services::Services::build(cfg, secrets, &conn).await?);

    // 二、先把欠引擎的发完，再去接新单
    match outbox_sender::drain(&conn, &engine).await {
        Ok((sent, conflicts)) if sent > 0 || conflicts > 0 => {
            tracing::info!(发出 = sent, 冲突 = conflicts, "补发了上次没发完的");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(原因 = %format!("{e:#}"), "启动时排空 outbox 失败，稍后再试"),
    }
    let stuck = outbox_conflicts(&conn)?;
    if !stuck.is_empty() {
        // 冲突要人核实，不该被下一轮盖过去
        tracing::error!("有 outbox 条目处在冲突状态，**先查任务状态对账**，不要换幂等键重试");
        csw_collector_core::alert::notify(
            svc.alert.as_ref(),
            &format!(
                "有 {} 条写引擎的记录卡在冲突上（seq {:?}）。先查任务状态对账，不要换幂等键重试",
                stuck.len(),
                stuck
            ),
        )
        .await;
    }

    // 三、把工作台挂起来。HTTP 与轮询各跑各的：
    // 页面不该因为某一轮在跑就打不开，轮次也不该因为没人看页面就不跑。
    let http_conn = csw_collector_core::store::open(&cfg.db_path())?;
    let app_state = std::sync::Arc::new(http::AppState {
        conn: tokio::sync::Mutex::new(http_conn),
        cfg: cfg.clone(),
        started: std::time::Instant::now(),
        svc: Some(svc.clone()),
    });
    let auth = std::sync::Arc::new(crate::bff::AuthState {
        engine: csw_collector_engineapi::admin::AdminClient::new(&cfg.engine.admin_url)?,
        store: Default::default(),
        van_usernames: cfg.web.van_usernames.clone(),
        secure_cookie: cfg.web.secure_cookie,
    });
    let app = axum::Router::new()
        .merge(http::ops_router(app_state.clone()))
        .merge(http::api_router(app_state.clone(), auth.clone()))
        .merge(write::write_router(app_state, auth.clone()))
        .merge(crate::bff::router(auth));
    let listener = tokio::net::TcpListener::bind(&cfg.listen)
        .await
        .with_context(|| format!("监听 {}", cfg.listen))?;
    tracing::info!(地址 = %cfg.listen, "工作台起来了");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(原因 = %e, "工作台 HTTP 退出了");
        }
    });

    // 四、轮询 + 定时。**同一个循环里做**：定时那几件活要和正式轮抢同一块 GPU
    // 与同一个网关闸门，放进同一个进程闸门才只有一套。
    let every = Duration::from_secs(cfg.engine.poll_secs.max(5));
    let jobs = schedule::jobs_from(cfg);
    tracing::info!(
        间隔秒 = every.as_secs(),
        地址 = %cfg.engine.base_url,
        定时 = ?jobs.iter().map(|j| format!("{} {}", j.name, j.at)).collect::<Vec<_>>(),
        "开始轮询派单"
    );
    let mut prev_min = schedule::now_minute();
    // 接下来但还没轮到做的任务，心跳要一直发着，否则引擎判「接单后无活动」
    let mut beats: std::collections::HashMap<i64, tasks::Beat> = Default::default();
    let mut magazine = crate::kb::MagazineBackfill::default();
    loop {
        if let Err(e) = tick(&conn, &engine, cfg, &svc, &mut beats).await {
            tracing::warn!(原因 = %format!("{e:#}"), "这一轮轮询没跑完");
        }
        let now_min = schedule::now_minute();
        for name in schedule::due(&jobs, prev_min, now_min) {
            run_job(name, cfg, &conn, &svc, &engine, now_min).await;
        }
        // 工作台排进来的活。**一次只做一件**：这个循环是单线程的，
        // 取两件也只能一件一件做，而多出来的那件会在 running 上挂着假装在跑。
        if let Err(e) = run_queued(cfg, &conn, &svc).await {
            tracing::warn!(原因 = %format!("{e:#}"), "排队的活没做成");
        }
        // 都没在跑时做一片杂志向量回填；接了单没做完（beats 非空）就不做
        crate::kb::magazine_backfill_tick(
            cfg,
            &conn,
            &svc.store,
            &svc.vector,
            &mut magazine,
            !beats.is_empty(),
        )
        .await;
        prev_min = now_min;
        tokio::time::sleep(every).await;
    }
}

/// 做一件工作台排进来的活（手动开轮、重跑）。没有就立刻返回。
///
/// **失败写进 `work_queue.note`**，不只写日志：那一行就是页面上「为什么没成」
/// 的唯一来源，写日志等于只有能 ssh 上去的人才看得见。
async fn run_queued(
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
) -> Result<()> {
    use csw_collector_core::workbench;
    let Some(item) = workbench::take_next(conn)? else {
        return Ok(());
    };
    if let Some(why) = disk_blocked(cfg) {
        workbench::finish_work(conn, item.id, false, &why)?;
        tracing::error!(活 = item.id, "{why}");
        return Ok(());
    }
    tracing::info!(活 = item.id, 类型 = %item.kind, 排队人 = %item.actor, "开始做排队的活");
    let payload: serde_json::Value =
        serde_json::from_str(&item.payload_json).unwrap_or(serde_json::json!({}));
    let r = match item.kind.as_str() {
        "manual_round" => manual_round(cfg, conn, svc, &payload).await,
        "rerun" => rerun_round(cfg, conn, svc, item.round_id, &payload).await,
        other => Err(anyhow::anyhow!("不认识的活：{other}")),
    };
    match r {
        Ok(note) => {
            workbench::finish_work(conn, item.id, true, &note)?;
            tracing::info!(活 = item.id, "{note}");
        }
        Err(e) => {
            let why = format!("{e:#}");
            workbench::finish_work(conn, item.id, false, &why)?;
            tracing::warn!(活 = item.id, 原因 = %why, "这件活没做成");
        }
    }
    Ok(())
}

/// 命令行跑一轮，跑完就退出。
///
/// 与工作台那个「开始一轮」的区别只有一个：**这里当场跑完**，
/// 而 HTTP 那个只往队列里写一行（一轮四十分钟，请求等不了）。
/// 不起 HTTP、不轮询派单、不写引擎——它是拿来看的，不是拿来交的。
pub async fn run_once(cfg: &Config, secrets: &Secrets, days: i64) -> Result<()> {
    // **先确认常驻服务没在跑。** 两个进程同时对着一个库干活会互相拆台：
    // 下面那句 `recover_interrupted` 会把 serve 正在跑的步一把标成 interrupted，
    // 而 SQLite 是单写者，两边还会互相等锁。探 healthz 是最省事的判据——
    // 它正是那个进程在监听的地址。
    //
    // 它挡不住的：两边用了**不同的配置文件**却指着同一个 data_dir。
    // 真要挡那个得上文件锁；现实里两边都读 `/opt/csw-collector/collector.toml`，
    // 这一道够用。
    if let Ok(resp) = reqwest::Client::new()
        .get(format!("http://{}/healthz", local_addr(&cfg.listen)))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        anyhow::bail!(
            "常驻服务正在跑（{} 上的 /healthz 回了 {}）。\n\
             两个进程同时对着一个库干活会互相拆台：这边一启动就会把那边正在跑的步\n\
             标成中断。要手动开一轮，用工作台的「开始一轮」——它排进队列，\n\
             由常驻循环去做。",
            cfg.listen,
            resp.status()
        );
    }
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    // 常驻服务可能刚崩在半路上。不先收拾，续跑逻辑会看到一堆挂着 running 的步
    rounds::recover_interrupted(&conn)?;
    if let Some(why) = disk_blocked(cfg) {
        anyhow::bail!("{why}");
    }
    let svc = services::Services::build(cfg, secrets, &conn).await?;
    let note = manual_round(cfg, &conn, &svc, &serde_json::json!({ "days": days })).await?;
    println!("{note}");
    Ok(())
}

/// 把监听地址换成本机能连上的那个。
///
/// `0.0.0.0` 与 `[::]` 是「监听所有网卡」，不是能连的目标地址。
fn local_addr(listen: &str) -> String {
    match listen.rsplit_once(':') {
        Some(("0.0.0.0" | "" | "[::]" | "::", port)) => format!("127.0.0.1:{port}"),
        _ => listen.to_string(),
    }
}

/// 手动开一轮：只跑第 2–5 步，**不写引擎**。它是拿来看的，不是拿来交的。
pub(crate) async fn manual_round(
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
    payload: &serde_json::Value,
) -> Result<String> {
    let (start, end) = match (
        payload.get("window_start").and_then(|v| v.as_str()),
        payload.get("window_end").and_then(|v| v.as_str()),
    ) {
        (Some(a), Some(b)) => (a.to_string(), b.to_string()),
        _ => {
            let days = payload.get("days").and_then(|v| v.as_i64()).unwrap_or(1);
            let now = jiff::Timestamp::now();
            // 必须走 window::days：`Timestamp - Span::new().days(n)` 会 panic，
            // 而这条路是工作台「开始一轮」走的——在这里 panic 等于点一下就把服务打挂
            (
                (now - csw_collector_core::window::days(days)).to_string(),
                now.to_string(),
            )
        }
    };
    let (r, _) = rounds::open_round(
        conn,
        &rounds::NewRound {
            kind: csw_collector_core::types::RoundKind::Manual,
            trigger: csw_collector_core::types::RoundTrigger::Manual,
            run_id: None,
            task_id: None,
            stage_code: Some("intake".into()),
            target_version: 0,
            parent_round_id: None,
            window_start: start,
            window_end: end,
            plan_version: 1,
            rubric_version: csw_collector_judge::rubric::RUBRIC_VERSION.into(),
            kb_snapshot: cfg.vector.embed_model.clone(),
            instructions_hash: String::new(),
        },
    )?;
    match round::run_manual(conn, &r, cfg, svc, round::Caches::default()).await {
        Ok(c) => {
            rounds::finish_round(conn, r.id, "done", "")?;
            Ok(format!(
                "第 {} 轮：取到 {} 条，判了 {}（推荐 {} 备选 {} 待核 {}）",
                r.id, c.candidates, c.judged, c.recommend, c.alternate, c.pending_check
            ))
        }
        Err(e) => {
            rounds::finish_round(conn, r.id, "failed", &format!("{e:#}"))?;
            Err(e)
        }
    }
}

/// 重跑一轮的第 2–5 步。**登记与提交不动**——引擎那边已经收到的台账要改，
/// 只能走补件，那是人的决定，不该是重跑的副作用。
async fn rerun_round(
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
    round_id: Option<i64>,
    payload: &serde_json::Value,
) -> Result<String> {
    let id = round_id.context("重跑要说重跑哪一轮")?;
    let r = rounds::get(conn, id)?.context("没有这一轮")?;
    let from = payload
        .get("from_step")
        .and_then(|v| v.as_str())
        .unwrap_or("judge");
    let caches = round::Caches::from_step(from);
    let c = round::run_manual(conn, &r, cfg, svc, caches).await?;
    Ok(format!(
        "从 {from} 重跑第 {id} 轮：判了 {}（推荐 {} 备选 {} 待核 {}）。         登记与提交没动——要更新引擎请走补件",
        c.judged, c.recommend, c.alternate, c.pending_check
    ))
}

/// 跑一件定时的活。**一件出错不影响别的**，也不影响轮询。
async fn run_job(
    name: &str,
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
    engine: &EngineClient,
    now_min: u32,
) {
    // 安静窗口里把 GPU 让给正式轮：两边抢卡会把双方都拖到四倍延迟，
    // 比任何一边单独跑都糟
    if schedule::in_quiet_window(
        &cfg.schedule.gpu_quiet_from_utc,
        &cfg.schedule.gpu_quiet_to_utc,
        now_min,
    ) {
        tracing::info!(活 = name, "在安静窗口里，让开 GPU，这次跳过");
        return;
    }
    tracing::info!(活 = name, "定时的活开始");
    let r = match name {
        "kb_sync_1" | "kb_sync_2" => kb_sync_job(cfg, conn, svc, engine).await,
        "lance_backup" => backup_job(cfg).await,
        "prefetch" => prefetch_job(cfg, conn, svc).await,
        other => Err(anyhow::anyhow!("不认识的定时活：{other}")),
    };
    match r {
        Ok(note) => tracing::info!(活 = name, "{note}"),
        Err(e) => {
            let why = format!("{e:#}");
            tracing::warn!(活 = name, 原因 = %why, "这件活没跑成，下一次再来");
            // 预取轮没跑成意味着明早那一轮要从零跑、会迟到。**这件事必须有人知道**
            csw_collector_core::alert::notify(
                svc.alert.as_ref(),
                &format!("定时的活 {name} 没跑成：{why}"),
            )
            .await;
        }
    }

    // 顺带把过期的图清掉。只删图不删记录——
    // media 表那一行要留着，否则「这条当时有几张图」就查不回来了
    match schedule::sweep_old_media(&cfg.blob_dir(), cfg.schedule.media_retention_days) {
        Ok(n) if n > 0 => tracing::info!(删了 = n, "清掉了过期的图"),
        Ok(_) => {}
        Err(e) => tracing::warn!(原因 = %format!("{e:#}"), "清图失败"),
    }
}

/// 预取轮：01:40 先把第 2–5 步跑一遍，产物落本地库。
///
/// **失败只记一笔。** 它是缓存不是前置条件：没跑成，05:30 那一轮照样
/// 从零跑得完，只是慢——而这件事要让日志说清楚，不能悄悄过去。
async fn prefetch_job(
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
) -> Result<String> {
    if let Some(why) = disk_blocked(cfg) {
        // 预取轮是缓存，盘紧的时候第一个该让路的就是它
        anyhow::bail!("{why}");
    }
    let (window_start, window_end, _) = window_for(conn, None);
    let (r, _) = rounds::open_round(
        conn,
        &rounds::NewRound {
            kind: csw_collector_core::types::RoundKind::Prefetch,
            trigger: csw_collector_core::types::RoundTrigger::Manual,
            // 不挂任何任务：预取轮不属于哪一期，也不许写引擎
            run_id: None,
            task_id: None,
            stage_code: Some("intake".into()),
            target_version: 0,
            parent_round_id: None,
            window_start,
            window_end,
            plan_version: 1,
            rubric_version: csw_collector_judge::rubric::RUBRIC_VERSION.into(),
            kb_snapshot: cfg.vector.embed_model.clone(),
            // 预取轮没有派单，作业标准从镜像里取上一次的（见 round::run_prefetch）
            instructions_hash: String::new(),
        },
    )?;
    match round::run_prefetch(conn, &r, cfg, svc).await {
        Ok(counts) => {
            rounds::finish_round(conn, r.id, "done", "")?;
            Ok(format!(
                "预取了 {} 条，判了 {}（推荐 {} 备选 {} 待核 {}）",
                counts.candidates,
                counts.judged,
                counts.recommend,
                counts.alternate,
                counts.pending_check
            ))
        }
        Err(e) => {
            let why = format!("{e:#}");
            rounds::finish_round(conn, r.id, "failed", &why)?;
            // 不往引擎报：引擎根本不知道有这一轮
            Err(e.context("预取轮没跑成，正式轮要从零跑，会慢四十分钟"))
        }
    }
}

/// 向量库备份。**到点检查、隔够天数才真备**——按周几判的话，
/// 进程哪天没在跑，那一整周就没有备份。
async fn backup_job(cfg: &Config) -> Result<String> {
    let root = cfg.backup_dir();
    if !schedule::needs_backup(&root, cfg.schedule.backup_every_days) {
        return Ok("离上一份还不够久，这次跳过".into());
    }
    // 备份要占一份库那么大的盘。盘紧的时候别雪上加霜
    if let Some(why) = disk_blocked(cfg) {
        anyhow::bail!("{why}");
    }
    let dest = schedule::backup_lance(&cfg.lance_path(), &root)?;
    Ok(format!("备到了 {}", dest.display()))
}

/// 知识库定时同步：**先从引擎和贴文库拉新材料，再算向量**。
///
/// 09-24 查到这里原先只算向量、从不拉数据——知识库停在最后一次手动 `kb sync`，
/// 选题记忆从没同步到本地（本地 0 条），判断拿不到新发的文章、新的 03 决定和 Van 的案例。
/// 各路各报各的：一路失败不让其余几路也不跑。
async fn kb_sync_job(
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
    engine: &EngineClient,
) -> Result<String> {
    use csw_collector_kb::sync;
    let mut notes: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (name, r) in [
        (
            "发布记录",
            sync::sync_ledger_posts(conn, engine, &svc.tok, false).await,
        ),
        (
            "历史决定",
            sync::sync_decisions(conn, engine, &svc.tok, false).await,
        ),
        (
            "已生成贴文",
            sync::sync_generated(conn, &svc.csw, &svc.tok, &svc.brands).await,
        ),
    ] {
        match r {
            Ok(r) => notes.push(format!(
                "{name} 取 {} 新 {} 变 {}",
                r.fetched, r.inserted, r.changed
            )),
            Err(e) => failed.push(format!("{name}：{e:#}")),
        }
    }
    match sync::sync_memory(conn, engine).await {
        Ok((rules, cases)) => notes.push(format!("选题记忆 准则 {rules} 案例 {cases}")),
        Err(e) => failed.push(format!("选题记忆：{e:#}")),
    }
    // 杂志背景库：读刊译台写在本机的清单。各本各报各的，一本失败不挡别的
    if !cfg.magazine.dir.as_os_str().is_empty() && cfg.magazine.dir.is_dir() {
        match csw_collector_kb::magazine::sync_dir(conn, &svc.tok, &cfg.magazine.dir, false) {
            Ok(reports) => {
                let (mut books, mut new, mut del) = (0, 0, 0);
                let (mut keys, mut imgs) = (Vec::new(), Vec::new());
                for r in reports {
                    match r {
                        Ok(r) => {
                            books += 1;
                            new += r.inserted + r.changed;
                            del += r.deleted;
                            keys.extend(r.drop_doc_keys);
                            imgs.extend(r.drop_image_blake3);
                        }
                        Err(e) => failed.push(format!("杂志：{e:#}")),
                    }
                }
                crate::kb::drop_vectors(&svc.store, &keys, &imgs).await;
                let _ = sync::set_cursor(
                    conn,
                    csw_collector_kb::magazine::SRC_MAGAZINE,
                    &jiff::Timestamp::now().to_string(),
                );
                if books > 0 {
                    notes.push(format!("杂志 {books} 本 新/变 {new} 删 {del}"));
                }
            }
            Err(e) => failed.push(format!("杂志：{e:#}")),
        }
    }
    if let Err(e) = csw_collector_kb::fts::optimize(conn) {
        tracing::warn!(原因 = %format!("{e:#}"), "整理全文索引失败");
    }
    for f in &failed {
        tracing::warn!("知识库同步有一路失败：{f}");
    }
    let rep = csw_collector_kb::sync::embed_pending(
        conn,
        &svc.store,
        &svc.vector,
        &cfg.vector.embed_model,
        usize::MAX,
    )
    .await?;
    // 夜间顺手整一次向量库：每次写都生成新文件，白天整会和检索抢 IO
    if let Err(e) = svc.store.optimize().await {
        tracing::warn!(原因 = %format!("{e:#}"), "整理向量库失败");
    }
    let msg = format!(
        "{}；算了 {} 条向量、失败 {}、还剩 {}",
        notes.join("、"),
        rep.embedded,
        rep.failed,
        rep.remaining
    );
    if failed.is_empty() {
        Ok(msg)
    } else {
        // 有一路没拉到就报失败，让告警与日志都看得见；拉到的那几路已经落库了
        anyhow::bail!("{msg}；失败的：{}", failed.join("；"))
    }
}

/// 轮询一次。
///
/// # 先把这一批都接下来，再一个个做
///
/// 做一轮是阻塞的（01 整轮四十分钟起，周一两小时），而接单与心跳都在
/// `start_round` 里面——于是**同一批派来的第二个任务，在第一个跑完之前连 ack
/// 都发不出去**。引擎那边按「派工后未接单」5 分钟提醒、15 分钟升级给中枢，
/// 全是假警报。
///
/// 这会真发生：05 是逐条阶段，Van 批 7 条引擎就派 7 个任务。
///
/// 所以接单与干活分开：先把这一批全 ack 并挂上心跳（`beats` 跨 tick 活着），
/// 再一个个跑。排队排到时限要到的那个，`should_fail_early` 会主动报失败，
/// 不会挂着假装在做。
async fn tick(
    conn: &rusqlite::Connection,
    engine: &EngineClient,
    cfg: &Config,
    svc: &services::Services,
    beats: &mut std::collections::HashMap<i64, tasks::Beat>,
) -> Result<()> {
    let mine = engine.my_tasks().await.context("取派单")?;
    let mut batch = tasks::plan(&mine.tasks);
    // 运营方手工交付中的任务：不接、不心跳、不返工、不报失败
    batch.retain(|(t, _)| {
        let off = cfg.engine.hands_off_tasks.contains(&t.task.id);
        if off {
            tracing::debug!(任务 = t.task.id, "在 hands_off_tasks 里，不接");
        }
        !off
    });

    // 一、先全接下来。ack 就是心跳，重复发是无害的。
    // **被退回的不接单**：引擎只让「已派工 / 进行中」的接单，退回的按意见改完直接重交
    //（09-28 r56 #592 被退回后，这里每 30 秒接一次单被拒，两个半小时一动不动）
    for (t, action) in &batch {
        if *action != Action::Start {
            continue;
        }
        if beats.contains_key(&t.task.id) {
            continue;
        }
        // 01 的窗口还没到截止就不接：定时器提前开了一期（10-09 r67 03:08 开出、窗口止于 06:00），
        // 现在开工会把止点截成「此刻」，主编凌晨被 @、当期少一段。到点再接
        if t.task.stage_code == "intake"
            && let Ok(Some((_, to))) = engine.run_window(t.task.run_id).await
            && !window_closed(&to, jiff::Timestamp::now())
        {
            tracing::debug!(任务 = t.task.id, 窗口止 = %to, "窗口还没截止，到点再接");
            continue;
        }
        match engine.ack(t.task.id).await {
            Ok(_) => {
                beats.insert(
                    t.task.id,
                    tasks::start_heartbeat(
                        engine.clone(),
                        t.task.id,
                        Duration::from_secs(cfg.engine.ack_secs.max(30)),
                    ),
                );
                tracing::info!(任务 = t.task.id, 阶段 = %t.task.stage_code, "接单");
            }
            // 接不下来就不跑它，下一次轮询再试
            Err(e) => tracing::warn!(任务 = t.task.id, 原因 = %format!("{e:#}"), "接单失败"),
        }
    }

    // 二、再一个个做
    for (t, action) in batch {
        // 引擎说在做、本地那一轮却已收尾，而派工时间晚于收尾：是主编退回后「取消 → 重开 →
        // 重新派工」来的新活（09-28 r55 #581）。**直接返工**，不等、也不走时限前报失败——
        // 晚交比报失败强，报了失败主编还得再重开一次
        if action == Action::Continue
            && let Ok(Some(prev)) = rounds::latest_for_task(conn, t.task.id)
            && prev.status != "running"
            && redispatched_after(conn, prev.id, &t.task.dispatched_at)
        {
            tracing::info!(任务 = t.task.id, 上一轮 = prev.id, "收尾后又派了一次：返工");
            beats.entry(t.task.id).or_insert_with(|| {
                tasks::start_heartbeat(
                    engine.clone(),
                    t.task.id,
                    Duration::from_secs(cfg.engine.ack_secs.max(30)),
                )
            });
            drive(conn, engine, cfg, svc, t, Action::Rework, beats).await;
            continue;
        }
        match action {
            Action::Start => {
                if !beats.contains_key(&t.task.id) {
                    continue; // 上面没接下来
                }
                drive(conn, engine, cfg, svc, t, action, beats).await;
            }
            Action::Rework => {
                tracing::info!(任务 = t.task.id, "被退回：按退回意见返工后重交");
                drive(conn, engine, cfg, svc, t, action, beats).await;
            }
            // 不再按时限主动报失败（10-08 r66 #752）：01 的派工时限 40 分钟，整池判断要几个小时；
            // 时限一过，旧逻辑每次轮询都走「报失败」分支（同幂等键回放，引擎不变），重启后的轮次永远续不上。
            // 主编 13:22 原话：Van 已取消限时，不再以临近 due_at 报失败
            Action::Continue => {
                let local = rounds::latest_for_task(conn, t.task.id)?;
                let decision = tasks::on_continue(
                    local
                        .as_ref()
                        .map(|r| (r.status.as_str(), r.trigger.as_str())),
                    beats.contains_key(&t.task.id),
                );
                let act = match decision {
                    tasks::OnContinue::Resume { returned: true } => Some(Action::Rework),
                    tasks::OnContinue::Resume { returned: false } | tasks::OnContinue::Retry => {
                        Some(Action::Start)
                    }
                    tasks::OnContinue::Wait => None,
                    tasks::OnContinue::NotOurs => {
                        tracing::debug!(
                            任务 = t.task.id,
                            "引擎说在做，但本地没有这一轮、本进程也没接过——多半是 Hermes 接的，不接手"
                        );
                        None
                    }
                };
                if let Some(act) = act {
                    // 重启后心跳表是空的：续跑前先把心跳挂回去，
                    // 否则引擎 10 分钟后报「接单后无活动」
                    beats.entry(t.task.id).or_insert_with(|| {
                        tasks::start_heartbeat(
                            engine.clone(),
                            t.task.id,
                            Duration::from_secs(cfg.engine.ack_secs.max(30)),
                        )
                    });
                    drive(conn, engine, cfg, svc, t, act, beats).await;
                }
            }
            Action::Wait => tracing::debug!(任务 = t.task.id, "等闸，只轮询"),
            Action::Ignore => {}
        }
    }
    // 三、本地还挂着「进行中」、引擎上这一期已作废或已结束的：收尾成已取消
    if let Err(e) = close_orphans(conn, engine).await {
        tracing::warn!(原因 = %format!("{e:#}"), "核对挂着的轮次没做成，下次再核");
    }
    let (sent, conflicts) = outbox_sender::drain(conn, engine).await?;
    if sent > 0 {
        tracing::info!(发出 = sent, 冲突 = conflicts, "发了几条");
    }
    Ok(())
}

/// 本地是「进行中」、所属一期在引擎上已作废或已结束的任务轮次，收尾成已取消。
///
/// 09-28 r55 作废时 #581 的返工轮（第 32 轮）正跑到一半，之后再没人续跑，页面上一直挂着「进行中」。
/// **不看任务还在不在派单里**：#581 在作废的一期里又被重开成 ready，一直留在派单列表上
///（那时引擎还没拦「作废的一期不能重开」）。一期还在进行的不动：可能是交完进程就重启了、
/// 本地没来得及改状态，下一次派单会接上。本函数在 `tick` 末尾跑，这时本进程没有在跑的轮次。
async fn close_orphans(conn: &rusqlite::Connection, engine: &EngineClient) -> Result<()> {
    for r in rounds::running_rounds(conn)? {
        let (Some(task), Some(run)) = (r.task_id, r.run_id) else {
            continue;
        };
        if r.kind != "task" {
            continue;
        }
        let status = engine.run_status(run).await?;
        let why = match status.as_str() {
            "aborted" => format!("r{run} 已作废，这一轮没跑完就停了"),
            "done" => format!("r{run} 已结束，这一轮没跑完就停了"),
            _ => continue,
        };
        rounds::finish_round(conn, r.id, "cancelled", &why)?;
        tracing::info!(轮次 = r.id, 任务 = task, "{why}");
    }
    Ok(())
}

/// 跑一个任务的一轮，并决定心跳的去留。
///
/// **开轮失败、本地又没有轮次时，心跳留着**：那是「接了单、开轮前出错」，下一轮
/// `tick` 要靠心跳表里还有它，才知道这单是我们接的、该重试（见 `tasks::on_continue`）。
/// 其余情况（跑完、报过失败、本地已有收尾的轮次）心跳都可以停了。
async fn drive(
    conn: &rusqlite::Connection,
    engine: &EngineClient,
    cfg: &Config,
    svc: &services::Services,
    t: &csw_collector_engineapi::types::MyTask,
    action: Action,
    beats: &mut std::collections::HashMap<i64, tasks::Beat>,
) {
    // 一个任务出错不该让别的任务也不跑
    let res = start_round(conn, engine, cfg, svc, t, action).await;
    if let Err(e) = &res {
        tracing::error!(任务 = t.task.id, 原因 = %format!("{e:#}"), "这一轮没跑起来");
    }
    let opened = rounds::latest_for_task(conn, t.task.id)
        .ok()
        .flatten()
        .is_some();
    if res.is_ok() || opened {
        beats.remove(&t.task.id);
    }
}

/// 接单 → 建轮 → 跑。
///
/// **先取详情再接单**：作业标准要连同哈希一起进这一轮，标准变了判断就不能复用。
/// 接单之后才开始心跳——接单前心跳是没有意义的。
async fn start_round(
    conn: &rusqlite::Connection,
    engine: &EngineClient,
    cfg: &Config,
    svc: &services::Services,
    t: &csw_collector_engineapi::types::MyTask,
    action: Action,
) -> Result<()> {
    // 盘满时的表现极难看懂：下载一半失败、SQLite 写不进去、zip 打到一半断掉，
    // 每一处报的都是别的错。**到线就不开新轮**，并如实告诉引擎为什么。
    if let Some(why) = disk_blocked(cfg) {
        tracing::error!(任务 = t.task.id, "{why}");
        // 这一条只有我们自己发现得了：引擎不知道我们的盘满了
        csw_collector_core::alert::notify(
            svc.alert.as_ref(),
            &format!("任务 {} 没接：{why}", t.task.id),
        )
        .await;
        let idem = format!("fail-{}-disk", t.task.id);
        engine
            .fail(t.task.id, &why, &idem)
            .await
            .context("报告磁盘满")?;
        return Ok(());
    }

    // 阶段在这里岔开。**不岔开的话 05／11 会走 01 的十步**，交出一份文不对题的
    // 台账——那种错在群播报里看不出来，要等主编打开交付物才发现。
    let known = [STAGE_INTAKE, item::STAGE_MATERIAL, item::STAGE_XHS_PICK];
    if !known.contains(&t.task.stage_code.as_str()) {
        let why = format!(
            "{} 阶段本服务做不了（只做 {}），请主编改派人工",
            t.task.stage_code,
            known.join(" / ")
        );
        tracing::error!(任务 = t.task.id, 阶段 = %t.task.stage_code, "{why}");
        let idem = format!("fail-{}-unsupported", t.task.id);
        engine
            .fail(t.task.id, &why, &idem)
            .await
            .context("报告不支持的阶段")?;
        return Ok(());
    }

    let detail = engine.task_detail(t.task.id).await.context("取任务详情")?;
    // 把这份派单镜像下来：预取轮 01:40 没有单可接，作业标准只能从这里取上一次的。
    // 存的是**拼好的那一段**，与这一轮送进模型的一字不差，指纹才对得上。
    if let Err(e) = csw_collector_core::mirror::put(
        conn,
        &csw_collector_core::mirror::TaskSnapshot {
            task_id: t.task.id,
            run_id: t.task.run_id,
            stage_code: t.task.stage_code.clone(),
            status: format!("{:?}", t.task.status).to_lowercase(),
            item_key: t.task.item_key.clone(),
            editor_note: detail.editor_note.clone(),
            work_standard: round::work_standard(&detail),
            upstreams_json: serde_json::to_string(&detail.upstreams).unwrap_or_default(),
            latest_review_json: serde_json::to_string(&detail.latest_review).unwrap_or_default(),
            deadline_at: t.task.due_at.clone(),
        },
    ) {
        // 镜像存不下只影响下一次预取轮的复用率，不该拦住这一轮
        tracing::warn!(任务 = t.task.id, 原因 = %format!("{e:#}"), "任务镜像没写成");
    }
    // 引擎派的窗口：取不到（网络、老引擎）就只按水位算
    let engine_window = match engine.run_window(t.task.run_id).await {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(期次 = t.task.run_id, 原因 = %format!("{e:#}"), "取不到期次窗口，按水位算");
            None
        }
    };
    let (window_start, window_end, truncated) = window_for(conn, engine_window.as_ref());
    // 主编退回后「取消 → 重开 → 重新派工」时，任务版本号不变、本地那一轮已经收尾，
    // 按 (任务, 版本, 触发) 查会以为「开过了」而什么都不做（09-28 r55 #581 接了单就停住）。
    // **派工时间晚于本地那一轮收尾时间**，就是一次新的派工：当返工。
    let prev = rounds::latest_for_task(conn, t.task.id).ok().flatten();
    let redispatched = prev.as_ref().is_some_and(|p| {
        p.status != "running" && redispatched_after(conn, p.id, &t.task.dispatched_at)
    });
    let rework = action == Action::Rework || redispatched;
    // 返工交的是下一版
    // 返工交的是引擎的下一版（当前版本 + 1）。**不按本地轮次累加**：返工中途重启一次，
    // 本地就多加一，交付物页头写 v4、引擎里却是 v2（09-28 r56）
    let target_version = if rework {
        i64::from(t.task.cur_version) + 1
    } else {
        i64::from(t.task.cur_version.max(1))
    };

    // 01 的返工**在原轮次上改，不重新采集**（09-28 用户）。原轮次重新置为进行中，版本改成这一版
    // 原轮次一条候选都没有（上游断料、窗口开错）的，原轮次上没有可改的：重新采集（09-29 r57）
    let prev_empty = prev.as_ref().is_some_and(|p| {
        conn.query_row(
            "SELECT COUNT(*) FROM round_candidates WHERE round_id = ?1",
            [p.id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
            == 0
    });
    // 主编最新的指示（派工单，或比派工更新的退回意见）写了【工作台不接】：让开，不做、不报失败。
    // 01 / 05 / 11 都认（10-08 r65 #751 是 05）
    if designated::hands_off(&detail) {
        tracing::info!(任务 = t.task.id, 阶段 = %t.task.stage_code, "主编最新指示写明工作台不接：不做，等人工交付");
        return Ok(());
    }
    // 主编重开 01 要求重新采集（10-07 r64：Van 全部退回，主编要滚动 24 小时重采）：开新轮次按新窗口采，
    // 不在原轮次上返工。窗口 = 派工时刻往前 24 小时
    let recollect = rework && t.task.stage_code == STAGE_INTAKE && designated::recollect(&detail);
    let (window_start, window_end) = if recollect {
        match parse_engine_ts(&t.task.dispatched_at) {
            Some(end) => {
                let start = end - jiff::SignedDuration::from_hours(24);
                tracing::info!(任务 = t.task.id, 窗口 = %format!("{start} ~ {end}"), "派工单要求重新采集：开新轮次");
                (start.to_string(), end.to_string())
            }
            None => (window_start, window_end),
        }
    } else {
        (window_start, window_end)
    };
    let in_place =
        rework && !recollect && t.task.stage_code == STAGE_INTAKE && prev.is_some() && !prev_empty;
    // 指定帖：重开 01、派工单点名了原轮次里没有的帖子（10-06 r63 #699）。只读这几条、直接交，
    // 不在原轮次上返工、不重登记、不过全池自查
    if rework && t.task.stage_code == STAGE_INTAKE {
        let codes = designated::new_codes(conn, prev.as_ref().map(|p| p.id), &detail.editor_note);
        if designated::is_designation(&detail.editor_note) && !codes.is_empty() {
            return designated::deliver(
                conn,
                engine,
                cfg,
                svc,
                t,
                &detail,
                prev.as_ref(),
                &codes,
                target_version,
            )
            .await;
        }
    }
    // 上一次返工算下来与已交的一模一样、没重交的：同一版退回意见不再反复重做
    //（否则每 30 秒一次轮询就重做一遍）。主编给了新意见（版本变了）才再做
    let unchanged_mark = format!("{UNCHANGED}@v{}", t.task.cur_version);
    // 主编重开、重新派工的（redispatched）不算：那是新的一次派工，要接着做
    if in_place && !redispatched && prev.as_ref().is_some_and(|p| p.note == unchanged_mark) {
        tracing::debug!(
            任务 = t.task.id,
            "上次返工与已交内容相同、未重交，等主编新意见"
        );
        return Ok(());
    }
    // 上一次交出去的内容指纹（记在轮次备注里）：返工算完一样就不重交
    // 上次交出去的内容指纹：先读专门的一列（不会被「未重交」标记盖掉），老轮次退回读备注
    //（09-30 r58：指纹只在备注里，被盖掉后主编重开再派时把同样的包又交了 8 遍）
    let last_state: Option<String> = prev.as_ref().and_then(|p| {
        conn.query_row(
            "SELECT submitted_state FROM rounds WHERE id = ?1",
            [p.id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .or_else(|| p.note.strip_prefix(SUBMITTED).map(str::to_string))
    });
    let (r, is_new) = if let Some(p) = prev.filter(|_| in_place) {
        conn.execute(
            "UPDATE rounds SET status = 'running', ended_at = NULL, target_version = ?2,
                    note = '返工：在原轮次上改'
             WHERE id = ?1",
            rusqlite::params![p.id, target_version],
        )?;
        // 窗口终点改成本期的止点：v1 把开工时刻当终点写进了交付物（09-28 r56 退回第 2 条）
        if let Some(e) = engine_window
            .as_ref()
            .and_then(|(_, to)| parse_engine_ts(to))
            && p.window_end.parse::<jiff::Timestamp>().is_ok_and(|w| e < w)
        {
            conn.execute(
                "UPDATE rounds SET window_end = ?2 WHERE id = ?1",
                rusqlite::params![p.id, e.to_string()],
            )?;
        }
        tracing::info!(
            任务 = t.task.id,
            轮次 = p.id,
            版本 = target_version,
            "返工：在原轮次上改，不重新采集"
        );
        let r = rounds::latest_for_task(conn, t.task.id)?
            .ok_or_else(|| anyhow::anyhow!("原轮次读不回来"))?;
        (r, false)
    } else {
        let (r, is_new) = rounds::open_round(
            conn,
            &rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: if rework {
                    csw_collector_core::types::RoundTrigger::Returned
                } else {
                    csw_collector_core::types::RoundTrigger::Dispatch
                },
                run_id: Some(t.task.run_id),
                task_id: Some(t.task.id),
                stage_code: Some(t.task.stage_code.clone()),
                target_version,
                parent_round_id: None,
                window_start,
                window_end,
                plan_version: 1,
                rubric_version: csw_collector_judge::rubric::RUBRIC_VERSION.into(),
                kb_snapshot: cfg.vector.embed_model.clone(),
                instructions_hash: detail.instructions_hash(),
            },
        )?;
        if !is_new {
            // 报过失败、主编又重开再派工的：同一版本号、同一触发，轮次撞在同一行上。
            // 不重新拉起的话，任务在引擎里一直「进行中」、本地一直跳过（09-29 r57 #603 23:50 重派后挂了一夜）
            if r.status == "failed" && redispatched {
                conn.execute(
                    "UPDATE rounds SET status = 'running', ended_at = NULL, note = '报过失败后重新派工：再跑一次' WHERE id = ?1",
                    [r.id],
                )?;
                tracing::info!(
                    任务 = t.task.id,
                    轮次 = r.id,
                    "报过失败后又派了一次：再跑一次"
                );
            } else if r.status != "running" {
                // 本地已收尾（交了、等闸、报过失败），等引擎状态跟上
                tracing::debug!(任务 = t.task.id, 轮次 = r.id, 状态 = %r.status, "这一轮已经开过了");
                return Ok(());
            }
            // 开过、还 running，而本进程此刻没在跑它——只能是中途重启过。接着跑：
            // 各步按输入指纹复用已完成的部分，写引擎靠幂等键不会重复
            tracing::warn!(
                任务 = t.task.id,
                轮次 = r.id,
                "上次这一轮没跑完（进程中途重启过），接着跑"
            );
        }
        (r, is_new)
    };

    // 接单与心跳在 `tick` 里已经做了——见那儿的模块注释，
    // 放在这里会让同一批的第二个任务在第一个跑完前连 ack 都发不出去
    tracing::info!(任务 = t.task.id, 轮次 = r.id, 窗口 = %format!("{} ~ {}", r.window_start, r.window_end), "开工");
    // 「接单」这一步记一笔：ack 在 tick 里已经发了，这里只是让十步的第一格有着落——
    // 不记的话页面上它永远是「未开始」，看的人会以为连单都没接到
    if is_new
        && let Ok(st) =
            rounds::begin_step(conn, r.id, csw_collector_core::types::StepCode::Intake, "")
    {
        let _ = rounds::end_step(
            conn,
            st.id,
            csw_collector_core::types::StepStatus::Succeeded,
            &serde_json::json!({ "任务": t.task.id, "引擎状态": format!("{:?}", t.task.status) }),
            "",
        );
    }

    // 05／11 走的是另一条短得多的路：条目取单条 → 原图 → 识别 → 交付（0 闸）。
    // **不走合并、对照、判断、深核**——那件事 01 已经做过，而且是 Van 拍的板。
    if t.task.stage_code != STAGE_INTAKE {
        let r2 = r.clone();
        let out = if t.task.stage_code == item::STAGE_MATERIAL {
            item::run_material(conn, &r2, &detail, cfg, svc, engine).await
        } else {
            item::run_xhs_pick(conn, &r2, &detail, cfg, svc, engine).await
        };
        return match out {
            Ok(built) => {
                finish::submit(conn, &r, engine, t.task.id, &built).await?;
                rounds::finish_round(conn, r.id, "awaiting_review", "")?;
                Ok(())
            }
            Err(e) => {
                let why = format!("{e:#}");
                rounds::finish_round(conn, r.id, "failed", &why)?;
                // 带上这次派工的时间：报过失败、主编重开再派后同一轮再报失败，键不同才会真的生效
                let idem = format!(
                    "fail-{}-r{}-{}",
                    t.task.id,
                    r.id,
                    t.task.dispatched_at.replace([':', '-', '.'], "")
                );
                if let Err(e2) = engine.fail(t.task.id, &why, &idem).await {
                    tracing::error!(任务 = t.task.id, 原因 = %format!("{e2:#}"), "连失败都没报出去");
                }
                Err(e)
            }
        };
    }

    // 主编校准：历次退回意见写明的去向 + 引擎里主编亲手改的条目状态（后者优先）。
    // 返工套用它；首页首批卡按它排
    // 意见里只写了末 6 位的条目键，按本轮已判的条目展开
    let known: Vec<String> = in_place
        .then_some(r.id)
        .and_then(|id| {
            let mut st = conn
                .prepare("SELECT candidate_key FROM judgements WHERE round_id = ?1")
                .ok()?;
            let v = st
                .query_map([id], |row| row.get::<_, String>(0))
                .ok()?
                .filter_map(Result::ok)
                .collect();
            Some(v)
        })
        .unwrap_or_default();
    let mut calib = calibration::from_reviews_with_keys(&detail, &known);
    if in_place {
        match engine.intake_trace(t.task.run_id).await {
            Ok(tr) => calibration::from_traces(&tr, "editor", &mut calib),
            Err(e) => tracing::warn!("读引擎条目流转失败，只按退回意见校准：{e:#}"),
        }
    }
    let ran = if in_place {
        let end = engine_window
            .as_ref()
            .and_then(|(_, to)| parse_engine_ts(to));
        round::rework_in_place(conn, &r, &detail, cfg, svc, end, &calib).await
    } else {
        round::run_intake(conn, &r, &detail, cfg, svc).await
    };
    match ran {
        Ok((counts, fin)) => {
            tracing::info!(轮次 = r.id, ?counts, "这一轮的账");
            // 登记要先发出去，自查才查得到真东西
            let (sent, conflicts) = outbox_sender::drain(conn, engine).await.unwrap_or((0, 0));
            tracing::info!(发出 = sent, 冲突 = conflicts, "登记发完了");

            // 九、自查。**报红也要往下走**，把红灯写进交付物的缺口。
            let check = finish::self_check(conn, &r, engine, t.task.run_id).await;
            let mut gaps = finish::check_gaps(check.as_ref());
            if truncated {
                // 水位太旧、窗口被截了：这一轮真的少扫了一段，主编要看得见
                gaps.push(format!(
                    "水位太旧，这一轮的窗口被截到上限（{} 小时），\
                     更早的那一段没有扫——要补的话手动开一轮指定窗口",
                    csw_collector_core::window::MAX_LOOKBACK_HOURS
                ));
            }
            if !gaps.is_empty() {
                tracing::warn!(条数 = gaps.len(), "自查有没过的判据，会原样写进交付物");
            }

            // 深核补出来的缺口要并进台账：只留在线程日志里等于没人看得见
            let mut judgements = fin.judgements;
            let added = finish::merge_deepcheck_gaps(&mut judgements, &fin.deep_gaps);
            if added > 0 {
                tracing::info!(条数 = added, "深核补了几条缺口进台账");
            }

            // 本包与引擎逐键对账（登记已发出）。结果进自检与 trace/engine_reconcile.json
            // 自查里只因「披露早于窗口」没过的：口径冲突，写进自检并在首页说明，不当越界
            let conflict = finish::window_rule_conflict(check.as_ref());
            let (mut reconciled, mut lines, mut consistent) =
                finish::reconcile(conn, &r, engine, t.task.run_id, &judgements, &fin.topics).await;
            // 本包已撤、引擎仍挂着的：补发撤下，再对一次账
            if !consistent {
                match finish::heal_dropped(conn, &r, t.task.run_id, &reconciled) {
                    Ok(n) if n > 0 => {
                        let (sent, _) = outbox_sender::drain(conn, engine).await.unwrap_or((0, 0));
                        tracing::info!(
                            补撤 = n,
                            发出 = sent,
                            "对账：本包已撤、引擎仍挂着的补发撤下"
                        );
                        (reconciled, lines, consistent) = finish::reconcile(
                            conn,
                            &r,
                            engine,
                            t.task.run_id,
                            &judgements,
                            &fin.topics,
                        )
                        .await;
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("对账后补撤没排进去：{e:#}"),
                }
            }
            // 交付前引擎自查必须全绿（登记、补撤都发完之后再查一遍，状态才是最终的）。
            // 有红就不交：报失败，写明哪条判据、影响哪些条目——主编原话「intake-check 无红；
            // 若无法修复交真实错误与受影响条目清单，不盲升版」（09-30 r58 v18/v19 退回）
            if let Some(res) = finish::self_check(conn, &r, engine, t.task.run_id).await
                && !res.ok
            {
                let red: Vec<String> = res
                    .checks
                    .iter()
                    .filter(|c| !c.ok && !c.skipped)
                    .map(|c| {
                        format!(
                            "{}——{}",
                            c.name,
                            c.detail.chars().take(400).collect::<String>()
                        )
                    })
                    .collect();
                if !red.is_empty() && rework {
                    // 返工 / 重开：报失败会盖掉任务现状（10-07 r64 #720：主编另交的 v4、v5 被退回后，
                    // 工作台 30 秒内接走、自查红、报失败，前后五次）。红项写进缺口照交，由主编在闸上看
                    tracing::warn!(
                        任务 = t.task.id,
                        轮次 = r.id,
                        红项 = red.len(),
                        "返工版自查有红，写进缺口照交，不报失败"
                    );
                    gaps.extend(red.iter().map(|l| {
                        format!("引擎自查红项（返工版照交、不报失败，请主编在闸上裁定）：{l}")
                    }));
                } else if !red.is_empty() {
                    let why = format!(
                        "引擎自查（intake-check）未全绿，本版不提交：{}。登记已按本地判断写入引擎，交付包未提交",
                        red.join("；")
                    );
                    tracing::warn!(任务 = t.task.id, 轮次 = r.id, "{why}");
                    rounds::finish_round(conn, r.id, "failed", &why)?;
                    let idem = format!(
                        "fail-{}-r{}-check-{}",
                        t.task.id,
                        r.id,
                        t.task.dispatched_at.replace([':', '-', '.'], "")
                    );
                    if let Err(e) = engine.fail(t.task.id, &why, &idem).await {
                        tracing::error!(任务 = t.task.id, 原因 = %format!("{e:#}"), "自查未过，连失败都没报出去");
                    }
                    return Ok(());
                }
            }
            if !consistent {
                // 不一致才是红灯；一致的那两行只进自检
                gaps.extend(
                    lines
                        .iter()
                        .filter(|l| l.contains("不一致") || l.contains("没做成"))
                        .cloned(),
                );
            }

            // 七、交付物；十、提交
            lines.extend(conflict);
            // 首批卡优先放退回意见点名的（09-30 r58：「主编优先核 ZANE ARTS YOMA 召回、HILLS FIELD BIG TOP」）
            let review_text = detail
                .latest_review
                .as_ref()
                .map(|v| {
                    ["comment", "return_direction", "return_location"]
                        .iter()
                        .filter_map(|k| v.get(*k).and_then(|x| x.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            // 有主编校准就按校准的条目排卡（主编「八键不换」）；没有才按点名找
            let pinned: Vec<String> = if calib.is_empty() {
                round::named_in_review(&review_text, &judgements, &fin.by_key)
            } else {
                calibration::pin_order(&calib, calibration::asks_replacement(&review_text))
            };
            let calib_labels: HashMap<String, String> = calib
                .iter()
                .map(|c| {
                    (
                        c.key.clone(),
                        format!("主编校准：{}（{}）", c.label(), c.source),
                    )
                })
                .collect();
            // 返工补采可能改了窗口止点：页头时间、台账窗口按库里的最新值（09-30 r58 v2 还写着 06:00:12）
            let r = rounds::get(conn, r.id)?.unwrap_or(r);
            let five = five_columns(engine, t.task.run_id, &judgements, &fin.topics, &calib).await;
            let built = finish::build_deliverable(
                conn,
                &r,
                cfg,
                t.task.id,
                &judgements,
                &fin.topics,
                &fin.sweeps,
                &fin.by_key,
                &gaps,
                Some((&reconciled, &lines)),
                &pinned,
                &calib_labels,
                Some(five),
            )?;
            // 这一版的内容指纹：判断与选题（不含版本号）。返工算完与已交的一样就不重交——
            // 主编明说「修复前不重复整包重交相同缺陷」（09-28 r56 v4 退回后 30 秒交了一样的 v5）
            // 登记条目也算进去、再加导出格式修订号：维护者改了导出（格式、登记口径）就算有变化
            let items = register::item_inputs_by_topic(&judgements, &fin.topics, |k| {
                fin.by_key.get(k).cloned()
            });
            // 宽取留痕也算：事后补了留痕（window-trace）就是新内容，该重交
            let trace = csw_collector_core::window_trace::of_round(conn, r.id)
                .unwrap_or_default()
                .iter()
                .map(|x| format!("{}|{}|{}|{}", x.sweep_key, x.source_id, x.outcome, x.source))
                .collect::<Vec<_>>()
                .join("\n");
            // 交付包里还有原文与译文、补采记录、窗口起止：这些变了导出就变，也要算进来。
            // 10-01 r59：主编改了 C65 的译文，指纹只看判断与选题，返工五次都说「内容相同」没重交
            let sources = export_sources(conn, &r, &judgements);
            // 与引擎的逐键对账结果也算：引擎侧归属修好了（10-08 r66 v3：两条旧归属清掉），
            // 包里的对账与红灯就变了，该重交，不能当「内容相同」
            // 只取不一致的键（报告里还有核对时间，每次都变，不能进指纹）
            let recon = serde_json::to_string(&serde_json::json!([
                reconciled.pointer("/条目/不一致"),
                reconciled.pointer("/判断/不一致"),
            ]))
            .unwrap_or_default();
            // 这一轮在引擎里实际登记成的状态（包头「工作台登记」那行就是它）：引擎侧改回来了也是新内容
            //（10-09 r66 v7：两个产品重发成 shortlisted、包头计数变了，却因判断与选题没变被当成「内容相同」）
            let registered =
                serde_json::to_string(&register::registered_items(conn, r.id)).unwrap_or_default();
            let state = blake3::hash(
                format!(
                    "{EXPORT_REV}\u{1}{}\u{1}{}\u{1}{}\u{1}{trace}\u{1}{sources}\u{1}{recon}\u{1}{registered}",
                    serde_json::to_string(&judgements).unwrap_or_default(),
                    serde_json::to_string(&fin.topics).unwrap_or_default(),
                    serde_json::to_string(&items).unwrap_or_default()
                )
                .as_bytes(),
            )
            .to_hex()
            .to_string();
            if in_place && last_state.as_deref() == Some(state.as_str()) {
                tracing::warn!(
                    任务 = t.task.id,
                    轮次 = r.id,
                    "返工后内容与已交的一样，不重交：退回意见要的是维护者或主编那边的动作"
                );
                rounds::finish_round(conn, r.id, "awaiting_review", &unchanged_mark)?;
                // **不能悄悄等**：主编以为工作台还在改，工作台在等主编的新意见，两边一起停住
                //（09-29 r56 v8 退回后停了 11 个小时）。报失败并写明原因，引擎会 @ 主编；
                // 主编重开再派工，工作台就接着在原轮次上返工
                // 逐条说清哪几句意见没落地：只说「内容相同」，主编只能一遍遍重开（10-01 r59 连失败 5 次）
                let left = unhandled_review_lines(&review_text, &judgements);
                let why = format!(
                    "按 v{v} 的退回意见在原轮次返工后，交付内容与已交的 v{v} 完全相同，没有重交。\
                     工作台返工能自动做的只有：按「条目键=待核/备选/停止/继续」改档、点名条目定点重判、\
                     补采窗口缺的一段、收回来源没给媒体的图文帖、成员正文或译文改过的选题重算综合、\
                     按库里最新原文与译文重新导出。{left}\
                     请改成「条目键=去向」写法，或交维护方处理；本地轮次保留，重开派工后接着返工，不重新采集",
                    v = t.task.cur_version
                );
                let idem = format!(
                    "fail-{}-v{}-unchanged-{}",
                    t.task.id,
                    t.task.cur_version,
                    t.task.dispatched_at.replace([':', '-', '.'], "")
                );
                if let Err(e) = engine.fail(t.task.id, &why, &idem).await {
                    tracing::error!(任务 = t.task.id, 原因 = %format!("{e:#}"), "返工无变化，连失败都没报出去");
                }
                return Ok(());
            }
            finish::submit(conn, &r, engine, t.task.id, &built).await?;
            conn.execute(
                "UPDATE rounds SET submitted_state = ?2 WHERE id = ?1",
                rusqlite::params![r.id, state],
            )?;

            rounds::finish_round(
                conn,
                r.id,
                "awaiting_review",
                &format!("{SUBMITTED}{state}"),
            )?;
            Ok(())
        }
        Err(e) => {
            let why = format!("{e:#}");
            rounds::finish_round(conn, r.id, "failed", &why)?;
            csw_collector_core::alert::notify(
                svc.alert.as_ref(),
                &format!("r{} 任务 {} 这一轮失败：{why}", t.task.run_id, t.task.id),
            )
            .await;
            // 报失败不是可选的：不报的话主编在群里看到的一直是「还在做」
            // 幂等键从任务与轮次派生，重试不会重复报
            // 带上这次派工的时间：报过失败、主编重开再派后同一轮再报失败，键不同才会真的生效
            let idem = format!(
                "fail-{}-r{}-{}",
                t.task.id,
                r.id,
                t.task.dispatched_at.replace([':', '-', '.'], "")
            );
            if let Err(e2) = engine.fail(t.task.id, &why, &idem).await {
                tracing::error!(任务 = t.task.id, 原因 = %format!("{e2:#}"), "连失败都没报出去");
            }
            Err(e)
        }
    }
}

/// 这一轮的窗口。按**首次入库时间**算，左闭右开。
///
/// 水位是**上一轮派单轮的窗口终点**：上一轮晚开了两小时，这一轮就该多覆盖
/// 两小时，不然中间那段没人看过。没有水位时退回一天（周一退回三天）。
/// 判据全在 [`csw_collector_core::window`]，这里只负责把水位查出来。
/// 五栏与目标缺口的数。成熟只认主编在引擎里亲手定的（最近一次状态变化是中枢改的 shortlisted）。
async fn five_columns(
    engine: &EngineClient,
    run_id: i64,
    judgements: &[csw_collector_core::types::Judgement],
    topics: &[csw_collector_core::types::Topic],
    calib: &[calibration::Calibration],
) -> finish::FiveColumns {
    use csw_collector_core::types::Tier;
    let calibrated: std::collections::HashSet<&str> =
        calib.iter().map(|c| c.key.as_str()).collect();
    let tier_of: HashMap<&str, Tier> = judgements
        .iter()
        .map(|j| (j.candidate_key.as_str(), j.tier))
        .collect();
    let mut f = finish::FiveColumns {
        post_leads: judgements
            .iter()
            .filter(|j| matches!(j.tier, Tier::Recommend | Tier::Alternate))
            .count(),
        post_pending: judgements
            .iter()
            .filter(|j| j.tier == Tier::PendingCheck)
            .count(),
        calib_stop: calib
            .iter()
            .filter(|c| c.tier == Tier::NotRecommend)
            .count(),
        calib_pending: calib
            .iter()
            .filter(|c| c.tier == Tier::PendingCheck)
            .count(),
        calib_continue: calib
            .iter()
            .filter(|c| matches!(c.tier, Tier::Recommend | Tier::Alternate))
            .count(),
        target: engine.run_target(run_id).await.ok().flatten(),
        ..Default::default()
    };
    // 独立事件 = 选题；档位取代表帖的（主编校准过的已写进判断）
    for t in topics {
        let k = t.primary_key.as_str();
        match tier_of.get(k).copied().or(t.tier) {
            Some(Tier::Recommend | Tier::Alternate) if !calibrated.contains(k) => {
                f.lead_keys.push(k.to_string())
            }
            Some(Tier::PendingCheck) => f.pending_keys.push(k.to_string()),
            _ => {}
        }
    }
    f.leads = f.lead_keys.len();
    f.pending = f.pending_keys.len();
    if let Ok(tr) = engine.intake_trace(run_id).await {
        let mut last: HashMap<String, String> = HashMap::new();
        for t in tr
            .get("traces")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if let (Some(k), Some(r)) = (
                t.get("item_key").and_then(|v| v.as_str()),
                t.get("actor_role").and_then(|v| v.as_str()),
            ) {
                last.insert(k.to_string(), r.to_string());
            }
        }
        for it in tr
            .get("items")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let key = it.get("item_key").and_then(|v| v.as_str()).unwrap_or("");
            match it.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                "approved_write" | "written" => f.van_keys.push(key.to_string()),
                "shortlisted" if last.get(key).is_some_and(|r| r == "editor") => {
                    f.mature_keys.push(key.to_string());
                    if it.get("rank").and_then(|v| v.as_str()) == Some("alt") {
                        f.mature_alt += 1;
                    } else {
                        f.mature_primary += 1;
                    }
                }
                _ => {}
            }
        }
        f.van_approved = f.van_keys.len();
    }
    f
}

fn window_for(
    conn: &rusqlite::Connection,
    engine: Option<&(String, String)>,
) -> (String, String, bool) {
    let last = rounds::last_task_window_end(conn)
        .unwrap_or(None)
        .and_then(|s| s.parse::<jiff::Timestamp>().ok());
    let engine_start = engine.and_then(|(from, _)| parse_engine_ts(from));
    let mut w = csw_collector_core::window::for_task(jiff::Timestamp::now(), engine_start, last);
    // 止点：引擎窗口的止点已经过了就止于它——取消生产限时不等于扩大采集窗口
    //（09-28 r56 12:18 触发、窗口止于 07:00，工作台扫到了开工那一刻，被主编退回）
    if let Some(e) = engine.and_then(|(_, to)| parse_engine_ts(to))
        && e < w.end
        && e > w.start
    {
        w.end = e;
    }
    if w.truncated {
        // 截断不许悄悄发生：它意味着这一轮少扫了一段
        tracing::warn!(
            窗口小时 = w.hours(),
            "水位太旧，窗口被截到上限。这一轮会少扫一段，**要写进交付物的缺口**"
        );
    }
    (w.start.to_string(), w.end.to_string(), w.truncated)
}

/// 轮次备注里记「已交的内容指纹」与「返工后没变、未重交」的前缀
const SUBMITTED: &str = "已交状态:";
const UNCHANGED: &str = "内容与上一版相同，未重交";
/// 交付物 / 登记导出格式的修订号。**改了导出（字段、登记口径、包内文件）就改它**，
/// 否则返工后内容指纹一样，修好的导出不会重交
const EXPORT_REV: &str = "2026-10-01a";

/// 引擎派的窗口止点是否已经过了。读不出来的当已过：不能因为格式问题永远不开工。
fn window_closed(to: &str, now: jiff::Timestamp) -> bool {
    parse_engine_ts(to).is_none_or(|end| end <= now)
}

/// 引擎窗口的时刻写法是 `2026-09-25T07:00+08:00`（没有秒），先按 RFC 3339 读，读不了补上秒再读。
fn parse_engine_ts(s: &str) -> Option<jiff::Timestamp> {
    s.parse::<jiff::Timestamp>().ok().or_else(|| {
        // 在时区偏移前补 ":00"
        let i = s.rfind(['+', '-']).filter(|i| *i > 10)?;
        format!("{}:00{}", &s[..i], &s[i..]).parse().ok()
    })
}

/// 任务的派工时间是否晚于本地那一轮的收尾时间。取不到时间就当不是（保守：不重复开轮）。
fn redispatched_after(conn: &rusqlite::Connection, round_id: i64, dispatched_at: &str) -> bool {
    let ended: Option<String> = conn
        .query_row(
            "SELECT ended_at FROM rounds WHERE id = ?1",
            [round_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    match (
        ended.and_then(|s| s.parse::<jiff::Timestamp>().ok()),
        dispatched_at.parse::<jiff::Timestamp>().ok(),
    ) {
        (Some(e), Some(d)) => d > e,
        _ => false,
    }
}

/// 磁盘到了拒开新轮的水位吗。到了就给一句能直接发给主编的话。
fn disk_blocked(cfg: &Config) -> Option<String> {
    let pct = csw_collector_core::disk::used_pct(&cfg.data_dir);
    let lv =
        csw_collector_core::disk::level(pct, cfg.limits.disk_warn_pct, cfg.limits.disk_block_pct);
    let note =
        csw_collector_core::disk::note(pct, cfg.limits.disk_warn_pct, cfg.limits.disk_block_pct);
    match lv {
        csw_collector_core::disk::Level::Block => {
            Some(format!("{note}，这一轮不开。先清理磁盘再让主编重开任务"))
        }
        csw_collector_core::disk::Level::Warn => {
            tracing::warn!("{note}");
            None
        }
        csw_collector_core::disk::Level::Ok => None,
    }
}

fn outbox_conflicts(conn: &rusqlite::Connection) -> Result<Vec<i64>> {
    let mut st =
        conn.prepare("SELECT seq FROM engine_outbox WHERE status='conflict' ORDER BY seq")?;
    Ok(st
        .query_map([], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect())
}

/// 退回意见里没有点到任何条目键、也不是「条目键=去向」写法的句子：工作台返工落不了地的那几句。
/// 至多列 4 句、每句 60 字，写进失败原因。
fn unhandled_review_lines(review: &str, js: &[csw_collector_core::types::Judgement]) -> String {
    let keys: Vec<String> = js.iter().map(|j| j.candidate_key.to_lowercase()).collect();
    let lines: Vec<String> = review
        .split(['\n', '；', '。'])
        .map(str::trim)
        .filter(|l| l.chars().count() >= 8)
        .filter(|l| {
            let low = l.to_lowercase();
            !keys.iter().any(|k| low.contains(k.as_str()))
        })
        .map(|l| l.chars().take(60).collect::<String>())
        .take(4)
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!(
            "以下意见不在这些范围内，没能自动落地：「{}」。",
            lines.join("」「")
        )
    }
}

/// 交付包里判断与选题之外、但会进导出的输入：各条的原文与译文、补采记录、窗口起止。
fn export_sources(
    conn: &rusqlite::Connection,
    r: &csw_collector_core::rounds::Round,
    judgements: &[csw_collector_core::types::Judgement],
) -> String {
    let mut out = format!("{}|{}", r.window_start, r.window_end);
    let mut st = match conn
        .prepare_cached("SELECT text, translated FROM candidates WHERE candidate_key = ?1")
    {
        Ok(s) => s,
        Err(_) => return out,
    };
    for j in judgements {
        if let Ok((t, tr)) = st.query_row([&j.candidate_key], |x| {
            Ok((x.get::<_, String>(0)?, x.get::<_, String>(1)?))
        }) {
            out.push_str(&format!("\u{1}{}|{t}|{tr}", j.candidate_key));
        }
    }
    for p in csw_collector_core::window_trace::patches_of(conn, r.id).unwrap_or_default() {
        out.push_str(&format!("\u{1}{p:?}"));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn 窗口止点没到不开工() {
        let now: jiff::Timestamp = "2026-10-08T19:08:31Z".parse().unwrap(); // 北京 03:08
        assert!(!window_closed("2026-10-09T06:00+08:00", now));
        assert!(window_closed("2026-10-09T03:00+08:00", now));
        assert!(window_closed("看不懂", now), "读不出来的不能永远不开工");
    }

    #[test]
    fn 引擎窗口的时刻没有秒也读得出来() {
        let t = super::parse_engine_ts("2026-09-25T07:00+08:00").unwrap();
        assert_eq!(
            t,
            "2026-09-24T23:00:00Z".parse::<jiff::Timestamp>().unwrap()
        );
        assert!(super::parse_engine_ts("2026-09-25T07:00:00+08:00").is_some());
        assert!(super::parse_engine_ts("不是时间").is_none());
    }

    use super::*;

    #[test]
    fn 返工落不了地的意见逐句列出() {
        use csw_collector_core::types::{Judgement, Tier};
        let js = vec![Judgement::fixture("drlv-cf21b4", Tier::Recommend)];
        let s = unhandled_review_lines(
            "drlv-cf21b4=待核；窗口摘要仍与 v3 字节相同。andwander 无图应待核\n短句",
            &js,
        );
        assert!(s.contains("窗口摘要仍与 v3 字节相同"));
        assert!(s.contains("andwander 无图应待核"));
        assert!(!s.contains("drlv"), "点到条目键的已经按校准处理");
        assert!(!s.contains("短句"));
        assert_eq!(unhandled_review_lines("", &js), "");
    }

    #[test]
    fn 监听所有网卡时探的是回环() {
        // 生产配置就是 0.0.0.0（nginx 在容器里，经 172.17.0.1 访问宿主）。
        // 不换算的话，CLI 会去连 http://0.0.0.0:8090，探不到就以为服务没跑，
        // 于是两个进程一起对着同一个库干活。
        assert_eq!(local_addr("0.0.0.0:8090"), "127.0.0.1:8090");
        assert_eq!(local_addr("[::]:8090"), "127.0.0.1:8090");
        // 已经是具体地址的原样不动
        assert_eq!(local_addr("127.0.0.1:18090"), "127.0.0.1:18090");
        assert_eq!(local_addr("192.168.1.9:8090"), "192.168.1.9:8090");
    }
    /// 同一批派来的任务，**每一个都要在开跑之前就接下来**。
    ///
    /// 这条钉的是一个真会发假警报的 bug：接单原本写在 `start_round` 里，
    /// 而跑一轮是阻塞的，于是第二个任务在第一个跑完之前连 ack 都没有。
    /// 05 是逐条阶段，Van 批 7 条就派 7 个任务——后面六个必然被引擎判
    /// 「派工后未接单」，5 分钟提醒、15 分钟升级给中枢。
    ///
    /// 测法：让 `/me/tasks` 返回三个派单，任务详情一律 400（4xx 不重试，
    /// 三轮都在取详情那步就断了，不真跑轮），然后数 ack 的次数。
    /// 接单发生在取详情之前，所以三个都该有。
    #[tokio::test]
    async fn 同一批派单在开跑之前就全接下来了() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let srv = wiremock::MockServer::start().await;
        let tasks = serde_json::json!({"tasks": (1..=3).map(|i| serde_json::json!({
            "task": {"id": i, "run_id": 48, "stage_code": "intake", "status": "dispatched",
                     "cur_version": 1, "due_at": "2099-01-01T00:00:00Z"}
        })).collect::<Vec<_>>()});
        Mock::given(method("GET"))
            .and(path("/api/v1/me/tasks"))
            // 引擎客户端不解信封（那是 csw 的格式），直接就是结构体
            .respond_with(ResponseTemplate::new(200).set_body_json(tasks))
            .mount(&srv)
            .await;
        // 接单都收下
        Mock::given(method("POST"))
            .and(path("/api/v1/tasks/1/ack"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&srv)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/tasks/2/ack"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&srv)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/tasks/3/ack"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&srv)
            .await;
        // 任务详情 400：4xx 不重试，三轮都在这步断掉——而那发生在接单之后
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "code": "bad_request", "message": "测试里不给详情"
            })))
            .mount(&srv)
            .await;

        let engine = EngineClient::new(
            &format!("{}/api/v1", srv.uri()),
            "t",
            Duration::from_secs(5),
        )
        .unwrap();
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("csw-tick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        // 磁盘水位闸读的是真机器的磁盘：开发机一满（09-23 用到 92%）这类测试就
        // 因「拒开新轮」挂掉，测的却是接单与续跑。水位闸另有自己的测试，这里关掉它
        cfg.limits.disk_warn_pct = 101;
        cfg.limits.disk_block_pct = 101;
        let secrets = Secrets {
            csw_api_key: "k".into(),
            sub2api_key: "k".into(),
            ..Default::default()
        };
        let svc = services::Services::build(&cfg, &secrets, &conn)
            .await
            .unwrap();
        let mut beats = std::collections::HashMap::new();
        let _ = tick(&conn, &engine, &cfg, &svc, &mut beats).await;

        let acked: std::collections::HashSet<String> = srv
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/ack"))
            .map(|r| r.url.path().to_string())
            .collect();
        assert_eq!(
            acked.len(),
            3,
            "三个都该在开跑前接下来，实际只接了 {acked:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 引擎说「在做」时：本地有 running 的轮次就续跑，没有就不碰。
    ///
    /// 这条钉的是 `Continue` 原来的 bug：注释说续跑，实现只看时限。进程在早上那一轮
    /// 中途重启，这一期就烂到时限。测法：派一条 in_progress 的任务，详情一律 400，
    /// **看它有没有去取详情**——取了就是走进了续跑。
    async fn continue_touches_task(local_running: bool, tag: &str) -> bool {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let srv = wiremock::MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me/tasks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tasks": [
                {"task": {"id": 5, "run_id": 48, "stage_code": "intake", "status": "in_progress",
                          "cur_version": 1, "due_at": "2099-01-01T00:00:00Z"}}
            ]})))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "code": "bad_request", "message": "测试里不给详情"
            })))
            .mount(&srv)
            .await;

        let engine = EngineClient::new(
            &format!("{}/api/v1", srv.uri()),
            "t",
            Duration::from_secs(5),
        )
        .unwrap();
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        if local_running {
            rounds::open_round(
                &conn,
                &rounds::NewRound {
                    kind: csw_collector_core::types::RoundKind::Task,
                    trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                    run_id: Some(48),
                    task_id: Some(5),
                    stage_code: Some("intake".into()),
                    target_version: 1,
                    parent_round_id: None,
                    window_start: "A".into(),
                    window_end: "B".into(),
                    plan_version: 1,
                    rubric_version: "v".into(),
                    kb_snapshot: "k".into(),
                    instructions_hash: String::new(),
                },
            )
            .unwrap();
        }
        let dir = std::env::temp_dir().join(format!("csw-cont-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        // 磁盘水位闸读的是真机器的磁盘：开发机一满（09-23 用到 92%）这类测试就
        // 因「拒开新轮」挂掉，测的却是接单与续跑。水位闸另有自己的测试，这里关掉它
        cfg.limits.disk_warn_pct = 101;
        cfg.limits.disk_block_pct = 101;
        let secrets = Secrets {
            csw_api_key: "k".into(),
            sub2api_key: "k".into(),
            ..Default::default()
        };
        let svc = services::Services::build(&cfg, &secrets, &conn)
            .await
            .unwrap();
        let mut beats = std::collections::HashMap::new();
        let _ = tick(&conn, &engine, &cfg, &svc, &mut beats).await;
        let touched = srv
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method.as_str() == "GET" && r.url.path() == "/api/v1/tasks/5");
        let _ = std::fs::remove_dir_all(&dir);
        touched
    }

    #[tokio::test]
    async fn 重启后本地还running的派单轮会接着跑() {
        assert!(
            continue_touches_task(true, "resume").await,
            "本地那一轮还 running，引擎说在做——该接着跑，却什么都没做"
        );
    }

    #[tokio::test]
    async fn 本地没有的在做任务不接手() {
        // agent 行与 Hermes 共用：引擎说在做、本地没有、本进程也没接过，多半是它接的
        assert!(!continue_touches_task(false, "notours").await);
    }

    #[tokio::test]
    async fn 一期作废后挂着的返工轮收尾成已取消() {
        use csw_collector_core::types::{RoundKind, RoundTrigger};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        for (run, st) in [(55, "aborted"), (56, "active")] {
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/runs/{run}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"run": {"id": run, "status": st}})),
                )
                .mount(&srv)
                .await;
        }
        let engine = EngineClient::new(
            &format!("{}/api/v1", srv.uri()),
            "t",
            Duration::from_secs(5),
        )
        .unwrap();
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let open = |run: i64, task: i64| {
            rounds::open_round(
                &conn,
                &rounds::NewRound {
                    kind: RoundKind::Task,
                    trigger: RoundTrigger::Returned,
                    run_id: Some(run),
                    task_id: Some(task),
                    stage_code: Some("intake".into()),
                    target_version: 2,
                    parent_round_id: None,
                    window_start: "2026-09-25T00:00:00Z".into(),
                    window_end: "2026-09-28T00:00:00Z".into(),
                    plan_version: 1,
                    rubric_version: "v".into(),
                    kb_snapshot: "k".into(),
                    instructions_hash: String::new(),
                },
            )
            .unwrap()
            .0
        };
        let dead = open(55, 581); // 一期作废
        let alive = open(56, 592); // 一期还在进行
        let reopened = open(55, 582); // 作废的一期里又被重开、还挂在派单列表上
        close_orphans(&conn, &engine).await.unwrap();

        let st = |id| rounds::get(&conn, id).unwrap().unwrap();
        assert_eq!(st(dead.id).status, "cancelled");
        assert!(st(dead.id).note.contains("r55 已作废"));
        assert_eq!(st(alive.id).status, "running");
        assert_eq!(st(reopened.id).status, "cancelled");
    }
}
