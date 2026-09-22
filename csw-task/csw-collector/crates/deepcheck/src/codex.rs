//! codex app-server 的**薄** stdio 客户端。
//!
//! # 为什么自己写，不用 `codex-app-server-protocol`
//!
//! 那个 crate 会拖进五百多个依赖，而且版本号是 `0.0.0`——锁不住，
//! 本地 main 与生产的 0.155.1 也不是同一份。我们只用到六七个消息，
//! 自己写的那部分反而更稳：**只为用到的字段写结构体，其余一律 `Value` 兜底**，
//! 上游加字段不会把我们编挂。
//!
//! # 协议要点（实测记录 §6，逐条对过源码）
//!
//! - NDJSON over stdio，JSON-RPC 风格：一行一条消息。
//! - `initialize` 之后**必须**发 `initialized` 通知，否则后续请求不被处理。
//! - `thread/start` 的线程 id 在 **`result.thread.id`**，不是 `result.threadId`。
//! - **`turn/start` 是异步的**：响应立刻回 `inProgress`，真正的完成信号是
//!   `turn/completed` 通知。等响应的写法会永远拿到「还在跑」。
//! - `turn/interrupt` 要 `threadId` **和** `turnId`，两个都必填。
//! - 审批类服务端请求（`*RequestApproval`、`elicitation`、`RequestUserInput`）
//!   **必须有处理器并回 `{"decision":"decline"}`**，否则那条请求挂住、整个回合卡死。
//! - 增量通知（`item/agentMessage/delta`、`item/reasoning/summaryTextDelta`）量很大，
//!   默认**丢掉**，只留 `item/completed` 做审计。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

/// 审批一律拒绝。深核是**只读**的：沙箱 read-only、审批策略 never，
/// 正常跑不出审批请求；真冒出来一个，说明有什么不对，拒掉并留痕，别放行。
const DECLINE: &str = "decline";

#[derive(Debug, Clone)]
pub struct CodexConfig {
    pub bin: PathBuf,
    /// `CODEX_HOME`，装着 config.toml 与 MCP 白名单
    pub home: PathBuf,
    /// 工作目录。深核不写文件，但 codex 要一个合法的 cwd。
    pub cwd: PathBuf,
    pub model: String,
    /// 一个回合最多跑多久
    pub turn_budget: Duration,
}

/// 一次回合的产物。
#[derive(Debug, Clone, Default)]
pub struct TurnOutcome {
    pub turn_id: String,
    /// completed / interrupted / failed
    pub status: String,
    /// 助手的最终文字（按出现顺序拼起来）
    pub text: String,
    /// 审计用的事件流，已经滤掉增量。每条是原样的 `item/completed` 参数。
    pub items: Vec<Value>,
    pub error: String,
}

impl TurnOutcome {
    pub fn ok(&self) -> bool {
        self.status == "completed"
    }
}

/// 一条输入。图片走**本地路径**，不走 base64——codex 自己会读盘。
#[derive(Debug, Clone)]
pub enum Input {
    Text(String),
    LocalImage(PathBuf),
}

impl Input {
    fn to_json(&self) -> Value {
        match self {
            Self::Text(t) => json!({"type": "text", "text": t}),
            Self::LocalImage(p) => json!({"type": "localImage", "path": p}),
        }
    }
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;
type Waiters = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

pub struct Codex {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    /// 出站消息都经这一条通道，免得两处同时写 stdin 把行搅在一起
    out_tx: mpsc::Sender<String>,
    next_id: AtomicI64,
    pending: Pending,
    /// threadId → 等这个线程的 `turn/completed`
    turn_waiters: Waiters,
    /// 每个线程收到的 `item/completed`
    items: Arc<Mutex<HashMap<String, Vec<Value>>>>,
    cfg: CodexConfig,
}

impl Codex {
    /// 拉起进程并握手。
    pub async fn start(cfg: CodexConfig) -> Result<Self> {
        let mut child = Command::new(&cfg.bin)
            .arg("app-server")
            // 不继承调用方的环境变量：里面有 csw 与网关的密钥，codex 用不着它们。
            // env_clear 要在所有 env 之前，否则前面设的会被清掉。
            .env_clear()
            .env("CODEX_HOME", &cfg.home)
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .current_dir(&cfg.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("拉起 {}", cfg.bin.display()))?;

        let stdin = child.stdin.take().context("没拿到 stdin")?;
        let stdout = child.stdout.take().context("没拿到 stdout")?;
        let stderr = child.stderr.take().context("没拿到 stderr")?;

        let pending: Pending = Default::default();
        let turn_waiters: Waiters = Default::default();
        let items: Arc<Mutex<HashMap<String, Vec<Value>>>> = Default::default();
        let (out_tx, mut out_rx) = mpsc::channel::<String>(64);

        let me = Self {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            out_tx: out_tx.clone(),
            next_id: AtomicI64::new(1),
            pending: pending.clone(),
            turn_waiters: turn_waiters.clone(),
            items: items.clone(),
            cfg,
        };

        // 写线程：所有出站消息都经这一条，免得两处同时写 stdin 把行搅在一起
        let w = me.stdin.clone();
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                let mut g = w.lock().await;
                if g.write_all(line.as_bytes()).await.is_err() || g.flush().await.is_err() {
                    break;
                }
            }
        });

        // stderr 单独收：codex 的报错只在这里，丢掉它等于把排障路堵死
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::debug!(来源 = "codex", "{l}");
            }
        });

        // 读线程
        let reply = out_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    tracing::debug!("codex 发来非 JSON 行，跳过");
                    continue;
                };
                route(v, &pending, &turn_waiters, &items, &reply).await;
            }
            // 进程没了：把所有等着的叫醒，别让调用方挂到超时
            let mut p = pending.lock().await;
            for (_, tx) in p.drain() {
                let _ = tx.send(Err("codex 进程已退出".into()));
            }
        });

        me.handshake(&out_tx).await?;
        Ok(me)
    }

    async fn handshake(&self, tx: &mpsc::Sender<String>) -> Result<()> {
        self.request_via(
            tx,
            "initialize",
            json!({
                "clientInfo": {"name": "csw-collector", "title": "情报收集员工作台", "version": env!("CARGO_PKG_VERSION")}
            }),
            Duration::from_secs(30),
        )
        .await
        .context("initialize")?;
        // **必须**发这条通知，否则后续请求不被处理
        self.notify_via(tx, "initialized", json!({})).await?;
        Ok(())
    }

    /// 建一条线程。深核是只读的：沙箱 read-only、审批策略 never。
    pub async fn start_thread(&self, developer_instructions: &str) -> Result<String> {
        let tx = self.sender();
        let r = self
            .request_via(
                &tx,
                "thread/start",
                json!({
                    "model": self.cfg.model,
                    "cwd": self.cfg.cwd,
                    "sandbox": "read-only",
                    "approvalPolicy": "never",
                    "developerInstructions": developer_instructions,
                }),
                Duration::from_secs(60),
            )
            .await
            .context("thread/start")?;
        // 线程 id 在 result.thread.id，不是 result.threadId
        r.get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("thread/start 的返回里没有 thread.id")
    }

    /// 跑一个回合并**等 `turn/completed` 通知**。
    ///
    /// 超时就发 `turn/interrupt` 再返回——留着一个跑飞的回合会一直烧 token。
    pub async fn run_turn(&self, thread_id: &str, input: &[Input]) -> Result<TurnOutcome> {
        self.run_turn_with_schema(thread_id, input, None).await
    }

    /// 带结构化输出的回合。
    ///
    /// **同一线程上一个回合没结束时，起一个带不同 `outputSchema` 的新回合会被拒**
    /// （`ActiveTurnOutputSchemaMismatch`）。所以深核是一条线程一个回合，
    /// 不复用线程。
    pub async fn run_turn_with_schema(
        &self,
        thread_id: &str,
        input: &[Input],
        schema: Option<&Value>,
    ) -> Result<TurnOutcome> {
        let tx = self.sender();
        let (done_tx, done_rx) = oneshot::channel();
        self.turn_waiters
            .lock()
            .await
            .insert(thread_id.to_string(), done_tx);
        self.items
            .lock()
            .await
            .insert(thread_id.to_string(), vec![]);

        let mut params = json!({
            "threadId": thread_id,
            "input": input.iter().map(Input::to_json).collect::<Vec<_>>(),
        });
        if let Some(s) = schema {
            params["outputSchema"] = s.clone();
        }
        let started = self
            .request_via(&tx, "turn/start", params, Duration::from_secs(60))
            .await
            .context("turn/start")?;
        // 响应里就有 turnId；打断要用它
        let turn_id = started
            .get("turn")
            .and_then(|t| t.get("id"))
            .or_else(|| started.get("turnId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match tokio::time::timeout(self.cfg.turn_budget, done_rx).await {
            Ok(Ok(note)) => {
                let items = self
                    .items
                    .lock()
                    .await
                    .remove(thread_id)
                    .unwrap_or_default();
                Ok(outcome(&note, items))
            }
            Ok(Err(_)) => anyhow::bail!("等 turn/completed 时通道断了（codex 可能已退出）"),
            Err(_) => {
                self.turn_waiters.lock().await.remove(thread_id);
                if !turn_id.is_empty() {
                    // 不打断的话它会一直跑下去烧 token
                    let _ = self
                        .notify_via(
                            &tx,
                            "turn/interrupt",
                            json!({"threadId": thread_id, "turnId": turn_id}),
                        )
                        .await;
                }
                let items = self
                    .items
                    .lock()
                    .await
                    .remove(thread_id)
                    .unwrap_or_default();
                Ok(TurnOutcome {
                    turn_id,
                    status: "interrupted".into(),
                    text: String::new(),
                    items,
                    error: format!("超过 {:?} 的时间盒", self.cfg.turn_budget),
                })
            }
        }
    }

    pub async fn shutdown(mut self) -> Result<()> {
        // 先客气地关 stdin，给它自己退出的机会；kill_on_drop 兜底
        drop(self.stdin.lock().await.shutdown().await);
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
        Ok(())
    }

    fn sender(&self) -> mpsc::Sender<String> {
        // 写线程持有真正的 stdin；这里只是拿一个发送端
        self.out_tx.clone()
    }

    async fn request_via(
        &self,
        tx: &mpsc::Sender<String>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (rtx, rrx) = oneshot::channel();
        self.pending.lock().await.insert(id, rtx);
        let line = format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        );
        tx.send(line).await.context("写 stdin")?;
        match tokio::time::timeout(timeout, rrx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => anyhow::bail!("{method} 失败：{e}"),
            Ok(Err(_)) => anyhow::bail!("{method} 的应答通道断了"),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                anyhow::bail!("{method} 超过 {timeout:?} 没有应答")
            }
        }
    }

    async fn notify_via(
        &self,
        tx: &mpsc::Sender<String>,
        method: &str,
        params: Value,
    ) -> Result<()> {
        let line = format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "method": method, "params": params})
        );
        tx.send(line).await.context("写 stdin")?;
        Ok(())
    }
}

/// 把一条进来的消息分派掉。
async fn route(
    v: Value,
    pending: &Pending,
    waiters: &Waiters,
    items: &Arc<Mutex<HashMap<String, Vec<Value>>>>,
    reply: &mpsc::Sender<String>,
) {
    let id = v.get("id").and_then(Value::as_i64);
    let method = v.get("method").and_then(Value::as_str).map(str::to_string);

    match (id, method.as_deref()) {
        // 应答
        (Some(id), None) => {
            if let Some(tx) = pending.lock().await.remove(&id) {
                let r = if let Some(e) = v.get("error") {
                    Err(e.to_string())
                } else {
                    Ok(v.get("result").cloned().unwrap_or(Value::Null))
                };
                let _ = tx.send(r);
            }
        }
        // 服务端请求：**必须回**，否则那条请求挂住、整个回合卡死
        (Some(id), Some(m)) => {
            tracing::warn!(方法 = m, "codex 发来审批请求，一律拒绝（深核是只读的）");
            let line = format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": id, "result": {"decision": DECLINE}})
            );
            let _ = reply.send(line).await;
        }
        // 通知
        (None, Some(m)) => {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            let thread = params
                .get("threadId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match m {
                "turn/completed" => {
                    if let Some(tx) = waiters.lock().await.remove(&thread) {
                        let _ = tx.send(params);
                    }
                }
                "item/completed" => {
                    // 只留完成的条目做审计；增量通知量很大，丢掉
                    items.lock().await.entry(thread).or_default().push(params);
                }
                _ => {}
            }
        }
        (None, None) => {}
    }
}

fn outcome(note: &Value, items: Vec<Value>) -> TurnOutcome {
    #[derive(Deserialize, Default)]
    struct Turn {
        #[serde(default)]
        id: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        error: Option<Value>,
    }
    let t: Turn = note
        .get("turn")
        .cloned()
        .and_then(|x| serde_json::from_value(x).ok())
        .unwrap_or_default();
    TurnOutcome {
        turn_id: t.id,
        status: t.status,
        text: agent_text(&items),
        error: t.error.map(|e| e.to_string()).unwrap_or_default(),
        items,
    }
}

/// 从条目流里把助手的文字按顺序拼出来。
fn agent_text(items: &[Value]) -> String {
    items
        .iter()
        .filter_map(|p| p.get("item"))
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("agentMessage"))
        .filter_map(|i| {
            i.get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| i.get("content").map(ToString::to_string))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
