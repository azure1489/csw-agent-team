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

pub mod finish;
pub mod http;
pub mod item;
pub mod outbox_sender;
pub mod register;
pub mod round;
pub mod schedule;
pub mod services;
pub mod tasks;
pub mod write;

use std::time::Duration;

use anyhow::{Context, Result};

use csw_collector_core::rounds;
use csw_collector_core::{Config, Secrets};
use csw_collector_engineapi::client::EngineClient;

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
    let svc = services::Services::build(cfg, secrets, &conn).await?;

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
    });
    let auth = std::sync::Arc::new(crate::bff::AuthState {
        engine: csw_collector_engineapi::admin::AdminClient::new(&cfg.engine.admin_url)?,
        store: Default::default(),
        van_usernames: cfg.web.van_usernames.clone(),
        secure_cookie: cfg.web.secure_cookie,
    });
    let app = axum::Router::new()
        .merge(http::ops_router(app_state.clone()))
        .merge(http::api_router(app_state.clone()))
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
    loop {
        if let Err(e) = tick(&conn, &engine, cfg, &svc).await {
            tracing::warn!(原因 = %format!("{e:#}"), "这一轮轮询没跑完");
        }
        let now_min = schedule::now_minute();
        for name in schedule::due(&jobs, prev_min, now_min) {
            run_job(name, cfg, &conn, &svc, now_min).await;
        }
        // 工作台排进来的活。**一次只做一件**：这个循环是单线程的，
        // 取两件也只能一件一件做，而多出来的那件会在 running 上挂着假装在跑。
        if let Err(e) = run_queued(cfg, &conn, &svc).await {
            tracing::warn!(原因 = %format!("{e:#}"), "排队的活没做成");
        }
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

/// 手动开一轮：只跑第 2–5 步，**不写引擎**。它是拿来看的，不是拿来交的。
async fn manual_round(
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
            (
                (now - jiff::Span::new().days(days)).to_string(),
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
        "kb_sync_1" | "kb_sync_2" => kb_sync_job(cfg, conn, svc).await,
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
    let (window_start, window_end) = window_for(cfg);
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

async fn kb_sync_job(
    cfg: &Config,
    conn: &rusqlite::Connection,
    svc: &services::Services,
) -> Result<String> {
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
    Ok(format!(
        "算了 {} 条向量、失败 {}、还剩 {}",
        rep.embedded, rep.failed, rep.remaining
    ))
}

/// 轮询一次。
async fn tick(
    conn: &rusqlite::Connection,
    engine: &EngineClient,
    cfg: &Config,
    svc: &services::Services,
) -> Result<()> {
    let mine = engine.my_tasks().await.context("取派单")?;
    for (t, action) in tasks::plan(&mine.tasks) {
        match action {
            Action::Start | Action::Rework => {
                // 一个任务出错不该让别的任务也不跑
                if let Err(e) = start_round(conn, engine, cfg, svc, t, action).await {
                    tracing::error!(任务 = t.task.id, 原因 = %format!("{e:#}"), "这一轮没跑起来");
                }
            }
            Action::Continue => {
                // 时限快到了就主动报失败，不挂着等超时——
                // 挂着的话主编在群里看到的是「还在做」，而实际上已经做不完了
                if tasks::should_fail_early(&t.task.due_at, jiff::Timestamp::now()) {
                    let why = format!(
                        "距时限不足 {} 分钟仍未完成，主动报失败，条目与台账留在本地可重开",
                        tasks::FAIL_BEFORE_DUE_SECS / 60
                    );
                    tracing::warn!(任务 = t.task.id, 时限 = %t.task.due_at, "{why}");
                    let idem = format!("fail-{}-due", t.task.id);
                    if let Err(e) = engine.fail(t.task.id, &why, &idem).await {
                        tracing::error!(任务 = t.task.id, 原因 = %format!("{e:#}"), "连失败都没报出去");
                    }
                } else {
                    tracing::debug!(任务 = t.task.id, "还在跑，继续");
                }
            }
            Action::Wait => tracing::debug!(任务 = t.task.id, "等闸，只轮询"),
            Action::Ignore => {}
        }
    }
    let (sent, conflicts) = outbox_sender::drain(conn, engine).await?;
    if sent > 0 {
        tracing::info!(发出 = sent, 冲突 = conflicts, "发了几条");
    }
    Ok(())
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
    let (window_start, window_end) = window_for(cfg);
    let (r, is_new) = rounds::open_round(
        conn,
        &rounds::NewRound {
            kind: csw_collector_core::types::RoundKind::Task,
            trigger: if action == Action::Rework {
                csw_collector_core::types::RoundTrigger::Returned
            } else {
                csw_collector_core::types::RoundTrigger::Dispatch
            },
            run_id: Some(t.task.run_id),
            task_id: Some(t.task.id),
            stage_code: Some(t.task.stage_code.clone()),
            target_version: i64::from(t.task.cur_version.max(1)),
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
        // 轮询把同一条派单读到两次是常态，不是错误
        tracing::debug!(任务 = t.task.id, 轮次 = r.id, "这一轮已经开过了");
        return Ok(());
    }

    engine.ack(t.task.id).await.context("接单")?;
    let _beat = tasks::start_heartbeat(
        engine.clone(),
        t.task.id,
        Duration::from_secs(cfg.engine.ack_secs.max(30)),
    );
    tracing::info!(任务 = t.task.id, 轮次 = r.id, 窗口 = %format!("{} ~ {}", r.window_start, r.window_end), "开工");

    // 05／11 走的是另一条短得多的路：条目取单条 → 原图 → 识别 → 交付（0 闸）。
    // **不走合并、对照、判断、深核**——那件事 01 已经做过，而且是 Van 拍的板。
    if t.task.stage_code != STAGE_INTAKE {
        let r2 = r.clone();
        let out = if t.task.stage_code == item::STAGE_MATERIAL {
            item::run_material(conn, &r2, &detail, cfg, svc).await
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
                let idem = format!("fail-{}-r{}", t.task.id, r.id);
                if let Err(e2) = engine.fail(t.task.id, &why, &idem).await {
                    tracing::error!(任务 = t.task.id, 原因 = %format!("{e2:#}"), "连失败都没报出去");
                }
                Err(e)
            }
        };
    }

    match round::run_intake(conn, &r, &detail, cfg, svc).await {
        Ok((counts, fin)) => {
            tracing::info!(轮次 = r.id, ?counts, "这一轮的账");
            // 登记要先发出去，自查才查得到真东西
            let (sent, conflicts) = outbox_sender::drain(conn, engine).await.unwrap_or((0, 0));
            tracing::info!(发出 = sent, 冲突 = conflicts, "登记发完了");

            // 九、自查。**报红也要往下走**，把红灯写进交付物的缺口。
            let check = finish::self_check(conn, &r, engine, t.task.run_id).await;
            let gaps = finish::check_gaps(check.as_ref());
            if !gaps.is_empty() {
                tracing::warn!(条数 = gaps.len(), "自查有没过的判据，会原样写进交付物");
            }

            // 深核补出来的缺口要并进台账：只留在线程日志里等于没人看得见
            let mut judgements = fin.judgements;
            let added = finish::merge_deepcheck_gaps(&mut judgements, &fin.deep_gaps);
            if added > 0 {
                tracing::info!(条数 = added, "深核补了几条缺口进台账");
            }

            // 七、交付物；十、提交
            let built = finish::build_deliverable(
                conn,
                &r,
                cfg,
                t.task.id,
                &judgements,
                &fin.sweeps,
                &fin.by_key,
                &gaps,
            )?;
            finish::submit(conn, &r, engine, t.task.id, &built).await?;

            rounds::finish_round(conn, r.id, "awaiting_review", "")?;
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
            let idem = format!("fail-{}-r{}", t.task.id, r.id);
            if let Err(e2) = engine.fail(t.task.id, &why, &idem).await {
                tracing::error!(任务 = t.task.id, 原因 = %format!("{e2:#}"), "连失败都没报出去");
            }
            Err(e)
        }
    }
}

/// 这一轮的窗口。按**首次入库时间**算，左闭右开。
///
/// 默认是「上一次到现在」，但没有上一次时退回一天——
/// 第一期不该因为没有水位就去扫一整年。
fn window_for(_cfg: &Config) -> (String, String) {
    let now = jiff::Timestamp::now();
    let from = now - jiff::Span::new().hours(24);
    (from.to_string(), now.to_string())
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
