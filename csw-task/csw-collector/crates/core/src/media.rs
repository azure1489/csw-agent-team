//! 采集产物落库：`media` 与 `media_descriptions`。
//!
//! # 不落库，预取轮就白跑
//!
//! 识别一张图要走一趟模型网关，整一轮下来是四十分钟里最大的一块。
//! 「01:40 先识别一遍、05:30 直接拿来用」这件事成立的前提就是**描述在库里**；
//! 只留在内存里的话，预取轮只是把同一笔钱花了两遍。
//!
//! # 描述按（图, 提示词版本）唯一
//!
//! 换了识别提示词就要重算：描述变了，判断的输入跟着变。库上的
//! `UNIQUE(blake3, prompt_version)` 是这条口径的落脚点，不靠代码自觉。
//!
//! # 两处刻意的取舍
//!
//! - **只写下载成功的图**（有 blake3 的）。下载失败的没有内容哈希，
//!   而 `media.blake3` 是主键。少写的那几行不会把账弄错：完整性判据是
//!   「候选自己有几张图」对「库里有几条描述」，缺了就不算命中缓存。
//! - **同一张图被两条候选共用时，只算第一条的**（主键与 `UNIQUE` 都会挡下第二次）。
//!   第二条因此永远命中不了缓存，每轮重识别一遍。这种转载实测极少，
//!   用一个可能错的描述去省那一次调用不划算。

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::types::{MediaDescription, MediaKind, MediaRef};

/// 把一条候选这一轮的媒体与描述写进库。返回（写了几行图, 写了几条描述）。
///
/// 已经有的原样留着——描述是幂等的，重写一遍只会把同样的内容再写一次。
pub fn put_prepared(
    conn: &Connection,
    candidate_key: &str,
    media: &[MediaRef],
    descriptions: &[MediaDescription],
) -> Result<(usize, usize)> {
    let mut rows = 0;
    for m in media {
        let Some(b3) = m.blake3.as_deref() else {
            // 没下下来的没有内容哈希，写不进主键——完整性靠候选自己的图数对账
            continue;
        };
        let described = descriptions.iter().any(|d| d.blake3 == b3);
        rows += conn
            .execute(
                "INSERT INTO media(blake3, candidate_key, ordinal, kind, source_hash, url, failed)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT DO NOTHING",
                params![
                    b3,
                    candidate_key,
                    i64::from(m.ordinal),
                    kind_str(m.kind),
                    m.source_hash,
                    m.url,
                    i64::from(m.kind == MediaKind::Photo && !described),
                ],
            )
            .context("写媒体")?;
    }

    let now = jiff::Timestamp::now().to_string();
    let mut descs = 0;
    for d in descriptions {
        descs += conn
            .execute(
                "INSERT INTO media_descriptions(blake3, candidate_key, matches_text, content,
                                                missing_from_text, kind, usable_as_figure,
                                                model, prompt_version, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT DO NOTHING",
                params![
                    d.blake3,
                    candidate_key,
                    d.matches_text,
                    d.content,
                    d.missing_from_text,
                    serde_json::to_value(d.kind)?.as_str().unwrap_or("product"),
                    i64::from(d.usable_as_figure),
                    d.model,
                    d.prompt_version,
                    now,
                ],
            )
            .context("写图片描述")?;
    }
    Ok((rows, descs))
}

/// 这条候选在这个提示词版本下已经识别过的描述，按图序排。
///
/// 按 `candidate_key` 取而不是按 blake3 逐张取：判断那一步要的是「这条候选的
/// 全部描述」，一张一张查会把 N 条候选变成 N×图数 次查询。
pub fn descriptions_for(
    conn: &Connection,
    candidate_key: &str,
    prompt_version: &str,
) -> Result<Vec<MediaDescription>> {
    let mut st = conn.prepare(
        "SELECT d.blake3, m.ordinal, d.matches_text, d.content, d.missing_from_text,
                d.kind, d.usable_as_figure, d.model, d.prompt_version
         FROM media_descriptions d
         JOIN media m ON m.blake3 = d.blake3
         WHERE d.candidate_key = ?1 AND d.prompt_version = ?2
         ORDER BY m.ordinal",
    )?;
    let rows = st.query_map(params![candidate_key, prompt_version], |r| {
        Ok(MediaDescription {
            blake3: r.get(0)?,
            ordinal: r.get::<_, i64>(1)? as u16,
            matches_text: r.get(2)?,
            content: r.get(3)?,
            missing_from_text: r.get(4)?,
            // 库里的值是我们自己按 serde 写的，认不出来只可能是被手改过
            kind: serde_json::from_value(serde_json::Value::String(r.get(5)?))
                .unwrap_or(crate::types::ImageKind::Unrelated),
            usable_as_figure: r.get::<_, i64>(6)? != 0,
            model: r.get(7)?,
            prompt_version: r.get(8)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 这条候选落过库的图片清单，按图序。返工时从库里还原候选要用它——
/// `ledger::get_candidate` 只还原元数据，不带图。
pub fn media_refs(conn: &Connection, candidate_key: &str) -> Result<Vec<MediaRef>> {
    let mut st = conn.prepare(
        "SELECT source_hash, kind, url, blake3, ordinal FROM media
         WHERE candidate_key = ?1 ORDER BY ordinal",
    )?;
    let rows = st.query_map(params![candidate_key], |r| {
        let kind: String = r.get(1)?;
        Ok(MediaRef {
            source_hash: r.get(0)?,
            kind: if kind == "video" {
                MediaKind::Video
            } else {
                MediaKind::Photo
            },
            url: r.get(2)?,
            blake3: Some(r.get(3)?),
            ordinal: r.get::<_, i64>(4)? as u16,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn kind_str(k: MediaKind) -> &'static str {
    match k {
        MediaKind::Photo => "photo",
        MediaKind::Video => "video",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Candidate, ImageKind, Platform};

    fn cand(key: &str) -> Candidate {
        Candidate {
            candidate_key: key.into(),
            platform: Platform::Instagram,
            source_id: key.into(),
            collector: "csw_window".into(),
            account: "acc".into(),
            url: format!("https://x/{key}"),
            text: "正文".into(),
            translated: String::new(),
            posted_at: None,
            ingested_at: None,
            likes: None,
            comments: None,
            followers: None,
            heat_ratio: None,
            content_type: "Carousel".into(),
            media: vec![],
            tags: vec![],
            hashtags: vec![],
        }
    }

    fn media(ordinal: u16, b3: Option<&str>) -> MediaRef {
        MediaRef {
            source_hash: format!("s{ordinal}"),
            kind: MediaKind::Photo,
            url: format!("https://img/{ordinal}"),
            blake3: b3.map(str::to_string),
            ordinal,
        }
    }

    fn desc(b3: &str, ordinal: u16, version: &str) -> MediaDescription {
        MediaDescription {
            blake3: b3.into(),
            ordinal,
            matches_text: "正文未提及".into(),
            content: format!("第 {ordinal} 张：灰色背包"),
            missing_from_text: String::new(),
            kind: ImageKind::Product,
            usable_as_figure: true,
            model: "m".into(),
            prompt_version: version.into(),
        }
    }

    fn setup() -> Connection {
        let c = crate::store::open_in_memory().unwrap();
        crate::ledger::upsert_candidate(&c, &cand("k1")).unwrap();
        c
    }

    #[test]
    fn 描述存进去还能原样取回来() {
        let c = setup();
        let ms = [media(0, Some("b0")), media(1, Some("b1"))];
        let ds = [desc("b0", 0, "recognize/v1"), desc("b1", 1, "recognize/v1")];
        assert_eq!(put_prepared(&c, "k1", &ms, &ds).unwrap(), (2, 2));

        let got = descriptions_for(&c, "k1", "recognize/v1").unwrap();
        assert_eq!(got.len(), 2);
        // 按图序回来：描述里的「第几张图」是判断的依据，顺序错了依据就指错人
        assert_eq!(got[0].blake3, "b0");
        assert_eq!(got[1].blake3, "b1");
        assert_eq!(got[0].content, "第 0 张：灰色背包");
        assert_eq!(got[0].kind, ImageKind::Product);
        assert!(got[0].usable_as_figure);
    }

    #[test]
    fn 换了提示词版本就不算识别过() {
        let c = setup();
        let ms = [media(0, Some("b0"))];
        put_prepared(&c, "k1", &ms, &[desc("b0", 0, "recognize/v1")]).unwrap();
        // 描述变了，判断的输入跟着变——旧的不能冒充新的
        assert!(
            descriptions_for(&c, "k1", "recognize/v2")
                .unwrap()
                .is_empty()
        );
        assert_eq!(descriptions_for(&c, "k1", "recognize/v1").unwrap().len(), 1);
    }

    #[test]
    fn 没下下来的图不写但也不报错() {
        let c = setup();
        let ms = [media(0, Some("b0")), media(1, None)];
        let (rows, _) = put_prepared(&c, "k1", &ms, &[desc("b0", 0, "recognize/v1")]).unwrap();
        assert_eq!(rows, 1);
        // 完整性靠「候选有几张图」对「库里有几条描述」，这里少一条正是要的信号
        assert_eq!(descriptions_for(&c, "k1", "recognize/v1").unwrap().len(), 1);
    }

    #[test]
    fn 识别失败的图记成failed() {
        let c = setup();
        let ms = [media(0, Some("b0")), media(1, Some("b1"))];
        // 只有第一张识别出来了
        put_prepared(&c, "k1", &ms, &[desc("b0", 0, "recognize/v1")]).unwrap();
        let failed: i64 = c
            .query_row("SELECT failed FROM media WHERE blake3='b1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(failed, 1);
        let ok: i64 = c
            .query_row("SELECT failed FROM media WHERE blake3='b0'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ok, 0);
    }

    #[test]
    fn 写两遍不翻倍也不报错() {
        let c = setup();
        let ms = [media(0, Some("b0"))];
        let ds = [desc("b0", 0, "recognize/v1")];
        put_prepared(&c, "k1", &ms, &ds).unwrap();
        // 续跑会把同一条候选再走一遍，写第二次必须无害
        assert_eq!(put_prepared(&c, "k1", &ms, &ds).unwrap(), (0, 0));
        assert_eq!(descriptions_for(&c, "k1", "recognize/v1").unwrap().len(), 1);
    }

    #[test]
    fn 崩在采集中间时已识别的留着没做完的重来() {
        let c = crate::store::open_in_memory().unwrap();
        let round = crate::rounds::open_round(
            &c,
            &crate::rounds::NewRound {
                kind: crate::types::RoundKind::Task,
                trigger: crate::types::RoundTrigger::Dispatch,
                run_id: Some(48),
                task_id: Some(1),
                stage_code: Some("intake".into()),
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap()
        .0
        .id;

        // 崩之前：第一条两张图都识别完了，第二条只做了一半
        let step = crate::rounds::begin_step(&c, round, crate::types::StepCode::Harvest, "hash-1")
            .unwrap();
        for (key, done) in [("aw-1", 2usize), ("ym-2", 1usize)] {
            let mut cc = cand(key);
            cc.media = (0..2)
                .map(|i| media(i, Some(&format!("{key}-b{i}"))))
                .collect();
            crate::ledger::upsert_candidate(&c, &cc).unwrap();
            let ds: Vec<_> = (0..done)
                .map(|i| desc(&format!("{key}-b{i}"), i as u16, "recognize/v1"))
                .collect();
            put_prepared(&c, key, &cc.media, &ds).unwrap();
        }
        assert_eq!(step.status, "running", "这时候进程被砍掉");

        // 重启
        assert_eq!(crate::rounds::recover_interrupted(&c).unwrap(), 1);
        assert_eq!(
            crate::rounds::latest(&c, round, crate::types::StepCode::Harvest)
                .unwrap()
                .unwrap()
                .status,
            "interrupted",
            "标 interrupted 而不是 failed：它多半做了一半"
        );

        // 续跑：第一条命中缓存，第二条只有一半
        assert_eq!(
            descriptions_for(&c, "aw-1", "recognize/v1").unwrap().len(),
            2
        );
        assert_eq!(
            descriptions_for(&c, "ym-2", "recognize/v1").unwrap().len(),
            1,
            "只做了一半的那条，命中判据是「每张图都有描述」，所以它会整条重识别"
        );
        // 再调一次恢复没活干：不该把已经收过的步又标一遍
        assert_eq!(crate::rounds::recover_interrupted(&c).unwrap(), 0);
    }

    #[test]
    fn 没识别过的候选给空不给半截() {
        let c = setup();
        assert!(
            descriptions_for(&c, "从没见过", "recognize/v1")
                .unwrap()
                .is_empty()
        );
    }
}
