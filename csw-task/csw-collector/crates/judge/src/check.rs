//! 逐条判断的第三小步：Jev 核对每条依据。
//!
//! 问的是「`material` 里到底有没有支持 `statement` 这句话」。0.7 回测的数字：
//!
//! | | supported 中位（四分位） |
//! |---|---|
//! | 真依据配本条正文 | 0.59（0.15/0.79） |
//! | **张冠李戴** | **0.02（0.02/0.03）** |
//!
//! | 阈值 | 真依据放行 | 张冠李戴抓出 |
//! |---|---|---|
//! | **0.3** | **68%** | **100%** |
//! | 0.5 | 60% | 100% |
//!
//! **编造的依据一条都没漏，而且贴在 0.02。** 所以阈值取 [`SUPPORTED_THRESHOLD`] = 0.3，
//! 低于它的**标出来给人看，不自动淘汰**——真依据只放行 68% 是因为回测时只给了正文，
//! 很多依据引的是「第 3 张图」，正文里当然核不到。正式运行时材料里有识别输出，
//! 这一项会上去；但在拿到新数字之前，不能拿它当闸。
//!
//! 送给 Jev 的 `material` **只有正文、译文与图片描述**——对照材料、Van 原话一概不送。

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;

use csw_collector_core::jev::{JevClient, Question};
use csw_collector_core::types::{Candidate, Dim, Judgement, MediaDescription};

/// 低于它就标出来给人看。**不是淘汰闸。**
pub const SUPPORTED_THRESHOLD: f64 = 0.3;

const FRAME: &str = "`statement` is a justification an editor wrote for the verdict on dimension \
`dimension`. `material` is the ONLY source material the editor was given. Does `material` actually \
support `statement`? Answer false if the statement quotes or asserts anything that is not in \
`material` — including a quoted sentence that does not appear there.";

/// 一条被标出来的依据。
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    pub candidate_key: String,
    pub dim: Dim,
    pub basis: String,
    /// 支持度。越低越像编的。
    pub supported: f64,
}

impl Flag {
    /// 写进台账的一句话。**措辞要像「请人看一眼」，不是「已判定为编造」**——
    /// 三成真依据会落在这条线下面。
    pub fn note(&self) -> String {
        format!(
            "{:?} 的依据在材料里核不到（支持度 {:.2}），请人过一眼：{}",
            self.dim,
            self.supported,
            self.basis.chars().take(80).collect::<String>()
        )
    }
}

/// 给 Jev 看的材料。**只有正文、译文与图片描述。**
pub fn material_of(c: &Candidate, descriptions: &[MediaDescription]) -> String {
    let mut s = String::new();
    if !c.text.trim().is_empty() {
        s.push_str(c.text.trim());
        s.push('\n');
    }
    if !c.translated.trim().is_empty() {
        s.push_str(c.translated.trim());
        s.push('\n');
    }
    for d in descriptions {
        s.push_str(&format!("第 {} 张图：{}\n", d.ordinal + 1, d.content));
        if !d.matches_text.trim().is_empty() {
            s.push_str(&format!("  与正文对应：{}\n", d.matches_text));
        }
    }
    s
}

/// 核对一条判断里的六条依据。**一次请求问完六个**——它们互不依赖。
///
/// 依据为空的维度跳过（`violations()` 已经拦过空依据，这里只是不白花一次调用）。
pub async fn check_one(
    jev: &JevClient,
    j: &Judgement,
    c: &Candidate,
    descriptions: &[MediaDescription],
) -> Result<Vec<Flag>> {
    let material = material_of(c, descriptions);
    let asked: Vec<(Dim, &str)> = j
        .dims
        .iter()
        .filter(|(_, d)| !d.basis.trim().is_empty())
        .map(|(k, d)| (*k, d.basis.as_str()))
        .collect();
    if asked.is_empty() || material.trim().is_empty() {
        return Ok(vec![]);
    }

    let mut questions = BTreeMap::new();
    for (dim, _) in &asked {
        questions.insert(
            key(*dim),
            Question::noul(
                FRAME,
                "Everything the statement asserts can be found in the material.",
                "The statement asserts something the material does not contain.",
            ),
        );
    }
    // 六个问题共用一份 state，但每个问的是不同的 statement——
    // 所以 statement 按维度分开放，问题里用 `dimension` 指路。
    let statements: serde_json::Map<String, serde_json::Value> =
        asked.iter().map(|(d, b)| (key(*d), json!(b))).collect();
    let state = json!({ "material": material, "statements": statements });

    let a = jev.ask(&state, &questions).await?;
    Ok(asked
        .into_iter()
        .filter_map(|(dim, basis)| {
            let p = a.noul(&key(dim))?;
            (p < SUPPORTED_THRESHOLD).then(|| Flag {
                candidate_key: j.candidate_key.clone(),
                dim,
                basis: basis.to_string(),
                supported: p,
            })
        })
        .collect())
}

fn key(d: Dim) -> String {
    serde_json::to_value(d)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{ImageKind, MediaKind, MediaRef, Platform, Tier};

    fn cand() -> Candidate {
        Candidate {
            candidate_key: "k".into(),
            platform: Platform::Instagram,
            source_id: "s".into(),
            collector: "c".into(),
            account: "a".into(),
            url: "u".into(),
            text: "新しいバックパックを発表".into(),
            translated: "发布了新背包".into(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Image".into(),
            media: vec![MediaRef {
                source_hash: "h".into(),
                kind: MediaKind::Photo,
                url: "u".into(),
                blake3: Some("b0".into()),
                ordinal: 0,
            }],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn judgement(bases: [&str; 6]) -> Judgement {
        let mut j = Judgement::fixture("k", Tier::Recommend);
        for ((_, d), b) in j.dims.iter_mut().zip(bases) {
            d.basis = b.into();
        }
        j
    }

    #[test]
    fn 材料只给正文译文与图片描述() {
        let d = MediaDescription {
            blake3: "b0".into(),
            ordinal: 0,
            matches_text: "正文说的那只包".into(),
            content: "一只灰色背包".into(),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: "v".into(),
        };
        let m = material_of(&cand(), &[d]);
        assert!(m.contains("新しいバックパック"));
        assert!(m.contains("发布了新背包"));
        assert!(m.contains("第 1 张图：一只灰色背包"), "{m}");
        // Jev 是第三方：对照材料与 Van 原话一概不送
        assert!(!m.contains("决定") && !m.contains("台账"));
    }

    #[tokio::test]
    async fn 低于阈值的标出来高于的放过() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "answers": {
                    "change": {"noul": 0.59},   // 真依据
                    "use": {"noul": 0.02},      // 张冠李戴
                    "gain": {"noul": 0.80},
                    "compare": {"noul": 0.29},  // 刚好在线下
                    "explain": {"noul": 0.30},  // 刚好在线上
                    "csw": {"noul": 0.95},
                }
            })))
            .mount(&server)
            .await;
        let jev = JevClient::new(
            csw_collector_core::jev::JevConfig {
                base_url: server.uri(),
                ..Default::default()
            },
            "k",
        )
        .unwrap();

        let j = judgement(["甲", "乙", "丙", "丁", "戊", "己"]);
        let flags = check_one(&jev, &j, &cand(), &[]).await.unwrap();
        let dims: Vec<Dim> = flags.iter().map(|f| f.dim).collect();
        assert_eq!(dims, [Dim::Use, Dim::Compare], "0.30 不算低于阈值");
        // 措辞是「请人看一眼」，不是「已判定为编造」——三成真依据会落在线下
        let note = flags[0].note();
        assert!(note.contains("请人过一眼"), "{note}");
        assert!(!note.contains("编造"), "{note}");
    }

    #[tokio::test]
    async fn 没有依据或没有材料就不白花一次调用() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let jev = JevClient::new(
            csw_collector_core::jev::JevConfig {
                base_url: server.uri(),
                ..Default::default()
            },
            "k",
        )
        .unwrap();

        let empty = judgement(["", "", "", "", "", ""]);
        assert!(
            check_one(&jev, &empty, &cand(), &[])
                .await
                .unwrap()
                .is_empty()
        );

        let mut blank = cand();
        blank.text = String::new();
        blank.translated = String::new();
        let j = judgement(["甲", "乙", "丙", "丁", "戊", "己"]);
        assert!(check_one(&jev, &j, &blank, &[]).await.unwrap().is_empty());
    }
}
