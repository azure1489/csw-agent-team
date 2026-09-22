//! 录制回放层：模型、Jev、向量、重排四个出网客户端共用一层。
//!
//! 为什么要它：一轮真跑要花 95 分钟网关时间和几百万 token（阶段 0.4 / 0.5 实测）。
//! 回归测试不能每次都付这个钱，也不能每次都等这么久。录一次 9/17–9/18 的窗口，
//! 之后所有回归都在回放模式下跑，既不出网也不花钱。
//!
//! **回放模式下未命中就是失败，绝不静默穿透。** 悄悄去调真接口的回放层等于没有回放层——
//! 测试会时快时慢、时对时错，而且会偷偷花钱。
//!
//! 键 = 规范化后的请求哈希。规范化时把 `data:image/...;base64,…` 换成
//! `blake3:<hex>`：夹具从几十 MB 降到几十 KB，而且同一张图无论重新下载多少次键都一样。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 直连，不读不写夹具
    Passthrough,
    /// 直连，同时把请求与响应写进夹具
    Record,
    /// 只读夹具；未命中即失败
    Replay,
}

impl Mode {
    pub fn from_env() -> Self {
        match std::env::var("CSW_COLLECTOR_RECORD")
            .unwrap_or_default()
            .as_str()
        {
            "record" => Self::Record,
            "replay" => Self::Replay,
            _ => Self::Passthrough,
        }
    }
}

pub struct Recorder {
    mode: Mode,
    dir: PathBuf,
}

impl Recorder {
    pub fn new(mode: Mode, dir: impl Into<PathBuf>) -> Self {
        Self {
            mode,
            dir: dir.into(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 包住一次出网调用。`client` 是夹具的分目录名（model / jev / embed / rerank）。
    pub async fn wrap<F, Fut>(&self, client: &str, request: &Value, call: F) -> Result<Value>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Value>>,
    {
        if self.mode == Mode::Passthrough {
            return call().await;
        }
        let key = fixture_key(request);
        let path = self.path(client, &key);
        if self.mode == Mode::Replay {
            let text = std::fs::read_to_string(&path).map_err(|_| {
                anyhow::anyhow!(
                    "回放未命中：{client}/{key}。请求变了就得重录，\
                     不会去调真接口——那样测试会偷偷花钱。"
                )
            })?;
            let saved: Fixture = serde_json::from_str(&text)
                .with_context(|| format!("夹具损坏 {}", path.display()))?;
            return Ok(saved.response);
        }
        // Record：先真调，再落盘
        let response = call().await?;
        let fx = Fixture {
            client: client.to_string(),
            key: key.clone(),
            request: normalize(request),
            response: response.clone(),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&fx)?)
            .with_context(|| format!("写夹具 {}", path.display()))?;
        Ok(response)
    }

    /// 某个客户端录过的全部请求。回放时拿来认出录的是哪个窗口——
    /// 人不该被要求记住上次录制用的是什么参数。
    pub fn requests(&self, client: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.dir.join(client)) else {
            return out;
        };
        for e in rd.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            if let Ok(fx) = serde_json::from_str::<Fixture>(&text) {
                out.push(fx.request);
            }
        }
        out
    }

    fn path(&self, client: &str, key: &str) -> PathBuf {
        self.dir.join(client).join(format!("{key}.json"))
    }

    /// 夹具里有多少条，按客户端分。跑回归前先看一眼，别拿空目录跑。
    pub fn inventory(&self) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for e in rd.flatten() {
            if e.path().is_dir() {
                let n = std::fs::read_dir(e.path())
                    .map(|d| d.flatten().count())
                    .unwrap_or(0);
                out.push((e.file_name().to_string_lossy().into_owned(), n));
            }
        }
        out.sort();
        out
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Fixture {
    client: String,
    key: String,
    request: Value,
    response: Value,
}

/// 规范化：把 base64 图片换成内容摘要。
///
/// 只换图，不动别的——正文、提示词、schema 一个字都不能变，
/// 否则「请求变了」这件事就查不出来了，而那正是回放该拦住的。
pub fn normalize(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(shrink_data_url(s)),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Object(o) => {
            // BTreeMap 序，键序稳定，哈希才稳定
            let mut m = serde_json::Map::new();
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            for k in keys {
                m.insert(k.clone(), normalize(&o[k]));
            }
            Value::Object(m)
        }
        other => other.clone(),
    }
}

fn shrink_data_url(s: &str) -> String {
    let Some(idx) = s.find(";base64,") else {
        return s.to_string();
    };
    let (head, tail) = s.split_at(idx + ";base64,".len());
    let mime = head.strip_prefix("data:").unwrap_or(head);
    format!(
        "data:{}blake3:{}",
        mime,
        blake3::hash(tail.as_bytes()).to_hex()
    )
}

pub fn fixture_key(request: &Value) -> String {
    let canon = serde_json::to_vec(&normalize(request)).unwrap_or_default();
    blake3::hash(&canon).to_hex()[..32].to_string()
}

/// 夹具目录默认位置
pub fn default_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("fixtures")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "csw-record-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn 键不受键序影响() {
        let a = json!({"model":"m","input":[1,2],"text":{"a":1,"b":2}});
        let b = json!({"input":[1,2],"text":{"b":2,"a":1},"model":"m"});
        assert_eq!(fixture_key(&a), fixture_key(&b));
    }

    #[test]
    fn 图片按内容摘要收进键里而不是整段base64() {
        let big = "A".repeat(200_000);
        let v = json!({"image_url":{"url":format!("data:image/jpeg;base64,{big}")}});
        let n = normalize(&v);
        let s = serde_json::to_string(&n).unwrap();
        assert!(s.len() < 200, "规范化后还是很大：{}", s.len());
        assert!(s.contains("blake3:"));
        // 同样的图 → 同样的键；换一张图 → 键要变
        let v2 =
            json!({"image_url":{"url":format!("data:image/jpeg;base64,{}", "B".repeat(200_000))}});
        assert_eq!(fixture_key(&v), fixture_key(&v.clone()));
        assert_ne!(fixture_key(&v), fixture_key(&v2));
    }

    #[test]
    fn 正文改一个字键就变() {
        let a = json!({"input":"这条贴文只换了配色"});
        let b = json!({"input":"这条贴文只换了配色。"});
        assert_ne!(fixture_key(&a), fixture_key(&b));
    }

    #[tokio::test]
    async fn 录完能回放且回放不再调真接口() {
        let dir = tmp();
        let req = json!({"model":"m","input":"hi"});
        let rec = Recorder::new(Mode::Record, &dir);
        let got = rec
            .wrap("model", &req, || async { Ok(json!({"ok":1})) })
            .await
            .unwrap();
        assert_eq!(got, json!({"ok":1}));
        assert_eq!(rec.inventory(), vec![("model".to_string(), 1)]);

        let rep = Recorder::new(Mode::Replay, &dir);
        let got = rep
            .wrap("model", &req, || async {
                panic!("回放模式不该调真接口")
            })
            .await
            .unwrap();
        assert_eq!(got, json!({"ok":1}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 回放未命中要报错不能偷偷穿透() {
        let dir = tmp();
        let rep = Recorder::new(Mode::Replay, &dir);
        let err = rep
            .wrap("model", &json!({"input":"没录过"}), || async {
                Ok(json!({"ok":1}))
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("回放未命中"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 直连模式不读不写夹具() {
        let dir = tmp();
        let rec = Recorder::new(Mode::Passthrough, &dir);
        rec.wrap("model", &json!({"a":1}), || async { Ok(json!({"ok":1})) })
            .await
            .unwrap();
        assert!(rec.inventory().is_empty());
    }
}

#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::model::{ModelClient, ModelConfig};
    use std::sync::Arc;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ok_body(text: &str) -> serde_json::Value {
        serde_json::json!({
            "output": [{"content": [{"type": "output_text", "text": text}]}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        })
    }

    async fn client(uri: &str, rec: Arc<Recorder>) -> ModelClient {
        ModelClient::new(ModelConfig {
            base_url: uri.into(),
            api_key: "k".into(),
            model: "m".into(),
            fallback_model: String::new(),
            concurrency: 2,
            timeout: std::time::Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap()
        .with_recorder(rec)
    }

    fn parts() -> Vec<crate::model::Part> {
        vec![crate::model::Part::Text("正文".into())]
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false
        })
    }

    #[tokio::test]
    async fn 录一次之后回放不再出网() {
        let dir = tempdir::TempDir::new("rec").unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(r#"{"a":"甲"}"#)))
            // 只该被调一次：回放那次不出网
            .expect(1)
            .mount(&server)
            .await;

        let rec = Arc::new(Recorder::new(Mode::Record, dir.path()));
        let out = client(&server.uri(), rec)
            .await
            .structured(&parts(), "s", &schema(), 100)
            .await
            .unwrap();
        assert_eq!(out.text, r#"{"a":"甲"}"#);

        // 换成回放：同样的请求，同样的答案，但不碰网络
        let rec = Arc::new(Recorder::new(Mode::Replay, dir.path()));
        let out = client(&server.uri(), rec)
            .await
            .structured(&parts(), "s", &schema(), 100)
            .await
            .unwrap();
        assert_eq!(out.text, r#"{"a":"甲"}"#);
        assert_eq!(out.input_tokens, 10, "用量也要录下来");
    }

    #[tokio::test]
    async fn 回放未命中要失败不许静默穿透() {
        let dir = tempdir::TempDir::new("rec").unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("不该被调到")))
            // 一次都不该被调：悄悄去调真接口的回放层等于没有回放层
            .expect(0)
            .mount(&server)
            .await;

        let rec = Arc::new(Recorder::new(Mode::Replay, dir.path()));
        let e = client(&server.uri(), rec)
            .await
            .structured(&parts(), "s", &schema(), 100)
            .await
            .unwrap_err();
        let msg = format!("{e:#}");
        // 报错里要带键，好去对照是哪一条请求变了
        assert!(msg.contains("未命中") || msg.contains("夹具"), "{msg}");
    }

    #[tokio::test]
    async fn 请求变了就该未命中() {
        let dir = tempdir::TempDir::new("rec").unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(r#"{"a":"甲"}"#)))
            .mount(&server)
            .await;
        let rec = Arc::new(Recorder::new(Mode::Record, dir.path()));
        client(&server.uri(), rec)
            .await
            .structured(&parts(), "s", &schema(), 100)
            .await
            .unwrap();

        // 正文改了一个字 → 键变了 → 回放该拦住。
        // 这正是回放要拦的东西：提示词悄悄改了而夹具没更新
        let rec = Arc::new(Recorder::new(Mode::Replay, dir.path()));
        let other = vec![crate::model::Part::Text("改过的正文".into())];
        assert!(
            client(&server.uri(), rec)
                .await
                .structured(&other, "s", &schema(), 100)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn 夹具清单看得见每类多少条() {
        let dir = tempdir::TempDir::new("rec").unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("x")))
            .mount(&server)
            .await;
        let rec = Arc::new(Recorder::new(Mode::Record, dir.path()));
        for t in ["甲", "乙"] {
            let p = vec![crate::model::Part::Text(t.into())];
            let _ = client(&server.uri(), rec.clone())
                .await
                .structured(&p, "s", &schema(), 100)
                .await;
        }
        // 跑回归前先看一眼，别拿空目录跑
        assert_eq!(rec.inventory(), [("model".to_string(), 2)]);
    }
}
