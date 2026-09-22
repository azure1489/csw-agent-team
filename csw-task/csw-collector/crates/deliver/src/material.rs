//! 05「配图与素材核」与 11「小红书选图包」的交付物。
//!
//! 两个阶段共用这一份：它们做的是同一件事的两半——**把某条资讯的原图备齐，
//! 并说清每张图里有什么**。差别只在交付形态：
//!
//! | | 05 配图与素材核 | 11 小红书选图包 |
//! |---|---|---|
//! | 派工 | 逐条 | 手动，一次一整期 |
//! | 交什么 | 三个图位 + 素材核对结论 | 按资讯分组的原图，不挑图位 |
//! | 挑不挑 | 挑，并说明为什么是这三张 | 不挑，图位由小红书作者定 |
//!
//! # 原图，不是缩略图
//!
//! 01 那一步下的是 w_768，够判断、不够排版。这两个阶段**一律下原图**：
//! 设计师拿 768 宽的图去做公众号头图，放大后糊得一眼就能看出来。
//!
//! # 原始披露时间与转载时间分开写
//!
//! 作业手册里的硬要求。同一条资讯被三个账号转过时，「谁先发的」
//! 决定了这条该怎么署来源；混成一个「时间」字段就再也分不出来了。

use std::fmt::Write;

use csw_collector_core::types::{ImageKind, MediaDescription};

use crate::pack::Entry;

/// 05 给几个图位。文案那一步每则正文写 3 个图位，这里就备 3 张。
pub const FIGURE_SLOTS: usize = 3;

/// 11 每条资讯最多放几张原图。原图一张两三百 KB，一期八条就是上百张——
/// 不设上限会把 48 MiB 的包撑爆，而撑爆是在**提交那一刻**才发现的。
pub const XHS_MAX_PER_ITEM: usize = 6;

/// 一张备选图。
#[derive(Debug, Clone)]
pub struct Shot {
    pub blake3: String,
    pub ordinal: u16,
    /// 原图地址，写进正文让人能回溯
    pub url: String,
    pub bytes: Vec<u8>,
    /// 扩展名，空的按 jpg
    pub ext: String,
    /// 识别结果。**没有就是没识别成功**，这张图不进图位
    pub desc: Option<MediaDescription>,
}

impl Shot {
    fn ext(&self) -> &str {
        if self.ext.trim().is_empty() {
            "jpg"
        } else {
            self.ext.trim()
        }
    }
    fn file(&self, item_key: &str) -> String {
        format!(
            "images/{}_{:02}.{}",
            safe_name(item_key),
            self.ordinal,
            self.ext()
        )
    }
}

/// 一条资讯连同它的图。
#[derive(Debug, Clone, Default)]
pub struct ItemShots {
    pub item_key: String,
    pub title: String,
    pub source_url: String,
    pub account: String,
    /// 原始披露时间（贴文自己的发布时间）
    pub posted_at: String,
    /// 转载时间（进我们库的时间）。**与上一行分开写**
    pub ingested_at: String,
    pub shots: Vec<Shot>,
    /// 下载或识别失败的、以及别的缺口
    pub gaps: Vec<String>,
}

/// 挑图位：**能当配图的才进**，按画面类型排。
///
/// 类型的先后不是审美偏好，是用途：产品图说清「是什么」，细节图说清
/// 「变在哪」，两张就够撑起一则三段正文；场景与穿着图是氛围，排在后面；
/// 海报多半带大字，压上标题会打架。**截图与无关图一张都不要。**
pub fn pick_figures(shots: &[Shot], n: usize) -> Vec<&Shot> {
    let mut usable: Vec<&Shot> = shots
        .iter()
        .filter(|s| s.desc.as_ref().is_some_and(|d| d.usable_as_figure))
        .filter(|s| {
            !matches!(
                s.desc.as_ref().map(|d| d.kind),
                Some(ImageKind::Screenshot) | Some(ImageKind::Unrelated)
            )
        })
        .collect();
    usable.sort_by_key(|s| (kind_rank(s.desc.as_ref().map(|d| d.kind)), s.ordinal));
    // 同一张图在一条轮播里出现两次是真有的事。三个图位放两张一样的，
    // 文案那边会以为是两个角度
    let mut seen = std::collections::HashSet::new();
    usable.retain(|s| seen.insert(s.blake3.clone()));
    usable.truncate(n);
    usable
}

fn kind_rank(k: Option<ImageKind>) -> u8 {
    match k {
        Some(ImageKind::Product) => 0,
        Some(ImageKind::Detail) => 1,
        Some(ImageKind::Outfit) => 2,
        Some(ImageKind::Scene) => 3,
        Some(ImageKind::Poster) => 4,
        _ => 5,
    }
}

/// 05 的正文：三个图位 + 素材核对。
pub fn material_body(item: &ItemShots, picked: &[&Shot]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# 配图与素材核 · {}", item.title);
    let _ = writeln!(s);
    let _ = writeln!(s, "条目：`{}`", item.item_key);
    let _ = writeln!(s);
    header_table(&mut s, item);

    let _ = writeln!(s, "## 图位");
    let _ = writeln!(s);
    if picked.is_empty() {
        // 一张都挑不出来要说清楚，别交一份看着正常的空壳
        let _ = writeln!(
            s,
            "**一张都没挑出来。** 这条的图要么没下到、要么没识别成功、要么全是截图或与产品无关，\
             逐张情况见下面的「素材核对」。"
        );
        let _ = writeln!(s);
    }
    for (i, shot) in picked.iter().enumerate() {
        let d = shot.desc.as_ref();
        let _ = writeln!(s, "### 图位 {}", i + 1);
        let _ = writeln!(s);
        let _ = writeln!(s, "![图位 {}]({})", i + 1, shot.file(&item.item_key));
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "- 画面：{}",
            d.map(|d| d.content.as_str()).unwrap_or("（未识别）")
        );
        let _ = writeln!(
            s,
            "- 对应正文：{}",
            d.map(|d| d.matches_text.as_str()).unwrap_or("")
        );
        if let Some(missing) = d
            .map(|d| d.missing_from_text.as_str())
            .filter(|x| !x.is_empty())
        {
            let _ = writeln!(s, "- 正文提到但画面没有：{missing}");
        }
        let _ = writeln!(s, "- 类型：{}", kind_cn(d.map(|d| d.kind)));
        let _ = writeln!(s, "- 原图：{}", shot.url);
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## 素材核对");
    let _ = writeln!(s);
    let _ = writeln!(s, "| 序 | 文件 | 类型 | 可作配图 | 画面 |");
    let _ = writeln!(s, "|---|---|---|---|---|");
    for shot in &item.shots {
        let d = shot.desc.as_ref();
        let _ = writeln!(
            s,
            "| {} | `{}` | {} | {} | {} |",
            shot.ordinal,
            shot.file(&item.item_key),
            kind_cn(d.map(|d| d.kind)),
            match d {
                Some(d) if d.usable_as_figure => "是",
                Some(_) => "否",
                None => "未识别",
            },
            cell(d.map(|d| d.content.as_str()).unwrap_or("（未识别）")),
        );
    }
    let _ = writeln!(s);
    gaps_section(&mut s, &item.gaps);
    s
}

/// 11 的正文：按资讯分组的原图，不挑图位。
pub fn pick_body(items: &[ItemShots]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# 小红书选图包");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "按资讯分组的原图，共 {} 条资讯、{} 张图。**没有挑图位**——\
         哪张进头图、哪张进正文由小红书图文作者定，这里只把料备齐。",
        items.len(),
        items.iter().map(|i| i.shots.len()).sum::<usize>()
    );
    let _ = writeln!(s);
    for item in items {
        let _ = writeln!(s, "## {}", item.title);
        let _ = writeln!(s);
        let _ = writeln!(s, "条目：`{}`", item.item_key);
        let _ = writeln!(s);
        header_table(&mut s, item);
        for shot in &item.shots {
            let d = shot.desc.as_ref();
            let _ = writeln!(
                s,
                "- `{}` · {} · {}",
                shot.file(&item.item_key),
                kind_cn(d.map(|d| d.kind)),
                d.map(|d| d.content.as_str()).unwrap_or("（未识别）")
            );
        }
        let _ = writeln!(s);
        gaps_section(&mut s, &item.gaps);
    }
    s
}

/// 来源那一小块。**原始披露时间与转载时间各占一行。**
fn header_table(s: &mut String, item: &ItemShots) {
    let _ = writeln!(s, "| | |");
    let _ = writeln!(s, "|---|---|");
    let _ = writeln!(s, "| 来源 | {} |", cell(&item.source_url));
    let _ = writeln!(s, "| 账号 | {} |", cell(&item.account));
    let _ = writeln!(s, "| 原始披露时间 | {} |", cell(or_dash(&item.posted_at)));
    let _ = writeln!(s, "| 转载时间 | {} |", cell(or_dash(&item.ingested_at)));
    let _ = writeln!(s);
}

fn gaps_section(s: &mut String, gaps: &[String]) {
    if gaps.is_empty() {
        return;
    }
    let _ = writeln!(s, "**缺口**");
    let _ = writeln!(s);
    for g in gaps {
        let _ = writeln!(s, "- {g}");
    }
    let _ = writeln!(s);
}

fn or_dash(s: &str) -> &str {
    if s.trim().is_empty() { "——" } else { s }
}

/// 表格单元：竖线与换行会把表格拆散。
fn cell(s: &str) -> String {
    s.replace('|', "｜").replace(['\n', '\r'], " ")
}

fn kind_cn(k: Option<ImageKind>) -> &'static str {
    match k {
        Some(ImageKind::Product) => "产品",
        Some(ImageKind::Detail) => "细节",
        Some(ImageKind::Scene) => "场景",
        Some(ImageKind::Poster) => "海报",
        Some(ImageKind::Outfit) => "穿着",
        Some(ImageKind::Screenshot) => "截图",
        Some(ImageKind::Unrelated) => "无关",
        None => "未识别",
    }
}

/// 05 的一份交付物：`index.md` + 全部原图。
///
/// **全部原图都放进去**，不只是挑中的三张：主编换图时不该再找我们要一次，
/// 而一条资讯的图统共也就十来张。
pub fn assemble_material(meta: crate::index::Meta, body: String, item: &ItemShots) -> Vec<Entry> {
    let mut out = vec![Entry::text(
        "index.md",
        crate::index::Document { meta, body }.render(),
    )];
    for shot in &item.shots {
        out.push(Entry::binary(
            &shot.file(&item.item_key),
            shot.bytes.clone(),
        ));
    }
    out
}

/// 11 的一份交付物：`index.md` + 按资讯分组的原图。
pub fn assemble_pick(meta: crate::index::Meta, body: String, items: &[ItemShots]) -> Vec<Entry> {
    let mut out = vec![Entry::text(
        "index.md",
        crate::index::Document { meta, body }.render(),
    )];
    for item in items {
        for shot in &item.shots {
            out.push(Entry::binary(
                &shot.file(&item.item_key),
                shot.bytes.clone(),
            ));
        }
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

    fn desc(kind: ImageKind, usable: bool) -> MediaDescription {
        MediaDescription {
            blake3: "b".into(),
            ordinal: 0,
            matches_text: "正文说的那个新背板".into(),
            content: format!("{kind:?} 的画面"),
            missing_from_text: String::new(),
            kind,
            usable_as_figure: usable,
            model: "m".into(),
            prompt_version: "recognize/v1".into(),
        }
    }

    fn shot(ordinal: u16, d: Option<MediaDescription>) -> Shot {
        Shot {
            blake3: format!("b{ordinal}"),
            ordinal,
            url: format!("https://img/{ordinal}.jpg"),
            bytes: vec![0xff, 0xd8, ordinal as u8],
            ext: "jpg".into(),
            desc: d,
        }
    }

    fn item(shots: Vec<Shot>) -> ItemShots {
        ItemShots {
            item_key: "and-wander-a1b2c3".into(),
            title: "and wander 40L 背包".into(),
            source_url: "https://www.instagram.com/p/ABC/".into(),
            account: "and_wander".into(),
            posted_at: "2026-09-17T08:00:00Z".into(),
            ingested_at: "2026-09-18T01:00:00Z".into(),
            shots,
            gaps: vec![],
        }
    }

    #[test]
    fn 图位按用途排产品细节在前() {
        let shots = vec![
            shot(0, Some(desc(ImageKind::Scene, true))),
            shot(1, Some(desc(ImageKind::Detail, true))),
            shot(2, Some(desc(ImageKind::Product, true))),
            shot(3, Some(desc(ImageKind::Poster, true))),
        ];
        let picked: Vec<u16> = pick_figures(&shots, FIGURE_SLOTS)
            .iter()
            .map(|s| s.ordinal)
            .collect();
        // 产品说清「是什么」、细节说清「变在哪」，两张就撑得起一则三段正文
        assert_eq!(picked, [2, 1, 0]);
    }

    #[test]
    fn 同一张图不占两个图位() {
        let mut a = shot(0, Some(desc(ImageKind::Product, true)));
        let mut b = shot(1, Some(desc(ImageKind::Detail, true)));
        // 一条轮播里放了两遍同一张图
        a.blake3 = "same".into();
        b.blake3 = "same".into();
        let shots = vec![a, b, shot(2, Some(desc(ImageKind::Scene, true)))];
        let picked: Vec<u16> = pick_figures(&shots, FIGURE_SLOTS)
            .iter()
            .map(|s| s.ordinal)
            .collect();
        // 三个图位放两张一样的，文案那边会以为是两个角度
        assert_eq!(picked, [0, 2]);
    }

    #[test]
    fn 截图与无关图一张都不要() {
        let shots = vec![
            shot(0, Some(desc(ImageKind::Screenshot, true))),
            shot(1, Some(desc(ImageKind::Unrelated, true))),
            shot(2, Some(desc(ImageKind::Product, true))),
        ];
        let picked: Vec<u16> = pick_figures(&shots, FIGURE_SLOTS)
            .iter()
            .map(|s| s.ordinal)
            .collect();
        assert_eq!(picked, [2]);
    }

    #[test]
    fn 没识别成功的和不能作配图的都不进图位() {
        let shots = vec![
            shot(0, None),
            shot(1, Some(desc(ImageKind::Product, false))),
            shot(2, Some(desc(ImageKind::Detail, true))),
        ];
        let picked: Vec<u16> = pick_figures(&shots, FIGURE_SLOTS)
            .iter()
            .map(|s| s.ordinal)
            .collect();
        assert_eq!(picked, [2]);
    }

    #[test]
    fn 一张都挑不出来要说清楚() {
        let it = item(vec![shot(0, None)]);
        let picked = pick_figures(&it.shots, FIGURE_SLOTS);
        assert!(picked.is_empty());
        let body = material_body(&it, &picked);
        // 别交一份看着正常的空壳
        assert!(body.contains("一张都没挑出来"), "{body}");
        // 但素材核对里那张图还是要列出来，写明「未识别」
        assert!(body.contains("未识别"));
    }

    #[test]
    fn 原始披露时间与转载时间分开写() {
        let it = item(vec![shot(0, Some(desc(ImageKind::Product, true)))]);
        let body = material_body(&it, &pick_figures(&it.shots, FIGURE_SLOTS));
        // 同一条被三个账号转过时，「谁先发的」决定这条该怎么署来源
        assert!(
            body.contains("| 原始披露时间 | 2026-09-17T08:00:00Z |"),
            "{body}"
        );
        assert!(
            body.contains("| 转载时间 | 2026-09-18T01:00:00Z |"),
            "{body}"
        );
    }

    #[test]
    fn 缺时间写破折号不写空() {
        let mut it = item(vec![]);
        it.posted_at = String::new();
        let body = material_body(&it, &[]);
        assert!(body.contains("| 原始披露时间 | —— |"), "{body}");
    }

    #[test]
    fn 全部原图都进包不只是挑中的三张() {
        let it = item(vec![
            shot(0, Some(desc(ImageKind::Product, true))),
            shot(1, Some(desc(ImageKind::Screenshot, true))),
        ]);
        let entries = assemble_material(
            crate::index::Meta::default(),
            material_body(&it, &pick_figures(&it.shots, FIGURE_SLOTS)),
            &it,
        );
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        // 主编换图时不该再找我们要一次
        assert!(
            paths.contains(&"images/and-wander-a1b2c3_00.jpg"),
            "{paths:?}"
        );
        assert!(
            paths.contains(&"images/and-wander-a1b2c3_01.jpg"),
            "{paths:?}"
        );
        assert_eq!(entries[0].path, "index.md");
    }

    #[test]
    fn 选图包按资讯分组且不挑图位() {
        let a = item(vec![shot(0, Some(desc(ImageKind::Product, true)))]);
        let mut b = item(vec![shot(0, Some(desc(ImageKind::Screenshot, true)))]);
        b.item_key = "yamatomichi-d4e5f6".into();
        b.title = "山と道 THREE".into();
        let body = pick_body(&[a.clone(), b.clone()]);
        assert!(body.contains("共 2 条资讯、2 张图"), "{body}");
        assert!(body.contains("## and wander 40L 背包"));
        assert!(body.contains("## 山と道 THREE"));
        // 11 不挑图位：哪张进头图由小红书作者定（正文里只说明这件事，不排图位）
        assert!(!body.contains("### 图位"), "{body}");
        assert!(body.contains("没有挑图位"), "{body}");
        // 截图也给——11 不做筛选，只把料备齐
        assert!(body.contains("截图"), "{body}");

        let entries = assemble_pick(crate::index::Meta::default(), body, &[a, b]);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"images/and-wander-a1b2c3_00.jpg"));
        assert!(paths.contains(&"images/yamatomichi-d4e5f6_00.jpg"));
    }

    #[test]
    fn 条目键里的怪字符不带进文件名() {
        let mut it = item(vec![shot(0, None)]);
        it.item_key = "../../etc/passwd".into();
        let entries = assemble_material(crate::index::Meta::default(), String::new(), &it);
        // 品牌名是从第三方正文来的，不能假设它干净
        assert_eq!(entries[1].path, "images/______etc_passwd_00.jpg");
    }

    #[test]
    fn 表格里的竖线与换行不拆散表格() {
        let mut it = item(vec![]);
        it.account = "a|b\nc".into();
        let body = material_body(&it, &[]);
        assert!(body.contains("| 账号 | a｜b c |"), "{body}");
    }
}
