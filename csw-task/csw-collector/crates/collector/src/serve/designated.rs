//! 01 指定帖模式：主编重开 01、派工单里贴了**原轮次里没有**的 Instagram 帖子链接时走这里。
//!
//! 只读这几条帖子的原文和原图，打一个限定包直接交，不跑全池十步、不重登记原轮次、
//! 不拿全池 intake-check 拦它（主编在派工单里明确豁免了全池门禁）。
//!
//! 为什么要单独一条路（10-06 r63 #699）：Van 在 03 之后追加三条链接，主编连续四次重开 01，
//! 工作台每次都在原 144 条上返工、再被全池自查拦下，三条链接一次都没读过；而且三条都不在
//! csw 来源库里（404），van-links 采集器也取不到。这里原文、账号、发布时间、原图都走
//! hires-service（它直接问 Instagram），不依赖来源库。
//!
//! 不做的事：不给档位、不跑六维判断——资格核查由主编做（派工单原话「不重复再做全池价值筛选」）。

use std::collections::HashSet;

use anyhow::{Context, Result};
use csw_collector_core::Config;
use csw_collector_core::rounds::Round;
use csw_collector_deliver::pack::{self, Entry};
use csw_collector_engineapi::client::EngineClient;
use csw_collector_engineapi::types::{DeliverableKind, MyTask, SubmitInput, TaskDetail};
use csw_collector_harvest::hires::HiresResult;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::hires_match;
use super::services::Services;

/// 派工单里的 Instagram 帖子短码，按出现顺序去重。认 `/p/`、`/reel/`、`/tv/`。
pub fn shortcodes_in(text: &str) -> Vec<String> {
    let re = regex::Regex::new(
        r"instagram\.com/(?:[A-Za-z0-9_.]+/)?(?:p|reel|tv)/([A-Za-z0-9_-]{5,20})",
    )
    .expect("静态正则");
    let mut seen = HashSet::new();
    re.captures_iter(text)
        .map(|c| c[1].to_string())
        .filter(|c| seen.insert(c.clone()))
        .collect()
}

/// 派工单是不是在点名要读某几条帖子。只有链接不够——退回意见里常引用别处的帖子作对照。
pub fn is_designation(note: &str) -> bool {
    ["指定", "追加", "Van"].iter().any(|w| note.contains(w))
}

/// 派工单要求重新采集的标记。只认这个标记：主编的措辞变化太大，按关键词猜会误判
///（10-07 r64：四张「只上传诊断包」的派工单里都出现过「采集」相关字样，主编并不要重采）。
pub const MARK_RECOLLECT: &str = "【重新采集】";
/// 派工单叫工作台让开、由人工直传的标记。
pub const MARK_HANDS_OFF: &str = "【工作台不接】";

/// 派工单要求重新采集（开新轮次，窗口 = 派工时刻往前 24 小时），不是在原轮次上改。
pub fn wants_recollect(note: &str) -> bool {
    note.contains(MARK_RECOLLECT)
}

/// 派工单叫工作台让开（主编要人工直传）。
pub fn hands_off_note(note: &str) -> bool {
    note.contains(MARK_HANDS_OFF)
}

/// 派工单里**原轮次没有**的短码。主编在退回意见里引用池内帖子的链接很常见，那些不算指定。
pub fn new_codes(conn: &Connection, round_id: Option<i64>, note: &str) -> Vec<String> {
    let codes = shortcodes_in(note);
    if codes.is_empty() {
        return codes;
    }
    let mut known: HashSet<String> = HashSet::new();
    if let Some(id) = round_id
        && let Ok(mut st) = conn.prepare(
            "SELECT c.url FROM round_candidates rc JOIN candidates c ON c.candidate_key = rc.candidate_key
             WHERE rc.round_id = ?1",
        )
        && let Ok(rows) = st.query_map([id], |r| r.get::<_, String>(0))
    {
        for u in rows.flatten() {
            known.extend(shortcodes_in(&u));
        }
    }
    codes.into_iter().filter(|c| !known.contains(c)).collect()
}

/// 条目键：与 `csw::candidate_key` 同一套（账号字母数字小写 + 链接 sha256 前 6 位）。
pub fn item_key(username: &str, shortcode: &str) -> String {
    csw_collector_harvest::csw::candidate_key(username, &post_url(shortcode), shortcode)
}

fn post_url(code: &str) -> String {
    format!("https://www.instagram.com/p/{code}/")
}

/// 一张图落进包里之后的样子。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Pic {
    pub order: usize,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub sha256: String,
    pub oss_url: String,
    pub file_key: String,
    /// 原字节，进包用；不写进 trace
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// 一条指定帖读到的东西。`error` 非空表示这条没读成，其余字段可能为空。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Post {
    pub shortcode: String,
    pub key: String,
    pub username: String,
    pub full_name: String,
    pub caption: String,
    /// RFC3339；Instagram 没给时空
    pub posted_at: String,
    pub in_window: Option<bool>,
    pub pics: Vec<Pic>,
    pub problems: Vec<String>,
    pub dup: Vec<String>,
    pub error: String,
}

/// 首页。纯函数，便于测试；时间用派工时间，保证同样的输入打出同样的包。
pub fn render_index(task: &MyTask, version: i64, window: (&str, &str), posts: &[Post]) -> String {
    let ok: Vec<&Post> = posts.iter().filter(|p| p.error.is_empty()).collect();
    let pics: usize = ok.iter().map(|p| p.pics.len()).sum();
    let old = ok.iter().filter(|p| p.in_window == Some(false)).count();
    let mut selfcheck = vec![
        format!(
            "本版只交派工单指定的 {} 条帖子（原轮次里没有的链接），不是全池；原轮次与已批条目不动、未重登记",
            posts.len()
        ),
        format!(
            "读成 {} 条、没读成 {} 条；读成的原文全文、账号、发布时间取自 Instagram（hires-service），完整轮播 {} 张原帖最大档，逐张实测尺寸、字节、SHA256",
            ok.len(),
            posts.len() - ok.len(),
            pics
        ),
        format!("早于本期窗口的 {old} 条已标旧披露，不按当天新品写"),
        "查重只查工作台知识库里的已发、范例、生成稿与决定（按账号名），结果逐条列出".into(),
        "不给档位、不做六维判断，资格核查由主编做；全池 intake-check 不适用于限定交付，未登记新的采集轮".into(),
    ];
    if posts.iter().any(|p| !p.error.is_empty()) {
        selfcheck.push("没读成的帖子逐条写了原因，其余照交".into());
    }
    let mut s = vec![
        "---".to_string(),
        format!(
            "任务: \"主编 · r{} 任务#{} 派工单\"",
            task.task.run_id, task.task.id
        ),
        "类型: \"产出\"".into(),
        "交付 Agent: \"情报收集员\"".into(),
        "阶段: \"01-情报逐条\"".into(),
        format!("版本: \"v{version}\""),
        format!("时间: \"{}\"", task.task.dispatched_at),
        "状态: \"待审\"".into(),
        "上游来源:".into(),
        format!(
            "  - \"主编 · r{} 任务#{} 派工单（指定帖限定读取）\"",
            task.task.run_id, task.task.id
        ),
        "自检:".into(),
    ];
    s.extend(
        selfcheck
            .iter()
            .map(|l| format!("  - \"{}\"", l.replace('"', "'"))),
    );
    s.push("---".into());
    s.push(String::new());
    s.push("# 指定帖 · 原文与原图".into());
    s.push(String::new());
    s.push(format!(
        "本期窗口：{} ~ {}（左闭右开）。条目来源一律 `van_link`。",
        window.0, window.1
    ));
    s.push(String::new());
    s.push("| 序 | 条目键 | 账号 | 发布时间（UTC） | 本期窗口 | 图数 |".into());
    s.push("|---|---|---|---|---|---|".into());
    for (i, p) in posts.iter().enumerate() {
        if p.error.is_empty() {
            let win = match p.in_window {
                Some(true) => "窗口内",
                Some(false) => "旧披露",
                None => "时间未知",
            };
            s.push(format!(
                "| {} | `{}` | @{} | {} | {win} | {} |",
                i + 1,
                p.key,
                p.username,
                if p.posted_at.is_empty() {
                    "未给"
                } else {
                    &p.posted_at
                },
                p.pics.len()
            ));
        } else {
            s.push(format!("| {} | — | — | — | 没读成 | 0 |", i + 1));
        }
    }
    s.push(String::new());
    for (i, p) in posts.iter().enumerate() {
        s.push(format!("## {}｜{}", i + 1, post_url(&p.shortcode)));
        s.push(String::new());
        if !p.error.is_empty() {
            s.push(format!("没读成：{}", p.error));
            s.push(String::new());
            continue;
        }
        s.push(format!("- 条目 `{}` ｜ origin=van_link", p.key));
        s.push(format!(
            "- 账号：@{}{}",
            p.username,
            if p.full_name.is_empty() {
                String::new()
            } else {
                format!("（{}）", p.full_name)
            }
        ));
        s.push(format!(
            "- 发布时间：{}",
            match (p.posted_at.is_empty(), p.in_window) {
                (true, _) => "Instagram 没给，待核".to_string(),
                (false, Some(false)) => format!("{}（早于本期窗口，旧披露）", p.posted_at),
                (false, _) => format!("{}（本期窗口内）", p.posted_at),
            }
        ));
        if let Some(first) = p.pics.first() {
            s.push(String::new());
            s.push("代表预览（第 1 张）：".into());
            s.push(String::new());
            s.push(format!("![{}]({})", p.key, first.file));
        }
        s.push(String::new());
        s.push("原文全文：".into());
        s.push(String::new());
        if p.caption.trim().is_empty() {
            s.push("> （Instagram 没给原文）".into());
        } else {
            for l in p.caption.lines() {
                s.push(if l.is_empty() {
                    ">".into()
                } else {
                    format!("> {l}")
                });
            }
        }
        s.push(String::new());
        s.push("图片原文件与图序映射（Instagram 原帖最大档，未截图、未插值、未缩放）：".into());
        s.push(String::new());
        s.push("| 原帖图序 | 包内文件 | 尺寸 | 字节 | SHA256 前 16 位 |".into());
        s.push("|---|---|---|---|---|".into());
        for x in &p.pics {
            s.push(format!(
                "| {} | [{}]({}) | {}×{} | {} | `{}` |",
                x.order,
                x.file.rsplit('/').next().unwrap_or(&x.file),
                x.file,
                x.width,
                x.height,
                x.bytes,
                &x.sha256[..16.min(x.sha256.len())]
            ));
        }
        s.push(String::new());
        if p.dup.is_empty() {
            s.push(format!(
                "- 查重：知识库里没有 @{} 的已发、范例、生成稿或决定",
                p.username
            ));
        } else {
            s.push(format!(
                "- 查重：知识库里与 @{} 相关的（请主编判断是否同一事件）：",
                p.username
            ));
            s.extend(p.dup.iter().map(|d| format!("  - {d}")));
        }
        s.push("- 未核：一手来源（本帖是否官方首发）、价格与发售信息、与本号口径的关联，待主编资格核查".into());
        if !p.problems.is_empty() {
            s.push("- 取图问题：".into());
            s.extend(p.problems.iter().map(|d| format!("  - {d}")));
        }
        s.push(String::new());
    }
    s.join("\n")
}

/// 发布时间（unix 秒）→ RFC3339 与是否在窗口内（左闭右开）。
fn when(taken_at: i64, window: (&str, &str)) -> (String, Option<bool>) {
    if taken_at <= 0 {
        return (String::new(), None);
    }
    let Ok(ts) = jiff::Timestamp::from_second(taken_at) else {
        return (String::new(), None);
    };
    let inside = match (
        window.0.parse::<jiff::Timestamp>(),
        window.1.parse::<jiff::Timestamp>(),
    ) {
        (Ok(a), Ok(b)) => Some(ts >= a && ts < b),
        _ => None,
    };
    (ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(), inside)
}

/// 知识库里按账号名找相关的已发、范例、生成稿与决定，最多 5 条。
fn dup_hits(conn: &Connection, username: &str, full_name: &str) -> Vec<String> {
    let mut words: Vec<String> = vec![username.trim_matches('_').replace("__", "_")];
    let core: String = username
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if core.len() >= 4 {
        words.push(core);
    }
    if full_name.chars().count() >= 3 {
        words.push(full_name.to_string());
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for w in words.iter().filter(|w| w.len() >= 3) {
        let like = format!("%{w}%");
        let Ok(mut st) = conn.prepare(
            "SELECT kind, title, IFNULL(published_at,''), url FROM kb_docs
             WHERE title LIKE ?1 OR brand LIKE ?1 OR body LIKE ?1 OR url LIKE ?1
             ORDER BY IFNULL(published_at,'') DESC LIMIT 5",
        ) else {
            continue;
        };
        let rows = st.query_map([&like], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        });
        if let Ok(rows) = rows {
            for (kind, title, at, url) in rows.flatten() {
                if seen.insert((kind.clone(), title.clone())) && out.len() < 5 {
                    out.push(format!(
                        "{kind}｜{}｜{title} {url}",
                        &at[..10.min(at.len())]
                    ));
                }
            }
        }
    }
    out
}

/// 读一条：hires-service 取原文与原图（OSS 转存），原图下载器下原字节。
async fn read_one(
    conn: &Connection,
    cfg: &Config,
    svc: &Services,
    code: &str,
    window: (&str, &str),
) -> Post {
    let mut p = Post {
        shortcode: code.to_string(),
        ..Default::default()
    };
    let Some(hires) = svc.hires.as_ref() else {
        p.error = "工作台没有配置 hires-service，取不了来源库以外的帖子".into();
        return p;
    };
    let res: HiresResult = match hires.media(code).await {
        Ok(r) => r,
        Err(e) => {
            p.error = format!("hires-service 取帖失败：{e}");
            return p;
        }
    };
    if res.post.username.is_empty() {
        p.error =
            "hires-service 没返回帖子账号（服务版本太旧或 Instagram 没给），不能确定条目键".into();
        return p;
    }
    p.username = res.post.username.clone();
    p.full_name = res.post.full_name.clone();
    p.caption = res.post.caption.clone();
    (p.posted_at, p.in_window) = when(res.post.taken_at, window);
    p.key = item_key(&p.username, code);

    let photos = res.photos();
    let urls: Vec<String> = photos
        .iter()
        .filter_map(|x| x.oss.as_ref().map(|o| o.url.clone()))
        .collect();
    let dl = match super::item::original_downloader(cfg) {
        Ok(d) => d,
        Err(e) => {
            p.error = format!("建原图下载器失败：{e:#}");
            return p;
        }
    };
    let report = dl.fetch_all(&urls).await;
    let by_url = report.by_url();
    for (i, x) in photos.iter().enumerate() {
        let order = x.index + 1;
        let Some(oss) = x.oss.as_ref() else {
            p.problems.push(format!(
                "第 {order} 张：hires-service 没转存：{}",
                x.error.clone().unwrap_or_else(|| "未说明".into())
            ));
            continue;
        };
        let Some(d) = by_url.get(oss.url.as_str()) else {
            p.problems.push(format!("第 {order} 张：下载转存原图失败"));
            continue;
        };
        let Ok(bytes) = std::fs::read(&d.path) else {
            p.problems.push(format!("第 {order} 张：读盘失败"));
            continue;
        };
        let (w, h) = hires_match::dimensions(&bytes).unwrap_or((x.width, x.height));
        let ext = match oss.mime_type.as_str() {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        };
        p.pics.push(Pic {
            order,
            file: format!("images/{}_{:02}.{ext}", p.key, i + 1),
            width: w,
            height: h,
            bytes: bytes.len() as u64,
            sha256: Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            oss_url: oss.url.clone(),
            file_key: x.file_key.clone(),
            data: bytes,
        });
    }
    if p.pics.is_empty() {
        p.error = format!(
            "一张原图都没取到：{}",
            if p.problems.is_empty() {
                "帖子没有图片".to_string()
            } else {
                p.problems.join("；")
            }
        );
    }
    p.dup = dup_hits(conn, &p.username, &p.full_name);
    p
}

/// 指定帖模式的整条路：读 → 打包 → 交。全部没读成就报失败，让任务离开队列。
#[allow(clippy::too_many_arguments)]
pub async fn deliver(
    conn: &Connection,
    engine: &EngineClient,
    cfg: &Config,
    svc: &Services,
    t: &MyTask,
    _detail: &TaskDetail,
    prev: Option<&Round>,
    codes: &[String],
    version: i64,
) -> Result<()> {
    let window = prev
        .map(|r| (r.window_start.as_str(), r.window_end.as_str()))
        .unwrap_or(("", ""));
    tracing::info!(
        任务 = t.task.id,
        条数 = codes.len(),
        ?codes,
        "指定帖模式：只读派工单里的新链接"
    );
    let mut posts = Vec::new();
    for c in codes {
        let p = read_one(conn, cfg, svc, c, window).await;
        if !p.error.is_empty() {
            tracing::warn!(任务 = t.task.id, 短码 = %c, 原因 = %p.error, "指定帖没读成");
        }
        posts.push(p);
    }
    if posts.iter().all(|p| !p.error.is_empty()) {
        let why = format!(
            "指定帖一条都没读成：{}",
            posts
                .iter()
                .map(|p| format!("{}：{}", p.shortcode, p.error))
                .collect::<Vec<_>>()
                .join("；")
        );
        let idem = format!(
            "fail-{}-designated-{}",
            t.task.id,
            t.task.dispatched_at.replace([':', '-', '.'], "")
        );
        engine
            .fail(t.task.id, &why, &idem)
            .await
            .context("报告指定帖失败")?;
        return Ok(());
    }

    let root = format!("情报逐条_情报收集员_r{}_v{version}", t.task.run_id);
    let mut entries = vec![Entry::text(
        "index.md",
        render_index(t, version, window, &posts),
    )];
    for p in &posts {
        for x in &p.pics {
            entries.push(Entry::binary(&x.file, x.data.clone()));
        }
    }
    entries.push(Entry::text(
        "trace/designated.json",
        serde_json::to_string_pretty(&posts)?,
    ));
    let out = cfg
        .data_dir
        .join("deliverables")
        .join(format!("{root}.zip"));
    let built = pack::build(&root, &entries, &out)?;
    let input = SubmitInput {
        task_id: t.task.id,
        kind: DeliverableKind::Output,
        zip_path: built.path.clone(),
        file_name: format!("{root}.zip"),
        idem_key: built.idem_key(t.task.id),
        note: format!(
            "指定帖限定交付：{} 条读成、{} 条没读成；原文原图取自 Instagram（hires-service），不给档位，资格核查由主编做",
            posts.iter().filter(|p| p.error.is_empty()).count(),
            posts.iter().filter(|p| !p.error.is_empty()).count()
        ),
        affects_deliverable_id: None,
        item_key: String::new(),
    };
    engine
        .submit(&input)
        .await
        .map_err(|e| anyhow::anyhow!("指定帖包提交失败：{e:#}"))?;
    tracing::info!(任务 = t.task.id, 包 = %built.path.display(), 字节 = built.bytes, "指定帖包已交");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 认得出派工单里的帖子链接() {
        let note = "1. https://www.instagram.com/p/DeHbYoHDFoP/?img_index=3&stkn=xx\n\
                    2. https://instagram.com/reel/Dd6E--IGN5F/ 3. instagram.com/ta__goto/p/Dd_qqt8iZw0/\n\
                    重复 https://www.instagram.com/p/DeHbYoHDFoP/";
        assert_eq!(
            shortcodes_in(note),
            vec!["DeHbYoHDFoP", "Dd6E--IGN5F", "Dd_qqt8iZw0"]
        );
        assert!(shortcodes_in("没有链接，只有 https://example.com/p/abc").is_empty());
    }

    #[test]
    fn 只有点名才算指定() {
        assert!(is_designation(
            "Van 三条指定原帖：https://www.instagram.com/p/AAAAA1/"
        ));
        assert!(!is_designation(
            "卡5 对照 https://www.instagram.com/p/AAAAA1/ 的写法"
        ));
    }

    #[test]
    fn 只认明确标记() {
        assert!(wants_recollect(
            "【重新采集】按滚动 24 小时取有热度的新资讯"
        ));
        assert!(!wants_recollect(
            "Van全部退回，重采滚动24小时有可核热度的新资讯"
        ));
        assert!(hands_off_note("【工作台不接】运营方直传 v5"));
        assert!(!hands_off_note("不跑旧工作台、累计未审不改0"));
    }

    #[test]
    fn 条目键与来源库同一套() {
        // 与 r63 手工包、工作台既有条目同一算法
        assert_eq!(
            item_key("highsnobietysneakers", "DeHbYoHDFoP"),
            "highsnobietysneakers-2ed563"
        );
        assert_eq!(item_key("ta__goto", "Dd_qqt8iZw0"), "tagoto-d735c7");
    }

    #[test]
    fn 窗口内外与没给时间() {
        let w = ("2026-10-04T23:00:00Z", "2026-10-05T23:00:00Z");
        assert_eq!(
            when(1791210630, w),
            ("2026-10-05T14:30:30Z".into(), Some(true))
        );
        assert_eq!(when(1790762677, w).1, Some(false));
        assert_eq!(when(0, w), (String::new(), None));
    }

    #[test]
    fn 原轮次里有的链接不算指定() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE candidates(candidate_key TEXT PRIMARY KEY, url TEXT);
             CREATE TABLE round_candidates(round_id INTEGER, candidate_key TEXT);
             INSERT INTO candidates VALUES('a-1','https://www.instagram.com/p/OLDCODE11/');
             INSERT INTO round_candidates VALUES(7,'a-1');",
        )
        .unwrap();
        let note = "卡5 https://www.instagram.com/p/OLDCODE11/ 另加 https://www.instagram.com/p/NEWCODE22/";
        assert_eq!(new_codes(&conn, Some(7), note), vec!["NEWCODE22"]);
        assert!(
            new_codes(
                &conn,
                Some(7),
                "只引用 https://www.instagram.com/p/OLDCODE11/"
            )
            .is_empty()
        );
    }

    fn task() -> MyTask {
        serde_json::from_value(serde_json::json!({
            "task": {"id": 699, "run_id": 63, "stage_code": "intake", "dispatched_at": "2026-10-06T08:14:03Z",
                     "cur_version": 3, "status": "dispatched"}
        }))
        .unwrap()
    }

    #[test]
    fn 首页如实写读成与没读成() {
        let ok = Post {
            shortcode: "DeHbYoHDFoP".into(),
            key: "highsnobietysneakers-2ed563".into(),
            username: "highsnobietysneakers".into(),
            caption: "Yes, that’s a tennis ball.\n\nSS27".into(),
            posted_at: "2026-10-05T14:30:30Z".into(),
            in_window: Some(true),
            pics: vec![Pic {
                order: 1,
                file: "images/highsnobietysneakers-2ed563_01.jpg".into(),
                width: 1080,
                height: 1350,
                bytes: 250292,
                sha256: "a06b5630ddbacfc4ffff".into(),
                oss_url: "https://x/1.jpg".into(),
                file_key: "k".into(),
                data: Vec::new(),
            }],
            ..Default::default()
        };
        let bad = Post {
            shortcode: "NEWCODE22".into(),
            error: "hires-service 取帖失败：not found".into(),
            ..Default::default()
        };
        let md = render_index(
            &task(),
            4,
            ("2026-10-04T23:00:00Z", "2026-10-05T23:00:00Z"),
            &[ok, bad],
        );
        assert!(md.contains("版本: \"v4\""));
        assert!(md.contains("读成 1 条、没读成 1 条"));
        assert!(md.contains("| 1 | `highsnobietysneakers-2ed563` | @highsnobietysneakers | 2026-10-05T14:30:30Z | 窗口内 | 1 |"));
        assert!(md.contains("> Yes, that’s a tennis ball.\n>\n> SS27"));
        assert!(md.contains("1080×1350 | 250292 | `a06b5630ddbacfc4`"));
        assert!(md.contains("没读成：hires-service 取帖失败"));
        assert!(md.contains("不给档位"));
    }
}
