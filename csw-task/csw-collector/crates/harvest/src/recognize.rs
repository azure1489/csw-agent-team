//! 图片识别：每张图连同该条正文送视觉模型，出结构化描述。
//!
//! 为什么必须自己做：线上 csw 的 `mediaList` 里 `shortCaption` / `detailedAlt` 覆盖率是 0，
//! 一期 1871 张图的描述一张都没有。
//!
//! 时间底数（阶段 0.5 实测，并发 8）：**每条贴文约 7.1 秒、每张图约 2.5 秒，
//! 一期 358 条约 42 分钟**——总方案原估的 5–7 分钟差了六到八倍。这一步必须放进预取轮。
//!
//! 失败的处理照口径来：**单张失败标「未识别」，候选照走**。
//! 它的 `image_seen` 会是 false，于是落待核——待核不是淘汰，补齐后重评。

use anyhow::Result;
use csw_collector_core::model::{ModelClient, Part};
use csw_collector_core::types::{ImageKind, MediaDescription};
use serde::Deserialize;

/// 提示词版本。**改提示词必须改这个**——`media_descriptions` 按
/// `(blake3, prompt_version)` 唯一，版本不变就不会重算，改了词却不改版本等于改了个寂寞。
pub const PROMPT_VERSION: &str = "recognize/v1";

/// 一批最多几张图。总方案定的是 ≤6；再多一批的延迟会顶到超时。
pub const MAX_IMAGES_PER_CALL: usize = 6;

const PROMPT: &str = r#"你在为户外生活媒体做素材识别。下面是一条贴文的正文和它的全部图片。

逐张判断，按给定顺序在 images 数组里各输出一条，index 从 1 开始，不要合并、不要漏、不要多。
每张图要写清：
- matches_text：这张图对应正文的哪一点。正文里找不到对应就写「正文未提及」。
- content：画面里实际有什么——产品、场景、可读到的文字。**只写看得见的，不要推测**。
- missing_from_text：正文说了但这张图里看不到的东西。没有就写「无」。
- kind：图片类型。
- usable_as_figure：这张图能不能直接当文章配图。

看不清就说看不清，不要编。"#;

const KINDS: [(&str, ImageKind); 7] = [
    ("产品图", ImageKind::Product),
    ("细节图", ImageKind::Detail),
    ("使用场景", ImageKind::Scene),
    ("海报或文字图", ImageKind::Poster),
    ("人物穿搭", ImageKind::Outfit),
    ("截图", ImageKind::Screenshot),
    ("无关", ImageKind::Unrelated),
];

#[derive(Debug, Deserialize)]
struct Batch {
    images: Vec<One>,
}

#[derive(Debug, Deserialize)]
struct One {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    matches_text: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    missing_from_text: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    usable_as_figure: bool,
}

/// 一张待识别的图：内容哈希 + base64。
#[derive(Debug, Clone)]
pub struct ImageRef {
    pub blake3: String,
    pub b64: String,
    pub ordinal: u16,
}

/// strict 模式要求每一层都写全 `required` 并关掉 `additionalProperties`。
pub fn schema() -> serde_json::Value {
    let kinds: Vec<&str> = KINDS.iter().map(|(k, _)| *k).collect();
    serde_json::json!({
        "type": "object", "additionalProperties": false,
        "required": ["images"],
        "properties": {
            "images": {
                "type": "array",
                "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["index", "matches_text", "content", "missing_from_text",
                                 "kind", "usable_as_figure"],
                    "properties": {
                        "index": {"type": "integer", "description": "第几张图，从 1 起"},
                        "matches_text": {"type": "string", "description": "对应正文哪一点；正文未提及就这么写"},
                        "content": {"type": "string", "description": "画面里实际有什么，只写看得见的"},
                        "missing_from_text": {"type": "string", "description": "正文提到但画面没有的；没有写「无」"},
                        "kind": {"type": "string", "enum": kinds},
                        "usable_as_figure": {"type": "boolean"}
                    }
                }
            }
        }
    })
}

fn parse_kind(s: &str) -> ImageKind {
    KINDS
        .iter()
        .find(|(k, _)| *k == s)
        .map(|(_, v)| *v)
        .unwrap_or(ImageKind::Unrelated)
}

/// 识别一条候选的一批图（≤[`MAX_IMAGES_PER_CALL`] 张）。
///
/// 返回的条数与输入对不上就是错：模型合并或漏了图，这种结果不能当成描述用——
/// 下标错位会让「第 3 张图显示…」这类依据全部指错地方。
pub async fn recognize_batch(
    model: &ModelClient,
    text: &str,
    images: &[ImageRef],
) -> Result<Vec<MediaDescription>> {
    anyhow::ensure!(!images.is_empty(), "没有图可识别");
    anyhow::ensure!(
        images.len() <= MAX_IMAGES_PER_CALL,
        "一批最多 {MAX_IMAGES_PER_CALL} 张，收到 {}",
        images.len()
    );
    // 正文是第三方写的，先过不可信边界：它马上要和「【正文】」这个结构标记
    // 拼在一起，而一条贴文的正文里原样写上那四个字就能伪造出一段
    let fenced = csw_collector_core::prompt::fence(text.trim());
    let mut parts = vec![Part::Text(format!(
        "{PROMPT}\n\n{}\n\n【正文】\n{}\n\n下面是这条贴文的 {} 张图，按顺序编号。",
        csw_collector_core::prompt::DATA_NOT_INSTRUCTIONS,
        if fenced.is_empty() {
            "（无正文）"
        } else {
            &fenced
        },
        images.len()
    ))];
    parts.extend(images.iter().map(|i| Part::ImageB64(i.b64.clone())));

    let out = model
        .structured(
            &parts,
            "media_descriptions",
            &schema(),
            400 * images.len() as u32 + 500,
        )
        .await?;
    let batch: Batch = out.parse()?;
    anyhow::ensure!(
        batch.images.len() == images.len(),
        "识别返回 {} 条，图有 {} 张——条数对不上会让「第几张图」的依据全部指错",
        batch.images.len(),
        images.len()
    );

    Ok(batch
        .images
        .into_iter()
        .enumerate()
        .map(|(i, one)| {
            // index 以我们给的顺序为准：模型偶尔会把 index 写错，但顺序一般是对的
            let img = &images[i];
            debug_assert!(one.index == 0 || one.index == i + 1, "index 与顺序不符");
            MediaDescription {
                blake3: img.blake3.clone(),
                ordinal: img.ordinal,
                matches_text: one.matches_text,
                content: one.content,
                missing_from_text: one.missing_from_text,
                kind: parse_kind(&one.kind),
                usable_as_figure: one.usable_as_figure,
                model: out.model.clone(),
                prompt_version: PROMPT_VERSION.to_string(),
            }
        })
        .collect())
}

/// 把一条候选的全部图切成若干批。
pub fn batches(images: &[ImageRef]) -> Vec<&[ImageRef]> {
    images.chunks(MAX_IMAGES_PER_CALL).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::model::ModelConfig;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn imgs(n: usize) -> Vec<ImageRef> {
        (0..n)
            .map(|i| ImageRef {
                blake3: format!("h{i}"),
                b64: "AAA".into(),
                ordinal: i as u16,
            })
            .collect()
    }

    fn client(uri: &str) -> ModelClient {
        ModelClient::new(ModelConfig {
            base_url: uri.into(),
            api_key: "sk".into(),
            model: "gpt-6-astra".into(),
            fallback_model: String::new(),
            concurrency: 2,
            timeout: Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap()
    }

    /// 数一数请求里有几张图，就回几条描述——真模型该这么干
    struct EchoImages;
    impl Respond for EchoImages {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let b: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            let n = b["input"][0]["content"]
                .as_array()
                .map(|a| a.iter().filter(|p| p["type"] == "input_image").count())
                .unwrap_or(0);
            let images: Vec<_> = (1..=n)
                .map(|i| {
                    serde_json::json!({
                        "index": i, "matches_text": "正文未提及", "content": format!("第{i}张：帐篷"),
                        "missing_from_text": "无", "kind": "产品图", "usable_as_figure": true
                    })
                })
                .collect();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "output": [{"content": [{"type": "output_text",
                    "text": serde_json::json!({"images": images}).to_string()}]}],
                "usage": {"input_tokens": 6104, "output_tokens": 666}
            }))
        }
    }

    #[test]
    fn schema每一层都关了additional_properties() {
        let s = schema();
        assert_eq!(s["additionalProperties"], false);
        assert_eq!(
            s["properties"]["images"]["items"]["additionalProperties"],
            false
        );
        let req = s["properties"]["images"]["items"]["required"]
            .as_array()
            .unwrap();
        assert_eq!(req.len(), 6, "六个字段一个都不能少，strict 模式会拒");
    }

    #[test]
    fn 一批最多六张() {
        assert_eq!(batches(&imgs(13)).len(), 3);
        assert_eq!(batches(&imgs(6)).len(), 1);
        assert_eq!(batches(&imgs(0)).len(), 0);
    }

    #[tokio::test]
    async fn 识别结果按顺序对回各自的图() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(EchoImages)
            .mount(&srv)
            .await;
        let got = recognize_batch(&client(&srv.uri()), "正文", &imgs(3))
            .await
            .unwrap();
        assert_eq!(got.len(), 3);
        for (i, d) in got.iter().enumerate() {
            assert_eq!(d.blake3, format!("h{i}"), "描述要对回它自己那张图");
            assert_eq!(d.ordinal, i as u16);
            assert!(d.content.contains(&format!("第{}张", i + 1)));
            assert_eq!(d.prompt_version, PROMPT_VERSION);
        }
        assert!(matches!(got[0].kind, ImageKind::Product));
    }

    #[tokio::test]
    async fn 条数对不上要报错不能将就() {
        // 模型把三张合并成两条描述：下标错位会让「第 3 张图显示…」的依据全部指错
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "output": [{"content": [{"type": "output_text", "text":
                    r#"{"images":[{"index":1,"matches_text":"a","content":"b","missing_from_text":"无","kind":"产品图","usable_as_figure":true}]}"#}]}],
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .mount(&srv)
            .await;
        let err = recognize_batch(&client(&srv.uri()), "正文", &imgs(3))
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("条数对不上"), "{err:#}");
    }

    #[tokio::test]
    async fn 超过一批上限直接拒绝而不是悄悄截断() {
        let srv = MockServer::start().await;
        let err = recognize_batch(&client(&srv.uri()), "正文", &imgs(7))
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("一批最多"), "{err:#}");
    }
}

/// 图片转 base64。**不引 base64 crate**：这一段十几行，而多一个依赖就多一份
/// 「开发机编得过、交叉编译过不去」的风险。
///
/// 05／11 也要用它（识别的入参是同一套），所以放在这里公开，不各写一份。
pub fn b64(bytes: &[u8]) -> String {
    use std::fmt::Write;
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let _ = write!(
            out,
            "{}{}",
            T[(n >> 18) as usize & 63] as char,
            T[(n >> 12) as usize & 63] as char
        );
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod b64_tests {
    use super::b64;

    #[test]
    fn base64编码对得上() {
        assert_eq!(b64(b"a"), "YQ==");
        assert_eq!(b64(b"ab"), "YWI=");
        assert_eq!(b64(b"abc"), "YWJj");
        assert_eq!(b64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }
}
