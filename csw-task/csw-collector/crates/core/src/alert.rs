//! 开发群告警：**只发这个服务自己的毛病**，流程上的事一律由引擎播报。
//!
//! # 为什么需要它
//!
//! 引擎会把派工、待审、退回、失败都播到编辑部群，但它只知道流程里的事。
//! 「磁盘 93%」「outbox 有条目卡在冲突上」「预取轮连着两天没跑成」——
//! 这些引擎压根不知道，而它们恰恰是**只有我们自己能发现**的。
//! 不发出去，就只有能 ssh 上服务器的人看得见。
//!
//! # 三条界线
//!
//! 1. **不往编辑部群发。** 这是给运维看的，发到编辑部群只会变成噪音，
//!    而噪音多了真事就没人看了。地址由配置给，默认空＝关。
//! 2. **不发内容，只发状态。** 候选正文、Van 原话、条目标题一律不进告警——
//!    群里的人不需要，而第三方机器人的消息记录不在我们手里。
//! 3. **发不出去不算错。** 告警失败只 warn，绝不让一轮活因为通知失败而停。

use std::time::Duration;

use anyhow::Result;

/// 一条告警。
pub struct Alerter {
    url: String,
    http: reqwest::Client,
}

impl Alerter {
    /// `webhook_url` 为空就返回 None——默认关着，开之前要有人把地址填进配置。
    pub fn new(webhook_url: &str) -> Option<Self> {
        let url = webhook_url.trim();
        if url.is_empty() {
            return None;
        }
        crate::ensure_crypto_provider();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .ok()?;
        Some(Self {
            url: url.to_string(),
            http,
        })
    }

    /// 发一条。**飞书自定义机器人的文本格式。**
    pub async fn send(&self, text: &str) -> Result<()> {
        let body = serde_json::json!({
            "msg_type": "text",
            "content": { "text": text }
        });
        let resp = self.http.post(&self.url).json(&body).send().await?;
        let status = resp.status();
        anyhow::ensure!(status.is_success(), "告警接口回了 {status}");
        Ok(())
    }
}

/// 发一条，发不出去只 warn。**告警失败绝不让一轮活停下。**
pub async fn notify(alerter: Option<&Alerter>, text: &str) {
    let Some(a) = alerter else {
        tracing::info!("（没配告警地址，这条只进日志）{text}");
        return;
    };
    if let Err(e) = a.send(text).await {
        tracing::warn!(原因 = %format!("{e:#}"), "告警没发出去：{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 没配地址就是关着的() {
        assert!(Alerter::new("").is_none());
        assert!(Alerter::new("   ").is_none());
        assert!(Alerter::new("https://open.feishu.cn/hook/x").is_some());
    }

    #[tokio::test]
    async fn 发的是飞书文本格式() {
        let srv = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "msg_type": "text",
                "content": {"text": "磁盘 93%"}
            })))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .expect(1)
            .mount(&srv)
            .await;
        Alerter::new(&srv.uri())
            .unwrap()
            .send("磁盘 93%")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn 发不出去不算错() {
        let srv = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .mount(&srv)
            .await;
        let a = Alerter::new(&srv.uri()).unwrap();
        assert!(a.send("x").await.is_err());
        // notify 吞掉它：告警失败绝不让一轮活停下
        notify(Some(&a), "x").await;
        notify(None, "没配地址也不该炸").await;
    }
}
