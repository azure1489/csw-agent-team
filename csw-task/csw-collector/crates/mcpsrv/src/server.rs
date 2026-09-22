//! MCP 的 stdio 服务端。**自己写，不引 `rmcp`。**
//!
//! 理由与设计决定 3（codex 客户端自写）是同一条：这台机器上跑的是交叉编译出来的
//! 二进制，每引一个新依赖树就多一份「在开发机编得过、在目标机跑不动」的风险——
//! lancedb 0.39 与 aws-lc-rs 都是这么踩到的。而一个只有工具的 MCP 服务端，
//! 协议面就四个方法：`initialize`、`notifications/initialized`、`tools/list`、`tools/call`
//! （外加一个 `ping`）。传输是行分隔的 JSON-RPC 2.0，与 codex app-server 同一套框架。
//!
//! **只读。** 这个服务端没有任何会改东西的工具，也不接受把它变成可写的参数。

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 我们实现的协议版本。客户端要更新的版本时照它自己的来，我们回自己支持的这个。
const PROTOCOL_VERSION: &str = "2025-06-18";

/// 一个工具。
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON Schema，描述 `arguments`
    fn input_schema(&self) -> Value;
    /// 执行。返回给模型看的文字。
    ///
    /// 返回的是盒装 future 而不是 `async fn`：工具要查向量库（异步），
    /// 而 `async fn` 在 trait 里不是对象安全的，装不进 `Box<dyn Tool>`。
    fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a>;
}

/// 工具返回的 future。**刻意不要求 `Send`。**
///
/// 检索要借 `rusqlite::Connection`，而它不是 `Sync`，借着它的 future 就不是 `Send`。
/// 要满足 `Send` 得把每次查询扔进 `spawn_blocking`，那是为了一个用不上的性质
/// 去改结构。
///
/// 用不上是因为：**stdio 上的 MCP 本来就是顺序的**——一条管道、一次一条请求，
/// 这个服务端也就一条循环、不为每次调用开任务。真要并发处理，
/// 那时再连同 `Send` 一起改。
pub type BoxFut<'a> = Pin<Box<dyn Future<Output = Result<String>> + 'a>>;

pub struct Server {
    tools: BTreeMap<&'static str, Box<dyn Tool>>,
    name: String,
    version: String,
}

impl Server {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            tools: BTreeMap::new(),
            name: name.into(),
            version: version.into(),
        }
    }

    pub fn with(mut self, t: Box<dyn Tool>) -> Self {
        self.tools.insert(t.name(), t);
        self
    }

    pub fn tool_names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    /// 跑到 stdin 关掉为止。
    pub async fn serve_stdio(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut lines = BufReader::new(stdin).lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let Some(reply) = self.handle_line(&line).await else {
                continue;
            };
            stdout.write_all(reply.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        Ok(())
    }

    /// 处理一行。返回 None 表示这条是通知，不用回。
    pub async fn handle_line(&self, line: &str) -> Option<String> {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            // 连 JSON 都不是：按协议回 -32700，但没有 id 可回就只能丢掉
            Err(_) => return Some(err_line(Value::Null, -32700, "不是合法 JSON")),
        };
        let id = v.get("id").cloned();
        let method = v.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = v.get("params").cloned().unwrap_or(Value::Null);

        // 没有 id 的是通知，一律不回
        let id = id?;

        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": self.name, "version": self.version},
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.values().map(|t| json!({
                    "name": t.name(),
                    "description": t.description(),
                    "inputSchema": t.input_schema(),
                })).collect::<Vec<_>>()
            })),
            "tools/call" => self.call_tool(&params).await,
            other => Err(Fault::MethodNotFound(other.to_string())),
        };

        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}).to_string(),
            Err(Fault::MethodNotFound(m)) => err_line(id, -32601, &format!("没有这个方法：{m}")),
            Err(Fault::BadParams(m)) => err_line(id, -32602, &m),
        })
    }

    async fn call_tool(&self, params: &Value) -> Result<Value, Fault> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Fault::BadParams("tools/call 缺 name".into()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let Some(tool) = self.tools.get(name) else {
            return Err(Fault::BadParams(format!("没有这个工具：{name}")));
        };
        // 工具自己的错误**不是协议错误**：按 MCP 的约定回 isError 的正常结果，
        // 让模型看见「这次没查到，原因是什么」，而不是让客户端以为协议坏了
        Ok(match tool.call(&args).await {
            Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
            Err(e) => json!({
                "content": [{"type": "text", "text": format!("工具执行失败：{e:#}")}],
                "isError": true
            }),
        })
    }
}

enum Fault {
    MethodNotFound(String),
    BadParams(String),
}

fn err_line(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl Tool for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn description(&self) -> &'static str {
            "原样回显"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
        }
        fn call<'a>(&'a self, args: &'a Value) -> BoxFut<'a> {
            Box::pin(async move {
                let t = args.get("text").and_then(Value::as_str).unwrap_or_default();
                anyhow::ensure!(!t.is_empty(), "text 不能空");
                Ok(t.to_string())
            })
        }
    }

    fn server() -> Server {
        Server::new("csw-kb", "0.1.0").with(Box::new(Echo))
    }

    async fn call(s: &Server, line: &str) -> Value {
        serde_json::from_str(&s.handle_line(line).await.expect("该有回复")).unwrap()
    }

    #[tokio::test]
    async fn 握手回自己支持的协议版本与工具能力() {
        let r = call(
            &server(),
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        )
        .await;
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r["result"]["serverInfo"]["name"], "csw-kb");
    }

    #[tokio::test]
    async fn 通知不回复() {
        // 没有 id 的是通知。回了会让客户端多收一条对不上号的消息
        assert!(
            server()
                .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn 列工具带schema() {
        let r = call(
            &server(),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .await;
        let tools = r["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo");
        assert!(tools[0]["inputSchema"]["properties"]["text"].is_object());
    }

    #[tokio::test]
    async fn 调用成功回文字内容() {
        let r = call(
            &server(),
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"你好"}}}"#,
        )
        .await;
        assert_eq!(r["result"]["content"][0]["text"], "你好");
        assert_eq!(r["result"]["isError"], false);
    }

    #[tokio::test]
    async fn 工具自己失败不算协议错误() {
        // 让模型看见「这次没查到、原因是什么」，而不是让客户端以为协议坏了
        let r = call(
            &server(),
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{"text":""}}}"#,
        )
        .await;
        assert!(r.get("error").is_none(), "不该是协议错误：{r}");
        assert_eq!(r["result"]["isError"], true);
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("不能空")
        );
    }

    #[tokio::test]
    async fn 没有的方法与没有的工具各报各的() {
        let r = call(
            &server(),
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#,
        )
        .await;
        assert_eq!(r["error"]["code"], -32601);
        let r = call(
            &server(),
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"没这个"}}"#,
        )
        .await;
        assert_eq!(r["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn 坏json回解析错误而不是崩掉() {
        let r: Value =
            serde_json::from_str(&server().handle_line("{不是 json").await.unwrap()).unwrap();
        assert_eq!(r["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn 每条回复都是单行() {
        // stdio 上是行分隔的，回复里夹一个换行就把协议冲垮了
        let line = server()
            .handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"echo","arguments":{"text":"第一行\n第二行"}}}"#)
            .await
            .unwrap();
        assert!(!line.contains('\n'), "{line}");
    }
}
