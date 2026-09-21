//! 采集器：取候选这一段的可插拔实现。
//!
//! 采集器**只负责取候选并归一成统一格式**；下载、识别、向量化由 [`crate::pipeline`]
//! 统一做——加一个新采集器不用管后面三段，也不该管。
//!
//! 计数由代码算，不由采集器自报：`found` / `fetched_unique` / `in_window` 三个数
//! 是 `intake-check` 的对账依据，让实现方自己填等于让它自己给自己打分。

use std::collections::HashSet;

use anyhow::Result;
use csw_collector_core::types::{Candidate, Timestamp};

/// 一次采集的产出。
#[derive(Debug, Default)]
pub struct Harvested {
    /// 归一后的候选，**未去重、未按窗口过滤**——那两件事由流水线统一做
    pub candidates: Vec<Candidate>,
    /// 接口返回的原始条数（含重复）。取不到就填候选数。
    pub found: i64,
    /// 是否翻到了最后一页。没翻完就说明可能漏，`intake-check` 要看这个。
    pub paged_to_end: bool,
    /// 找了什么：关键词、账号、窗口，写给人看的
    pub query: String,
}

/// 一次采集的结果，失败也是一种结果——**失败要如实上报，不能当没找到**。
pub enum Outcome {
    Ok(Harvested),
    /// 必扫采集器失败会让整步停下；非必扫的记一笔继续
    Failed(String),
    /// 开关没开
    Disabled,
}

/// 采集器。
#[allow(async_fn_in_trait)]
pub trait Collector {
    /// 采集轮的键，`(run_id, sweep_key)` 幂等
    fn sweep_key(&self) -> String;
    /// instagram | xhs | web | other
    fn platform(&self) -> &'static str;
    fn source_key(&self) -> String {
        "channel".to_string()
    }
    /// 必扫的采集器失败会让整步停下并告警，**不用旧接口兜底**
    fn required(&self) -> bool {
        false
    }
    /// 窗口是 UTC 的左闭右开区间，按**首次入库时间**算
    async fn collect(&self, from: Timestamp, to: Timestamp) -> Outcome;
}

/// csw 贴文窗口——主力采集器。
pub struct CswWindow {
    pub client: std::sync::Arc<crate::csw::CswClient>,
    /// 取候选时往前多看几天：接口只按发布时间过滤，宽取再本地按入库时间收口
    pub lookback_days: i64,
}

impl Collector for CswWindow {
    fn sweep_key(&self) -> String {
        "csw-window".into()
    }
    fn platform(&self) -> &'static str {
        "instagram"
    }
    fn required(&self) -> bool {
        true
    }

    async fn collect(&self, from: Timestamp, to: Timestamp) -> Outcome {
        // 接口按发布时间过滤，所以把起点往前推；终点也放宽一天，
        // 免得「发布时间晚于入库时间」这种时钟偏差把边界上的条目挡在外面
        let start = date_of(from - days(self.lookback_days));
        let end = date_of(to + days(1));
        match self.client.window(&start, &end).await {
            Ok(raw) => {
                let found = raw.len() as i64;
                let candidates = raw
                    .iter()
                    .map(|p| crate::csw::to_candidate(p, "csw_window"))
                    .collect::<Vec<_>>();
                Outcome::Ok(Harvested {
                    candidates,
                    found,
                    paged_to_end: true, // window() 内部翻到 has_more=false 才返回
                    query: format!("发布时间 {start}~{end}（本地按入库时间 {from}~{to} 收口）"),
                })
            }
            Err(e) => Outcome::Failed(format!("{e:#}")),
        }
    }
}

/// Van 手动补的链接。**当期采用的常常全部来自这里**，所以它和窗口采集器同等重要。
pub struct VanLinks {
    pub client: std::sync::Arc<crate::csw::CswClient>,
    /// 短码列表
    pub short_codes: Vec<String>,
}

impl Collector for VanLinks {
    fn sweep_key(&self) -> String {
        "van-links".into()
    }
    fn platform(&self) -> &'static str {
        "instagram"
    }

    async fn collect(&self, _from: Timestamp, _to: Timestamp) -> Outcome {
        if self.short_codes.is_empty() {
            return Outcome::Ok(Harvested {
                query: "Van 本期没有补链接".into(),
                paged_to_end: true,
                ..Default::default()
            });
        }
        let mut out = Vec::new();
        let mut failed = Vec::new();
        for code in &self.short_codes {
            match self.client.post(code).await {
                Ok(p) => out.push(crate::csw::to_candidate(&p, "van_link")),
                Err(e) => failed.push(format!("{code}：{e:#}")),
            }
        }
        // Van 补的链接一条都不能丢：取不到就报失败，让人去看，而不是悄悄少一条
        if !failed.is_empty() {
            return Outcome::Failed(format!("Van 补的链接有取不到的：{}", failed.join("；")));
        }
        let n = out.len() as i64;
        Outcome::Ok(Harvested {
            candidates: out,
            found: n,
            paged_to_end: true,
            query: format!("Van 补的 {n} 条链接"),
        })
    }
}

/// 开关关着的采集器：实现了但默认不开（小红书、网页、自定义三层）。
///
/// 做成一个类型而不是在编排里写 if：**开关状态要出现在采集轮里**，
/// 否则「没扫」与「没开」在台账上看着一样。
pub struct Disabled {
    pub key: &'static str,
    pub platform: &'static str,
    pub why: &'static str,
}

impl Collector for Disabled {
    fn sweep_key(&self) -> String {
        self.key.into()
    }
    fn platform(&self) -> &'static str {
        self.platform
    }
    async fn collect(&self, _: Timestamp, _: Timestamp) -> Outcome {
        tracing::info!(采集器 = self.key, 原因 = self.why, "采集器未启用");
        Outcome::Disabled
    }
}

/// 按（平台，来源 id）去重，**保留先出现的那条**。
///
/// 为什么保留先出现的：采集器按顺序跑，窗口采集器在前、Van 链接在后；
/// 同一条贴文两边都取到时，窗口那条带着完整的媒体列表，更该留。
pub fn dedup(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|c| seen.insert((c.platform, c.source_id.clone())))
        .collect()
}

fn date_of(t: Timestamp) -> String {
    t.strftime("%Y-%m-%d").to_string()
}

/// `Timestamp` 上不能加减日历单位（jiff 的规定：那要先挂上时区）。
/// 我们内部一律 UTC，一天就是 24 小时，换成小时即可。
fn days(n: i64) -> jiff::Span {
    jiff::Span::new().hours(n * 24)
}

/// 解析结果里的日期，供测试与日志用
pub fn utc(s: &str) -> Result<Timestamp> {
    Ok(s.parse::<Timestamp>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{MediaKind, MediaRef, Platform};

    fn cand(id: &str, collector: &str) -> Candidate {
        Candidate {
            candidate_key: format!("k-{id}"),
            platform: Platform::Instagram,
            source_id: id.into(),
            collector: collector.into(),
            account: "a".into(),
            url: String::new(),
            text: String::new(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![MediaRef {
                source_hash: "h".into(),
                kind: MediaKind::Photo,
                url: "u".into(),
                blake3: None,
                ordinal: 0,
            }],
            tags: vec![],
            hashtags: vec![],
        }
    }

    #[test]
    fn 去重保留先出现的那条() {
        let got = dedup(vec![
            cand("1", "csw_window"),
            cand("2", "csw_window"),
            cand("1", "van_link"),
        ]);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0].collector, "csw_window",
            "窗口那条带着完整媒体列表，该留它"
        );
    }

    #[test]
    fn 取候选的发布时间窗口比目标窗口宽() {
        let from = utc("2026-09-22T00:00:00Z").unwrap();
        assert_eq!(date_of(from - days(7)), "2026-09-15");
        assert_eq!(date_of(from + days(1)), "2026-09-23");
        // 跨月与跨年也要对
        let y = utc("2026-01-02T00:00:00Z").unwrap();
        assert_eq!(date_of(y - days(7)), "2025-12-26");
    }

    #[tokio::test]
    async fn 关着的采集器要在台账上留下痕迹() {
        let d = Disabled {
            key: "xhs",
            platform: "xhs",
            why: "默认关闭",
        };
        assert!(matches!(
            d.collect(
                utc("2026-09-22T00:00:00Z").unwrap(),
                utc("2026-09-23T00:00:00Z").unwrap()
            )
            .await,
            Outcome::Disabled
        ));
        assert_eq!(d.sweep_key(), "xhs");
    }

    #[test]
    fn 只有窗口采集器是必扫的() {
        let d = Disabled {
            key: "web",
            platform: "web",
            why: "",
        };
        assert!(!d.required());
    }
}
