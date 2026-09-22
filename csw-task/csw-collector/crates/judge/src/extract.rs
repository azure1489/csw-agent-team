//! 品牌产品名、发售日期、价格：**代码找候选 + Jev 选一个**。
//!
//! 为什么不让生成模型直接写这几个字段：它会「顺手补全」。正文里没写价格时，
//! 一个生成模型很可能给出一个看着合理的数字，而那个数字在台账里与真价格长得一模一样，
//! **没有任何办法事后分辨**。
//!
//! 换成「代码从正文里切出所有像日期 / 像价格的片段，Jev 从这些片段里挑一个（或挑「没有」）」，
//! 就把编造这条路堵死了：**模型选不出没被切出来的东西**。
//!
//! 代价是切的那一步必须**宁滥勿缺**——切漏了，Jev 再聪明也选不到。
//! 所以下面的模式刻意写得宽，宁可多切几个让 Jev 去挑。

use std::collections::BTreeMap;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde_json::json;

use csw_collector_core::jev::{JevClient, Question};

/// 「正文里没有」这个选项的键。**必须始终提供**——
/// 不给这个选项，模型就只能在几个都不对的片段里硬挑一个。
pub const NONE: &str = "none";

/// 最多给 Jev 几个候选。多了它也挑不准，而且费 token。
const MAX_CANDIDATES: usize = 8;

static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // 2026-09-18 / 2026/9/18 / 2026.09.18 / 2026年9月18日
        r"(?x)
        \d{4}\s*[-/.年]\s*\d{1,2}\s*[-/.月]\s*\d{1,2}\s*日?
        |",
        // 9月18日 / 9/18（不带年）
        r"\d{1,2}\s*[/月]\s*\d{1,2}\s*日?
        |",
        // September 18 / Sep 18, 2026 / 18 September
        r"(?i)(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+\d{1,2}(\s*,\s*\d{4})?
        |
        \d{1,2}\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?(\s+\d{4})?"
    ))
    .expect("日期模式")
});

static PRICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // ¥12,800 / ￥12800 / $129.00 / €99 / £85 / ₩120,000
        r"(?x)
        [¥￥$€£₩]\s*\d[\d,.\s]*\d|[¥￥$€£₩]\s*\d
        |",
        // 12,800円 / 899 元 / 129 USD / 1200 yen
        r"(?i)\d[\d,.\s]*\d\s*(円|元|圆|usd|jpy|eur|gbp|rmb|cny|yen|dollars?)
        |
        \d\s*(円|元|usd|jpy|yen)"
    ))
    .expect("价格模式")
});

/// 切出所有像日期的片段。**宁滥勿缺。**
pub fn date_candidates(text: &str) -> Vec<String> {
    dedup(DATE.find_iter(text).map(|m| m.as_str().trim().to_string()))
}

/// 切出所有像价格的片段。
pub fn price_candidates(text: &str) -> Vec<String> {
    dedup(PRICE.find_iter(text).map(|m| m.as_str().trim().to_string()))
}

/// 切出可能是产品名的片段：品牌别名之后那一小段。
///
/// 产品名在正文里几乎总是紧跟品牌名（`and wander 40L Backpack`、
/// `山と道 THREE`），所以从每个品牌命中处往后取一小段当候选。
pub fn product_candidates(text: &str, brand_aliases: &[&str]) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    for alias in brand_aliases {
        let a = alias.to_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..].find(&a) {
            let start = from + pos + a.len();
            // 往后取到换行、句号或三十个字符为止
            let tail: String = text[start.min(text.len())..]
                .chars()
                .take(30)
                .take_while(|c| !matches!(c, '\n' | '。' | '.' | '!' | '！' | '#'))
                .collect();
            let t = tail
                .trim()
                .trim_start_matches(['的', ':', '：', '-', '—'])
                .trim();
            if t.chars().count() >= 2 {
                out.push(format!("{alias} {t}").trim().to_string());
            }
            from = start.max(from + 1);
            if from >= lower.len() {
                break;
            }
        }
    }
    dedup(out.into_iter())
}

fn dedup(it: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    it.filter(|s| !s.trim().is_empty())
        .filter(|s| seen.insert(s.to_lowercase()))
        .take(MAX_CANDIDATES)
        .collect()
}

/// 要 Jev 挑的一个字段。
pub struct Field<'a> {
    /// 结果里的键
    pub key: &'a str,
    /// 问它什么。要说清楚「挑哪一个」而不是「答什么」。
    pub instructions: &'a str,
    pub candidates: &'a [String],
}

/// 让 Jev 从切出来的片段里挑。返回 `键 → 选中的片段`；选了「没有」就不进结果。
///
/// **所有字段一次问完**——它们共用同一份正文、互不依赖。
pub async fn pick(
    jev: &JevClient,
    text: &str,
    fields: &[Field<'_>],
) -> Result<BTreeMap<String, String>> {
    let live: Vec<&Field<'_>> = fields.iter().filter(|f| !f.candidates.is_empty()).collect();
    if live.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut questions = BTreeMap::new();
    let mut state = json!({ "caption": text });
    for f in &live {
        let mut opts: Vec<(&str, String)> = f
            .candidates
            .iter()
            .map(|c| (c.as_str(), format!("正文里说的就是「{c}」")))
            .collect();
        // 不给「没有」这个选项，模型就只能在几个都不对的片段里硬挑一个
        opts.push((NONE, "正文里没有说，或切出来的这些都不是".to_string()));
        let refs: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        questions.insert(f.key.to_string(), Question::choice(f.instructions, &refs));
        state[format!("{}_candidates", f.key)] = json!(f.candidates);
    }

    let a = jev.ask(&state, &questions).await?;
    Ok(live
        .iter()
        .filter_map(|f| {
            let picked = a.choice(f.key)?;
            (picked != NONE).then(|| (f.key.to_string(), picked.to_string()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日期切得够宽() {
        // 切漏了，Jev 再聪明也选不到——所以宁滥勿缺
        for (text, want) in [
            ("発売日は2026年9月18日です", "2026年9月18日"),
            ("released 2026-09-18 worldwide", "2026-09-18"),
            ("9月18日より順次発売", "9月18日"),
            ("Drops September 18, 2026", "September 18, 2026"),
            ("out 18 Sep 2026", "18 Sep 2026"),
            ("上市时间 2026/9/18", "2026/9/18"),
        ] {
            let got = date_candidates(text);
            assert!(
                got.iter()
                    .any(|g| g.replace(' ', "") == want.replace(' ', "")),
                "{text} → {got:?}，想要 {want}"
            );
        }
    }

    #[test]
    fn 价格切得够宽() {
        for (text, want) in [
            ("価格は¥12,800（税込）", "¥12,800"),
            ("Price $129.00", "$129.00"),
            ("14,080円 税込", "14,080円"),
            ("售价 899 元", "899 元"),
            ("€99 EU only", "€99"),
        ] {
            let got = price_candidates(text);
            assert!(
                got.iter()
                    .any(|g| g.replace(' ', "") == want.replace(' ', "")),
                "{text} → {got:?}，想要 {want}"
            );
        }
    }

    #[test]
    fn 没有日期价格就切不出东西() {
        assert!(date_candidates("新色が登場しました").is_empty());
        assert!(price_candidates("新色が登場しました").is_empty());
    }

    #[test]
    fn 产品名从品牌名往后取() {
        let got = product_candidates(
            "and wander 40L Backpack が登場。\n山と道 THREE も同時発売。",
            &["and wander", "山と道"],
        );
        assert!(got.iter().any(|g| g.contains("40L Backpack")), "{got:?}");
        assert!(got.iter().any(|g| g.contains("THREE")), "{got:?}");
        // 每个候选都带上品牌名，免得 Jev 看到一串裸名字不知道属于谁
        assert!(
            got.iter()
                .all(|g| g.starts_with("and wander") || g.starts_with("山と道"))
        );
    }

    #[test]
    fn 品牌名在句尾时不会切出空串() {
        let got = product_candidates("今日のおすすめは and wander", &["and wander"]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn 候选去重且有上限() {
        let text = "¥100 ¥100 ¥200 ¥300 ¥400 ¥500 ¥600 ¥700 ¥800 ¥900 ¥1000";
        let got = price_candidates(text);
        // 去重：写了两遍的 ¥100 只留一个
        assert_eq!(got.iter().filter(|x| *x == "¥100").count(), 1, "{got:?}");
        // 上限：十个不同的价格只留前八个。给多了 Jev 也挑不准，而且费 token
        assert_eq!(got.len(), MAX_CANDIDATES, "{got:?}");
        assert!(!got.iter().any(|x| x == "¥1000"), "超出上限的该被截掉");
    }

    #[tokio::test]
    async fn 选项里必须有没有这一项() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "answers": {"release_date": {"choice": "none"}, "price": {"choice": "¥12,800"}}
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

        let dates = date_candidates("9月18日");
        let prices = price_candidates("¥12,800 と ¥9,800");
        let got = pick(
            &jev,
            "正文",
            &[
                Field {
                    key: "release_date",
                    instructions: "哪一个是这条贴文说的发售日期？",
                    candidates: &dates,
                },
                Field {
                    key: "price",
                    instructions: "哪一个是这条贴文说的价格？",
                    candidates: &prices,
                },
            ],
        )
        .await
        .unwrap();
        // 选了「没有」就不进结果——不许拿一个不对的片段顶上
        assert!(!got.contains_key("release_date"));
        assert_eq!(got.get("price").map(String::as_str), Some("¥12,800"));
    }

    #[tokio::test]
    async fn 一个候选都没切出来就别问了() {
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
        let empty: Vec<String> = vec![];
        let got = pick(
            &jev,
            "正文",
            &[Field {
                key: "price",
                instructions: "哪一个是价格？",
                candidates: &empty,
            }],
        )
        .await
        .unwrap();
        assert!(got.is_empty());
    }
}
