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
//! 「原图」要说清是哪一层的原图：来源库（Bright Data）2026-06 起只存 Instagram 的 640px 档，
//! 所谓「按原始尺寸下载」只是没有二次缩放，**不是 Instagram 原帖的最大档**（10-02 r60 Van：
//! 预览里的图都比较模糊）。真正的高清图经 hires-service 从原帖取，逐张记来历与校验结论，
//! 见 [`HiresShot`] 与「高清原图溯源」一节。
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
    /// 换进包里的高清原图的来历。`None` 时 `bytes` 是来源库的 640 图
    pub hires: Option<HiresShot>,
}

/// 一张高清原图的来历，**逐项可核**：原帖序号 → Instagram 文件名 → 取得地址 →
/// 实际像素/字节/SHA → 与来源库 640 图的对应校验。主编验收按这张表读，不看自检的一句话。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct HiresShot {
    /// Instagram CDN 文件名，所有分辨率同名
    pub file_key: String,
    /// Instagram CDN 签名地址，会过期，只作证据
    pub source_url: String,
    /// 转存到 OSS 的长期地址
    pub oss_url: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub sha256: String,
    pub mime_type: String,
    /// 被它替换掉的来源库图：像素与字节
    pub source_width: u32,
    pub source_height: u32,
    pub source_bytes: u64,
    /// 对应方式与画面校验结论（人话一句）
    pub matched: String,
}

/// 这条资讯取高清图的总账。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct HiresSummary {
    pub service: String,
    pub shortcode: String,
    pub pk: String,
    /// 服务端命中缓存（24 小时内同一帖子不再打 Instagram）
    pub cached: bool,
    /// 换进包里的张数
    pub replaced: usize,
    /// 来源库里的图片张数
    pub total: usize,
    /// 没换成或只换了一部分的原因；全换成是空的
    pub note: String,
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
    /// 高清原图的总账；没配 hires-service 时是 None，包里就是来源库 640 图
    pub hires: Option<HiresSummary>,
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
        match &shot.hires {
            Some(h) => {
                let _ = writeln!(
                    s,
                    "- 高清原图：{}（{}×{}，{} B，sha256 {}；Instagram 文件 {}）",
                    h.oss_url,
                    h.width,
                    h.height,
                    h.bytes,
                    short_sha(&h.sha256),
                    h.file_key
                );
                let _ = writeln!(
                    s,
                    "- 来源库转存图（{}×{}，{} B，已被上面的高清图替换）：{}",
                    h.source_width, h.source_height, h.source_bytes, shot.url
                );
                let _ = writeln!(s, "- 对应：{}", h.matched);
            }
            None => {
                let _ = writeln!(
                    s,
                    "- 原图（来源库转存，640px 档，不是 Instagram 最大档）：{}",
                    shot.url
                );
            }
        }
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
    hires_section(&mut s, item);
    gaps_section(&mut s, &item.gaps);
    s
}

/// 「高清原图溯源」：逐张从原帖序号追到包内文件，每一步都是可以拿去核的数。
/// 没配 hires-service 时不写这一节——自检里会说明图是来源库 640 档。
fn hires_section(s: &mut String, item: &ItemShots) {
    let Some(h) = &item.hires else {
        return;
    };
    let _ = writeln!(s, "## 高清原图溯源");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "经 hires-service（{}）从 Instagram 原帖取 `image_versions2` 最大一档：shortcode `{}`、pk `{}`，\
         {}；换进包 {} / {} 张。按轮播序号与来源库 640 图对应，每张再做画面校验（dHash）。",
        h.service,
        h.shortcode,
        or_dash(&h.pk),
        if h.cached {
            "服务端命中缓存"
        } else {
            "本次新取"
        },
        h.replaced,
        h.total
    );
    let _ = writeln!(s);
    if !h.note.trim().is_empty() {
        let _ = writeln!(s, "> {}", h.note.trim());
        let _ = writeln!(s);
    }
    let _ = writeln!(
        s,
        "| 序 | Instagram 文件 | 高清来源（OSS） | 像素 | 字节 | SHA256 | 包内文件 | 来源库 640 图 | 校验 |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|---|---|---|");
    for shot in &item.shots {
        match &shot.hires {
            Some(x) => {
                let _ = writeln!(
                    s,
                    "| {} | `{}` | {} | {}×{} | {} | `{}` | `{}` | {} | {} |",
                    shot.ordinal,
                    cell(&x.file_key),
                    cell(&x.oss_url),
                    x.width,
                    x.height,
                    x.bytes,
                    x.sha256,
                    shot.file(&item.item_key),
                    cell(&shot.url),
                    cell(&x.matched)
                );
            }
            None => {
                let _ = writeln!(
                    s,
                    "| {} | —— | —— | —— | —— | —— | `{}` | {} | 未换：包内仍是来源库 640 图 |",
                    shot.ordinal,
                    shot.file(&item.item_key),
                    cell(&shot.url)
                );
            }
        }
    }
    let _ = writeln!(s);
}

fn short_sha(s: &str) -> String {
    let n = s.len().min(16);
    format!("{}…", &s[..n])
}

/// `trace/hires.json`：与「高清原图溯源」同一份数据，给程序读。
pub fn hires_trace(item: &ItemShots) -> Option<String> {
    let h = item.hires.as_ref()?;
    let shots: Vec<serde_json::Value> = item
        .shots
        .iter()
        .map(|s| {
            serde_json::json!({
                "ordinal": s.ordinal,
                "file": s.file(&item.item_key),
                "source_640_url": s.url,
                "hires": s.hires,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "item_key": item.item_key,
        "summary": h,
        "shots": shots,
    }))
    .ok()
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
                "- `{}` · {} · {}{}",
                shot.file(&item.item_key),
                kind_cn(d.map(|d| d.kind)),
                d.map(|d| d.content.as_str()).unwrap_or("（未识别）"),
                match &shot.hires {
                    Some(h) => format!(" · 高清 {}×{}", h.width, h.height),
                    None => " · 来源库 640 档".to_string(),
                }
            );
        }
        let _ = writeln!(s);
        hires_section(&mut s, item);
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
    if let Some(t) = hires_trace(item) {
        out.push(Entry::text("trace/hires.json", t));
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
        if let Some(t) = hires_trace(item) {
            out.push(Entry::text(
                &format!("trace/hires_{}.json", safe_name(&item.item_key)),
                t,
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
            hires: None,
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
            hires: None,
        }
    }

    /// 10-02 r60：Van 要原帖高清图。换进包的每张都要能从原帖序号追到包内文件
    #[test]
    fn 高清图换进图位并逐张写溯源() {
        let mut a = shot(0, Some(desc(ImageKind::Product, true)));
        a.bytes = vec![1, 2, 3];
        a.hires = Some(HiresShot {
            file_key: "830695363_18019224638948673_2645447066939608343_n.jpg".into(),
            source_url: "https://scontent.cdninstagram.com/v/a.jpg?oe=1".into(),
            oss_url: "https://cws-file.oss-cn-guangzhou.aliyuncs.com/upload/2026/10/05/HD.jpg"
                .into(),
            width: 1440,
            height: 1800,
            bytes: 366184,
            sha256: "7455608d6194e3d559d5193490fd0a7b74124a0b2cef19be472446e822d7bc77".into(),
            mime_type: "image/jpeg".into(),
            source_width: 512,
            source_height: 640,
            source_bytes: 58851,
            matched: "序号对应，画面校验通过（dHash 差 2 位）".into(),
        });
        let b = shot(1, Some(desc(ImageKind::Detail, true)));
        let mut it = item(vec![a, b]);
        it.hires = Some(HiresSummary {
            service: "https://hires.aworld.ltd:9877".into(),
            shortcode: "ABC".into(),
            pk: "3999057414473785641".into(),
            cached: false,
            replaced: 1,
            total: 2,
            note: "第 1 张：上传 OSS 失败（upload failed: 502）".into(),
        });
        let picked = pick_figures(&it.shots, FIGURE_SLOTS);
        let body = material_body(&it, &picked);
        assert!(body.contains("## 高清原图溯源"), "{body}");
        assert!(
            body.contains("830695363_18019224638948673"),
            "文件名要在表里"
        );
        assert!(body.contains("7455608d6194e3d559d5193490fd0a7b74124a0b2cef19be472446e822d7bc77"));
        assert!(body.contains("1440×1800"));
        assert!(body.contains("换进包 1 / 2 张"));
        assert!(body.contains("画面校验通过"));
        assert!(
            body.contains("未换：包内仍是来源库 640 图"),
            "没换的那张要如实写"
        );
        assert!(body.contains("上传 OSS 失败"), "原因要进包");
        // 图位里写清哪张是高清、哪张还是转存
        assert!(body.contains("- 高清原图：https://cws-file"));
        assert!(body.contains("- 原图（来源库转存，640px 档"));
        // trace 文件跟着进包
        let entries = assemble_material(crate::index::Meta::default(), body, &it);
        let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(names.contains(&"trace/hires.json"), "{names:?}");
        let t = entries
            .iter()
            .find(|e| e.path == "trace/hires.json")
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&t.bytes).unwrap();
        assert_eq!(v["summary"]["replaced"], 1);
        assert_eq!(v["shots"][0]["hires"]["width"], 1440);
        assert!(v["shots"][1]["hires"].is_null());
    }

    #[test]
    fn 没配高清服务时不写溯源节_图位注明是转存图() {
        let it = item(vec![shot(0, Some(desc(ImageKind::Product, true)))]);
        let body = material_body(&it, &pick_figures(&it.shots, 3));
        assert!(!body.contains("高清原图溯源"));
        assert!(body.contains("来源库转存，640px 档"));
        assert!(hires_trace(&it).is_none());
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
