//! 生成模型网关（csw-subapi）的客户端，走 Responses 接口。
//!
//! **和向量客户端一样放在 `core`，理由也一样：并发上限是全进程的。**
//! 识别（`harvest`）与判断（`judge`）走同一个网关，各持一个客户端就等于把并发翻倍。
//! 阶段 0.4 实测：并发从 8 提到 16，24 个请求里 9 个吃 429，墙钟几乎没变——
//! 多出来的并发全换成了退避。所以闸门必须在共用的这一层。
//!
//! 实测得到的三条参数，默认值直接照抄：
//! - **并发 8**。再高只换来 429。
//! - **超时不低于 420 秒**。单批延迟实测到过 405 秒。
//! - **批量 6 条一请求**省三分之二输入 token（7292 → 2370），吞吐不变——
//!   批量是为了成本，不是为了速度。
//!
//! 还有一条不是参数的事实：**吞吐是网关定的，约 8–10 秒一条候选**。
//! 一轮 300 条就是 42–50 分钟纯模型时间，这不是客户端能优化掉的，
//! 只能靠预取轮把它挪到夜里。

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// 只用 csw-subapi 这一套。域名与密钥必须配对，混搭直接 401。
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 同网关内的降级目标
    pub fallback_model: String,
    pub concurrency: usize,
    pub timeout: Duration,
    pub max_attempts: u32,
}

/// 一次调用的产物。`usage` 要落 `model_calls`——每日预算熔断靠它。
#[derive(Debug, Clone)]
pub struct ModelOutput {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub latency_ms: u64,
    pub attempts: u32,
    /// 实际用的模型。降级过就和 `cfg.model` 不一样。
    pub model: String,
}

impl ModelOutput {
    /// 按严格 schema 要的结构解析。解析不了就是 schema 没生效，该吵。
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.text).with_context(|| {
            format!(
                "返回不是合 schema 的 JSON；前 300 字：{}",
                self.text.chars().take(300).collect::<String>()
            )
        })
    }
}

#[derive(Debug, Deserialize)]
struct RespBody {
    #[serde(default)]
    output: Vec<OutputItem>,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    #[serde(default)]
    content: Vec<ContentPart>,
}

#[derive(Debug, Deserialize)]
struct ContentPart {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

/// 一段输入：文本或图片。图片是本地 base64——app-server 与这里都不收远程地址。
#[derive(Debug, Clone)]
pub enum Part {
    Text(String),
    ImageB64(String),
}

impl Part {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Text(t) => serde_json::json!({"type": "input_text", "text": t}),
            Self::ImageB64(b) => {
                serde_json::json!({"type": "input_image", "image_url": format!("data:image/jpeg;base64,{b}")})
            }
        }
    }
}

#[derive(Clone)]
pub struct ModelClient {
    cfg: ModelConfig,
    http: reqwest::Client,
    /// 全进程的网关闸门。识别与判断共用。
    gate: Arc<Semaphore>,
    /// 录制回放。`None` 等同直连。
    rec: Option<Arc<crate::record::Recorder>>,
}

impl ModelClient {
    pub fn new(cfg: ModelConfig) -> Result<Self> {
        crate::ensure_crypto_provider();
        anyhow::ensure!(!cfg.api_key.is_empty(), "缺 SUB2API_API_KEY");
        let http = reqwest::Client::builder().timeout(cfg.timeout).build()?;
        let gate = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
        Ok(Self {
            cfg,
            http,
            gate,
            rec: None,
        })
    }

    /// 挂上录制回放层。回放模式下**不出网**，未命中即失败。
    pub fn with_recorder(mut self, rec: Arc<crate::record::Recorder>) -> Self {
        self.rec = Some(rec);
        self
    }

    /// 发一次带严格 schema 的调用。
    ///
    /// `schema_name` 只是给网关看的标识；`schema` 必须是完整展开的 JSON Schema——
    /// strict 模式要求每一层都写全 `required` 并关掉 `additionalProperties`，
    /// 留 `{}` 占位会被拒。
    pub async fn structured(
        &self,
        parts: &[Part],
        schema_name: &str,
        schema: &serde_json::Value,
        max_output_tokens: u32,
    ) -> Result<ModelOutput> {
        let content: Vec<_> = parts.iter().map(Part::to_json).collect();
        let body = serde_json::json!({
            "model": self.cfg.model,
            "input": [{"role": "user", "content": content}],
            "text": {"format": {
                "type": "json_schema", "name": schema_name, "strict": true, "schema": schema
            }},
            "max_output_tokens": max_output_tokens,
            // 不让网关留存：判断材料里有第三方贴文正文
            "store": false,
        });
        self.send_with_fallback(body).await
    }

    async fn send_with_fallback(&self, mut body: serde_json::Value) -> Result<ModelOutput> {
        match self.send(&body, &self.cfg.model).await {
            Ok(o) => Ok(o),
            Err(e)
                if self.cfg.fallback_model.is_empty()
                    || self.cfg.fallback_model == self.cfg.model =>
            {
                Err(e)
            }
            Err(e) => {
                // 降级只在同一个网关内换模型：用户指定只用这一套凭据，
                // 换网关就要换密钥，那是另一回事，不能在这里悄悄做。
                tracing::warn!(原模型 = %self.cfg.model, 降级到 = %self.cfg.fallback_model, 原因 = %e, "模型调用失败，降级重试");
                body["model"] = serde_json::Value::String(self.cfg.fallback_model.clone());
                self.send(&body, &self.cfg.fallback_model).await
            }
        }
    }

    async fn send(&self, body: &serde_json::Value, model: &str) -> Result<ModelOutput> {
        // 录制回放挂在**最外层**：回放时连 HTTP 客户端都不碰，
        // 也就不会因为超时、退避、闸门这些东西让回放结果和录制时不一样。
        if let Some(rec) = &self.rec {
            let t0 = Instant::now();
            let raw = rec
                .wrap("model", body, || async { self.send_http(body).await })
                .await?;
            let parsed: RespBody = serde_json::from_value(raw).context("解析回放的响应")?;
            let out = extract(&parsed).context("回放的响应里没有输出文本")?;
            return Ok(ModelOutput {
                text: out,
                input_tokens: parsed.usage.input_tokens,
                output_tokens: parsed.usage.output_tokens,
                latency_ms: t0.elapsed().as_millis() as u64,
                attempts: 1,
                model: model.to_string(),
            });
        }
        self.send_direct(body, model).await
    }

    /// 只负责把请求发出去、把 JSON 拿回来。录制层要的是这一层。
    async fn send_http(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        let out = self.send_direct(body, &self.cfg.model).await?;
        // 录下来的是**原始响应**，不是我们解析后的结构——
        // 解析逻辑以后会变，夹具不该跟着变
        Ok(serde_json::json!({
            "output": [{"content": [{"type": "output_text", "text": out.text}]}],
            "usage": {"input_tokens": out.input_tokens, "output_tokens": out.output_tokens}
        }))
    }

    async fn send_direct(&self, body: &serde_json::Value, model: &str) -> Result<ModelOutput> {
        let url = format!("{}/responses", self.cfg.base_url.trim_end_matches('/'));
        let t0 = Instant::now();
        let mut attempt = 0;
        loop {
            attempt += 1;
            // 闸门只在真正发请求时占着，退避期间要放开——否则退避会把并发名额一起冻住
            let outcome = {
                let _permit = self.gate.acquire().await.context("网关闸门已关闭")?;
                self.http
                    .post(&url)
                    .bearer_auth(&self.cfg.api_key)
                    .json(body)
                    .send()
                    .await
            };
            match outcome {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        let parsed: RespBody = serde_json::from_str(&text).with_context(|| {
                            format!(
                                "解析响应；前 200 字：{}",
                                text.chars().take(200).collect::<String>()
                            )
                        })?;
                        let out = extract(&parsed).ok_or_else(|| {
                            anyhow::anyhow!(
                                "响应里没有输出文本；前 300 字：{}",
                                text.chars().take(300).collect::<String>()
                            )
                        })?;
                        return Ok(ModelOutput {
                            text: out,
                            input_tokens: parsed.usage.input_tokens,
                            output_tokens: parsed.usage.output_tokens,
                            latency_ms: t0.elapsed().as_millis() as u64,
                            attempts: attempt,
                            model: model.to_string(),
                        });
                    }
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if !retryable || attempt >= self.cfg.max_attempts {
                        bail!(
                            "网关返回 {status}：{}",
                            text.chars().take(300).collect::<String>()
                        );
                    }
                }
                Err(e) if attempt >= self.cfg.max_attempts => {
                    return Err(e).context("请求模型网关");
                }
                Err(_) => {}
            }
            backoff(attempt).await;
        }
    }
}

fn extract(r: &RespBody) -> Option<String> {
    let found = r
        .output
        .iter()
        .flat_map(|item| &item.content)
        .filter(|c| matches!(c.kind.as_str(), "output_text" | "text"))
        .filter_map(|c| c.text.as_ref())
        .find(|t| !t.trim().is_empty());
    // 有些实现只给顶层的 output_text，没有 output[].content
    found
        .cloned()
        .or_else(|| r.output_text.clone().filter(|t| !t.trim().is_empty()))
}

async fn backoff(attempt: u32) {
    // 加抖动：一批候选同时撞 429 时不要整齐地一起回来
    let base = 2u64.saturating_pow(attempt).min(16) * 1000;
    let jitter = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis())
        .unwrap_or(0) as u64)
        % 700;
    tokio::time::sleep(Duration::from_millis(base + jitter)).await;
}

/// 每日 token 预算的熔断器。
///
/// 为什么要它：一轮识别加判断约 290 万输入 token（0.4 / 0.5 外推）。
/// 一个死循环或一次重试风暴能在没人看着的凌晨把一个月的预算烧完。
#[derive(Debug)]
pub struct TokenBudget {
    input_limit: i64,
    output_limit: i64,
    input_used: std::sync::atomic::AtomicI64,
    output_used: std::sync::atomic::AtomicI64,
}

impl TokenBudget {
    pub fn new(input_limit: i64, output_limit: i64) -> Self {
        Self {
            input_limit,
            output_limit,
            input_used: Default::default(),
            output_used: Default::default(),
        }
    }

    pub fn record(&self, o: &ModelOutput) {
        use std::sync::atomic::Ordering::Relaxed;
        self.input_used.fetch_add(o.input_tokens, Relaxed);
        self.output_used.fetch_add(o.output_tokens, Relaxed);
    }

    /// 还能不能再发。超了就停——**宁可这一轮不完整地上报，也不要静默烧钱**。
    pub fn exhausted(&self) -> Option<String> {
        use std::sync::atomic::Ordering::Relaxed;
        let (i, o) = (
            self.input_used.load(Relaxed),
            self.output_used.load(Relaxed),
        );
        if self.input_limit > 0 && i >= self.input_limit {
            return Some(format!(
                "输入 token 已用 {i}，超出当日预算 {}",
                self.input_limit
            ));
        }
        if self.output_limit > 0 && o >= self.output_limit {
            return Some(format!(
                "输出 token 已用 {o}，超出当日预算 {}",
                self.output_limit
            ));
        }
        None
    }

    pub fn used(&self) -> (i64, i64) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.input_used.load(Relaxed),
            self.output_used.load(Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn cfg(uri: &str) -> ModelConfig {
        ModelConfig {
            base_url: uri.into(),
            api_key: "sk-test".into(),
            model: "gpt-6-astra".into(),
            fallback_model: String::new(),
            concurrency: 4,
            timeout: Duration::from_secs(5),
            max_attempts: 3,
        }
    }

    fn ok_body(text: &str) -> serde_json::Value {
        serde_json::json!({
            "output": [{"content": [{"type": "output_text", "text": text}]}],
            "usage": {"input_tokens": 7292, "output_tokens": 1045}
        })
    }

    #[test]
    fn 图片走本地base64不走远程地址() {
        let p = Part::ImageB64("AAA".into()).to_json();
        assert_eq!(p["type"], "input_image");
        assert_eq!(p["image_url"], "data:image/jpeg;base64,AAA");
    }

    #[tokio::test]
    async fn 严格schema的返回能解析且记下用量() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(ok_body(r#"{"tier":"recommend"}"#)),
            )
            .mount(&srv)
            .await;
        let c = ModelClient::new(cfg(&srv.uri())).unwrap();
        let out = c
            .structured(
                &[Part::Text("判一下".into())],
                "j",
                &serde_json::json!({"type": "object"}),
                1000,
            )
            .await
            .unwrap();
        assert_eq!(out.input_tokens, 7292);
        let v: serde_json::Value = out.parse().unwrap();
        assert_eq!(v["tier"], "recommend");
        assert_eq!(out.model, "gpt-6-astra");
    }

    #[tokio::test]
    async fn 四百二十九会退避重试() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&srv)
            .await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("{}")))
            .mount(&srv)
            .await;
        let c = ModelClient::new(cfg(&srv.uri())).unwrap();
        let out = c
            .structured(&[], "j", &serde_json::json!({}), 10)
            .await
            .unwrap();
        assert_eq!(out.attempts, 2);
    }

    /// 记录每次请求用的模型，用来验证降级
    struct ByModel;
    impl Respond for ByModel {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let b: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            match b.get("model").and_then(|m| m.as_str()) {
                // 主模型一直 500，逼它降级
                Some("gpt-6-astra") => ResponseTemplate::new(500).set_body_string("模型忙"),
                _ => ResponseTemplate::new(200).set_body_json(ok_body(r#"{"ok":1}"#)),
            }
        }
    }

    #[tokio::test]
    async fn 主模型不行会在同网关内降级() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ByModel)
            .mount(&srv)
            .await;
        let mut c = cfg(&srv.uri());
        c.fallback_model = "gpt-5.6-sol".into();
        c.max_attempts = 1; // 不重试，直接降级
        let out = ModelClient::new(c)
            .unwrap()
            .structured(&[], "j", &serde_json::json!({}), 10)
            .await
            .unwrap();
        assert_eq!(out.model, "gpt-5.6-sol", "应当降级到同网关的备用模型");
    }

    #[test]
    fn 预算超了要停而不是静默烧钱() {
        let b = TokenBudget::new(10_000, 5_000);
        assert!(b.exhausted().is_none());
        b.record(&ModelOutput {
            text: String::new(),
            input_tokens: 9_999,
            output_tokens: 10,
            latency_ms: 0,
            attempts: 1,
            model: "m".into(),
        });
        assert!(b.exhausted().is_none(), "没到线就不该停");
        b.record(&ModelOutput {
            text: String::new(),
            input_tokens: 2,
            output_tokens: 1,
            latency_ms: 0,
            attempts: 1,
            model: "m".into(),
        });
        let why = b.exhausted().expect("过线要停");
        assert!(why.contains("输入 token"), "{why}");
        assert_eq!(b.used().0, 10_001);
    }

    #[test]
    fn 不限额时预算器不挡路() {
        let b = TokenBudget::new(0, 0);
        b.record(&ModelOutput {
            text: String::new(),
            input_tokens: 999_999_999,
            output_tokens: 999_999_999,
            latency_ms: 0,
            attempts: 1,
            model: "m".into(),
        });
        assert!(b.exhausted().is_none());
    }
}
