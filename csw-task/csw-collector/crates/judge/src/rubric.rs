//! Van 的判断框架。**这是口径，不是可调参数**——改任何一条都要她本人点头。
//!
//! 放在代码里是因为它要跟着提示词走；改动通过 [`RUBRIC_VERSION`] 进
//! `judgements.inputs_hash`，让「框架变了所以结论要重算」这件事自动成立。

/// 框架版本。**改了下面任何一条常量就要改它**，否则旧结论会被当成还能用。
///
/// v2（09-23）：六维换成 Van 本人认可的新定义（09-22 判断台账反馈第一项），
/// 核心问题从「有没有值得解释的变化」改成「值不值得推荐给中国户外潮流读者」。
pub const RUBRIC_VERSION: &str = "van-rubric/v2";

/// 核心问题。六维、三句话、七类都是围着它转的。
pub const CORE_QUESTION: &str = "这条值不值得推荐给中国户外潮流读者？凭什么——读者看完能得到什么有用的信息、有趣的认识，或值得了解的设计与文化内容？";

/// 三句话。成稿前答不清就先不写，并写明是缺资料、角度未成立还是价值不足。
pub const THREE_QUESTIONS: [&str; 3] = [
    "是什么？（具体对象：哪个品牌、哪件产品或哪件事）",
    "为什么值得中国户外潮流读者看？",
    "依据是什么？（正文原话或第几张图；有旧款证据才谈变化）",
];

/// 四问要分清，别混成一句。
pub const FOUR_QUESTIONS: [&str; 4] = [
    "读者关联：跟谁有关？",
    "读者价值：为什么值得看？",
    "报道角度：具体讲什么？",
    "CSW 适配度：为什么适合由 CSW 来讲？",
];

/// 六维不是打勾表。
pub const NOT_A_CHECKLIST: &str = "「值得推荐的价值」（gain）是核心判断，其他五维为它提供支撑。\
六维不是必须全部通过的打勾表，也不按成立数量定档：**推荐必须 gain 成立且有依据**；\
gain 不成立，其余几项再齐也最多备选。不能用参数、价格、发售日期齐全证明价值，\
也不能因品牌知名或国外发布就自动推荐。";

/// 中国读者价值的具体含义（Van 原框架里的三个例子）。
pub const CHINA_READER: &str = "中国读者价值不等于必须在国内买得到、去得了。国外设计、品牌文化和空间实践也可以有阅读价值，\
但要说明跨越地域后仍成立的看点。例如：国外店铺开业，只有地址和营业时间通常价值有限，独特的设计、服务或文化内容可能值得介绍；\
日本限定装备，限定身份本身不够，具体结构、工艺和审美表达可以成为看点；\
海外活动，普通报名通知价值有限，值得了解的社群组织方式或文化实践可能有报道价值。";

/// 七类优先关注。**是倾向，不是白名单**——命中不等于推荐。
/// 新品与改款**不是前提**：非新品也可以值得报道。
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

/// 六维的中文定义与锚点，给生成模型看。**键名不变**（引擎按键名校验），含义是 v2 的。
///
/// 顺序与 [`csw_collector_core::types::Dim::ALL`] 一致。
pub const DIM_ANCHORS: [(&str, &str); 6] = [
    (
        "change",
        "具体看点：这条内容最值得报道的具体事实、变化或角度是什么？可以是产品改款，\
         也可以是有依据的设计、文化、历史或生活方式内容，不强求新品。说不出具体物的不算。",
    ),
    (
        "use",
        "与读者有关：它与中国户外潮流爱好者的使用、穿搭、审美、兴趣或生活方式有什么联系？\
         不限于解决实际功能问题。",
    ),
    (
        "gain",
        "值得推荐的价值（核心）：为什么值得推荐给中国户外潮流读者？看完能获得什么有用的信息、\
         有趣的认识，或值得了解的设计与文化内容？",
    ),
    (
        "compare",
        "差异与背景：有什么参照能帮助理解它的特别之处？可以是同类、历史、品牌背景或既有做法，\
         不强求上一代产品对比。",
    ),
    (
        "explain",
        "报道角度与依据：CSW 准备抓住哪个具体角度讲？哪些事实支持这个角度，能够讲到什么程度？\
         不要求每条资讯都制造趋势或宏大观点。",
    ),
    (
        "csw",
        "CSW 适配度：是否符合 CSW 的内容取舍、审美与文化视角？结合给出的实际采用和否决案例说明，\
         不能只因出现露营、户外、旅行等词就判定符合。",
    ),
];

/// 与选题价值**分开记**的制作条件（写进 readiness，不参与定档）。
pub const READINESS_NOTE: &str = "事实可靠性、图片可用性、查重和制作资料完整度单独记在 readiness 与 comparison 里，不与选题价值混在一起。";

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
    s.push_str("\n【六维怎么用】\n");
    s.push_str(NOT_A_CHECKLIST);
    s.push('\n');
    s.push_str(READINESS_NOTE);
    s.push_str("\n\n【四问要分清】\n");
    for q in FOUR_QUESTIONS {
        s.push_str(&format!("- {q}\n"));
    }
    s.push_str("\n【中国读者价值】\n");
    s.push_str(CHINA_READER);
    s.push('\n');
    s.push_str("\n【三句话】成稿前要答得上：\n");
    for q in THREE_QUESTIONS {
        s.push_str(&format!("- {q}\n"));
    }
    s.push_str("\n【优先关注（倾向，不是白名单；新品不是前提）】\n");
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
        assert!(s.contains("推荐必须 gain 成立"));
        assert!(s.contains("日本限定装备"));
        for q in FOUR_QUESTIONS {
            assert!(s.contains(q));
        }
        // 「不是维度」这一段最容易被漏掉，而漏掉的后果是模型拿知名度当维度
        assert!(s.contains("品牌知名度"));
        assert!(s.contains("来源不加权"));
    }
}
