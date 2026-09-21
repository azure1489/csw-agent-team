//! 契约导出：把 `core::types` 的 JSON Schema 打出来，给前端生成 TS 类型。
//!
//! 阶段 1 的「契约冻结」就冻在这里。改了 `core::types` 就要重跑
//! `csw-collector schema -o ../csw-collector-web/src/contract.json`，
//! 前端的类型跟着变——**不要两边各写一份**。

use anyhow::Result;
use csw_collector_core::types::*;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(components(schemas(
    RoundKind,
    RoundTrigger,
    StepCode,
    StepStatus,
    Platform,
    Candidate,
    MediaKind,
    MediaRef,
    MediaDescription,
    ImageKind,
    MaterialKind,
    Material,
    Tier,
    Verdict,
    Dim,
    DimJudgement,
    ThreeSentences,
    Unanswered,
    ComparisonVerdict,
    Comparison,
    Judgement,
    Triage,
    EventGroup,
    OutboxKind,
    OutboxStatus,
)))]
struct Contract;

pub fn run(out: Option<&str>) -> Result<()> {
    let doc = Contract::openapi();
    let text = serde_json::to_string_pretty(&doc)?;
    match out {
        Some(p) => {
            if let Some(d) = std::path::Path::new(p).parent() {
                std::fs::create_dir_all(d).ok();
            }
            std::fs::write(p, &text)?;
            let n = doc
                .components
                .as_ref()
                .map(|c| c.schemas.len())
                .unwrap_or(0);
            println!("写入 {p}，{n} 个类型，{} 字节", text.len());
        }
        None => println!("{text}"),
    }
    Ok(())
}
