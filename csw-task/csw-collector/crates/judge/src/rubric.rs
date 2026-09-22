//! Van 的判断框架。**这是口径，不是可调参数**——改任何一条都要她本人点头。
//!
//! 放在代码里是因为它要跟着提示词走；改动通过 [`RUBRIC_VERSION`] 进
//! `judgements.inputs_hash`，让「框架变了所以结论要重算」这件事自动成立。

/// 框架版本。**改了下面任何一条常量就要改它**，否则旧结论会被当成还能用。
pub const RUBRIC_VERSION: &str = "van-rubric/v1";

/// 核心问题。六维、三句话、七类都是围着它转的。
pub const CORE_QUESTION: &str = "这条里有没有一个具体到值得 CSW 向读者解释一次的变化？";

/// 三句话。成稿前答不清就先不写，并写明是缺资料、角度未成立还是价值不足。
pub const THREE_QUESTIONS: [&str; 3] = [
    "发生了什么变化？",
    "为什么值得户外用户知道？",
    "它和以前有什么不一样？",
];

/// 七类优先关注。**是倾向，不是白名单**——命中不等于推荐。
pub const PRIORITY: [&str; 7] = [
    "老产品结构性改款",
    "装备解决具体使用问题",
    "品牌新的空间、社群或服务方式",
    "装备进入新生活场景且有超出造型挪用的信息",
    "小众品类值得解释的新设计",
    "有真实争议或取舍的产品",
    "能从产品解释有证据的消费或生活方式变化",
];

/// 七类降低优先级。**是倾向，不是黑名单**——命中不等于不推荐。
pub const LOWER: [&str; 7] = [
    "只有新配色或普通 logo 联名",
    "换材质说不出差异",
    "宣称创新无可核实变化",
    "纯明星带货",
    "缺可靠来源先补证",
    "只能说好看",
    "必须强行拔高成趋势",
];

/// 六维的中文定义与锚点，给生成模型看。
///
/// 顺序与 [`csw_collector_core::types::Dim::ALL`] 一致。
pub const DIM_ANCHORS: [(&str, &str); 6] = [
    (
        "change",
        "具体变化：说得出是哪个产品、哪一代、换了什么结构或材料、办了什么事。\
         「上新了」「全新登场」这类没有具体物的不算。",
    ),
    (
        "use",
        "使用关联：与怎么穿、怎么背、怎么扎营、怎么做饭、怎么出行有关。\
         只讲外观或只宣布一件事不算。",
    ),
    (
        "gain",
        "信息增量：读者能学到点什么——设计意图、规格、发售日期、价格、渠道、背景。\
         只有一个名字不算。",
    ),
    (
        "compare",
        "比较参照：有上一代、品牌过去的做法、或同类产品可比。暗示但没说出来的算「不明」。",
    ),
    (
        "explain",
        "可解释性：有一个有事实支撑的编辑判断——某个设计选择能连到某种使用、需求或历史。\
         「好看」「有意思」不算。",
    ),
    (
        "csw",
        "CSW 视角：落在户外、露营、城市户外生活的范围内，带设计或文化角度，\
         对中国读者说得通。纯本地店铺通知不算。",
    ),
];

/// 不作为维度的东西。写在这里是因为它们**很容易被误当成维度**。
pub const NOT_DIMENSIONS: [&str; 4] = ["品牌知名度", "点赞与评论数", "csw 标签", "话题"];

/// 把框架拼成给生成模型看的一段。每条候选的提示词里原样注入。
pub fn as_prompt_block() -> String {
    let mut s = String::new();
    s.push_str("【核心问题】\n");
    s.push_str(CORE_QUESTION);
    s.push_str("\n\n【六个维度】\n");
    for (k, v) in DIM_ANCHORS {
        s.push_str(&format!("- {k}：{v}\n"));
    }
    s.push_str("\n【三句话】成稿前要答得上：\n");
    for q in THREE_QUESTIONS {
        s.push_str(&format!("- {q}\n"));
    }
    s.push_str("\n【优先关注（倾向，不是白名单）】\n");
    for x in PRIORITY {
        s.push_str(&format!("- {x}\n"));
    }
    s.push_str("\n【降低优先级（倾向，不是黑名单）】\n");
    for x in LOWER {
        s.push_str(&format!("- {x}\n"));
    }
    s.push_str("\n【不是维度】");
    s.push_str(&NOT_DIMENSIONS.join("、"));
    s.push_str("；来源不加权。\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::Dim;

    #[test]
    fn 六维锚点与类型里的维度一一对应() {
        // 少一个或拼错一个，提示词里就会缺一维，而模型多半会照缺的那样输出
        let keys: Vec<&str> = DIM_ANCHORS.iter().map(|(k, _)| *k).collect();
        let dims: Vec<String> = Dim::ALL
            .iter()
            .map(|d| {
                serde_json::to_value(d)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(keys, dims);
    }

    #[test]
    fn 框架块把七类都写进去了() {
        let s = as_prompt_block();
        for x in PRIORITY.iter().chain(LOWER.iter()) {
            assert!(s.contains(x), "框架块里少了「{x}」");
        }
        assert!(s.contains(CORE_QUESTION));
        // 「不是维度」这一段最容易被漏掉，而漏掉的后果是模型拿知名度当维度
        assert!(s.contains("品牌知名度"));
        assert!(s.contains("来源不加权"));
    }
}
