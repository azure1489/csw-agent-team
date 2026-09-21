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
