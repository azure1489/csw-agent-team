//! 任务驱动：轮询派单 → 开轮 → 接单 → 心跳。
//!
//! # 派单是流程的起点，不是「有缓存就跳过」
//!
//! 预取轮先跑过一遍、结果都在缓存里，这一轮照样要从**接单**开始：
//! 引擎是唯一真相，没接单就没有这一轮。缓存只影响「跑得快不快」，
//! 不影响「跑不跑」。
//!
//! # 心跳就是重复 ack
//!
//! 引擎没有独立的心跳接口，重复 `POST /tasks/:id/ack` 就是心跳
//! （实测确认）。心跳**不带幂等键**——带了的话第二次就被当成重放，
//! 引擎那边看到的还是第一次的时间。

use std::collections::HashSet;
use std::time::Duration;

use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::{MyTask, TaskStatus};

/// 我们接哪些阶段的单。其余的不是我们的活，看见也不碰。
///
/// **认得不等于做得了**：目前只有 `intake` 真跑得起来，另外两个会被
/// `serve::start_round` 挡下并**如实向引擎报失败**（见那里的分支）。
/// 写在这里而不是删掉，是因为「派到我们头上却没人应」比「明说做不了」更糟：
/// 前者要等引擎的未接单提醒升级到中枢，主编才知道出事了。
pub const OUR_STAGES: [&str; 3] = ["intake", "material", "xhs_pick"];

/// 这一条派单该怎么处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 新派的单：开一轮，接单，从头跑
    Start,
    /// 已经接了、还在跑：续跑并继续心跳
    Continue,
    /// 退回返工：开新一轮（挂在原来那轮下面）
    Rework,
    /// 等闸，只轮询不干活
    Wait,
    /// 终态或不是我们的活
    Ignore,
}

/// 看一条任务该干什么。
///
/// **`review` 是等闸，不是「做完了」**：这时候要继续轮询，因为闸可能退回。
pub fn decide(t: &MyTask) -> Action {
    if !OUR_STAGES.contains(&t.task.stage_code.as_str()) {
        return Action::Ignore;
    }
    match t.task.status {
        TaskStatus::Dispatched => Action::Start,
        TaskStatus::InProgress => Action::Continue,
        TaskStatus::Returned => Action::Rework,
        TaskStatus::Review => Action::Wait,
        // ready 是还没派到我们头上；其余是终态
        _ => Action::Ignore,
    }
}

/// 引擎说「这单在做」（in_progress）时，本进程该干什么。
///
/// # 为什么要有它
///
/// `Continue` 原本只检查时限——注释写着「续跑并继续心跳」，实现里两样都没做。
/// 进程在早上那一轮中途重启的话：步被标成 interrupted、轮次留在 running，
/// 引擎那边任务还是 in_progress，`start_round` 又对同一任务返回「已经开过了」——
/// **没有任何一处会把它接着跑完**，心跳也断了，一直烂到时限前 5 分钟主动报失败。
///
/// # 四种情形
///
/// | 本地这一任务的轮次 | 本进程接过这单 | 做什么 |
/// |---|---|---|
/// | 有，running | — | 续跑，沿用原轮次的触发方式 |
/// | 没有 | 是 | 重试开轮：接了单、开轮前失败了（取详情偶发出错之类） |
/// | 有，已收尾 | — | 等引擎状态跟上，什么都不做 |
/// | 没有 | 否 | **不接手**：收集员的 agent 行与 Hermes 共用，多半是它接的 |
///
/// 第二行是「接单挪到 `tick` 第一段」之后才需要的：改之前开轮失败时任务还是
/// dispatched，下一轮会当新单重试；改之后它已经是 in_progress 了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnContinue {
    /// 续跑。`true` = 原轮次是返工触发的
    Resume { returned: bool },
    /// 重新开轮（按新派单）
    Retry,
    /// 本地已收尾，等引擎
    Wait,
    /// 不是我们开的，不碰
    NotOurs,
}

/// `local` = 本地最近一轮的（状态, 触发方式）；`acked_here` = 本进程这次运行里接过这单。
pub fn on_continue(local: Option<(&str, &str)>, acked_here: bool) -> OnContinue {
    match local {
        Some(("running", trigger)) => OnContinue::Resume {
            returned: trigger == "returned",
        },
        Some(_) => OnContinue::Wait,
        None if acked_here => OnContinue::Retry,
        None => OnContinue::NotOurs,
    }
}

/// 心跳器：接单后每隔一会儿重复 ack 一次。
///
/// 停下的方式是丢掉返回的 `Beat`——它一被 drop，后台那个循环就退出。
pub struct Beat {
    stop: tokio::sync::watch::Sender<bool>,
}

impl Drop for Beat {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// 开始心跳。`every` 取 `engine.ack_secs`。
pub fn start_heartbeat(engine: EngineClient, task_id: i64, every: Duration) -> Beat {
    let (stop, mut rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(every) => {
                    // 心跳失败不该让这一轮停下——它只是让引擎那边看起来久了点
                    if let Err(e) = engine.ack(task_id).await {
                        tracing::warn!(任务 = task_id, 原因 = %format!("{e:#}"), "心跳没发出去");
                    }
                }
                _ = rx.changed() => break,
            }
            if *rx.borrow() {
                break;
            }
        }
        tracing::debug!(任务 = task_id, "心跳停了");
    });
    Beat { stop }
}

/// 把这一批派单按处理方式分好。**同一个任务只留一条**——
/// 轮询接口偶尔会把同一条读到两次。
pub fn plan(tasks: &[MyTask]) -> Vec<(&MyTask, Action)> {
    let mut seen = HashSet::new();
    tasks
        .iter()
        .filter(|t| seen.insert(t.task.id))
        .map(|t| (t, decide(t)))
        .filter(|(_, a)| *a != Action::Ignore)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_engineapi::types::Task;

    fn task(id: i64, stage: &str, status: TaskStatus) -> MyTask {
        MyTask {
            task: Task {
                id,
                run_id: 48,
                stage_code: stage.into(),
                stage_name: String::new(),
                role_code: "collector".into(),
                status,
                item_key: String::new(),
                action_class: String::new(),
                due_at: String::new(),
                dispatched_at: String::new(),
                cur_version: 1,
                sla_minutes: 30,
                rework_pending: false,
            },
            run_subject: String::new(),
        }
    }

    #[test]
    fn 只接我们那三个阶段() {
        assert_eq!(
            decide(&task(1, "intake", TaskStatus::Dispatched)),
            Action::Start
        );
        assert_eq!(
            decide(&task(1, "material", TaskStatus::Dispatched)),
            Action::Start
        );
        assert_eq!(
            decide(&task(1, "xhs_pick", TaskStatus::Dispatched)),
            Action::Start
        );
        // 别人的活看见也不碰
        assert_eq!(
            decide(&task(1, "write", TaskStatus::Dispatched)),
            Action::Ignore
        );
        assert_eq!(
            decide(&task(1, "select", TaskStatus::Dispatched)),
            Action::Ignore
        );
    }

    #[test]
    fn 等闸时要继续轮询而不是收工() {
        // review 是等闸，不是「做完了」——闸可能退回
        assert_eq!(decide(&task(1, "intake", TaskStatus::Review)), Action::Wait);
        assert_eq!(
            decide(&task(1, "intake", TaskStatus::Returned)),
            Action::Rework
        );
        for s in [
            TaskStatus::Passed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
            TaskStatus::Ready,
        ] {
            assert_eq!(decide(&task(1, "intake", s)), Action::Ignore, "{s:?}");
        }
    }

    #[test]
    fn 同一个任务读到两次只算一条() {
        // 轮询接口偶尔会把同一条派单读到两次
        let ts = [
            task(7, "intake", TaskStatus::Dispatched),
            task(7, "intake", TaskStatus::Dispatched),
            task(8, "material", TaskStatus::InProgress),
        ];
        let p = plan(&ts);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].1, Action::Start);
        assert_eq!(p[1].1, Action::Continue);
    }

    #[test]
    fn 不是我们的活不进计划() {
        let ts = [task(1, "write", TaskStatus::Dispatched)];
        assert!(plan(&ts).is_empty());
    }

    #[tokio::test]
    async fn 心跳丢掉就停() {
        let e = EngineClient::new("http://127.0.0.1:1", "t", Duration::from_millis(50)).unwrap();
        let beat = start_heartbeat(e, 1, Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(beat);
        // 不崩、不卡住就算过；心跳失败只 warn 不影响这一轮
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[test]
    fn 引擎说在做时本进程该干什么() {
        // 进程中途重启过：本地那一轮还 running，接着跑
        assert_eq!(
            on_continue(Some(("running", "dispatch")), false),
            OnContinue::Resume { returned: false }
        );
        // 返工轮续跑时要沿用「返工」，否则会按新派单另开一轮、原来那轮永远挂着
        assert_eq!(
            on_continue(Some(("running", "returned")), false),
            OnContinue::Resume { returned: true }
        );
        // 接了单、开轮前出错：本地没有轮次，但确实是我们接的——重试
        assert_eq!(on_continue(None, true), OnContinue::Retry);
        // 本地已收尾（交了、等闸、报过失败）：等引擎状态跟上
        for st in ["awaiting_review", "done", "failed"] {
            assert_eq!(on_continue(Some((st, "dispatch")), false), OnContinue::Wait);
        }
        // 本地没有、也不是本进程接的：agent 行与 Hermes 共用，多半是它接的——不碰
        assert_eq!(on_continue(None, false), OnContinue::NotOurs);
    }
}
