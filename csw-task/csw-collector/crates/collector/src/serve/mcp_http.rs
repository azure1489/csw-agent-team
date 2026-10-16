//! 知识库 MCP 的远程入口：`POST /mcp`（Streamable HTTP 的最小子集）。
//!
//! 给**其他机器上的** codex 用（docs/情报收集员工作台_知识库MCP对外方案_20261016.md）。
//! 本机仍用 stdio 的 `csw-collector mcp`。两边是同一套工具、同一个 `Server::handle_line`，
//! 只换传输。
//!
//! - **专职线程**：工具的 future 借着 `rusqlite::Connection`，刻意不是 `Send`，
//!   放不进 axum 的处理函数。这里起一条线程、自己的 runtime，`Server` 住在里面；
//!   HTTP 那边只把一行 JSON-RPC 递进通道、等回复。请求因此排一个队顺序处理。
//! - **鉴权**：`Authorization: Bearer cswkb_…`，库里只存 SHA-256；令牌带权限范围
//!   （`magazine` / `kb`），`tools/list` 与 `tools/call` 都按它过滤。`memory`（Van 原话）不对外。
//! - **限速**：按令牌，每分钟 60 次，其中以图搜图 6 次。内存计数，重启清零。
//! - **审计**：每次请求一行 `mcp_calls`（含鉴权失败），与人工操作的 `audit` 表分开，保留 90 天。
//! - **无状态**：不发 `Mcp-Session-Id`，不提供 GET 推送流；批量请求回 400。
//!
//! 令牌与审计用**自己的一个库连接**，不去抢工作台页面那一个。

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use rand::Rng;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc, oneshot};

use csw_collector_core::Config;
use csw_collector_mcpsrv::server::{Server, peek};

/// 请求体上限：给 base64 图片（解码后 ≤5MB）留余量
const MAX_BODY: usize = 8 << 20;
/// 单次调用最多等多久
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// 排队上限。满了回 503，不无限堆积
const QUEUE: usize = 16;
/// 每枚令牌每分钟最多多少次请求
const RATE_ALL: usize = 60;
/// 其中以图搜图最多多少次
const RATE_IMAGE: usize = 6;
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// `last_used_at` 至多隔这么久写一次
const TOUCH_EVERY: Duration = Duration::from_secs(60);
/// 审计保留天数
const KEEP_DAYS: u32 = 90;
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 3600);
/// 令牌前缀
const PREFIX: &str = "cswkb_";

/// 可签的权限范围与各自开放的工具。`memory` 不在这里：选题准则卡里是 Van 的原话，不对外。
pub const SCOPES: &[(&str, &[&str])] = &[
    ("magazine", &["kb_magazine_search", "kb_image_search"]),
    ("kb", &["kb_search", "kb_similar_selected"]),
];

/// 专职线程里的 `Server` 怎么建。在那条线程自己的 runtime 上执行。
pub type ServerFactory =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<Server>>>> + Send + 'static>;

struct Job {
    line: String,
    allow: HashSet<&'static str>,
    reply: oneshot::Sender<Option<String>>,
}

pub struct McpHttp {
    tx: mpsc::Sender<Job>,
    db: Mutex<Connection>,
    /// 允许的浏览器 Origin（工作台自己的公网地址）
    origin: String,
    rate: std::sync::Mutex<HashMap<(i64, bool), VecDeque<Instant>>>,
    touched: std::sync::Mutex<HashMap<i64, Instant>>,
    pruned: std::sync::Mutex<Option<Instant>>,
}

/// 生产用：开自己的库连接，起专职线程（向量客户端克隆工作台那份，共用 GPU 锁），
/// 安静窗口接 `[schedule].gpu_quiet_*`。
pub fn router(cfg: &Config, svc: Arc<super::services::Services>) -> Result<Router> {
    let db = csw_collector_core::store::open(&cfg.db_path()).context("MCP 入口开库")?;
    let (from, to) = (
        cfg.schedule.gpu_quiet_from_utc.clone(),
        cfg.schedule.gpu_quiet_to_utc.clone(),
    );
    let paused: csw_collector_mcpsrv::tools::GpuPaused = Arc::new(move || {
        super::schedule::in_quiet_window(&from, &to, super::schedule::now_minute())
    });
    let c = cfg.clone();
    let vector = svc.vector.clone();
    let factory: ServerFactory = Box::new(move || {
        Box::pin(async move { crate::mcp::build_remote_server(&c, vector, paused).await })
    });
    Ok(build(db, &cfg.web.public_url, factory))
}

/// 装配：起专职线程，返回挂好 `/mcp` 的路由。测试也走这里。
pub fn build(db: Connection, origin: &str, factory: ServerFactory) -> Router {
    let (tx, rx) = mpsc::channel(QUEUE);
    spawn_worker(rx, factory);
    let st = Arc::new(McpHttp {
        tx,
        db: Mutex::new(db),
        origin: origin.trim_end_matches('/').to_string(),
        rate: Default::default(),
        touched: Default::default(),
        pruned: Default::default(),
    });
    Router::new()
        .route("/mcp", post(handle).get(not_allowed).delete(not_allowed))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .with_state(st)
}

/// 专职线程：2 个 worker 的多线程 runtime（lancedb 内部 spawn 的任务交给 worker），
/// `Server` 的 `!Send` future 在 `LocalSet` 里跑。一条一条处理。
fn spawn_worker(mut rx: mpsc::Receiver<Job>, factory: ServerFactory) {
    std::thread::Builder::new()
        .name("mcp-http".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("mcp-http-rt")
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(原因 = %e, "MCP 远程入口起不来 runtime，/mcp 将回 503");
                    return;
                }
            };
            let local = tokio::task::LocalSet::new();
            rt.block_on(local.run_until(async move {
                let server = match factory().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(原因 = %format!("{e:#}"), "MCP 远程入口建不起来，/mcp 将回 503");
                        return;
                    }
                };
                tracing::info!(工具 = ?server.tool_names(), "MCP 远程入口起来了（POST /mcp）");
                while let Some(job) = rx.recv().await {
                    // 调用方已经超时走了就不必再算
                    if job.reply.is_closed() {
                        continue;
                    }
                    let allow = &job.allow;
                    let r = server
                        .handle_line_scoped(&job.line, &|n: &str| allow.contains(n))
                        .await;
                    let _ = job.reply.send(r);
                }
            }));
        })
        .expect("起 MCP 专职线程");
}

async fn not_allowed() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        "只支持 POST（不提供服务端推送流，也不维护会话）",
    )
        .into_response()
}

/// 鉴权通过的令牌
#[derive(Debug, Clone)]
pub struct Authed {
    pub id: i64,
    pub name: String,
    pub scopes: Vec<String>,
}

impl Authed {
    fn allow(&self) -> HashSet<&'static str> {
        SCOPES
            .iter()
            .filter(|(s, _)| self.scopes.iter().any(|x| x == s))
            .flat_map(|(_, tools)| tools.iter().copied())
            .collect()
    }
}

async fn handle(State(st): State<Arc<McpHttp>>, headers: HeaderMap, body: Bytes) -> Response {
    let started = Instant::now();
    let ip = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // 浏览器跨站调用：带了 Origin 又不是工作台自己的，拒。codex 这类客户端不带 Origin
    if let Some(o) = headers.get(header::ORIGIN)
        && o.to_str().ok().map(|s| s.trim_end_matches('/')) != Some(st.origin.as_str())
    {
        st.audit(None, &ip, "origin_denied", "", "", false, started)
            .await;
        return (StatusCode::FORBIDDEN, "Origin 不被允许").into_response();
    }

    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim)
        .unwrap_or("");
    let authed = {
        let db = st.db.lock().await;
        verify(&db, token).unwrap_or_else(|e| {
            tracing::warn!(原因 = %format!("{e:#}"), "MCP 令牌校验查库失败");
            None
        })
    };
    let Some(authed) = authed else {
        st.audit(None, &ip, "auth_failed", "", "", false, started)
            .await;
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "令牌无效、已吊销或已过期",
        )
            .into_response();
    };
    st.touch(&authed).await;

    let v: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            st.audit(Some(authed.id), &ip, "bad_json", "", "", false, started)
                .await;
            return json_reply(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "不是合法 JSON"}}),
            );
        }
    };
    if v.is_array() {
        st.audit(Some(authed.id), &ip, "batch", "", "", false, started)
            .await;
        return json_reply(
            StatusCode::BAD_REQUEST,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "不支持批量请求"}}),
        );
    }
    let (method, tool) = peek(&v);
    let (method, tool) = (method.to_string(), tool.to_string());
    let brief = args_brief(&v);

    if let Some(wait) = st.rate_check(authed.id, tool == "kb_image_search") {
        st.audit(Some(authed.id), &ip, &method, &tool, "限速", false, started)
            .await;
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, wait.to_string())],
            "请求太频繁，稍后再试",
        )
            .into_response();
    }

    // 通知与客户端的回复：收下，不回内容
    if v.get("id").is_none() || v.get("method").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    if let Some(pv) = headers.get("mcp-protocol-version") {
        tracing::debug!(版本 = ?pv, "MCP 客户端协议版本");
    }

    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let (reply_tx, reply_rx) = oneshot::channel();
    let job = Job {
        line: String::from_utf8_lossy(&body).into_owned(),
        allow: authed.allow(),
        reply: reply_tx,
    };
    if st.tx.try_send(job).is_err() {
        st.audit(Some(authed.id), &ip, &method, &tool, &brief, false, started)
            .await;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "MCP 服务忙或没起来，稍后再试",
        )
            .into_response();
    }
    let reply = match tokio::time::timeout(CALL_TIMEOUT, reply_rx).await {
        Ok(Ok(Some(r))) => r,
        // 有 id 的请求 handle_line 一定回；走到这里是专职线程没了
        Ok(Ok(None)) | Ok(Err(_)) => {
            st.audit(Some(authed.id), &ip, &method, &tool, &brief, false, started)
                .await;
            return (StatusCode::SERVICE_UNAVAILABLE, "MCP 服务没起来").into_response();
        }
        Err(_) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "超时（60 秒），稍后再试"}}).to_string(),
    };
    let ok = serde_json::from_str::<Value>(&reply).is_ok_and(|r| {
        r.get("error").is_none() && r.pointer("/result/isError") != Some(&Value::Bool(true))
    });
    if !tool.is_empty() {
        tracing::info!(令牌 = %authed.name, 工具 = %tool, 成功 = ok, 毫秒 = started.elapsed().as_millis() as u64, "远程 MCP 调用");
    }
    st.audit(Some(authed.id), &ip, &method, &tool, &brief, ok, started)
        .await;
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        reply,
    )
        .into_response()
}

fn json_reply(code: StatusCode, v: Value) -> Response {
    (
        code,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        v.to_string(),
    )
        .into_response()
}

/// 审计里记的参数摘要：查询词前 200 字；图片只记来源与大小，不记内容
fn args_brief(v: &Value) -> String {
    let Some(a) = v.pointer("/params/arguments") else {
        return String::new();
    };
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("");
    if !s("image_url").is_empty() {
        return format!(
            "image_url {}",
            s("image_url").chars().take(200).collect::<String>()
        );
    }
    if !s("image_base64").is_empty() {
        return format!("image_base64 {} 字符", s("image_base64").len());
    }
    let q = ["query", "text", "keyword"]
        .iter()
        .map(|k| s(k))
        .find(|x| !x.is_empty())
        .unwrap_or("");
    q.chars().take(200).collect()
}

impl McpHttp {
    /// 超限时返回要等的秒数
    fn rate_check(&self, id: i64, image: bool) -> Option<u64> {
        let now = Instant::now();
        let mut m = self.rate.lock().unwrap_or_else(|e| e.into_inner());
        let mut over = None;
        let mut buckets = vec![(false, RATE_ALL)];
        if image {
            buckets.push((true, RATE_IMAGE));
        }
        for (k, cap) in &buckets {
            let q = m.entry((id, *k)).or_default();
            while q
                .front()
                .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
            {
                q.pop_front();
            }
            if q.len() >= *cap {
                let wait = RATE_WINDOW.saturating_sub(now.duration_since(q[0]));
                over = Some(over.unwrap_or(0).max(wait.as_secs() + 1));
            }
        }
        if over.is_none() {
            for (k, _) in &buckets {
                m.entry((id, *k)).or_default().push_back(now);
            }
        }
        over
    }

    async fn touch(&self, a: &Authed) {
        {
            let mut t = self.touched.lock().unwrap_or_else(|e| e.into_inner());
            if t.get(&a.id).is_some_and(|x| x.elapsed() < TOUCH_EVERY) {
                return;
            }
            t.insert(a.id, Instant::now());
        }
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "UPDATE mcp_tokens SET last_used_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?1",
            [a.id],
        ) {
            tracing::warn!(原因 = %e, "记 MCP 令牌最近使用时间失败");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn audit(
        &self,
        token_id: Option<i64>,
        ip: &str,
        method: &str,
        tool: &str,
        brief: &str,
        ok: bool,
        started: Instant,
    ) {
        let prune = {
            let mut p = self.pruned.lock().unwrap_or_else(|e| e.into_inner());
            let due = p.is_none_or(|t| t.elapsed() >= PRUNE_EVERY);
            if due {
                *p = Some(Instant::now());
            }
            due
        };
        let db = self.db.lock().await;
        let ms = started.elapsed().as_millis() as i64;
        if let Err(e) = db.execute(
            "INSERT INTO mcp_calls(token_id, ip, method, tool, args_brief, ok, ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
            params![token_id, ip, method, tool, brief, ok, ms],
        ) {
            tracing::warn!(原因 = %e, "写 MCP 审计失败");
        }
        if prune
            && let Err(e) = db.execute(
                "DELETE FROM mcp_calls WHERE created_at < strftime('%Y-%m-%dT%H:%M:%SZ','now', ?1)",
                [format!("-{KEEP_DAYS} days")],
            )
        {
            tracing::warn!(原因 = %e, "清理旧的 MCP 审计失败");
        }
    }
}

// ─────────────────────────── 令牌 ───────────────────────────

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_token() -> String {
    const T: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::rng();
    let tail: String = (0..32)
        .map(|_| T[rng.random_range(0..T.len())] as char)
        .collect();
    format!("{PREFIX}{tail}")
}

/// 校验权限范围：只认 [`SCOPES`] 里的；`memory` 明确拒。返回去重后的规范串。
pub fn parse_scopes(raw: &str) -> Result<String> {
    let mut out: Vec<&str> = Vec::new();
    for s in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        anyhow::ensure!(
            s != "memory",
            "memory（选题准则卡，含 Van 原话）不对外，不能签"
        );
        let Some((name, _)) = SCOPES.iter().find(|(n, _)| *n == s) else {
            anyhow::bail!("不认识的权限范围 {s}（可选：magazine、kb）");
        };
        if !out.contains(name) {
            out.push(name);
        }
    }
    anyhow::ensure!(!out.is_empty(), "至少要给一个权限范围");
    Ok(out.join(","))
}

/// 签一枚令牌。返回 (id, 明文)。**明文只在这里出现一次**，库里只存哈希。
pub fn issue(
    conn: &Connection,
    name: &str,
    scopes: &str,
    days: Option<u32>,
) -> Result<(i64, String)> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "要写这枚令牌给谁用");
    let scopes = parse_scopes(scopes)?;
    let token = random_token();
    conn.execute(
        "INSERT INTO mcp_tokens(name, token_sha256, scopes, created_at, expires_at)
         VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ','now'),
                 CASE WHEN ?4 IS NULL THEN NULL
                      ELSE strftime('%Y-%m-%dT%H:%M:%SZ','now', '+' || ?4 || ' days') END)",
        params![name, sha256_hex(&token), scopes, days],
    )?;
    Ok((conn.last_insert_rowid(), token))
}

/// 查一枚令牌：没有、已吊销、已过期都是 None。
pub fn verify(conn: &Connection, token: &str) -> Result<Option<Authed>> {
    if !token.starts_with(PREFIX) {
        return Ok(None);
    }
    let row = conn
        .query_row(
            "SELECT id, name, scopes FROM mcp_tokens
             WHERE token_sha256 = ?1 AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
            [sha256_hex(token)],
            |r| {
                Ok(Authed {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    scopes: r
                        .get::<_, String>(2)?
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// 吊销。返回是否真吊销了一枚（不存在或早已吊销为 false）。
pub fn revoke(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE mcp_tokens SET revoked_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
         WHERE id = ?1 AND revoked_at IS NULL",
        [id],
    )?;
    Ok(n > 0)
}

/// 列出令牌（不含令牌本身）：id、名字、权限、签发、到期、吊销、最近使用
pub fn list(conn: &Connection) -> Result<Vec<[String; 7]>> {
    let mut st = conn.prepare(
        "SELECT id, name, scopes, created_at, COALESCE(expires_at,'不过期'),
                COALESCE(revoked_at,''), COALESCE(last_used_at,'')
         FROM mcp_tokens ORDER BY id",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok([
                r.get::<_, i64>(0)?.to_string(),
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ])
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use csw_collector_mcpsrv::server::{BoxFut, Tool};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// 假工具：回显工具名与参数，可选睡一会儿
    struct Fake(&'static str);
    impl Tool for Fake {
        fn name(&self) -> &'static str {
            self.0
        }
        fn description(&self) -> &'static str {
            "假的"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
            Box::pin(async move {
                if let Some(ms) = args.get("sleep_ms").and_then(Value::as_u64) {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                Ok(format!(
                    "{}:{}",
                    self.0,
                    args.get("query").and_then(Value::as_str).unwrap_or("")
                ))
            })
        }
    }

    fn app() -> (Router, std::path::PathBuf, tempdir::TempDir) {
        let d = tempdir::TempDir::new("mcphttp").unwrap();
        let p = d.path().join("c.db");
        let db = csw_collector_core::store::open(&p).unwrap();
        let factory: ServerFactory = Box::new(|| {
            Box::pin(async {
                Ok(Server::new("csw-kb", "t")
                    .with(Box::new(Fake("kb_search")))
                    .with(Box::new(Fake("kb_similar_selected")))
                    .with(Box::new(Fake("memory_lookup")))
                    .with(Box::new(Fake("kb_magazine_search")))
                    .with(Box::new(Fake("kb_image_search"))))
            })
        });
        (build(db, "https://collector.aworld.ltd/", factory), p, d)
    }

    fn db(p: &std::path::Path) -> Connection {
        csw_collector_core::store::open(p).unwrap()
    }

    fn req(token: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    async fn send(app: &Router, r: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let resp = app.clone().oneshot(r).await.unwrap();
        let (parts, body) = resp.into_parts();
        let b = body.collect().await.unwrap().to_bytes();
        (
            parts.status,
            parts.headers,
            String::from_utf8_lossy(&b).into_owned(),
        )
    }

    fn call(id: u64, tool: &str, args: Value) -> String {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":tool,"arguments":args}})
            .to_string()
    }

    const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

    #[test]
    fn 权限范围只认magazine与kb() {
        assert_eq!(parse_scopes("magazine, kb,kb").unwrap(), "magazine,kb");
        assert!(format!("{:#}", parse_scopes("kb,memory").unwrap_err()).contains("不对外"));
        assert!(parse_scopes("admin").is_err());
        assert!(parse_scopes(" , ").is_err());
    }

    #[tokio::test]
    async fn 没令牌错令牌吊销过期都是401() {
        let (app, p, _d) = app();
        let c = db(&p);
        let (_, good) = issue(&c, "甲", "magazine", None).unwrap();
        let (rid, revoked) = issue(&c, "乙", "magazine", None).unwrap();
        assert!(revoke(&c, rid).unwrap());
        assert!(!revoke(&c, rid).unwrap(), "重复吊销不算");
        let (eid, expired) = issue(&c, "丙", "magazine", Some(30)).unwrap();
        c.execute(
            "UPDATE mcp_tokens SET expires_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
            [eid],
        )
        .unwrap();

        let (s, _, _) = send(&app, req(Some(&good), LIST)).await;
        assert_eq!(s, StatusCode::OK);
        for t in [
            None,
            Some("cswkb_nope"),
            Some("x"),
            Some(revoked.as_str()),
            Some(expired.as_str()),
        ] {
            let (s, h, _) = send(&app, req(t, LIST)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{t:?}");
            assert_eq!(h.get("www-authenticate").unwrap(), "Bearer");
        }
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM mcp_calls WHERE method='auth_failed' AND token_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 5);
        // 库里只有哈希
        let hit: i64 = c
            .query_row(
                "SELECT count(*) FROM mcp_tokens WHERE token_sha256 = ?1",
                [&good],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hit, 0);
        let rows = list(&c).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter()
                .all(|r| !r.iter().any(|f| f.starts_with(PREFIX)))
        );
    }

    #[tokio::test]
    async fn 传输层的几种回法() {
        let (app, p, _d) = app();
        let (_, t) = issue(&db(&p), "甲", "magazine,kb", None).unwrap();
        // 通知 202、空体
        let (s, _, b) = send(
            &app,
            req(
                Some(&t),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            ),
        )
        .await;
        assert_eq!((s, b.as_str()), (StatusCode::ACCEPTED, ""));
        // 批量 400
        let (s, _, _) = send(&app, req(Some(&t), &format!("[{LIST}]"))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        // 坏 JSON 400
        let (s, _, _) = send(&app, req(Some(&t), "{不是")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        // GET / DELETE 405
        for m in ["GET", "DELETE"] {
            let r = Request::builder()
                .method(m)
                .uri("/mcp")
                .body(Body::empty())
                .unwrap();
            let (s, _, _) = send(&app, r).await;
            assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED, "{m}");
        }
        // Origin 不对 403；对的放行
        let mut r = req(Some(&t), LIST);
        r.headers_mut()
            .insert("origin", HeaderValue::from_static("https://evil.example"));
        assert_eq!(send(&app, r).await.0, StatusCode::FORBIDDEN);
        let mut r = req(Some(&t), LIST);
        r.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://collector.aworld.ltd"),
        );
        assert_eq!(send(&app, r).await.0, StatusCode::OK);
        // initialize：JSON 回复，不发会话 id
        let (s, h, b) = send(
            &app,
            req(
                Some(&t),
                r#"{"jsonrpc":"2.0","id":9,"method":"initialize","params":{}}"#,
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(h.get("content-type").unwrap(), "application/json");
        assert!(h.get("mcp-session-id").is_none());
        let v: Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["id"], 9);
        assert_eq!(v["result"]["serverInfo"]["name"], "csw-kb");
    }

    #[tokio::test]
    async fn 按权限范围过滤且memory永不出现() {
        let (app, p, _d) = app();
        let c = db(&p);
        let (_, mag) = issue(&c, "甲", "magazine", None).unwrap();
        let (_, both) = issue(&c, "乙", "magazine,kb", None).unwrap();
        // 就算库里被人手改成带 memory，也不开放
        let (mid, sneaky) = issue(&c, "丙", "kb", None).unwrap();
        c.execute(
            "UPDATE mcp_tokens SET scopes='kb,memory' WHERE id=?1",
            [mid],
        )
        .unwrap();

        let names = |b: &str| -> Vec<String> {
            let v: Value = serde_json::from_str(b).unwrap();
            v["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let (_, _, b) = send(&app, req(Some(&mag), LIST)).await;
        assert_eq!(names(&b), ["kb_image_search", "kb_magazine_search"]);
        let (_, _, b) = send(&app, req(Some(&both), LIST)).await;
        assert_eq!(
            names(&b),
            [
                "kb_image_search",
                "kb_magazine_search",
                "kb_search",
                "kb_similar_selected"
            ]
        );
        let (_, _, b) = send(&app, req(Some(&sneaky), LIST)).await;
        assert!(!names(&b).contains(&"memory_lookup".to_string()), "{b}");

        // 越权调用：isError，并记审计
        let (s, _, b) = send(
            &app,
            req(
                Some(&mag),
                &call(3, "kb_search", json!({"query": "山と道"})),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let v: Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["result"]["isError"], true, "{b}");
        let (s, _, b) = send(
            &app,
            req(
                Some(&sneaky),
                &call(4, "memory_lookup", json!({"query": "配色"})),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(b.contains("没有 memory_lookup 的权限"), "{b}");
        let (_, _, b) = send(
            &app,
            req(
                Some(&mag),
                &call(5, "kb_magazine_search", json!({"query": "snow peak"})),
            ),
        )
        .await;
        assert!(b.contains("kb_magazine_search:snow peak"), "{b}");

        let rows: Vec<(String, String, i64)> = c
            .prepare(
                "SELECT tool, args_brief, ok FROM mcp_calls WHERE method='tools/call' ORDER BY id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                ("kb_search".into(), "山と道".into(), 0),
                ("memory_lookup".into(), "配色".into(), 0),
                ("kb_magazine_search".into(), "snow peak".into(), 1),
            ]
        );
        let used: Option<String> = c
            .query_row(
                "SELECT last_used_at FROM mcp_tokens WHERE name='甲'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(used.is_some());
    }

    #[tokio::test]
    async fn 以图搜图第7次限速() {
        let (app, p, _d) = app();
        let (_, t) = issue(&db(&p), "甲", "magazine", None).unwrap();
        for i in 0..6 {
            let (s, _, _) = send(
                &app,
                req(
                    Some(&t),
                    &call(i, "kb_image_search", json!({"image_base64": "AAAA"})),
                ),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "第 {} 次", i + 1);
        }
        let (s, h, _) = send(
            &app,
            req(
                Some(&t),
                &call(7, "kb_image_search", json!({"image_base64": "AAAA"})),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert!(h.get("retry-after").is_some());
        // 文字检索不受以图搜图那一档影响
        let (s, _, _) = send(
            &app,
            req(
                Some(&t),
                &call(8, "kb_magazine_search", json!({"query": "a"})),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let n: i64 = db(&p)
            .query_row(
                "SELECT count(*) FROM mcp_calls WHERE args_brief='限速'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn 总量每分钟60次() {
        let (app, p, _d) = app();
        let (_, t) = issue(&db(&p), "甲", "magazine", None).unwrap();
        for _ in 0..RATE_ALL {
            assert_eq!(send(&app, req(Some(&t), LIST)).await.0, StatusCode::OK);
        }
        assert_eq!(
            send(&app, req(Some(&t), LIST)).await.0,
            StatusCode::TOO_MANY_REQUESTS
        );
        // 另一枚令牌不受影响
        let (_, t2) = issue(&db(&p), "乙", "magazine", None).unwrap();
        assert_eq!(send(&app, req(Some(&t2), LIST)).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn 并发请求回复不串号() {
        let (app, p, _d) = app();
        let (_, t) = issue(&db(&p), "甲", "magazine,kb", None).unwrap();
        let mut hs = Vec::new();
        for i in 0..10u64 {
            let app = app.clone();
            let t = t.clone();
            hs.push(tokio::spawn(async move {
                let q = format!("q{i}");
                let (s, _, b) = send(
                    &app,
                    req(
                        Some(&t),
                        &call(
                            100 + i,
                            "kb_search",
                            json!({"query": q, "sleep_ms": (10 - i) * 5}),
                        ),
                    ),
                )
                .await;
                assert_eq!(s, StatusCode::OK);
                let v: Value = serde_json::from_str(&b).unwrap();
                assert_eq!(v["id"], 100 + i);
                assert_eq!(v["result"]["content"][0]["text"], format!("kb_search:q{i}"));
            }));
        }
        for h in hs {
            h.await.unwrap();
        }
    }

    #[test]
    fn 审计摘要不记图片内容() {
        let v = json!({"params":{"arguments":{"image_base64": "A".repeat(1000)}}});
        assert_eq!(args_brief(&v), "image_base64 1000 字符");
        let v = json!({"params":{"arguments":{"query": "字".repeat(300)}}});
        assert_eq!(args_brief(&v).chars().count(), 200);
    }
}
