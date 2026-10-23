//! 来源库没给译文时补译。
//!
//! 译文本该随贴文从来源库（`translatedText`）来，但来源库会漏：10-23 r86 两条日文主选
//! 只有原文，01 包把译文位置写成「原文是中文」，主编连退两次，最后靠维护方人工补译。
//! 现在导出前把本包要带的、非中文且没有译文的正文交生成模型完整译一遍，写回库里；
//! 补译失败不拦路，导出时如实写「来源库没给、补译也失败」。

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use csw_collector_core::model::{ModelClient, Part};

/// 补译写回库里时的开头标注：读的人要知道这段不是来源库给的。
pub const MARK: &str = "【工作台补译：来源库未提供译文，由生成模型按原文完整翻译】";

/// 正文是不是需要中文译文：有假名（日文），或拉丁字母多于汉字（英文等）。
/// 只有汉字没有假名的当中文，不译。
pub fn needs_translation(text: &str) -> bool {
    let mut han = 0usize;
    let mut latin = 0usize;
    for ch in text.chars() {
        match ch {
            '\u{3040}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' | '\u{ff66}'..='\u{ff9d}' => {
                return true;
            }
            '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' => han += 1,
            c if c.is_ascii_alphabetic() => latin += 1,
            _ => {}
        }
    }
    // 只有话题标签、账号名这种零星字母不算要译
    latin > han && latin >= 20
}

#[derive(Deserialize)]
struct Wire {
    translation: String,
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"translation": {"type": "string"}},
        "required": ["translation"],
        "additionalProperties": false
    })
}

/// 把一条贴文正文完整译成简体中文。返回的译文已带 [`MARK`] 开头。
pub async fn translate(model: &ModelClient, text: &str) -> Result<String> {
    let mut s = String::from(
        "把下面这条社交媒体贴文的正文完整翻译成简体中文，给中文编辑核对原文用。\n\
         - 逐段对应原文，不删、不并、不概括、不加评论；段落与换行照原文。\n\
         - 价格、尺寸、重量、容量、日期、型号、地址、电话、营业时间等数字与专有名词原样保留，必要时在括号里附原文。\n\
         - 限定条件、注意事项、免责声明都要译全。\n\
         - 网址原样保留；话题标签保留原文，后面括号里附中文意思。\n\
         - 品牌名、产品名保留原文写法，可在括号里附中文。\n\
         - 只输出译文本身。\n\n",
    );
    s.push_str("【边界】\n");
    s.push_str(csw_collector_core::prompt::DATA_NOT_INSTRUCTIONS);
    s.push_str("\n\n正文：\n");
    s.push_str(&csw_collector_core::prompt::fence(text.trim()));
    let out = model
        .structured(&[Part::Text(s)], "csw_translate", &schema(), 6000)
        .await
        .context("补译")?;
    let w: Wire = out.parse()?;
    let t = w.translation.trim();
    anyhow::ensure!(!t.is_empty(), "补译回了空译文");
    Ok(format!("{MARK}\n{t}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日文要译中文不译() {
        assert!(needs_translation(
            "ルーフコンテナボックス\n車中泊やバンライフの積載ストレスを解消"
        ));
        assert!(needs_translation(
            "New drop: the Trail Pack 30 is back in three colors, shipping next week worldwide."
        ));
        assert!(!needs_translation(
            "新品发布：三色帐篷下周到货 #camping #outdoor"
        ));
        assert!(!needs_translation("#vanlife #garagelife"));
        assert!(!needs_translation(""));
    }
}
