//! 01「情报逐条」的交付物：判断台账 + 溯源文件。
//!
//! 正文的组织方式是**按档分组、档内按台账顺序**。不列分数、不列排名，
//! 因为根本没有分数——四档本身就是顺序。
//!
//! 每条都写出来，**包括不推荐的**。主编要能看见「这 358 条都过了一遍」，
//! 而不是只看见挑出来的二十条；只给推荐的等于把「审阅覆盖 100%」这件事
//! 变成一句无法验证的话。

use std::fmt::Write as _;

use csw_collector_core::types::{Candidate, Judgement, Tier, Verdict};

/// 台账正文。`by_key` 用来补候选侧的信息（链接、时间），取不到就略过那几项。
pub fn ledger_body(
    window: (&str, &str),
    judgements: &[Judgement],
    by_key: impl Fn(&str) -> Option<Candidate>,
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# 情报逐条 · 判断台账\n");
    let _ = writeln!(
        s,
        "窗口（按首次入库时间，左闭右开）{} ~ {}\n",
        window.0, window.1
    );
    let _ = writeln!(s, "{}\n", counts_line(judgements));

    for tier in [
        Tier::Recommend,
        Tier::Alternate,
        Tier::PendingCheck,
        Tier::NotRecommend,
    ] {
        let group: Vec<&Judgement> = judgements.iter().filter(|j| j.tier == tier).collect();
        let _ = writeln!(s, "## {}（{} 条）\n", tier_name(tier), group.len());
        if group.is_empty() {
            let _ = writeln!(s, "无。\n");
            continue;
        }
        for j in group {
            write_one(&mut s, j, by_key(&j.candidate_key).as_ref());
        }
    }
    s
}

fn counts_line(js: &[Judgement]) -> String {
    let n = |t: Tier| js.iter().filter(|j| j.tier == t).count();
    let unseen = js.iter().filter(|j| !j.image_seen).count();
    format!(
        "共 {} 条：推荐 {}、备选 {}、待核 {}、不推荐 {}；其中未读到实图 {} 条（待核，不是淘汰）。",
        js.len(),
        n(Tier::Recommend),
        n(Tier::Alternate),
        n(Tier::PendingCheck),
        n(Tier::NotRecommend),
        unseen
    )
}

fn write_one(s: &mut String, j: &Judgement, c: Option<&Candidate>) {
    let title = c
        .map(|c| first_line(&c.text))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| j.candidate_key.clone());
    let brand = c.map(|c| c.account.clone()).unwrap_or_default();
    let _ = writeln!(s, "### {brand}｜{title}\n");
    let _ = writeln!(s, "- 条目键：`{}`", j.candidate_key);
    if let Some(c) = c {
        let _ = writeln!(s, "- 链接：{}", c.url);
        // 原始披露时间与转载时间分开写——把转载时间当成发布时间是一类真错
        let _ = writeln!(
            s,
            "- 原始披露时间：{}",
            c.posted_at
                .map(|t| t.to_string())
                .unwrap_or_else(|| "不详".into())
        );
        let _ = writeln!(
            s,
            "- 首次入库时间：{}",
            c.ingested_at
                .map(|t| t.to_string())
                .unwrap_or_else(|| "不详".into())
        );
    }
    if !j.heat_note.is_empty() {
        let _ = writeln!(s, "- 热度：{}（是输入的呈现，不是维度）", j.heat_note);
    }
    let _ = writeln!(
        s,
        "- 读到实图：{}",
        if j.image_seen { "是" } else { "**否**" }
    );

    let _ = writeln!(s, "\n**六维**\n");
    for (d, dj) in &j.dims {
        let _ = writeln!(
            s,
            "- {}：{} —— {}",
            dim_name(*d),
            verdict_name(dj.verdict),
            dj.basis
        );
    }

    let t = &j.three_sentences;
    if [&t.what_changed, &t.why_it_matters, &t.how_different]
        .iter()
        .any(|x| !x.trim().is_empty())
    {
        let _ = writeln!(s, "\n**三句话**\n");
        let _ = writeln!(s, "- 发生了什么变化：{}", t.what_changed);
        let _ = writeln!(s, "- 为什么值得户外用户知道：{}", t.why_it_matters);
        let _ = writeln!(s, "- 它和以前有什么不一样：{}", t.how_different);
    }
    if j.unanswered != csw_collector_core::types::Unanswered::None {
        let _ = writeln!(s, "- 答不清的原因：{}", unanswered_name(j.unanswered));
    }

    let _ = writeln!(
        s,
        "\n**对照结论**：{}{}",
        comparison_name(j.comparison.verdict),
        if j.comparison.against.is_empty() {
            String::new()
        } else {
            format!("（对照 {}）", j.comparison.against)
        }
    );
    if !j.comparison.note.trim().is_empty() {
        let _ = writeln!(s, "{}", j.comparison.note);
    }
    if !j.look.trim().is_empty() {
        let _ = writeln!(s, "\n**实图所见**：{}", j.look);
    }
    for (label, xs) in [
        ("优先关注", &j.priority_hits),
        ("降低优先级", &j.lower_hits),
        ("缺口", &j.gaps),
    ] {
        if !xs.is_empty() {
            let _ = writeln!(s, "\n**{label}**：{}", xs.join("、"));
        }
    }
    if !j.jev_disagreement.trim().is_empty() {
        let _ = writeln!(s, "\n**与初评的分歧**：{}", j.jev_disagreement);
    }
    s.push('\n');
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(50)
        .collect()
}

fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Recommend => "推荐",
        Tier::Alternate => "备选",
        Tier::PendingCheck => "待核",
        Tier::NotRecommend => "不推荐",
    }
}

fn dim_name(d: csw_collector_core::types::Dim) -> &'static str {
    use csw_collector_core::types::Dim::*;
    match d {
        Change => "具体变化",
        Use => "使用关联",
        Gain => "信息增量",
        Compare => "比较参照",
        Explain => "可解释性",
        Csw => "CSW 视角",
    }
}

fn verdict_name(v: Verdict) -> &'static str {
    match v {
        Verdict::Yes => "成立",
        Verdict::No => "不成立",
        Verdict::Unclear => "不明",
    }
}

fn unanswered_name(u: csw_collector_core::types::Unanswered) -> &'static str {
    use csw_collector_core::types::Unanswered::*;
    match u {
        None => "",
        MissingMaterial => "缺资料",
        AngleNotFormed => "角度未成立",
        LowValue => "价值不足",
    }
}

fn comparison_name(v: csw_collector_core::types::ComparisonVerdict) -> &'static str {
    use csw_collector_core::types::ComparisonVerdict::*;
    match v {
        SameFactNoGain => "同一事实、无增量",
        SameBrandWithGain => "同品牌、有增量",
        Unrelated => "不相干",
    }
}

/// 一条候选的预览图：交付物里每条放一张。
pub struct Preview {
    pub candidate_key: String,
    /// 图片字节。**已经是缩略图**，别把原图塞进来——48 MiB 的上限不经花。
    pub bytes: Vec<u8>,
    /// 扩展名（jpg / png / webp），决定 zip 里用不用压缩
    pub ext: String,
}

/// 把 01 的交付物拼成 zip 的内容清单。
///
/// 形态是协议定死的：`index.md` + `images/` + `trace/`。
/// 图片按 `候选键.扩展名` 命名，**不用序号**——序号在补件里会错位，
/// 而条目键在整轮里是稳定的。
pub fn assemble(
    meta: crate::index::Meta,
    body: String,
    previews: &[Preview],
    sweeps_jsonl: String,
    items_jsonl: String,
) -> Vec<crate::pack::Entry> {
    let mut out = vec![
        crate::pack::Entry::text("index.md", crate::index::Document { meta, body }.render()),
        crate::pack::Entry::text("trace/sweeps.jsonl", sweeps_jsonl),
        crate::pack::Entry::text("trace/items.jsonl", items_jsonl),
    ];
    for p in previews {
        let ext = if p.ext.trim().is_empty() {
            "jpg"
        } else {
            p.ext.trim()
        };
        out.push(crate::pack::Entry::binary(
            &format!("images/{}.{ext}", safe_name(&p.candidate_key)),
            p.bytes.clone(),
        ));
    }
    out
}

/// 条目键进文件名前过一遍。键本身是「品牌小写 + `-` + 哈希前六位」，
/// 正常不会有问题；但品牌名是从第三方正文来的，**不能假设它干净**。
fn safe_name(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::{
        Comparison, ComparisonVerdict, Dim, DimJudgement, Platform, ThreeSentences, Unanswered,
    };

    fn j(key: &str, tier: Tier, image_seen: bool) -> Judgement {
        Judgement {
            candidate_key: key.into(),
            tier,
            dims: Dim::ALL
                .into_iter()
                .map(|d| {
                    (
                        d,
                        DimJudgement {
                            verdict: Verdict::Yes,
                            basis: format!("{d:?} 的依据"),
                        },
                    )
                })
                .collect(),
            three_sentences: ThreeSentences {
                what_changed: "换了结构".into(),
                why_it_matters: "背得更稳".into(),
                how_different: "上一代是软背板".into(),
            },
            unanswered: Unanswered::None,
            comparison: Comparison {
                verdict: ComparisonVerdict::SameBrandWithGain,
                against: "r47 的那条".into(),
                note: "多了发售日期".into(),
            },
            heat_note: "369 赞 · 常态 71 的 5.2×".into(),
            look: "灰色主体、铝合金背板".into(),
            image_seen,
            gaps: if image_seen {
                vec![]
            } else {
                vec!["未读到实图".into()]
            },
            priority_hits: vec!["老产品结构性改款".into()],
            lower_hits: vec![],
            jev_disagreement: String::new(),
            kb_refs: vec![],
            memory_refs: vec![],
            inputs_hash: "h".into(),
        }
    }

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw-window".into(),
            account: "yamatomichi".into(),
            url: format!("https://instagram.com/p/{key}"),
            text: "新しいバックパックを発表しました\n第二行".into(),
            translated: String::new(),
            posted_at: "2026-09-17T08:00:00Z".parse().ok(),
            ingested_at: "2026-09-18T01:00:00Z".parse().ok(),
            likes: Some(369),
            comments: Some(12),
            followers: Some(90000),
            heat_ratio: Some(5.2),
            content_type: "Carousel".into(),
            media: vec![],
            tags: vec![],
            hashtags: vec![],
        }
    }

    /// 合成的周一：四百五十条的台账打成包有多大。
    ///
    /// 交付物上限 48 MiB，而台账是**每条都写**（包括不推荐的）。
    /// 这一条盯的是「正文会不会自己把包撑爆」——图是另算的，
    /// 预览图每条一张，真要撑爆是图先撑。
    #[test]
    fn 合成的周一四百五十条的台账不会把包撑爆() {
        const N: usize = 450;
        let tiers = [
            Tier::Recommend,
            Tier::Alternate,
            Tier::PendingCheck,
            Tier::NotRecommend,
        ];
        let js: Vec<Judgement> = (0..N)
            .map(|i| j(&format!("k{i:03}"), tiers[i % 4], i % 7 != 0))
            .collect();
        let cs: Vec<Candidate> = (0..N).map(|i| cand(&format!("k{i:03}"))).collect();
        let by: std::collections::HashMap<String, Candidate> = cs
            .iter()
            .map(|c| (c.candidate_key.clone(), c.clone()))
            .collect();

        let body = ledger_body(("2026-09-19T16:00:00Z", "2026-09-22T16:00:00Z"), &js, |k| {
            by.get(k).cloned()
        });
        // 每条都写：四百五十条一条不少
        for i in [0usize, 137, N - 1] {
            assert!(body.contains(&format!("k{i:03}")), "第 {i} 条没写进去");
        }

        let entries = assemble(
            crate::index::Meta {
                at: "2026-09-22T16:00:00Z".into(),
                ..Default::default()
            },
            body,
            &[],
            String::new(),
            String::new(),
        );
        let dir = tempdir::TempDir::new("mon").unwrap();
        let out = dir.path().join("mon.zip");
        let built = crate::pack::build("mon", &entries, &out).unwrap();

        // 上限 48 MiB。纯台账应当只占其中很小一块——剩下的留给图
        assert!(
            built.bytes < crate::pack::MAX_ZIP_BYTES / 10,
            "光台账就占了 {} 字节，图还没进来",
            built.bytes
        );
        // 两次构建仍要一样：确定性不该随条数变
        let out2 = dir.path().join("mon2.zip");
        assert_eq!(
            built.sha256,
            crate::pack::build("mon", &entries, &out2).unwrap().sha256
        );
    }

    #[test]
    fn 四档都出现哪怕是空的() {
        let body = ledger_body(("A", "B"), &[j("k1", Tier::Recommend, true)], |k| {
            Some(cand(k))
        });
        // 只给推荐的等于把「审阅覆盖 100%」变成一句无法验证的话
        for name in ["推荐", "备选", "待核", "不推荐"] {
            assert!(body.contains(&format!("## {name}")), "少了 {name}");
        }
        assert!(body.contains("## 不推荐（0 条）"));
        assert!(body.contains("无。"));
    }

    #[test]
    fn 总览把待核与未读实图分开说() {
        let js = [
            j("k1", Tier::Recommend, true),
            j("k2", Tier::PendingCheck, false),
        ];
        let body = ledger_body(("A", "B"), &js, |k| Some(cand(k)));
        assert!(
            body.contains("共 2 条：推荐 1、备选 0、待核 1、不推荐 0"),
            "{body}"
        );
        assert!(body.contains("未读到实图 1 条（待核，不是淘汰）"), "{body}");
    }

    #[test]
    fn 原始披露时间与转载时间分开写() {
        // 把转载时间当成发布时间是一类真错
        let body = ledger_body(("A", "B"), &[j("k1", Tier::Recommend, true)], |k| {
            Some(cand(k))
        });
        assert!(
            body.contains("原始披露时间：2026-09-17T08:00:00Z"),
            "{body}"
        );
        assert!(
            body.contains("首次入库时间：2026-09-18T01:00:00Z"),
            "{body}"
        );
    }

    #[test]
    fn 六维逐条带依据() {
        let body = ledger_body(("A", "B"), &[j("k1", Tier::Recommend, true)], |k| {
            Some(cand(k))
        });
        for name in [
            "具体变化",
            "使用关联",
            "信息增量",
            "比较参照",
            "可解释性",
            "CSW 视角",
        ] {
            assert!(body.contains(name), "少了 {name}");
        }
        assert!(body.contains("的依据"), "每一维都要带依据");
        // 不列分数、不列排名——根本没有分数
        assert!(!body.contains("分数") && !body.contains("评分"));
    }

    #[test]
    fn 没读到实图要显眼() {
        let body = ledger_body(("A", "B"), &[j("k1", Tier::PendingCheck, false)], |k| {
            Some(cand(k))
        });
        assert!(body.contains("读到实图：**否**"), "{body}");
        assert!(body.contains("缺口**：未读到实图"), "{body}");
    }

    #[test]
    fn 取不到候选也能出台账() {
        // 候选侧信息拿不到时略过那几项，而不是整条不写
        let body = ledger_body(("A", "B"), &[j("k1", Tier::Recommend, true)], |_| None);
        assert!(body.contains("k1"));
        assert!(!body.contains("链接："));
    }

    #[test]
    fn 交付物的形态是协议定死的() {
        let entries = assemble(
            crate::index::Meta {
                at: "2026-09-18T05:40:00Z".into(),
                ..Default::default()
            },
            "正文".into(),
            &[Preview {
                candidate_key: "yamatomichi-ab12cd".into(),
                bytes: vec![1, 2, 3],
                ext: "jpg".into(),
            }],
            "{}\n".into(),
            "{}\n".into(),
        );
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"index.md"));
        assert!(paths.contains(&"trace/sweeps.jsonl"));
        assert!(paths.contains(&"trace/items.jsonl"));
        // 图片按条目键命名，不用序号——序号在补件里会错位
        assert!(
            paths.contains(&"images/yamatomichi-ab12cd.jpg"),
            "{paths:?}"
        );
    }

    #[test]
    fn 键里的脏字符进不了文件名() {
        // 品牌名是从第三方正文来的，不能假设它干净
        assert_eq!(safe_name("a/../b"), "a____b");
        assert_eq!(
            safe_name("山と道-ab12cd"),
            "___-ab12cd",
            "三个字符三个下划线"
        );
        assert_eq!(safe_name("nanga-ab12cd"), "nanga-ab12cd");
    }

    #[test]
    fn 扩展名空着就当jpg() {
        let entries = assemble(
            crate::index::Meta::default(),
            String::new(),
            &[Preview {
                candidate_key: "k".into(),
                bytes: vec![1],
                ext: "  ".into(),
            }],
            String::new(),
            String::new(),
        );
        assert!(entries.iter().any(|e| e.path == "images/k.jpg"));
    }

    #[test]
    fn 渲染两次一模一样() {
        let js = [j("k1", Tier::Recommend, true)];
        let a = ledger_body(("A", "B"), &js, |k| Some(cand(k)));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(a, ledger_body(("A", "B"), &js, |k| Some(cand(k))));
    }
}
