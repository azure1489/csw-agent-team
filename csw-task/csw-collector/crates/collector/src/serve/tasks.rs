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

use anyhow::Result;

use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::{MyTask, TaskStatus};

/// 我们接哪些阶段的单。其余的不是我们的活，看见也不碰。
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

/// 时限前多久就主动报失败。**不要挂着等超时**——
/// 挂着的话主编在群里看到的是「还在做」，而实际上已经做不完了。
pub const FAIL_BEFORE_DUE_SECS: i64 = 5 * 60;

/// 现在该不该主动报失败。`due_at` 是引擎给的 RFC3339。
pub fn should_fail_early(due_at: &str, now: jiff::Timestamp) -> bool {
    let Ok(due) = due_at.parse::<jiff::Timestamp>() else {
        return false;
    };
    (due.as_second() - now.as_second()) <= FAIL_BEFORE_DUE_SECS
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

    #[test]
    fn 时限前五分钟就主动报失败() {
        let now: jiff::Timestamp = "2026-09-18T06:00:00Z".parse().unwrap();
        // 挂着的话主编看到的是「还在做」，而实际上已经做不完了
        assert!(should_fail_early("2026-09-18T06:04:00Z", now));
        assert!(
            should_fail_early("2026-09-18T05:59:00Z", now),
            "已经过了更要报"
        );
        assert!(!should_fail_early("2026-09-18T06:30:00Z", now));
        // 引擎没给时限就不猜
        assert!(!should_fail_early("", now));
        assert!(!should_fail_early("不是时间", now));
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
}
