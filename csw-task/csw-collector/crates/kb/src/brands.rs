//! 品牌命中的**主路**：别名表 + 代码精确匹配。
//!
//! 为什么主路不是分词器：品牌名多是拉丁文（`and wander`）、假名（`山と道`）、
//! 两字中文（`不二吉選`）、带符号的（`笑's -SHO 's`）、甚至花体 Unicode
//! （`𝐂𝐎𝐋𝐎𝐍𝐈𝐒𝐓𝐀®`）——分词器对这些一个都切不稳。精确匹配反而简单可靠。
//!
//! 阶段 0.3 实测：543 个在册账号 → 926 个别名，其中 347 个在 9/17–9/18 的窗口正文里
//! 真出现过，覆盖 438 条里的 423 条（97%）。

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

/// 别名的最短长度，**按字形分**。
///
/// 拉丁别名要 4 个字符：`GO`、`and` 这类在正文里到处都是，命中了也没意义。
/// 但中日文品牌名本来就短——`山と道`（3）、`不二吉選`（4）、`風街道具店`（5）——
/// 一律按 4 卡会把一批真品牌剔掉。CJK 两个字就已经足够specific了。
pub const MIN_ALIAS_CHARS_LATIN: usize = 4;
pub const MIN_ALIAS_CHARS_CJK: usize = 2;

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // 假名
        | 0x3400..=0x4DBF // 扩展 A
        | 0x4E00..=0x9FFF // 常用汉字
        | 0xF900..=0xFAFF // 兼容汉字
        | 0xAC00..=0xD7AF // 谚文
    )
}

/// 这个别名至少要几个字符
pub fn min_chars(alias: &str) -> usize {
    if alias.chars().any(is_cjk) {
        MIN_ALIAS_CHARS_CJK
    } else {
        MIN_ALIAS_CHARS_LATIN
    }
}

#[derive(Debug, Clone)]
pub struct Alias {
    pub brand: String,
    pub alias: String,
    /// 小写归一后的形态，匹配时用它
    pub alias_lc: String,
    /// 含空白的别名进不了 jieba 用户词典（词典按空白分列），建词典时要跳过。
    /// 精确匹配不受影响。
    pub has_space: bool,
}

impl Alias {
    pub fn new(brand: &str, alias: &str) -> Option<Self> {
        let alias = alias.trim();
        if alias.chars().count() < min_chars(alias) {
            return None;
        }
        Some(Self {
            brand: brand.trim().to_string(),
            alias: alias.to_string(),
            alias_lc: normalize(alias),
            has_space: alias.chars().any(char::is_whitespace),
        })
    }
}

/// 归一：小写 + 去掉零宽字符。
///
/// 刻意**不**做更激进的归一（去符号、全角转半角）：`笑's` 与 `笑s` 是不是同一个品牌
/// 该由别名表说了算，不该由匹配函数猜。猜错了没人发现得了。
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// 品牌键：小写、只留字母数字（含中日韩文字）。
///
/// 同一个品牌在库里有两种写法：别名表与已生成贴文用展示名（`SNOW PEAK`、
/// `GORDON MILLER`），引擎的决定与台账用条目键里的小写短横写法（`snowpeak`、
/// `gordon-miller`）。按原样比，品牌路就只认得前一种——线上跑 M2 时
/// 394 条决定只有 30 条能从品牌路捞到。比之前先都折成这个键。
pub fn brand_key(s: &str) -> String {
    normalize(s)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// 别名表。建一次，整轮复用。
pub struct BrandIndex {
    aliases: Vec<Alias>,
}

impl BrandIndex {
    pub fn new(aliases: Vec<Alias>) -> Self {
        let mut aliases = aliases;
        // 长的排前面：`and wander` 应当先于 `wander` 命中，否则前者永远被后者吃掉
        aliases.sort_by_key(|a| std::cmp::Reverse(a.alias_lc.chars().count()));
        Self { aliases }
    }

    /// 从库里读。
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut st = conn.prepare(
            "SELECT b.name, a.alias, a.alias_lc, a.has_space
             FROM brand_aliases a JOIN brands b ON b.id = a.brand_id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Alias {
                brand: r.get(0)?,
                alias: r.get(1)?,
                alias_lc: r.get(2)?,
                has_space: r.get::<_, i64>(3)? == 1,
            })
        })?;
        Ok(Self::new(rows.filter_map(Result::ok).collect()))
    }

    /// 写进库。重复别名按 `alias_lc` 去重（库上有 UNIQUE）。
    pub fn save(&self, conn: &Connection, source: &str) -> Result<usize> {
        let now = jiff::Timestamp::now().to_string();
        let mut n = 0;
        for a in &self.aliases {
            conn.execute(
                "INSERT INTO brands(name, created_at) VALUES (?1, ?2) ON CONFLICT(name) DO NOTHING",
                rusqlite::params![a.brand, now],
            )?;
            let brand_id: i64 =
                conn.query_row("SELECT id FROM brands WHERE name=?1", [&a.brand], |r| {
                    r.get(0)
                })?;
            let done = conn.execute(
                "INSERT INTO brand_aliases(brand_id, alias, alias_lc, has_space, source)
                 VALUES (?1,?2,?3,?4,?5) ON CONFLICT(alias_lc) DO NOTHING",
                rusqlite::params![
                    brand_id,
                    a.alias,
                    a.alias_lc,
                    i64::from(a.has_space),
                    source
                ],
            )?;
            n += done;
        }
        Ok(n)
    }

    /// 文本里出现了哪些品牌。返回 `品牌 → 命中的别名`。
    ///
    /// 同一个品牌的多个别名都命中时只留最长的那个——「命中了」这件事不该被
    /// 数成三次，那会让后面按命中数排序的逻辑偏向别名多的品牌。
    pub fn hits(&self, text: &str) -> HashMap<String, String> {
        let hay = normalize(text);
        let mut out: HashMap<String, String> = HashMap::new();
        for a in &self.aliases {
            if !hay.contains(&a.alias_lc) {
                continue;
            }
            out.entry(a.brand.clone())
                .or_insert_with(|| a.alias.clone());
        }
        out
    }

    /// 能进 jieba 用户词典的那些别名（不含空白）。
    pub fn dict_words(&self) -> Vec<&str> {
        self.aliases
            .iter()
            .filter(|a| !a.has_space)
            .map(|a| a.alias.as_str())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.aliases.len()
    }
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> BrandIndex {
        BrandIndex::new(
            [
                ("and wander", "and wander"),
                ("and wander", "andwander"),
                ("山と道", "山と道"),
                ("山と道", "Yamatomichi"),
                ("Snow Peak", "snowpeak_official"),
                ("GO", "GO"), // 太短，会被剔掉
            ]
            .iter()
            .filter_map(|(b, a)| Alias::new(b, a))
            .collect(),
        )
    }

    #[test]
    fn 别名下限按字形分() {
        // 拉丁要四个字符：GO / and 这类在正文里到处都是
        assert!(Alias::new("GO", "GO").is_none());
        assert!(Alias::new("and", "and").is_none());
        assert!(Alias::new("KEEN", "KEEN").is_some());
        // 中日文两个字就够：一律按 4 卡会把一批真品牌剔掉
        assert!(
            Alias::new("山と道", "山と道").is_some(),
            "三字日文品牌不该被剔掉"
        );
        assert!(Alias::new("不二吉選", "不二吉選").is_some());
        assert!(Alias::new("波里", "波里").is_some());
        assert!(Alias::new("山", "山").is_none(), "一个字还是太泛");
        assert_eq!(idx().len(), 5, "六个里只有 GO 被剔掉");
    }

    #[test]
    fn 大小写不敏感且假名中文都认() {
        let i = idx();
        let h = i.hits("今天 AND WANDER 和 山と道 都上新了");
        assert_eq!(h.get("and wander").map(String::as_str), Some("and wander"));
        assert!(h.contains_key("山と道"));
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn 同一品牌多个别名命中只算一次() {
        // and wander 与 andwander 都在，但「命中了」不该被数成两次
        let h = idx().hits("and wander / andwander 双写");
        assert_eq!(h.len(), 1);
        assert_eq!(h["and wander"], "and wander", "留最长的那个别名");
    }

    #[test]
    fn 长别名优先不被短的吃掉() {
        let i = BrandIndex::new(
            [("and wander", "and wander"), ("Wander", "wander")]
                .iter()
                .filter_map(|(b, a)| Alias::new(b, a))
                .collect(),
        );
        let h = i.hits("and wander 新品");
        // 两个品牌都会命中（"wander" 是 "and wander" 的子串），这是事实不是 bug；
        // 排序保证了 and wander 先被看到，后面按命中长度取舍时不会丢掉它
        assert!(h.contains_key("and wander"));
    }

    #[test]
    fn 含空格的别名进不了jieba词典但不影响精确匹配() {
        let i = idx();
        let dict = i.dict_words();
        assert!(
            !dict.contains(&"and wander"),
            "含空格的进不了 jieba 用户词典"
        );
        assert!(dict.contains(&"andwander"));
        // 精确匹配照样命中
        assert!(i.hits("and wander").contains_key("and wander"));
    }

    #[test]
    fn 零宽字符不该让匹配失效() {
        // 贴文正文里夹零宽字符很常见（复制粘贴带进来的、或刻意用来躲关键词的）。
        // normalize 会把它们剔掉，所以夹在词中间也照样命中——这正是剔掉它的目的。
        let i = idx();
        assert!(
            i.hits("and\u{200b} wander").contains_key("and wander"),
            "词中间的零宽要剔掉"
        );
        assert!(
            i.hits("\u{feff}and wander").contains_key("and wander"),
            "前缀的 BOM 要剔掉"
        );
        assert!(
            i.hits("a\u{200d}n\u{200d}d wander")
                .contains_key("and wander"),
            "逐字夹也要剔掉"
        );
    }

    #[test]
    fn 存取往返() {
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let i = idx();
        let n = i.save(&conn, "accounts").unwrap();
        assert_eq!(n, 5);
        // 重跑幂等
        assert_eq!(i.save(&conn, "accounts").unwrap(), 0);
        let back = BrandIndex::load(&conn).unwrap();
        assert_eq!(back.len(), 5);
        assert!(back.hits("Yamatomichi 的新包").contains_key("山と道"));
    }

    #[test]
    fn 展示名与条目键折成同一个品牌键() {
        assert_eq!(brand_key("SNOW PEAK"), brand_key("snowpeak"));
        assert_eq!(brand_key("GORDON MILLER"), brand_key("gordon-miller"));
        assert_eq!(brand_key("tech_country"), brand_key("tech-country"));
        assert_eq!(brand_key("山と道"), "山と道");
        assert_ne!(brand_key("NEMO Equipment Japan"), brand_key("nemo"));
    }
}
