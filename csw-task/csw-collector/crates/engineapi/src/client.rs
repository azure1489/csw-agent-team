//! 引擎运行面客户端。
//!
//! 三条来自实测、必须长在客户端里的规矩：
//!
//! 1. **`Idempotency-Key` 只对 POST 生效**（`middleware/idempotency.go:63`）。PUT 靠
//!    `UNIQUE + ON CONFLICT` 幂等，不必带键；POST 必须带，而且**一旦写下就不许换**——
//!    请求中途崩溃后同键永久 409，换键会造成重复提交。
//! 2. **`idempotency_in_progress_or_uncertain` 不是可重试错误。** 它的意思是
//!    「这个键的请求处理到一半，结果未知」。正确动作是去查任务状态对账，不是换键再发。
//! 3. **没有独立心跳接口**，重复 `POST /tasks/:id/ack` 就是心跳。

use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::types::*;

/// 引擎返回的错误体
#[derive(Debug, Deserialize)]
struct ErrBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("引擎返回 {status} {code}：{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    /// 同一幂等键的请求结果未知。**不要换键重试**，去查任务状态对账。
    #[error("幂等键 {idem_key} 的结果未知，须核实任务状态后再决定")]
    IdempotencyUncertain { idem_key: String },
    #[error("网络或解析失败：{0}")]
    Transport(String),
}

impl EngineError {
    pub fn code(&self) -> &str {
        match self {
            Self::Api { code, .. } => code,
            Self::IdempotencyUncertain { .. } => "idempotency_in_progress_or_uncertain",
            Self::Transport(_) => "transport",
        }
    }
    /// 值不值得重试。4xx 里只有 429 值得。
    fn retryable(&self) -> bool {
        match self {
            Self::Api { status, .. } => *status == 429 || *status >= 500,
            Self::Transport(_) => true,
            Self::IdempotencyUncertain { .. } => false,
        }
    }
}

pub struct EngineClient {
    base: String,
    token: String,
    http: reqwest::Client,
    max_attempts: u32,
}

impl EngineClient {
    pub fn new(base_url: &str, token: &str, timeout: Duration) -> Result<Self> {
        csw_collector_core::ensure_crypto_provider();
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::Client::builder().timeout(timeout).build()?,
            max_attempts: 4,
        })
    }

    // ── 接单与生命周期 ────────────────────────────────────────────────

    /// 我的开放任务。**注意**：任务在触发时就钉到 `ActiveAgentByRole` 返回的那一行 agent，
    /// 所以采集服务必须用**同一 agent 行**再签一枚 token，新建 agent 接不到单。
    pub async fn my_tasks(&self) -> Result<MyTasks, EngineError> {
        self.get("/me/tasks").await
    }

    pub async fn task_detail(&self, task_id: i64) -> Result<TaskDetail, EngineError> {
        self.get(&format!("/tasks/{task_id}")).await
    }

    /// 接单，兼作心跳——引擎没有独立心跳接口，重复调它就是心跳。
    ///
    /// 心跳不带幂等键：它本来就该可以重复调，带了反而会在第二次被幂等层挡回来。
    pub async fn ack(&self, task_id: i64) -> Result<serde_json::Value, EngineError> {
        self.post_json(
            &format!("/tasks/{task_id}/ack"),
            &serde_json::json!({}),
            None,
        )
        .await
    }

    /// 报告无法完成。时限前主动报，不要挂着让主编等超时。
    pub async fn fail(
        &self,
        task_id: i64,
        reason: &str,
        idem_key: &str,
    ) -> Result<(), EngineError> {
        let _: serde_json::Value = self
            .post_json(
                &format!("/tasks/{task_id}/fail"),
                &serde_json::json!({ "reason": reason }),
                Some(idem_key),
            )
            .await?;
        Ok(())
    }

    // ── 登记 ──────────────────────────────────────────────────────────

    pub async fn put_items(
        &self,
        run_id: i64,
        items: &[ItemInput],
    ) -> Result<serde_json::Value, EngineError> {
        self.put_json(
            &format!("/runs/{run_id}/items"),
            &serde_json::json!({ "items": items }),
        )
        .await
    }

    pub async fn put_sweeps(
        &self,
        run_id: i64,
        sweeps: &[SweepInput],
    ) -> Result<serde_json::Value, EngineError> {
        self.put_json(
            &format!("/runs/{run_id}/sweeps"),
            &serde_json::json!({ "sweeps": sweeps }),
        )
        .await
    }

    /// 判断台账。引擎侧限单次 200 条，这里自动分批——一轮三百多条，调用方不该操心批次。
    pub async fn put_judgements(
        &self,
        run_id: i64,
        js: &[JudgementInput],
    ) -> Result<usize, EngineError> {
        const BATCH: usize = 200;
        let mut sent = 0;
        for chunk in js.chunks(BATCH) {
            self.put_json::<serde_json::Value>(
                &format!("/runs/{run_id}/intake-judgements"),
                &serde_json::json!({ "judgements": chunk }),
            )
            .await?;
            sent += chunk.len();
        }
        Ok(sent)
    }

    // ── 只读 ──────────────────────────────────────────────────────────

    /// 提交前自查。判红就修，别带着红交上去让主编替我们发现。
    pub async fn intake_check(&self, run_id: i64) -> Result<IntakeCheckResult, EngineError> {
        self.get(&format!("/runs/{run_id}/intake-check")).await
    }

    pub async fn intake_trace(&self, run_id: i64) -> Result<serde_json::Value, EngineError> {
        self.get(&format!("/runs/{run_id}/intake-trace")).await
    }

    /// 五类对照材料的第四类：历史决定与原话。
    pub async fn ledger_decisions(
        &self,
        since: &str,
        limit: u32,
    ) -> Result<Vec<Decision>, EngineError> {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(default)]
            decisions: Vec<Decision>,
        }
        let w: Wrap = self
            .get(&format!("/ledger/decisions?since={since}&limit={limit}"))
            .await?;
        Ok(w.decisions)
    }

    /// 第一类：正式已发布的条目（也带回标了 `is_reference` 的范例，即第二类）。
    pub async fn ledger_posts(
        &self,
        since: &str,
        brand: &str,
    ) -> Result<serde_json::Value, EngineError> {
        let mut path = format!("/ledger/posts?since={since}");
        if !brand.is_empty() {
            path.push_str(&format!("&brand={}", urlencode(brand)));
        }
        self.get(&path).await
    }

    pub async fn memory_feedback(&self, limit: u32) -> Result<serde_json::Value, EngineError> {
        self.get(&format!("/memory/feedback?limit={limit}")).await
    }

    // ── 提交 ──────────────────────────────────────────────────────────

    /// 一步式提交。**只发已落盘的那一份 zip**，不在这里现构建。
    ///
    /// 幂等键由调用方给且必须稳定；遇到 `idempotency_in_progress_or_uncertain`
    /// 直接把错误抛上去——上层要去查任务状态对账，绝不能换键再发。
    pub async fn submit(&self, input: &SubmitInput) -> Result<serde_json::Value, EngineError> {
        let bytes = std::fs::read(&input.zip_path)
            .map_err(|e| EngineError::Transport(format!("读 {}：{e}", input.zip_path.display())))?;
        let url = format!("{}/tasks/{}/deliverables", self.base, input.task_id);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let part = reqwest::multipart::Part::bytes(bytes.clone())
                .file_name(input.file_name.clone())
                .mime_str("application/zip")
                .map_err(|e| EngineError::Transport(e.to_string()))?;
            let mut form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("kind", input.kind.clone());
            if !input.note.is_empty() {
                form = form.text("note", input.note.clone());
            }
            if !input.item_key.is_empty() {
                form = form.text("item_key", input.item_key.clone());
            }
            if let Some(id) = input.affects_deliverable_id {
                form = form.text("affects_deliverable_id", id.to_string());
            }
            let req = self
                .http
                .post(&url)
                .bearer_auth(&self.token)
                .header("Idempotency-Key", &input.idem_key)
                .multipart(form);
            match self.send(req, Some(&input.idem_key)).await {
                Ok(v) => return Ok(v),
                Err(e) if e.retryable() && attempt < self.max_attempts => {
                    backoff(attempt).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    // ── 底层 ──────────────────────────────────────────────────────────

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let req = self
                .http
                .get(format!("{}{}", self.base, path))
                .bearer_auth(&self.token);
            match self.send_typed::<T>(req, None).await {
                Ok(v) => return Ok(v),
                Err(e) if e.retryable() && attempt < self.max_attempts => backoff(attempt).await,
                Err(e) => return Err(e),
            }
        }
    }

    async fn put_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            // PUT 不带幂等键：引擎侧靠 UNIQUE + ON CONFLICT 幂等，带了也不生效
            let req = self
                .http
                .put(format!("{}{}", self.base, path))
                .bearer_auth(&self.token)
                .json(body);
            match self.send_typed::<T>(req, None).await {
                Ok(v) => return Ok(v),
                Err(e) if e.retryable() && attempt < self.max_attempts => backoff(attempt).await,
                Err(e) => return Err(e),
            }
        }
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
        idem_key: Option<&str>,
    ) -> Result<T, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut req = self
                .http
                .post(format!("{}{}", self.base, path))
                .bearer_auth(&self.token)
                .json(body);
            if let Some(k) = idem_key {
                req = req.header("Idempotency-Key", k);
            }
            match self.send_typed::<T>(req, idem_key).await {
                Ok(v) => return Ok(v),
                Err(e) if e.retryable() && attempt < self.max_attempts => backoff(attempt).await,
                Err(e) => return Err(e),
            }
        }
    }

    async fn send_typed<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
        idem_key: Option<&str>,
    ) -> Result<T, EngineError> {
        let v = self.send(req, idem_key).await?;
        serde_json::from_value(v).map_err(|e| EngineError::Transport(format!("解析响应：{e}")))
    }

    async fn send(
        &self,
        req: reqwest::RequestBuilder,
        idem_key: Option<&str>,
    ) -> Result<serde_json::Value, EngineError> {
        let resp = req
            .send()
            .await
            .map_err(|e| EngineError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        if (200..300).contains(&status) {
            if text.trim().is_empty() {
                return Ok(serde_json::Value::Null);
            }
            return serde_json::from_str(&text).map_err(|e| {
                EngineError::Transport(format!("解析响应：{e}；原文前 200 字：{}", head(&text)))
            });
        }
        let err: ErrBody = serde_json::from_str(&text).unwrap_or(ErrBody {
            code: String::new(),
            message: head(&text),
        });
        if err.code == "idempotency_in_progress_or_uncertain" {
            return Err(EngineError::IdempotencyUncertain {
                idem_key: idem_key.unwrap_or_default().to_string(),
            });
        }
        Err(EngineError::Api {
            status,
            code: err.code,
            message: err.message,
        })
    }
}

async fn backoff(attempt: u32) {
    // 指数退避加抖动：多个候选同时撞到 429 时不要整齐地一起重试
    let base = 2u64.saturating_pow(attempt).min(16);
    let jitter = rand_frac();
    tokio::time::sleep(Duration::from_millis(
        (base as f64 * 1000.0 * (0.5 + jitter)) as u64,
    ))
    .await;
}

/// 不引 rand 只为一点抖动：拿系统时间的纳秒位凑一个 0–1 的小数就够了。
fn rand_frac() -> f64 {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (n % 1000) as f64 / 1000.0
}

fn head(s: &str) -> String {
    s.chars().take(200).collect()
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// 提交的幂等键：`submit-{task_id}-{zip sha256 前 16}`。
///
/// 从**内容**派生而不是随机生成：重试时必然算出同一个键，
/// 而内容变了（真的改了交付物）就必然是新键。
pub fn submit_idem_key(task_id: i64, zip_sha256: &str) -> String {
    format!(
        "submit-{task_id}-{}",
        &zip_sha256[..zip_sha256.len().min(16)]
    )
}
