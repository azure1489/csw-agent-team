//! `index.md` 的元信息头与渲染。
//!
//! 交付物协议里说明了头里有什么：任务 / 类型 / 交付 Agent / 阶段 / 版本 / 时间 /
//! 上游来源 / 状态 / 自检；按条目交付的再加「条目」。
//!
//! **时间由调用方给，这里绝不叫 `now()`。** 确定性打包的六条钉死了 zip 的元数据，
//! 但只要正文里写了当前时间，同样的内容还是会打出不同的哈希，六条全白做。

use std::fmt::Write as _;

/// `index.md` 顶部的 YAML 元信息头。
#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// 「主编 · r48 任务#123 派工单」这种引用写法
    pub task: String,
    /// 产出 | 补件 | 定点编辑
    pub kind: String,
    pub agent: String,
    /// 01-情报逐条
    pub stage: String,
    pub version: String,
    /// RFC3339。**调用方给**，不要用当前时间。
    pub at: String,
    pub upstreams: Vec<String>,
    pub status: String,
    /// 自检逐条
    pub checks: Vec<String>,
    /// 按条目交付时才有
    pub item_key: String,
}

impl Meta {
    pub fn to_yaml(&self) -> String {
        let mut s = String::from("---\n");
        for (k, v) in [
            ("任务", &self.task),
            ("类型", &self.kind),
            ("交付 Agent", &self.agent),
            ("阶段", &self.stage),
            ("版本", &self.version),
            ("时间", &self.at),
            ("状态", &self.status),
        ] {
            let _ = writeln!(s, "{k}: {}", scalar(v));
        }
        if !self.item_key.is_empty() {
            let _ = writeln!(s, "条目: {}", scalar(&self.item_key));
        }
        list(&mut s, "上游来源", &self.upstreams);
        list(&mut s, "自检", &self.checks);
        s.push_str("---\n");
        s
    }
}

fn list(s: &mut String, key: &str, items: &[String]) {
    if items.is_empty() {
        // 空列表要写成 `[]` 而不是省略：省略会让「没有上游」和「忘了写」长得一样
        let _ = writeln!(s, "{key}: []");
        return;
    }
    let _ = writeln!(s, "{key}:");
    for x in items {
        let _ = writeln!(s, "  - {}", scalar(x));
    }
}

/// YAML 标量。
///
/// 这里**一律加引号**，不去猜哪些值「看起来安全」。判断标题里带冒号、
/// 井号、开头是破折号的情况多得是，猜错一次就是整份 YAML 解析失败。
fn scalar(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 一份 `index.md`。
pub struct Document {
    pub meta: Meta,
    /// 正文，Markdown。**不要在里面写当前时间。**
    pub body: String,
}

impl Document {
    pub fn render(&self) -> String {
        format!("{}\n{}", self.meta.to_yaml(), self.body.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            task: "主编 · r48 任务#123 派工单".into(),
            kind: "产出".into(),
            agent: "情报收集员".into(),
            stage: "01-情报逐条".into(),
            version: "v1".into(),
            at: "2026-09-18T05:40:00Z".into(),
            upstreams: vec!["主编 · r48 任务#123 派工单".into()],
            status: "待审".into(),
            checks: vec!["窗口内 358 条全部判过".into()],
            item_key: String::new(),
        }
    }

    #[test]
    fn 头里该有的字段一个不少() {
        let y = meta().to_yaml();
        for k in [
            "任务",
            "类型",
            "交付 Agent",
            "阶段",
            "版本",
            "时间",
            "状态",
            "上游来源",
            "自检",
        ] {
            assert!(y.contains(&format!("{k}:")), "少了 {k}：\n{y}");
        }
        assert!(y.starts_with("---\n") && y.trim_end().ends_with("---"));
    }

    #[test]
    fn 按条目交付才有条目字段() {
        assert!(!meta().to_yaml().contains("条目:"));
        let m = Meta {
            item_key: "nanga-ab12cd".into(),
            ..meta()
        };
        assert!(m.to_yaml().contains("条目: \"nanga-ab12cd\""));
    }

    #[test]
    fn 带冒号井号的值不会把yaml弄坏() {
        // 判断标题里带冒号、井号、开头是破折号的情况多得是
        let m = Meta {
            task: "r48: 第一批 #热点 - 见下".into(),
            checks: vec!["引号 \" 与反斜杠 \\ 都要转义".into()],
            ..meta()
        };
        let y = m.to_yaml();
        assert!(y.contains(r#"任务: "r48: 第一批 #热点 - 见下""#), "{y}");
        assert!(y.contains(r#"引号 \" 与反斜杠 \\ 都要转义"#), "{y}");
    }

    #[test]
    fn 空列表写成方括号不省略() {
        // 省略会让「没有上游」和「忘了写」长得一样
        let m = Meta {
            upstreams: vec![],
            checks: vec![],
            ..meta()
        };
        let y = m.to_yaml();
        assert!(y.contains("上游来源: []"), "{y}");
        assert!(y.contains("自检: []"), "{y}");
    }

    #[test]
    fn 渲染两次一模一样() {
        let d = Document {
            meta: meta(),
            body: "## 判断台账\n\n共 358 条。\n".into(),
        };
        let a = d.render();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        // 正文里写了当前时间的话，确定性打包的六条就全白做了
        assert_eq!(a, d.render());
        assert!(a.contains("2026-09-18T05:40:00Z"));
        assert!(a.contains("## 判断台账"));
    }
}
