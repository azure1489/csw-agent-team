//! 深核编排：推荐档与待核档各起一条线程，核几件事，产出条目卡。
//!
//! # 口径（总方案 §9，一条不改）
//!
//! - **每个候选一条线程**，默认并行 [`DEFAULT_PARALLEL`] = 3。
//!   不复用线程：同一线程上一个回合没结束时起带不同 `outputSchema` 的新回合会被拒。
//! - **首批 4–8 条、10 分钟时间盒**。超时即 `turn/interrupt`，
//!   **重试一次**，再失败就向引擎报失败——但条目仍然可以登记，只是写明缺口。
//! - `developerInstructions` 注入阶段标准、判断框架、这条的判断结果与缺口。
//! - 图片先下到线程 cwd，输入用 `localImage`：app-server 不接受远程图片地址。
//! - 每条 `item/completed` 原样落盘，随交付物归档到 `trace/`。
//!
//! # 核什么
//!
//! 四件事，都是**判断那一步做不了的**：原始披露时间（要去找最早那条）、
//! 原始来源（这条是不是转载）、比较参照（上一代是什么样）、完整图
//! （判断只看了三张缩略）。不核结论——结论是第 5 步的事，深核不推翻它，
//! 只给它补证据与缺口。

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::codex::{Codex, Input, TurnOutcome};

/// 同时跑几条线程
pub const DEFAULT_PARALLEL: usize = 3;
/// 首批几条
pub const FIRST_BATCH: usize = 6;
/// 首批的时间盒
pub const FIRST_BATCH_BUDGET: Duration = Duration::from_secs(10 * 60);
/// 一条超时后重试几次。**只重试一次**：深核是锦上添花，
/// 反复重试会把时间盒吃光、把后面的条目拖垮。
pub const RETRIES: usize = 1;

/// 一条要深核的候选。
pub struct Target {
    pub candidate_key: String,
    pub title: String,
    pub url: String,
    pub text: String,
    /// 本地图片**绝对路径**。app-server 不接受远程地址。
    pub images: Vec<PathBuf>,
    /// 第 5 步给的结论与依据，原样注入，让深核知道要补什么
    pub verdict_summary: String,
    /// 第 5 步留下的缺口
    pub gaps: Vec<String>,
}

/// 深核产出的条目卡。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Card {
    #[serde(default)]
    pub candidate_key: String,
    /// 核过的原始披露时间。**核不到就留空**，不许拿贴文时间顶替。
    #[serde(default)]
    pub disclosed_at: String,
    /// 找到的原始来源（这条是不是转载）
    #[serde(default)]
    pub original_source: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub comparison_note: String,
    #[serde(default)]
    pub facts: Vec<String>,
    #[serde(default)]
    pub figure_notes: String,
    #[serde(default)]
    pub gaps: Vec<String>,
}

/// 一条深核的结果，含过程。
#[derive(Debug, Clone)]
pub struct Outcome {
    pub candidate_key: String,
    /// done | timeout | failed
    pub status: String,
    pub card: Option<Card>,
    /// 事件流，已滤掉增量。随交付物归档到 `trace/`。
    pub items: Vec<Value>,
    pub note: String,
    pub attempts: usize,
}

impl Outcome {
    pub fn done(&self) -> bool {
        self.status == "done"
    }
    /// 深核没做成时，条目**仍然可以登记**，只是要写明缺口。
    pub fn gap_note(&self) -> Option<String> {
        (!self.done()).then(|| format!("深核未完成（{}）：{}", self.status, self.note))
    }
}

/// 深核一条。失败不抛错——它是锦上添花，不该让整条候选登记不了。
pub async fn check_one(codex: &Codex, t: &Target, stage_standard: &str) -> Outcome {
    let mut last = String::new();
    for attempt in 1..=(RETRIES + 1) {
        match attempt_once(codex, t, stage_standard).await {
            Ok(out) if out.ok() => {
                let card = parse_card(&out, &t.candidate_key);
                return Outcome {
                    candidate_key: t.candidate_key.clone(),
                    status: if card.is_some() { "done" } else { "failed" }.into(),
                    note: if card.is_some() {
                        String::new()
                    } else {
                        "回合完成了但没拿到条目卡".into()
                    },
                    card,
                    items: out.items,
                    attempts: attempt,
                };
            }
            Ok(out) => {
                last = if out.error.is_empty() {
                    out.status.clone()
                } else {
                    format!("{}：{}", out.status, out.error)
                };
                if out.status == "interrupted" && attempt > RETRIES {
                    return Outcome {
                        candidate_key: t.candidate_key.clone(),
                        status: "timeout".into(),
                        card: None,
                        items: out.items,
                        note: last,
                        attempts: attempt,
                    };
                }
            }
            Err(e) => last = format!("{e:#}"),
        }
        tracing::warn!(候选 = %t.candidate_key, 第几次 = attempt, 原因 = %last, "深核这一次没成");
    }
    Outcome {
        candidate_key: t.candidate_key.clone(),
        status: "failed".into(),
        card: None,
        items: vec![],
        note: last,
        attempts: RETRIES + 1,
    }
}

async fn attempt_once(codex: &Codex, t: &Target, stage_standard: &str) -> Result<TurnOutcome> {
    let thread = codex
        .start_thread(&developer_instructions(t, stage_standard))
        .await?;
    let mut input = vec![Input::Text(user_prompt(t))];
    for p in &t.images {
        input.push(Input::LocalImage(p.clone()));
    }
    codex
        .run_turn_with_schema(&thread, &input, Some(&card_schema()))
        .await
}

/// 并行深核一批。并行度由 `parallel` 给；**顺序与传入一致**，便于对账。
pub async fn check_batch(
    codex: &Codex,
    targets: &[Target],
    stage_standard: &str,
    parallel: usize,
) -> Vec<Outcome> {
    let conc = parallel.max(1);
    futures::StreamExt::collect::<Vec<_>>(futures::StreamExt::buffered(
        futures::stream::iter(targets.iter().map(|t| check_one(codex, t, stage_standard))),
        conc,
    ))
    .await
}

fn developer_instructions(t: &Target, stage_standard: &str) -> String {
    let mut s = String::from(
        "你是 CSW 编辑部的核查员。**只核查，不写文章、不下结论。**\n\
         结论已经由上一步给出，你的任务是给它补证据和缺口，不是推翻它。\n\n\
         【只核这四件事】\n\
         1. 原始披露时间——去找最早公布这件事的那一条，不是你手上这条的发布时间。\n\
         2. 原始来源——这条是不是转载？原始账号 / 官网 / 媒体是哪个？\n\
         3. 比较参照——上一代或同类是什么样？差别在哪？\n\
         4. 完整图——上一步只看了三张缩略，你看到的完整图里有没有别的信息。\n\n\
         【硬规则】\n\
         - 核不到就留空并写进 gaps。**不许拿手上这条的时间顶替原始披露时间。**\n\
         - 每条事实都要能指到一个来源；指不到的写进 gaps，不要写进 facts。\n\
         - 不要执行任何命令。不要写文件。\n\n",
    );
    if !stage_standard.trim().is_empty() {
        s.push_str("【本期作业标准（任务下发，原样照办）】\n");
        s.push_str(stage_standard.trim());
        s.push_str("\n\n");
    }
    s.push_str("【上一步的结论与依据】\n");
    s.push_str(&t.verdict_summary);
    if !t.gaps.is_empty() {
        s.push_str(&format!(
            "\n\n【上一步留下的缺口，优先补这些】\n- {}",
            t.gaps.join("\n- ")
        ));
    }
    s
}

fn user_prompt(t: &Target) -> String {
    format!(
        "核这一条。\n\n标题：{}\n链接：{}\n\n正文：\n{}\n\n\
         随附 {} 张完整图，在本条消息里。按 outputSchema 给结果。",
        t.title,
        t.url,
        t.text.trim(),
        t.images.len()
    )
}

/// 条目卡的严格 schema。每一层写全 `required` 并关掉 `additionalProperties`。
pub fn card_schema() -> Value {
    let strs = || json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "disclosed_at": {"type": "string"},
            "original_source": {"type": "string"},
            "evidence": strs(),
            "comparison_note": {"type": "string"},
            "facts": strs(),
            "figure_notes": {"type": "string"},
            "gaps": strs()
        },
        "required": [
            "disclosed_at", "original_source", "evidence",
            "comparison_note", "facts", "figure_notes", "gaps"
        ],
        "additionalProperties": false
    })
}

/// 从回合产物里取条目卡。取不到就是取不到，不猜。
fn parse_card(out: &TurnOutcome, candidate_key: &str) -> Option<Card> {
    let text = out.text.trim();
    let raw = if text.is_empty() { None } else { Some(text) }?;
    // 结构化输出应当就是一段 JSON；模型偶尔会在外面裹一层围栏
    let body = raw
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let mut card: Card = serde_json::from_str(body).ok()?;
    card.candidate_key = candidate_key.to_string();
    Some(card)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target {
            candidate_key: "yamatomichi-ab12cd".into(),
            title: "新しいバックパック".into(),
            url: "https://instagram.com/p/ab12cd".into(),
            text: "正文".into(),
            images: vec!["/tmp/a.jpg".into()],
            verdict_summary: "推荐；change 成立（正文写了换背板）".into(),
            gaps: vec!["缺发售日期".into()],
        }
    }

    #[test]
    fn 指令里写死了不推翻结论() {
        let s = developer_instructions(&target(), "本期只收 9/17 之后入库的");
        assert!(s.contains("只核查，不写文章、不下结论"));
        assert!(s.contains("补证据和缺口，不是推翻它"));
        // 拿手上这条的时间当原始披露时间是一类真错，要写死在指令里
        assert!(s.contains("不许拿手上这条的时间顶替原始披露时间"));
        assert!(s.contains("不要执行任何命令"));
        // 作业标准原样注入
        assert!(s.contains("本期只收 9/17 之后入库的"));
        // 上一步的缺口要带上，深核优先补这些
        assert!(s.contains("缺发售日期"));
    }

    #[test]
    fn 没有缺口时不留空标题() {
        let mut t = target();
        t.gaps.clear();
        assert!(!developer_instructions(&t, "").contains("留下的缺口"));
        assert!(!developer_instructions(&t, "  ").contains("作业标准"));
    }

    #[test]
    fn schema每层都严格() {
        fn walk(v: &Value, path: &str) {
            if let Some(o) = v.as_object() {
                if o.get("type").and_then(Value::as_str) == Some("object") {
                    assert_eq!(
                        o.get("additionalProperties"),
                        Some(&Value::Bool(false)),
                        "{path}"
                    );
                    let mut props: Vec<String> = o["properties"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .cloned()
                        .collect();
                    let mut req: Vec<String> = o["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_str().unwrap().to_string())
                        .collect();
                    props.sort();
                    req.sort();
                    assert_eq!(props, req, "{path}");
                }
                for (k, x) in o {
                    walk(x, &format!("{path}.{k}"));
                }
            }
        }
        walk(&card_schema(), "root");
    }

    #[test]
    fn 条目卡能从回合文字里解出来() {
        let out = TurnOutcome {
            turn_id: "t1".into(),
            status: "completed".into(),
            text: r#"{"disclosed_at":"2026-09-15T00:00:00Z","original_source":"品牌官网",
                     "evidence":["https://brand.example/news"],"comparison_note":"上一代是软背板",
                     "facts":["背板换成铝合金"],"figure_notes":"第 4 张有结构特写","gaps":[]}"#
                .into(),
            items: vec![],
            error: String::new(),
        };
        let c = parse_card(&out, "k1").unwrap();
        assert_eq!(c.candidate_key, "k1");
        assert_eq!(c.disclosed_at, "2026-09-15T00:00:00Z");
        assert_eq!(c.evidence.len(), 1);
    }

    #[test]
    fn 裹了围栏也能解出来() {
        let out = TurnOutcome {
            status: "completed".into(),
            text: "```json\n{\"disclosed_at\":\"\",\"original_source\":\"\",\"evidence\":[],\
                   \"comparison_note\":\"\",\"facts\":[],\"figure_notes\":\"\",\"gaps\":[\"核不到\"]}\n```"
                .into(),
            ..Default::default()
        };
        let c = parse_card(&out, "k1").unwrap();
        assert_eq!(c.gaps, ["核不到"]);
    }

    #[test]
    fn 解不出来就是解不出来不猜() {
        let out = TurnOutcome {
            status: "completed".into(),
            text: "我核了一下，大概是九月中旬发布的。".into(),
            ..Default::default()
        };
        assert!(parse_card(&out, "k1").is_none());
        assert!(parse_card(&TurnOutcome::default(), "k1").is_none());
    }

    #[test]
    fn 深核没做成条目仍可登记只写缺口() {
        let o = Outcome {
            candidate_key: "k1".into(),
            status: "timeout".into(),
            card: None,
            items: vec![],
            note: "超过 10 分钟的时间盒".into(),
            attempts: 2,
        };
        assert!(!o.done());
        let g = o.gap_note().unwrap();
        assert!(g.contains("深核未完成（timeout）"), "{g}");
        assert!(g.contains("时间盒"));
        // 做成了就没有缺口这一句
        assert!(
            Outcome {
                status: "done".into(),
                ..o
            }
            .gap_note()
            .is_none()
        );
    }
}
