//! TypeSafe Jev 客户端（System One）。
//!
//! Jev 回的是**带概率的定型答案**，不是文字。三种问法在这里各有一个构造器：
//! - [`Question::noul`] —— 这件事成不成立，回一个概率；
//! - [`Question::score`] —— 在一条有序刻度上落在哪一档，回每档的概率；
//! - [`Question::choice`] —— 从给定选项里挑一个。
//!
//! ## 它在这套系统里的位置
//!
//! **只做三件事：初评排序、同一事件、核对依据。** 0.7 实测把边界划清楚了：
//! 六维初评方向全对，但总判 `worth` 当淘汰闸不行——阈值 0.30 要丢掉 29% 本该写的，
//! 所以它**只排序不淘汰**；核对依据抓编造 100% 且贴在 0.02，但会误伤三成真依据，
//! 所以被标出来的**给人看不自动淘汰**。
//!
//! ## 送什么、不送什么
//!
//! **只送贴文正文与图片描述。** Van 的原话、编辑部的内部决定、会话记录一律不送——
//! 它是第三方服务。这条约束在调用侧（`judge`）靠显式拼 state 来守：
//! 这里不做也做不了内容检查，所以**拼 state 的地方必须逐字段列举，不许整个结构体扔进来**。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct JevConfig {
    pub base_url: String,
    pub model: String,
    pub concurrency: usize,
    pub timeout: Duration,
    pub max_attempts: u32,
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-latest".into(),
            concurrency: 8,
            timeout: Duration::from_secs(60),
            max_attempts: 6,
        }
    }
}

/// 一档刻度。`level` 是给人看的档名，`what` 是这一档具体长什么样。
///
/// **每一档都要能独立看懂**——写成「比上一档好一些」这种相对描述，模型分不出来。
#[derive(Debug, Clone, Serialize)]
pub struct Level {
    pub level: String,
    pub what: String,
}

impl Level {
    pub fn new(level: &str, what: &str) -> Self {
        Self {
            level: level.into(),
            what: what.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Noul {
        instructions: String,
        /// `{"true": …, "false": …}`——noul 的 criteria 是**对象**
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        /// score 的 criteria 是**数组**，顺序就是刻度顺序
        criteria: Vec<Level>,
    },
    Choice {
        instructions: String,
        /// choice 的 criteria 是**对象**：选项名 → 这个选项什么时候成立
        criteria: BTreeMap<String, String>,
    },
}

impl Question {
    pub fn noul(instructions: &str, yes: &str, no: &str) -> Self {
        Self::Noul {
            instructions: instructions.into(),
            criteria: BTreeMap::from([
                ("true".to_string(), yes.to_string()),
                ("false".to_string(), no.to_string()),
            ]),
        }
    }

    /// 三档刻度：不成立 / 不明 / 成立。取值时用 [`Answers::score_top`]。
    pub fn score3(instructions: &str, no: &str, unclear: &str, yes: &str) -> Self {
        Self::Score {
            instructions: instructions.into(),
            criteria: vec![
                Level::new("不成立", no),
                Level::new("不明", unclear),
                Level::new("成立", yes),
            ],
        }
    }

    pub fn choice(instructions: &str, options: &[(&str, &str)]) -> Self {
        Self::Choice {
            instructions: instructions.into(),
            criteria: options
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

/// 一次问答的回答。**按键取值，取不到就是 None，不替调用方猜默认值**——
/// 「没回答」与「回答了 0」在阈值判断里是完全不同的两件事。
#[derive(Debug, Clone, Default)]
pub struct Answers {
    raw: serde_json::Map<String, Value>,
}

impl Answers {
    /// noul 的概率（「成立」的概率）
    pub fn noul(&self, key: &str) -> Option<f64> {
        self.raw.get(key)?.get("noul")?.as_f64()
    }

    /// 三档 score 里「成立」那一档的概率。
    ///
    /// **概率键是字符串**（`"2"`），不是数字——按数字取会静默拿到 None，
    /// 然后一整列阈值判断全变成「没回答」。这是 0.7 实测踩过的。
    pub fn score_top(&self, key: &str) -> Option<f64> {
        let p = self.raw.get(key)?.get("probabilities")?;
        // 最后一档就是最高那档；三档时是 "2"
        let last = p
            .as_object()?
            .keys()
            .filter_map(|k| k.parse::<i64>().ok())
            .max()?;
        p.get(last.to_string())?.as_f64()
    }

    pub fn choice(&self, key: &str) -> Option<&str> {
        self.raw.get(key)?.get("choice")?.as_str()
    }

    /// 分布集中度。低不等于答错，只等于「几个选项它都觉得说得通」。
    pub fn confidence(&self, key: &str) -> Option<f64> {
        self.raw.get(key)?.get("confidence")?.as_f64()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.raw.keys()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
}

/// **克隆它是安全的**——并发闸在 `Arc<Semaphore>` 里，克隆出来的还是同一把。
#[derive(Clone)]
pub struct JevClient {
    cfg: JevConfig,
    http: reqwest::Client,
    api_key: String,
    sem: Arc<Semaphore>,
    /// 录制回放。`None` 等同直连。
    rec: Option<Arc<crate::record::Recorder>>,
}

impl JevClient {
    pub fn new(cfg: JevConfig, api_key: &str) -> Result<Self> {
        crate::ensure_crypto_provider();
        anyhow::ensure!(!api_key.trim().is_empty(), "Jev 密钥是空的");
        let sem = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
        Ok(Self {
            http: reqwest::Client::builder().timeout(cfg.timeout).build()?,
            cfg,
            api_key: api_key.to_string(),
            sem,
            rec: None,
        })
    }

    /// 挂上录制回放层。回放模式下**不出网**，未命中即失败。
    pub fn with_recorder(mut self, rec: Arc<crate::record::Recorder>) -> Self {
        self.rec = Some(rec);
        self
    }

    /// 问一组问题。
    ///
    /// **同一份 state 上互不依赖的问题要放在一次请求里**：它们并行跑、互相看不见彼此的
    /// 答案，分成几次请求既慢又贵。要分次，只有当后一个问题得先看到前一个的答案
    /// 才能构造出来。
    pub async fn ask(
        &self,
        state: &Value,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Answers> {
        anyhow::ensure!(!questions.is_empty(), "一个问题都没有");
        let body = json!({
            "state": state,
            "model": self.cfg.model,
            "questions": questions,
        });
        if let Some(rec) = &self.rec {
            let raw = rec
                .wrap("jev", &body, || async { self.ask_http(&body).await })
                .await?;
            let answers = raw
                .get("answers")
                .and_then(Value::as_object)
                .cloned()
                .context("回放的 Jev 答案里没有 answers")?;
            anyhow::ensure!(!answers.is_empty(), "回放的 Jev 答案是空的");
            return Ok(Answers { raw: answers });
        }
        self.ask_http(&body).await.and_then(|v| {
            let answers = v
                .get("answers")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            anyhow::ensure!(!answers.is_empty(), "Jev 回了空答案");
            Ok(Answers { raw: answers })
        })
    }

    /// 只负责发请求拿 JSON。录制层要的是这一层。
    async fn ask_http(&self, body: &Value) -> Result<Value> {
        let url = format!("{}/v1/systemone", self.cfg.base_url.trim_end_matches('/'));
        let _permit = self.sem.acquire().await?;
        let mut last = String::new();
        for attempt in 0..self.cfg.max_attempts {
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    return r.json().await.context("解析 Jev 回答");
                }
                Ok(r) => {
                    let code = r.status();
                    let text = r.text().await.unwrap_or_default();
                    // 密钥或问题写错了，重试多少次都一样
                    if !retryable(code.as_u16()) {
                        anyhow::bail!("Jev {code}：{}", head(&text));
                    }
                    last = format!("{code}：{}", head(&text));
                }
                Err(e) => last = e.to_string(),
            }
            backoff(attempt).await;
        }
        anyhow::bail!("Jev 重试 {} 次仍失败：{last}", self.cfg.max_attempts)
    }
}

fn retryable(code: u16) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504 | 529)
}

/// 错误正文只留开头：Jev 的错误里可能回显我们发过去的 state，
/// 整段进日志就把贴文正文也写进去了。
fn head(s: &str) -> String {
    s.chars().take(200).collect()
}

async fn backoff(attempt: u32) {
    let base = Duration::from_millis(500 * 2u64.pow(attempt.min(4)));
    let jitter = Duration::from_millis(fastrand_ms());
    tokio::time::sleep(base.min(Duration::from_secs(8)) + jitter).await;
}

/// 不值得为抖动引一个随机数库：拿纳秒的低位就够散了。
fn fastrand_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos() % 1000))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn cfg(uri: &str) -> JevConfig {
        JevConfig {
            base_url: uri.into(),
            timeout: Duration::from_secs(5),
            max_attempts: 3,
            ..Default::default()
        }
    }

    fn triage_questions() -> BTreeMap<String, Question> {
        BTreeMap::from([
            (
                "change".to_string(),
                Question::score3("有没有具体变化", "没有", "说不清", "有"),
            ),
            (
                "worth".to_string(),
                Question::noul("值不值得写", "值得", "不值得"),
            ),
        ])
    }

    #[test]
    fn 三种问法的criteria形状不一样() {
        // 这三个形状是实测出来的，写错了服务端会 400。
        // noul 与 choice 是对象，score 是数组——顺序就是刻度顺序。
        let noul = serde_json::to_value(Question::noul("q", "是", "否")).unwrap();
        assert_eq!(noul["type"], "noul");
        assert!(noul["criteria"].is_object());
        assert_eq!(noul["criteria"]["true"], "是");

        let score = serde_json::to_value(Question::score3("q", "低", "中", "高")).unwrap();
        assert_eq!(score["type"], "score");
        assert!(score["criteria"].is_array());
        assert_eq!(score["criteria"][2]["level"], "成立");
        assert_eq!(score["criteria"][2]["what"], "高");

        let choice = serde_json::to_value(Question::choice(
            "q",
            &[("a", "甲成立时"), ("b", "乙成立时")],
        ))
        .unwrap();
        assert_eq!(choice["type"], "choice");
        assert!(choice["criteria"].is_object());
        assert_eq!(choice["criteria"]["a"], "甲成立时");
    }

    #[tokio::test]
    async fn 取值按键来取不到就是空() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "answers": {
                    // 概率键是字符串，不是数字
                    "change": {"probabilities": {"0": 0.1, "1": 0.2, "2": 0.7}, "confidence": 0.8},
                    "worth": {"noul": 0.46},
                }
            })))
            .mount(&server)
            .await;

        let c = JevClient::new(cfg(&server.uri()), "k").unwrap();
        let a = c
            .ask(&serde_json::json!({"caption": "正文"}), &triage_questions())
            .await
            .unwrap();
        assert_eq!(a.score_top("change"), Some(0.7), "三档取最高那档");
        assert_eq!(a.noul("worth"), Some(0.46));
        assert_eq!(a.confidence("change"), Some(0.8));
        // 「没回答」与「回答了 0」在阈值判断里完全不同，不许替调用方猜
        assert_eq!(a.noul("没问过这个"), None);
        assert_eq!(a.score_top("worth"), None, "noul 没有 probabilities");
    }

    struct Flaky(std::sync::atomic::AtomicU32);
    impl Respond for Flaky {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(429)
            } else {
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"answers": {"worth": {"noul": 0.9}}}))
            }
        }
    }

    #[tokio::test]
    async fn 限流要退避重试() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(Flaky(std::sync::atomic::AtomicU32::new(0)))
            .mount(&server)
            .await;
        let c = JevClient::new(cfg(&server.uri()), "k").unwrap();
        let a = c
            .ask(&serde_json::json!({}), &triage_questions())
            .await
            .unwrap();
        assert_eq!(a.noul("worth"), Some(0.9));
    }

    #[tokio::test]
    async fn 密钥错了不该白重试() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .expect(1) // 只发一次：401 重试多少遍都一样
            .mount(&server)
            .await;
        let c = JevClient::new(cfg(&server.uri()), "k").unwrap();
        let e = c
            .ask(&serde_json::json!({}), &triage_questions())
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("401"), "{e}");
    }

    #[tokio::test]
    async fn 空答案要报错而不是当成都没成立() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        let c = JevClient::new(cfg(&server.uri()), "k").unwrap();
        assert!(
            c.ask(&serde_json::json!({}), &triage_questions())
                .await
                .is_err()
        );
    }

    #[test]
    fn 错误正文只留开头() {
        // Jev 的错误里可能回显我们发过去的 state，整段进日志就把贴文正文也写进去了
        let long = "正".repeat(500);
        assert_eq!(head(&long).chars().count(), 200);
    }

    #[test]
    fn 空密钥直接拒绝构造() {
        assert!(JevClient::new(JevConfig::default(), "   ").is_err());
    }
}
