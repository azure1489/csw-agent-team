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
pub mod outbox_sender;
pub mod register;
pub mod round;
pub mod schedule;
pub mod services;
pub mod tasks;

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
    tracing::info!(中断的步 = n, 还在跑的轮 = running.len(), "启动自检");

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
    if !outbox_conflicts(&conn)?.is_empty() {
        // 冲突要人核实，不该被下一轮盖过去
        tracing::error!("有 outbox 条目处在冲突状态，**先查任务状态对账**，不要换幂等键重试");
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
        .merge(http::api_router(app_state))
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
        prev_min = now_min;
        tokio::time::sleep(every).await;
    }
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
        "prefetch" => prefetch_job(cfg, conn, svc).await,
        other => Err(anyhow::anyhow!("不认识的定时活：{other}")),
    };
    match r {
        Ok(note) => tracing::info!(活 = name, "{note}"),
        Err(e) => tracing::warn!(活 = name, 原因 = %format!("{e:#}"), "这件活没跑成，下一次再来"),
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
    // 05 配图与 11 选图包也是我们的活，但还**没实现**。在这里岔开：
    // 不岔开的话它们会走 01 的十步，交出一份文不对题的台账——
    // 那种错在群播报里看不出来，要等主编打开交付物才发现。
    if t.task.stage_code != STAGE_INTAKE {
        let why = format!(
            "{} 阶段本服务尚未实现（只做 {STAGE_INTAKE}），请主编改派人工或等版本上线",
            t.task.stage_code
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

fn outbox_conflicts(conn: &rusqlite::Connection) -> Result<Vec<i64>> {
    let mut st =
        conn.prepare("SELECT seq FROM engine_outbox WHERE status='conflict' ORDER BY seq")?;
    Ok(st
        .query_map([], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect())
}
