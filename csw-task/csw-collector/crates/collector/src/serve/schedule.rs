//! 进程内定时：预取轮、知识库同步、夜间维护。
//!
//! # 为什么不用 systemd timer
//!
//! 这些活要和正式轮**抢同一块 GPU 和同一个网关闸门**。放进同一个进程，
//! 闸门就只有一套；拆成外部 timer 的话，两个进程各拿各的信号量，
//! 8 并发立刻变成 16——0.4 实测那样会把 429 打满，而墙钟一点没降。
//!
//! # 预取轮是必需的，不是优化
//!
//! 0.4/0.5 实测：一轮从零跑网关要四十分钟、GPU 要二十多分钟。
//! 05:30 接单再从头跑，首批 ≤20 分钟的目标不可能达成。
//! 所以 01:40 先跑一遍第 2–5 步，结果写本地缓存；正式轮按 `input_hash`
//! 命中即复用，只补增量。
//!
//! **但它只是缓存。** 没有缓存整轮也能从零跑完，只是慢——而且要
//! **如实告诉主编会迟到**，不能假装一切正常。

use anyhow::Result;

/// 一件定时的活。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub name: &'static str,
    /// UTC 的 `HH:MM`
    pub at: String,
}

/// 从 `HH:MM` 解析成当天的第几分钟。解析不了就返回 None——
/// **不猜**：配错了时刻宁可这件活不跑，也不要在一个随机时间跑。
pub fn minute_of_day(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// 上次检查到现在之间，这个时刻过去了吗。
///
/// 用「区间是否覆盖」而不是「现在是不是等于」：进程被调度晚了一分钟、
/// 或者一次 tick 跑了两分钟，按相等判就会**整天不跑**。
/// 跨午夜的区间也要认（`prev` 比 `now` 大的时候）。
pub fn fired(prev_min: u32, now_min: u32, at_min: u32) -> bool {
    if prev_min == now_min {
        return false;
    }
    if prev_min < now_min {
        at_min > prev_min && at_min <= now_min
    } else {
        // 跨了午夜
        at_min > prev_min || at_min <= now_min
    }
}

/// 现在是不是在「把 GPU 让给正式轮」的安静窗口里。
///
/// 知识库回填这类活在这段时间要停手——0.5 实测两边抢卡会把双方
/// 都拖到四倍延迟，那比任何一边单独跑都糟。
pub fn in_quiet_window(from: &str, to: &str, now_min: u32) -> bool {
    let (Some(f), Some(t)) = (minute_of_day(from), minute_of_day(to)) else {
        return false;
    };
    if f <= t {
        now_min >= f && now_min < t
    } else {
        now_min >= f || now_min < t
    }
}

/// 现在是当天的第几分钟（UTC）。
pub fn now_minute() -> u32 {
    let now = jiff::Timestamp::now();
    let secs = now.as_second().rem_euclid(86_400);
    (secs / 60) as u32
}

/// 这一次 tick 该跑哪些活。
pub fn due(jobs: &[Job], prev_min: u32, now_min: u32) -> Vec<&'static str> {
    jobs.iter()
        .filter(|j| minute_of_day(&j.at).is_some_and(|m| fired(prev_min, now_min, m)))
        .map(|j| j.name)
        .collect()
}

/// 从配置拼出这一份活单。
pub fn jobs_from(cfg: &csw_collector_core::Config) -> Vec<Job> {
    let mut v = vec![Job {
        name: "prefetch",
        at: cfg.schedule.prefetch_at_utc.clone(),
    }];
    for (i, at) in cfg.schedule.kb_sync_at_utc.iter().enumerate() {
        // 名字带序号，好在日志里分清是哪一次
        v.push(Job {
            name: if i == 0 { "kb_sync_1" } else { "kb_sync_2" },
            at: at.clone(),
        });
    }
    v
}

/// 图片保留期到了的，删掉。返回删了几个。
///
/// **只删图，不删记录**：`media` 表里那一行要留着，
/// 否则「这条当时有几张图」就查不回来了。
pub fn sweep_old_media(blob_dir: &std::path::Path, keep_days: u32) -> Result<usize> {
    let cutoff = std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(
        u64::from(keep_days) * 86_400,
    ));
    let Some(cutoff) = cutoff else { return Ok(0) };
    let mut n = 0;
    for a in std::fs::read_dir(blob_dir).into_iter().flatten().flatten() {
        if !a.path().is_dir() {
            continue;
        }
        for f in std::fs::read_dir(a.path()).into_iter().flatten().flatten() {
            let old = f
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t < cutoff)
                .unwrap_or(false);
            if old && std::fs::remove_file(f.path()).is_ok() {
                n += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 时刻解析不了就不跑() {
        assert_eq!(minute_of_day("01:40"), Some(100));
        assert_eq!(minute_of_day(" 23:59 "), Some(1439));
        assert_eq!(minute_of_day("00:00"), Some(0));
        // 配错了宁可这件活不跑，也不要在一个随机时间跑
        assert_eq!(minute_of_day("24:00"), None);
        assert_eq!(minute_of_day("01:60"), None);
        assert_eq!(minute_of_day("乱写"), None);
        assert_eq!(minute_of_day(""), None);
    }

    #[test]
    fn 晚了一分钟也要跑不能整天不跑() {
        // 按「现在是不是等于」判的话，进程被调度晚一分钟就整天不跑了
        assert!(fired(99, 101, 100), "跨过去了就该跑");
        assert!(fired(99, 100, 100));
        assert!(!fired(100, 101, 100), "刚跑过不该再跑");
        assert!(!fired(101, 105, 100));
        // 一次 tick 跑了十分钟也要补上
        assert!(fired(95, 105, 100));
    }

    #[test]
    fn 跨午夜也要认() {
        // prev 在 23:55，now 在 00:05，01:40 还没到
        assert!(!fired(1435, 5, 100));
        // 00:02 的活，跨午夜时要跑
        assert!(fired(1435, 5, 2));
        // 23:58 落在 23:55 → 00:05 这段里，也要跑
        assert!(fired(1435, 5, 1438));
        // 23:55 是区间左端（开），刚跑过不该再跑
        assert!(!fired(1435, 5, 1435));
    }

    #[test]
    fn 同一分钟内重复检查不会重复跑() {
        assert!(!fired(100, 100, 100));
    }

    #[test]
    fn 安静窗口跨午夜也算得对() {
        // 05:25 ~ 06:30
        assert!(in_quiet_window("05:25", "06:30", 5 * 60 + 30));
        assert!(!in_quiet_window("05:25", "06:30", 5 * 60 + 20));
        assert!(!in_quiet_window("05:25", "06:30", 6 * 60 + 30), "右开");
        // 22:00 ~ 02:00 这种跨午夜的
        assert!(in_quiet_window("22:00", "02:00", 23 * 60));
        assert!(in_quiet_window("22:00", "02:00", 60));
        assert!(!in_quiet_window("22:00", "02:00", 12 * 60));
        // 配错了就当没有安静窗口，别把所有活都卡住
        assert!(!in_quiet_window("乱写", "02:00", 60));
    }

    #[test]
    fn 活单从配置拼出来() {
        let mut cfg = csw_collector_core::Config::default();
        cfg.schedule.prefetch_at_utc = "01:40".into();
        cfg.schedule.kb_sync_at_utc = vec!["02:30".into(), "14:30".into()];
        let jobs = jobs_from(&cfg);
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].name, "prefetch");
        // 名字带序号，好在日志里分清是哪一次
        assert_eq!(jobs[1].name, "kb_sync_1");
        assert_eq!(jobs[2].name, "kb_sync_2");

        assert_eq!(due(&jobs, 99, 101), ["prefetch"]);
        assert_eq!(due(&jobs, 149, 151), ["kb_sync_1"]);
        assert!(due(&jobs, 200, 210).is_empty());
    }

    #[test]
    fn 过期的图删掉新的留着() {
        let dir = tempdir::TempDir::new("blobs").unwrap();
        let sub = dir.path().join("ab");
        std::fs::create_dir_all(&sub).unwrap();
        let f = sub.join("abcdef.jpg");
        std::fs::write(&f, b"x").unwrap();
        // 刚写的不该被删
        assert_eq!(sweep_old_media(dir.path(), 120).unwrap(), 0);
        assert!(f.exists());
        // 保留期 0 天：全过期
        assert_eq!(sweep_old_media(dir.path(), 0).unwrap(), 1);
        assert!(!f.exists());
    }

    #[test]
    fn 目录不在也不报错() {
        assert_eq!(
            sweep_old_media(std::path::Path::new("/没有这个目录"), 30).unwrap(),
            0
        );
    }
}
