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

/// **克隆它是安全的**：`reqwest::Client` 内部是 `Arc`，克隆出来的还是同一个连接池。
/// 心跳那条后台任务要自己持有一份，所以需要这个。
#[derive(Clone)]
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

    /// 期次详情：`{run, tasks:[…]}`。05 用它找同条目已通过的 04。
    pub async fn run_detail(&self, run_id: i64) -> Result<serde_json::Value, EngineError> {
        self.get(&format!("/runs/{run_id}")).await
    }

    pub async fn task_detail(&self, task_id: i64) -> Result<TaskDetail, EngineError> {
        let d: TaskDetail = self.get(&format!("/tasks/{task_id}")).await?;
        Ok(d.normalize())
    }

    /// 这一期派下来的窗口 `(起, 止)`：取时间线里 `run_created` 那条事件的输入 `窗口`。
    /// 期次详情接口不带输入，只有时间线里有。取不到返回 `None`（调用方退回按水位算）。
    /// 这一期的目标数（主选，备选）。取不到返回 `None`。
    pub async fn run_target(&self, run_id: i64) -> Result<Option<(usize, usize)>, EngineError> {
        let v: serde_json::Value = self.get(&format!("/runs/{run_id}/timeline")).await?;
        Ok(run_target_of(&v))
    }

    pub async fn run_window(&self, run_id: i64) -> Result<Option<(String, String)>, EngineError> {
        let v: serde_json::Value = self.get(&format!("/runs/{run_id}/timeline")).await?;
        Ok(run_window_of(&v))
    }

    /// 这一期的状态：`active` / `paused` / `done` / `aborted`。
    pub async fn run_status(&self, run_id: i64) -> Result<String, EngineError> {
        let v: serde_json::Value = self.get(&format!("/runs/{run_id}")).await?;
        Ok(v.pointer("/run/status")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string())
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
    ///
    /// `with_body` 才回正文与发布凭据。查重那条路用不上正文，上千篇正文纯属浪费；
    /// 知识库同步要靠它做嵌入与全文索引，必须要。
    pub async fn ledger_posts(
        &self,
        since: &str,
        brand: &str,
        with_body: bool,
    ) -> Result<LedgerPosts, EngineError> {
        let mut path = format!("/ledger/posts?since={since}");
        if !brand.is_empty() {
            path.push_str(&format!("&brand={}", urlencode(brand)));
        }
        if with_body {
            path.push_str("&include_body=1");
        }
        self.get(&path).await
    }

    /// 这一期的条目与整期目标。11 选图包按它挑「Van 批准可写」的那几条。
    pub async fn run_items(&self, run_id: i64) -> Result<RunItems, EngineError> {
        self.get(&format!("/runs/{run_id}/items")).await
    }

    /// 这一期引擎里的判断台账（逐条）。对账用。
    pub async fn intake_judgements(&self, run_id: i64) -> Result<serde_json::Value, EngineError> {
        self.get(&format!("/runs/{run_id}/intake-judgements")).await
    }

    pub async fn memory_feedback(&self, limit: u32) -> Result<serde_json::Value, EngineError> {
        self.get(&format!("/memory/feedback?limit={limit}")).await
    }

    /// 选题准则卡。
    ///
    /// `only_confirmed` **判断那一步必须传真**：未经她确认的归纳只是我们的猜测，
    /// 拿它当依据等于以她的名义下判断。展示用的地方传假，但要把
    /// `confirmed_by_van` 一起显示出来。
    pub async fn memory_rules(
        &self,
        only_confirmed: bool,
        limit: u32,
    ) -> Result<Vec<MemoryRuleRow>, EngineError> {
        #[derive(serde::Deserialize)]
        struct W {
            #[serde(default)]
            rules: Vec<MemoryRuleRow>,
        }
        let c = if only_confirmed { "1" } else { "0" };
        let w: W = self
            .get(&format!("/memory/rules?confirmed={c}&limit={limit}"))
            .await?;
        Ok(w.rules)
    }

    /// 选题案例。带着她的原话。
    pub async fn memory_cases(
        &self,
        decision: &str,
        limit: u32,
    ) -> Result<Vec<MemoryCaseRow>, EngineError> {
        #[derive(serde::Deserialize)]
        struct W {
            #[serde(default)]
            cases: Vec<MemoryCaseRow>,
        }
        let w: W = self
            .get(&format!("/memory/cases?decision={decision}&limit={limit}"))
            .await?;
        Ok(w.cases)
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
                .text("kind", input.kind.as_str());
            if !input.note.is_empty() {
                form = form.text("note", input.note.clone());
            }
            if !input.self_check.is_empty() {
                form = form.text("self_check", input.self_check.clone());
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
                // 上传按包大小给时限：整个客户端的 120 秒管不住大包（10-08 r66：41 MB 传了 129 秒，
                // 客户端先断、引擎 context canceled，同键重试撞 409「结果未知」，整轮报失败）
                .timeout(upload_timeout(bytes.len()))
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

    /// 原样发一份已经拼好的 JSON。
    ///
    /// 给 outbox 用：那边存的是**要发的确切字节**，重试必须发同样的字节。
    /// 反序列化成结构体再序列化回去，字段顺序与 `skip_serializing_if`
    /// 都可能让字节变样，那就不是同一份请求了。
    /// 把 outbox 里存的那一份 JSON **原样**发出去。
    ///
    /// 收 `&str` 而不是 `serde_json::Value`：`Value` 的对象是 `BTreeMap`，
    /// 解析再序列化会把键重排——发出去的字节就和 `body_sha` 对不上了，
    /// 而「重试必须发同样的字节」是这一层唯一的承诺。
    /// （这个错是崩溃续跑那条测试抓出来的：它比对了实际发出去的请求体。）
    pub async fn put_raw(&self, path: &str, body: &str) -> Result<serde_json::Value, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            // PUT 不带幂等键：引擎侧靠 UNIQUE + ON CONFLICT 幂等，带了也不生效
            let req = self
                .http
                .put(format!("{}{}", self.base, path))
                .bearer_auth(&self.token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
            match self.send_typed::<serde_json::Value>(req, None).await {
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
/// 上传时限：120 秒起，每 64 KiB 加 1 秒，最多 30 分钟。实测 agent 主机到引擎约 320 KB/s，留四倍余量。
pub fn upload_timeout(bytes: usize) -> Duration {
    Duration::from_secs((120 + bytes as u64 / 65_536).min(1800))
}

pub fn submit_idem_key(task_id: i64, zip_sha256: &str) -> String {
    format!(
        "submit-{task_id}-{}",
        &zip_sha256[..zip_sha256.len().min(16)]
    )
}

/// 从时间线里挑出 `run_created` 的 `窗口.起 / 窗口.止`。事件详情可能是字符串化的 JSON，也可能已是对象。
/// 这一期的目标数（主选，备选）：时间线里 `run_created` 输入的 `目标`。
pub fn run_target_of(timeline: &serde_json::Value) -> Option<(usize, usize)> {
    let ev = timeline
        .get("events")?
        .as_array()?
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("run_created"))?;
    let detail = match ev.get("detail")? {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok()?,
        other => other.clone(),
    };
    let t = detail.get("目标")?;
    let n = |k: &str| t.get(k).and_then(|v| v.as_u64()).map(|v| v as usize);
    Some((n("主选")?, n("备选").unwrap_or(0)))
}

pub fn run_window_of(timeline: &serde_json::Value) -> Option<(String, String)> {
    let ev = timeline
        .get("events")?
        .as_array()?
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("run_created"))?;
    let detail = match ev.get("detail")? {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok()?,
        other => other.clone(),
    };
    let w = detail.get("窗口")?;
    let from = w.get("起")?.as_str()?.to_string();
    let to = w.get("止")?.as_str()?.to_string();
    Some((from, to))
}

#[cfg(test)]
mod run_window_tests {
    use super::*;

    #[test]
    fn 从时间线取目标数() {
        let v = serde_json::json!({"events": [{"type": "run_created", "detail":
            "{\"目标\":{\"主选\":6,\"备选\":2},\"窗口\":{\"起\":\"a\",\"止\":\"b\"}}"}]});
        assert_eq!(run_target_of(&v), Some((6, 2)));
        assert_eq!(run_target_of(&serde_json::json!({"events": []})), None);
    }

    #[test]
    fn 大包上传给够时间() {
        assert_eq!(upload_timeout(0), Duration::from_secs(120));
        // r66 的 41 MB：120 + 625 秒，远大于实测的 129 秒
        assert!(upload_timeout(40_963_384) >= Duration::from_secs(700));
        assert_eq!(upload_timeout(usize::MAX / 2), Duration::from_secs(1800));
    }

    #[test]
    fn 从时间线取期次窗口() {
        let tl = serde_json::json!({"events": [
            {"type": "task_ready", "detail": null},
            {"type": "run_created",
             "detail": "{\"窗口\":{\"止\":\"2026-09-28T07:00+08:00\",\"起\":\"2026-09-25T07:00+08:00\"}}"}
        ]});
        assert_eq!(
            run_window_of(&tl),
            Some((
                "2026-09-25T07:00+08:00".into(),
                "2026-09-28T07:00+08:00".into()
            ))
        );
        assert_eq!(run_window_of(&serde_json::json!({"events": []})), None);
    }
}
