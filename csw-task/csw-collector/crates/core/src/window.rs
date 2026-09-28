//! 一轮的时间窗口。
//!
//! # 三条口径，每一条都吃过亏
//!
//! **一、按首次入库时间（`ingestedAt`）算，不按发布时间。** 一条三天前发的贴文
//! 今天才进 csw 的库，它对我们就是今天的新东西；按发布时间算会把它永远漏掉。
//!
//! **二、左闭右开。** `[start, end)`。两轮之间用同一个时刻当边界，
//! 闭区间会让边界上那一条被判两次——台账上出现两行一模一样的条目，
//! 而「为什么有两行」没人查得清。
//!
//! **三、内部一律 UTC，只有「今天周几」这件事按北京时间判。**
//! 流程是按北京时间跑的（05:30 接单），而周一要回溯到上周五。
//! 用固定 +8 偏移而不是时区数据库：中国不实行夏令时，`+08:00` 一年到头都对，
//! 而少一份对 tzdb 的依赖就少一种「目标机上没有 /usr/share/zoneinfo」的死法。

use jiff::{Span, Timestamp, tz::Offset};

/// 没有水位时回溯多久。第一期不该因为没有水位就去扫一整年。
pub const DEFAULT_HOURS: i64 = 24;

/// 周一没有水位时回溯多久。**周五晚到周日发的那些，周一是第一次有机会看见。**
pub const MONDAY_HOURS: i64 = 72;

/// 水位再旧也不往回扫超过这么久。
///
/// 服务停了好几周再起来时，按水位算的窗口会是几千条候选——那一轮跑不完，
/// 而跑不完比少扫几条更糟。**截断要标出来**，不许悄悄发生。
///
/// 216 小时（9 天）盖得住国庆：10-01 ~ 10-07 放假，10-08 那一期的水位是 09-30 05:30，
/// 回溯约 8 天。原来的 96 小时连中秋都卡边（09-28 那期水位正好 96 小时 14 秒前，
/// 截掉 14 秒，交付物里却写成「更早的那一段没有扫」）。
pub const MAX_LOOKBACK_HOURS: i64 = 216;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: Timestamp,
    pub end: Timestamp,
    /// 水位太旧、被 [`MAX_LOOKBACK_HOURS`] 截断了。
    /// **要写进轮次备注与交付物缺口**，不是内部细节。
    pub truncated: bool,
}

impl Window {
    /// 这个时刻在窗口里吗。左闭右开。
    pub fn contains(&self, t: Timestamp) -> bool {
        t >= self.start && t < self.end
    }
    pub fn hours(&self) -> f64 {
        (self.end.as_second() - self.start.as_second()) as f64 / 3600.0
    }
}

/// 算这一轮的窗口。
///
/// `last_end` 是上一轮的终点。**它是水位，不是「昨天这个点」**：
/// 上一轮晚开了两小时，这一轮就该多覆盖两小时，不然中间那段没人看过。
/// `n` 天，换算成小时。
///
/// **`Timestamp` 上不能加减日历单位**——jiff 的规定是那要先挂上时区，
/// 直接 `Span::new().days(n)` 会 **panic**（不是返回 Err）。
/// 我们内部一律 UTC，一天就是 24 小时。
///
/// 这个陷阱踩过两次：`harvest::collector` 那处躲过了，
/// `serve::manual_round` 那处没躲过——工作台上点一次「开始一轮」就能把服务打挂。
/// 所以把它放在这里，让两边引同一个。
pub fn days(n: i64) -> Span {
    Span::new().hours(n * 24)
}

pub fn for_round(now: Timestamp, last_end: Option<Timestamp>) -> Window {
    let floor = now - Span::new().hours(MAX_LOOKBACK_HOURS);
    let (start, truncated) = match last_end {
        // 水位在未来：钟走回去了，或者库被手改过。不信它，按默认回溯。
        Some(t) if t >= now => (default_start(now), false),
        Some(t) if t < floor => (floor, true),
        Some(t) => (t, false),
        None => (default_start(now), false),
    };
    Window {
        start,
        end: now,
        truncated,
    }
}

/// 派单轮的窗口：**引擎派的窗口起点**与**上一轮终点（水位）**取更早的那个，终点是现在。
///
/// 只按水位算，同一天重新触发的一期（或前一期被作废）会只剩几小时：09-28 r54 被作废后
/// 重新触发的 r55，水位是 r54 的 05:30，窗口只剩 05:30~08:16，交出一个 0 条的空包。
/// 只按引擎窗口算，每天 05:30 开工、引擎窗口止于 07:00，05:30~07:00 进库的那段下一期
/// 从 07:00 算起就永远没人扫。两个取早的，两头都盖住。回溯上限照旧。
pub fn for_task(
    now: Timestamp,
    engine_start: Option<Timestamp>,
    last_end: Option<Timestamp>,
) -> Window {
    let Some(es) = engine_start.filter(|t| *t < now) else {
        return for_round(now, last_end);
    };
    // 水位在未来（钟走回去、库被手改）不信它，只看引擎窗口
    let want = match last_end.filter(|t| *t < now) {
        Some(mark) => es.min(mark),
        None => es,
    };
    let floor = now - Span::new().hours(MAX_LOOKBACK_HOURS);
    let (start, truncated) = if want < floor {
        (floor, true)
    } else {
        (want, false)
    };
    Window {
        start,
        end: now,
        truncated,
    }
}

fn default_start(now: Timestamp) -> Timestamp {
    now - Span::new().hours(default_hours(now))
}

/// 没有水位时回溯多久。周一多回溯两天。
fn default_hours(now: Timestamp) -> i64 {
    if is_monday_in_beijing(now) {
        MONDAY_HOURS
    } else {
        DEFAULT_HOURS
    }
}

/// 北京时间的今天是不是周一。
///
/// 固定 +8 偏移：中国不实行夏令时，这一行一年到头都对，
/// 而且不必假设目标机上有时区数据库。
fn is_monday_in_beijing(t: Timestamp) -> bool {
    let z = t.to_zoned(Offset::constant(8).to_time_zone());
    z.weekday() == jiff::civil::Weekday::Monday
}

#[cfg(test)]
mod tests {
    /// `Timestamp` 上加减日历单位会 **panic**，不是返回 Err。
    ///
    /// 这条钉的是一个真 bug：`serve::manual_round` 曾经直接写
    /// `now - Span::new().days(days)`，而那正是工作台「开始一轮」走的路——
    /// 在页面上点一下就能把常驻服务打挂（panic 不是 Err，`run_queued` 的
    /// `match r` 接不住它）。
    #[test]
    fn 按天回溯不能用日历单位() {
        let now: Timestamp = "2026-09-22T05:30:00Z".parse().unwrap();
        // 我们的换算：一天 = 24 小时
        let a = now - days(3);
        assert_eq!(a.to_string(), "2026-09-19T05:30:00Z");
        // 而直接用日历单位会 panic——钉住这个事实，免得有人"顺手改回去"
        let boom = std::panic::catch_unwind(|| now - Span::new().days(3));
        assert!(boom.is_err(), "jiff 不再 panic 了？那就该回来简化 days()");
    }

    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn 左闭右开() {
        let w = for_round(ts("2026-09-22T21:30:00Z"), Some(ts("2026-09-21T21:30:00Z")));
        // 两轮之间用同一个时刻当边界，闭区间会让边界上那一条被判两次
        assert!(w.contains(w.start));
        assert!(!w.contains(w.end));
        assert!(w.contains(ts("2026-09-22T00:00:00Z")));
        assert!(!w.contains(ts("2026-09-21T21:29:59Z")));
    }

    #[test]
    fn 有水位就从水位起算() {
        // 上一轮晚开了两小时，这一轮就该多覆盖两小时
        let w = for_round(ts("2026-09-22T21:30:00Z"), Some(ts("2026-09-21T23:30:00Z")));
        assert_eq!(w.start, ts("2026-09-21T23:30:00Z"));
        assert_eq!(w.hours(), 22.0);
        assert!(!w.truncated);
    }

    #[test]
    fn 没有水位就回溯一天() {
        // 周二
        let w = for_round(ts("2026-09-22T21:30:00Z"), None);
        assert_eq!(w.start, ts("2026-09-21T21:30:00Z"));
        assert_eq!(w.hours(), DEFAULT_HOURS as f64);
    }

    #[test]
    fn 周一没有水位就回溯到上周五() {
        // 2026-09-21 是周一（北京时间）。UTC 21:30 = 北京 22 日 05:30 —— 周二了
        let monday_beijing = ts("2026-09-20T21:30:00Z"); // 北京 09-21 05:30，周一
        assert!(is_monday_in_beijing(monday_beijing), "这个时刻北京是周一");
        let w = for_round(monday_beijing, None);
        // 周五晚到周日发的那些，周一是第一次有机会看见
        assert_eq!(w.hours(), MONDAY_HOURS as f64);

        // 前一天同一时刻（北京周日）就只回溯一天
        let sunday_beijing = ts("2026-09-19T21:30:00Z");
        assert!(!is_monday_in_beijing(sunday_beijing));
        assert_eq!(
            for_round(sunday_beijing, None).hours(),
            DEFAULT_HOURS as f64
        );
    }

    #[test]
    fn 判周几按北京不按utc() {
        // 北京 09-21 00:30（周一）对应 UTC 09-20 16:30（周日）。
        // 按 UTC 判就会漏掉周一那次回溯
        let t = ts("2026-09-20T16:30:00Z");
        assert!(is_monday_in_beijing(t), "北京已经是周一了");
    }

    #[test]
    fn 水位太旧要截断而且要标出来() {
        let now = ts("2026-09-22T21:30:00Z");
        // 服务停了两周
        let w = for_round(now, Some(ts("2026-09-08T21:30:00Z")));
        assert_eq!(w.hours(), MAX_LOOKBACK_HOURS as f64);
        // 截断要标出来，不许悄悄发生
        assert!(w.truncated);
    }

    #[test]
    fn 同一天重新触发的一期按引擎窗口起点扫() {
        // r54 被作废后 08:16 重新触发 r55：水位是 r54 的 05:30，引擎窗口从 09-25 07:00 起
        let w = for_task(
            ts("2026-09-28T00:16:58Z"),
            Some(ts("2026-09-24T23:00:00Z")),
            Some(ts("2026-09-27T21:30:21Z")),
        );
        assert_eq!(w.start, ts("2026-09-24T23:00:00Z"));
        assert!(!w.truncated);
    }

    #[test]
    fn 平常一天水位更早就从水位起_不漏五点半到七点() {
        // 昨天 05:30 开工，今天引擎窗口从昨天 07:00 起：取水位 05:30
        let w = for_task(
            ts("2026-09-29T21:30:00Z"),
            Some(ts("2026-09-28T23:00:00Z")),
            Some(ts("2026-09-28T21:30:00Z")),
        );
        assert_eq!(w.start, ts("2026-09-28T21:30:00Z"));
    }

    #[test]
    fn 没有引擎窗口就按水位() {
        let now = ts("2026-09-29T21:30:00Z");
        let last = Some(ts("2026-09-28T21:30:00Z"));
        assert_eq!(for_task(now, None, last), for_round(now, last));
    }

    #[test]
    fn 国庆长假回来那一期不截() {
        // 10-01 ~ 10-07 放假：10-08 05:30 那期的水位是 09-30 05:30
        let w = for_round(ts("2026-10-07T21:30:00Z"), Some(ts("2026-09-29T21:30:00Z")));
        assert!(!w.truncated);
        assert_eq!(w.hours(), 192.0);
    }

    #[test]
    fn 水位在未来就不信它() {
        let now = ts("2026-09-22T21:30:00Z");
        // 钟走回去了，或者库被手改过
        let w = for_round(now, Some(ts("2026-09-23T00:00:00Z")));
        assert_eq!(w.start, ts("2026-09-21T21:30:00Z"));
        assert!(!w.truncated);
        assert!(w.start < w.end, "不信它之后也不能算出一个倒着的窗口");
        // 水位正好等于现在：也当成「没有可用水位」，否则窗口是空的
        let w = for_round(now, Some(now));
        assert!(w.hours() > 0.0);
    }

    #[test]
    fn 任何输入都算不出倒着的窗口() {
        let now = ts("2026-09-22T21:30:00Z");
        for last in [
            None,
            Some(ts("2020-01-01T00:00:00Z")),
            Some(ts("2026-09-22T21:29:59Z")),
            Some(now),
            Some(ts("2030-01-01T00:00:00Z")),
        ] {
            let w = for_round(now, last);
            assert!(w.start < w.end, "{last:?} 算出了倒着的窗口");
            assert!(
                w.hours() <= MAX_LOOKBACK_HOURS as f64 + 0.001,
                "{last:?} 超了上限"
            );
            // 同一份输入算两次必须一样——窗口进轮次记录，随机的窗口没法复用也没法对账
            assert_eq!(w, for_round(now, last));
        }
    }
}
