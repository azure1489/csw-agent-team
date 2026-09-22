//! Qwen3-VL 向量与重排服务的客户端。
//!
//! **放在 `core` 而不是 `harvest` 或 `kb`，是因为它必须全进程只有一条队列。**
//! 阶段 0.5 实测：向量化与 rerank 并发时双方都被拖到约 4000 ms（单独跑分别是
//! 945 ms 与 3561 ms）——单卡串行，并发不但没有收益，还制造了 4 倍的延迟方差。
//! 两个 crate 各持一个客户端就等于又并发了，所以队列得在共用的这一层。
//!
//! 另外两条实测事实：
//! - **入参是扁平数组**，元素要么是字符串（纯文本），要么是 **content-part 对象数组**
//!   （`{"type":"text",…}` / `{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,…"}}`）。
//!   裸字符串放进内层数组会 400。
//! - **显存很脆，而且 OOM 会赖着不走**：22 GB 卡上批 24（1500 字正文）直接 CUDA OOM，
//!   紧接着的批 1 也会失败。所以 OOM 要当可重试、对半降批，并且**退避要给它时间回收**。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::sync::Mutex;

/// 一条输入：纯文本，或图文融合（正文 + 若干张图）。
#[derive(Debug, Clone)]
pub enum EmbedInput {
    Text(String),
    /// 图文融合成**一个**向量
    Fused {
        text: String,
        images_b64: Vec<String>,
    },
    Image(String),
}

impl EmbedInput {
    fn to_json(&self) -> serde_json::Value {
        match self {
            // 纯文本直接放字符串——放进内层数组会 400
            Self::Text(t) => serde_json::Value::String(t.clone()),
            Self::Fused { text, images_b64 } => {
                let mut parts = vec![serde_json::json!({"type": "text", "text": text})];
                parts.extend(images_b64.iter().map(img_part));
                serde_json::Value::Array(parts)
            }
            Self::Image(b64) => serde_json::Value::Array(vec![img_part(b64)]),
        }
    }

    /// 这条输入在显存上大致有多"重"，用来决定一批装多少。
    /// 图比文本贵一个数量级（实测：纯文本约 100 ms，含一张图约 460 ms）。
    fn weight(&self) -> usize {
        match self {
            Self::Text(t) => 1 + t.chars().count() / 800,
            Self::Fused { text, images_b64 } => {
                2 + text.chars().count() / 800 + images_b64.len() * 4
            }
            Self::Image(_) => 4,
        }
    }
}

fn img_part(b64: &String) -> serde_json::Value {
    serde_json::json!({"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{b64}")}})
}

#[derive(Debug, Clone)]
pub struct VectorConfig {
    pub base_url: String,
    pub timeout: Duration,
    /// 一批的"重量"上限。8 大致等于 8 条短文本、或 2 条图文融合、或 2 张图。
    /// 实测的安全批量：文本 ≤8、图文融合 ≤4、纯图 ≤2。
    pub batch_weight: usize,
    /// OOM 之后先歇一会儿再试——显存不是立刻还回来的
    pub oom_cooldown: Duration,
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8022".into(),
            timeout: Duration::from_secs(120),
            batch_weight: 8,
            oom_cooldown: Duration::from_secs(3),
        }
    }
}

#[derive(Debug, Deserialize)]
struct EmbedResp {
    data: Vec<EmbedItem>,
}

#[derive(Debug, Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct RerankResp {
    #[serde(default)]
    results: Vec<RerankItem>,
}

#[derive(Debug, Deserialize)]
pub struct RerankItem {
    pub index: usize,
    #[serde(default)]
    pub relevance_score: f64,
}

/// 向量服务客户端。**克隆它是安全的**——队列在 `Arc<Mutex>` 里，克隆出来的还是同一条。
#[derive(Clone)]
pub struct VectorClient {
    cfg: VectorConfig,
    http: reqwest::Client,
    /// 录制回放。`None` 等同直连。
    rec: Option<Arc<crate::record::Recorder>>,
    /// 单卡串行的那把锁。**所有**出网调用都要先拿到它。
    gpu: Arc<Mutex<()>>,
}

impl VectorClient {
    pub fn new(cfg: VectorConfig) -> Result<Self> {
        crate::ensure_crypto_provider();
        let http = reqwest::Client::builder().timeout(cfg.timeout).build()?;
        Ok(Self {
            cfg,
            http,
            gpu: Arc::new(Mutex::new(())),
            rec: None,
        })
    }

    /// 挂上录制回放层。回放模式下**不出网**，未命中即失败。
    pub fn with_recorder(mut self, rec: Arc<crate::record::Recorder>) -> Self {
        self.rec = Some(rec);
        self
    }

    /// 向量化。按重量自动分批，OOM 自动对半降批。
    ///
    /// 返回的顺序与输入一致——调用方靠下标对回自己的候选，乱序会静默错配。
    pub async fn embed(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(inputs.len());
        for batch in pack(inputs, self.cfg.batch_weight) {
            let mut vs = self.embed_batch(&inputs[batch.clone()]).await?;
            out.append(&mut vs);
        }
        anyhow::ensure!(
            out.len() == inputs.len(),
            "向量条数与输入对不上：{} vs {}",
            out.len(),
            inputs.len()
        );
        Ok(out)
    }

    /// 一批，带 OOM 降批。降到 1 条还 OOM 就认输——那是单条太大，分批救不了。
    async fn embed_batch(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        match self.embed_once(inputs).await {
            Ok(v) => Ok(v),
            Err(e) if is_oom(&e) && inputs.len() > 1 => {
                let half = inputs.len() / 2;
                tracing::warn!(原批 = inputs.len(), 新批 = half, "显存不足，对半降批重试");
                // 给显存一点时间回收：OOM 之后紧接着的小批也会失败
                tokio::time::sleep(self.cfg.oom_cooldown).await;
                let mut a = Box::pin(self.embed_batch(&inputs[..half])).await?;
                let b = Box::pin(self.embed_batch(&inputs[half..])).await?;
                a.extend(b);
                Ok(a)
            }
            Err(e) => Err(e),
        }
    }

    async fn embed_once(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        let body = serde_json::json!({
            "input": inputs.iter().map(EmbedInput::to_json).collect::<Vec<_>>()
        });
        let _guard = self.gpu.lock().await;
        let r: EmbedResp = self.post("/v1/embeddings", &body).await?;
        anyhow::ensure!(r.data.len() == inputs.len(), "返回条数与输入对不上");
        Ok(r.data.into_iter().map(|d| d.embedding).collect())
    }

    /// 重排。返回 `(原下标, 分数)`，**按服务返回的顺序**，不替调用方排序——
    /// 排序规则是业务的事，客户端不该替它定。
    pub async fn rerank(
        &self,
        query: &str,
        documents: &[String],
        instruction: &str,
    ) -> Result<Vec<RerankItem>> {
        if documents.is_empty() {
            return Ok(vec![]);
        }
        let mut body = serde_json::json!({ "query": query, "documents": documents });
        if !instruction.is_empty() {
            body["instruction"] = serde_json::Value::String(instruction.to_string());
        }
        let _guard = self.gpu.lock().await;
        let r: RerankResp = self.post("/v1/rerank", &body).await?;
        Ok(r.results)
    }

    pub async fn healthy(&self) -> bool {
        self.http
            .get(format!(
                "{}/health",
                self.cfg.base_url.trim_end_matches('/')
            ))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        if let Some(rec) = &self.rec {
            // 夹具按用途分目录：embed 与 rerank 的请求形状不一样，混在一起不好查
            let client = if path.contains("rerank") {
                "rerank"
            } else {
                "embed"
            };
            let raw = rec
                .wrap(client, body, || async { self.post_http(path, body).await })
                .await?;
            return serde_json::from_value(raw).with_context(|| format!("解析回放的 {path}"));
        }
        self.post_http(path, body)
            .await
            .and_then(|v| serde_json::from_value(v).with_context(|| format!("解析 {path}")))
    }

    async fn post_http(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        let resp = self
            .http
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("请求 {path}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "{path} 返回 {status}：{}",
                text.chars().take(300).collect::<String>()
            );
        }
        serde_json::from_str(&text).with_context(|| {
            format!(
                "解析 {path}；原文前 200 字：{}",
                text.chars().take(200).collect::<String>()
            )
        })
    }
}

/// 显存不足？服务把 CUDA OOM 包在 500 里回来。
fn is_oom(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}");
    s.contains("out of memory") || s.contains("CUDA out of memory")
}

/// 按重量把输入切成若干段。单条超重也自成一段——降批救不了单条太大，
/// 让它去撞 OOM 并如实报错，比在这里默默丢掉强。
fn pack(inputs: &[EmbedInput], max_weight: usize) -> Vec<std::ops::Range<usize>> {
    let max_weight = max_weight.max(1);
    let mut out = Vec::new();
    let (mut start, mut w) = (0usize, 0usize);
    for (i, x) in inputs.iter().enumerate() {
        let xw = x.weight();
        if i > start && w + xw > max_weight {
            out.push(start..i);
            start = i;
            w = 0;
        }
        w += xw;
    }
    if start < inputs.len() {
        out.push(start..inputs.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 纯文本放字符串图文融合放内层数组() {
        let t = EmbedInput::Text("露营".into()).to_json();
        assert!(t.is_string(), "裸字符串放进内层数组会 400");

        let f = EmbedInput::Fused {
            text: "正文".into(),
            images_b64: vec!["AAA".into()],
        }
        .to_json();
        let arr = f.as_array().unwrap();
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "image_url");
        assert_eq!(arr[1]["image_url"]["url"], "data:image/jpeg;base64,AAA");
    }

    #[test]
    fn 按重量分批而不是按条数() {
        let short: Vec<_> = (0..10).map(|_| EmbedInput::Text("短".into())).collect();
        // 短文本每条重 1，批重 8 → 8 + 2
        let b = pack(&short, 8);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], 0..8);

        // 图文融合每条重 6（2 + 0 + 4），批重 8 → 一批只装一条
        let fused: Vec<_> = (0..4)
            .map(|_| EmbedInput::Fused {
                text: "x".into(),
                images_b64: vec!["a".into()],
            })
            .collect();
        assert_eq!(pack(&fused, 8).len(), 4);

        // 长正文更重：1500 字的一条重 1 + 1 = 2
        let long = EmbedInput::Text("字".repeat(1600));
        assert!(long.weight() > EmbedInput::Text("短".into()).weight());
    }

    #[test]
    fn 单条超重也要自成一段而不是被丢掉() {
        let huge = vec![EmbedInput::Image("x".into())];
        assert_eq!(pack(&huge, 1), vec![0..1]);
    }

    #[test]
    fn 认得出显存不足() {
        assert!(is_oom(&anyhow::anyhow!(
            "/v1/embeddings 返回 500：{{\"detail\":{{\"error\":{{\"message\":\"Embed processing error: CUDA out of memory.\"}}}}}}"
        )));
        assert!(!is_oom(&anyhow::anyhow!(
            "/v1/embeddings 返回 400：input 非法"
        )));
    }

    /// 照着请求里的条数回同样多条向量——真服务就是这么回的。
    /// 固定回一条的 mock 会让「分批对不对」这件事测不出来。
    struct EchoLen;

    impl wiremock::Respond for EchoLen {
        fn respond(&self, req: &wiremock::Request) -> wiremock::ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            let n = body
                .get("input")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let data: Vec<_> = (0..n)
                .map(|_| serde_json::json!({"embedding": [0.1, 0.2]}))
                .collect();
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data}))
        }
    }

    #[tokio::test]
    async fn 显存不足会对半降批() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        // 头两次（批 4、批 2）都 OOM，之后放行
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                r#"{"detail":{"error":{"message":"Embed processing error: CUDA out of memory."}}}"#,
            ))
            .up_to_n_times(2)
            .mount(&srv)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(EchoLen)
            .mount(&srv)
            .await;

        let c = VectorClient::new(VectorConfig {
            base_url: srv.uri(),
            batch_weight: 100, // 一批装下全部四条，逼它去降批
            oom_cooldown: Duration::from_millis(1),
            ..Default::default()
        })
        .unwrap();
        let inputs: Vec<_> = (0..4).map(|i| EmbedInput::Text(format!("t{i}"))).collect();
        let vs = c.embed(&inputs).await.unwrap();
        assert_eq!(vs.len(), 4, "降到单条后四条都要拿到向量");
    }

    #[tokio::test]
    async fn 降到单条还不行就如实报错() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(500).set_body_string("CUDA out of memory"))
            .mount(&srv)
            .await;
        let c = VectorClient::new(VectorConfig {
            base_url: srv.uri(),
            oom_cooldown: Duration::from_millis(1),
            ..Default::default()
        })
        .unwrap();
        let err = c.embed(&[EmbedInput::Text("x".into())]).await.unwrap_err();
        assert!(is_oom(&err), "应当如实报显存不足，而不是吞掉：{err:#}");
    }
}
