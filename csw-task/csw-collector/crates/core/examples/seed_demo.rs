//! 往本地库里造一轮样例数据，**专为走查十一页的样式**。
//!
//! 空库上走查等于没查：每页都是「还没有数据」，看不出表格对不对齐、
//! 四档的配色分不分得开、长正文会不会撑破卡片。
//!
//! ```bash
//! cargo run -p csw-collector-core --example seed_demo -- /tmp/cdata/collector.db
//! ```
//!
//! 造的东西覆盖各页要看的东西：
//!
//! | 页 | 靠什么撑起来 |
//! |---|---|
//! | 今日总览 | 一轮派单轮 + 十步 + 四档计数 + 一条排队的活 |
//! | 判断台账 | 12 条候选，四档都有、六维齐、三句话、对照结论 |
//! | 事件详情 | 每条都有正文、图、描述、缺口、热度 |
//! | Van 模式 | 推荐与备选 + 三种勾选 |
//! | 采集与覆盖 | 两个采集器、一条失败的 |
//! | 待核结转 | 两条待核（一条是没读到实图） |
//! | 指标 | 原判与生效档不一致的两条（捞回 / 压下） |
//! | 运行与设置 | 留痕若干条 |
//!
//! **不造知识库那几页**：它们要真的向量库与检索客户端，造不出来，
//! 页面会如实回 503——那本身也是要看的一种状态。

use anyhow::Result;
use csw_collector_core::types::*;
use csw_collector_core::{ledger, media, rounds, store, workbench};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("用法：seed_demo <collector.db 的路径>");
        std::process::exit(2);
    });
    let conn = store::open(std::path::Path::new(&path))?;

    let (round, _) = rounds::open_round(
        &conn,
        &rounds::NewRound {
            kind: RoundKind::Task,
            trigger: RoundTrigger::Dispatch,
            run_id: Some(48),
            task_id: Some(311),
            stage_code: Some("intake".into()),
            target_version: 1,
            parent_round_id: None,
            window_start: "2026-09-21T21:30:00Z".into(),
            window_end: "2026-09-22T21:30:00Z".into(),
            plan_version: 1,
            rubric_version: "van-rubric/v1".into(),
            kb_snapshot: "qwen3-vl@8022".into(),
            instructions_hash: "demo".into(),
        },
    )?;
    println!("开了第 {} 轮", round.id);

    // 十步：前八步做完，深核 partial，提交还没到
    for (i, step) in StepCode::ALL.iter().enumerate() {
        let s = rounds::begin_step(&conn, round.id, *step, "")?;
        let (status, note) = match i {
            0..=4 => (StepStatus::Succeeded, ""),
            5 => (StepStatus::Partial, "首批 6 条做完，其余排队"),
            6..=7 => (StepStatus::Succeeded, ""),
            _ => continue, // 后两步留在 pending，总览上能看见「还没走到」
        };
        rounds::end_step(
            &conn,
            s.id,
            status,
            &serde_json::json!({"处理": 12, "跳过": 0}),
            note,
        )?;
    }

    // 十二条候选，四档铺开
    let rows: Vec<(&str, &str, Tier, bool, &str)> = vec![
        ("and_wander", "and_wander", Tier::Recommend, true, "秋冬新色上架，配色与去年同款有明确区别"),
        ("snowpeak", "snow_peak", Tier::Recommend, true, "折叠桌加了一个卡扣，收纳厚度少了两厘米"),
        ("goldwin", "goldwin", Tier::Recommend, true, "与建筑事务所联名，门店陈列同步更新"),
        ("nanga", "nanga_official", Tier::Recommend, true, "羽绒睡袋换了新的填充配比，标注温标下调"),
        ("helinox", "helinox", Tier::Alternate, true, "椅子出了新色，结构没变"),
        ("mystery", "mysteryranch", Tier::Alternate, true, "背包的肩带调节方式改了，细节图能看出来"),
        ("karrimor", "karrimor", Tier::Alternate, true, "复刻了九十年代的配色"),
        ("montbell", "montbell", Tier::NotRecommend, true, "只是门店活动预告，没有产品信息"),
        ("patagonia", "patagonia", Tier::NotRecommend, true, "转发了一条旧内容，无新事实"),
        ("chums", "chums", Tier::NotRecommend, true, "同一件事上周已经发过"),
        ("norrona", "norrona", Tier::PendingCheck, true, "提到发售日期但没写清是哪个市场"),
        ("yeti", "yeti", Tier::PendingCheck, false, "图全下失败，只有正文"),
    ];

    for (key, account, tier, image_seen, text) in &rows {
        let c = cand(key, account, text, *image_seen);
        ledger::upsert_candidate(&conn, &c)?;
        ledger::attach_candidate(&conn, round.id, key, "csw-window", false)?;
        if *image_seen {
            // 两张图，描述齐
            media::put_prepared(
                &conn,
                key,
                &c.media,
                &[desc(&format!("{key}-b0"), 0), desc(&format!("{key}-b1"), 1)],
            )?;
        } else {
            // 图在库里但没有描述 → 覆盖页上看得见「有图没识别」
            media::put_prepared(&conn, key, &c.media, &[])?;
        }
        ledger::put_judgement(
            &conn,
            round.id,
            &judgement(key, *tier, *image_seen, text),
            &[],
            "gpt-6-astra",
            "van-rubric/v1",
        )?;
    }

    // 主编改了两档：一条捞回、一条压下。指标页看的就是这两者的重合度
    workbench::put_override(&conn, round.id, "chums", Tier::Alternate, "这条其实是新配色，上周那条是另一个系列", "editor")?;
    workbench::put_override(&conn, round.id, "goldwin", Tier::Alternate, "联名信息还没官宣，先放备选", "editor")?;

    // Van 的勾选：三种都来一个
    workbench::put_van_mark(&conn, round.id, "and_wander", "like", "", "editor")?;
    workbench::put_van_mark(&conn, round.id, "snowpeak", "doubt", "", "editor")?;
    workbench::put_van_mark(&conn, round.id, "nanga", "note", "温标下调这点要在正文里说清楚", "editor")?;

    // 首批深核
    workbench::set_first_batch(&conn, round.id, &["and_wander".into(), "snowpeak".into(), "nanga".into()])?;

    // 一件**已经做完**的活 + 几条留痕。
    //
    // 这里曾经留的是一件「排队中」的活，为的是让总览页有东西看。
    // 后果是：服务一启动，主循环的 `run_queued` 就把它当真活取出来执行了——
    // 开了第二轮、去打了真的 csw 接口（假密钥被 401 拒绝）。
    // **样例数据不能留任何「待执行」的东西**：它会被真的执行。
    let work = workbench::enqueue(&conn, "manual_round", None, &serde_json::json!({"days": 2}), "editor")?;
    workbench::take_next(&conn)?; // 标 running
    workbench::finish_work(&conn, work, true, "第 1 轮：取到 12 条，判了 12（推荐 4 备选 3 待核 2）")?;
    workbench::audit(&conn, "editor", "first_batch", &format!("r{}", round.id), &serde_json::json!({"要的": 3, "标上的": 3}))?;
    workbench::audit(&conn, "van", "van_mark", &format!("r{}/and_wander", round.id), &serde_json::json!({"mark": "like"}))?;

    // 轮次收尾。不收的话总览页会一直显示「还在跑」，而且重启后的
    // `recover_interrupted` 会把它的步标成中断——样例数据不该看着像出了事
    rounds::finish_round(&conn, round.id, "done", "")?;

    println!(
        "造好了：{} 条候选、{} 条判断、2 条改档、3 个勾选、1 件做完的活",
        rows.len(),
        rows.len()
    );
    println!("起服务时把 CSW_COLLECTOR_DATA_DIR 指到 {} 的上级目录", path);
    Ok(())
}

fn cand(key: &str, account: &str, text: &str, with_photos: bool) -> Candidate {
    Candidate {
        candidate_key: key.into(),
        platform: Platform::Instagram,
        source_id: format!("C{key}01"),
        collector: "csw-window".into(),
        account: account.into(),
        url: format!("https://www.instagram.com/p/C{key}01/"),
        text: text.into(),
        translated: String::new(),
        posted_at: "2026-09-22T02:15:00Z".parse().ok(),
        ingested_at: "2026-09-22T03:40:00Z".parse().ok(),
        likes: Some(1284),
        comments: Some(37),
        followers: Some(286_000),
        heat_ratio: Some(2.4),
        content_type: "Carousel".into(),
        media: (0..2)
            .map(|i| MediaRef {
                source_hash: format!("{key}-s{i}"),
                kind: MediaKind::Photo,
                url: format!("https://oss.example/{key}-{i}.jpg"),
                blake3: with_photos.then(|| format!("{key}-b{i}")),
                ordinal: i,
            })
            .collect(),
        tags: vec!["outdoor".into()],
        hashtags: vec!["camp".into()],
    }
}

fn desc(hash: &str, ordinal: u16) -> MediaDescription {
    MediaDescription {
        blake3: hash.into(),
        ordinal,
        matches_text: "对应正文第一句说的新配色".into(),
        content: "深橄榄色的冲锋衣正面，左胸有织标，背景是白墙".into(),
        missing_from_text: "正文提到的内衬颜色，画面里看不到".into(),
        kind: if ordinal == 0 { ImageKind::Product } else { ImageKind::Detail },
        usable_as_figure: true,
        model: "gpt-6-astra".into(),
        prompt_version: "v1".into(),
    }
}

fn judgement(key: &str, tier: Tier, image_seen: bool, text: &str) -> Judgement {
    let v = match tier {
        Tier::Recommend => Verdict::Yes,
        Tier::Alternate => Verdict::Unclear,
        _ => Verdict::No,
    };
    Judgement {
        candidate_key: key.into(),
        tier,
        dims: Dim::ALL
            .iter()
            .map(|d| {
                (
                    *d,
                    DimJudgement {
                        verdict: v,
                        basis: format!("正文原话：「{}」", &text[..text.len().min(24)]),
                    },
                )
            })
            .collect(),
        three_sentences: ThreeSentences {
            what_changed: text.into(),
            why_it_matters: "读者出门前会关心的那一类改动".into(),
            how_different: "与上一代相比，差别在可量的那一处".into(),
        },
        unanswered: if tier == Tier::PendingCheck {
            Unanswered::MissingMaterial
        } else {
            Unanswered::None
        },
        comparison: Comparison {
            verdict: ComparisonVerdict::Unrelated,
            against: "近 30 天已发的同品牌条目".into(),
            note: "同品牌上次发的是另一个系列，角度不重合".into(),
        },
        heat_note: "点赞是该账号近 90 天中位的 2.4 倍".into(),
        look: if image_seen {
            "实图上能看清织标位置与面料纹理".into()
        } else {
            String::new()
        },
        image_seen,
        gaps: if image_seen {
            vec![]
        } else {
            vec!["两张图都下载失败，没读到实图".into()]
        },
        priority_hits: vec!["具体的产品改动".into()],
        lower_hits: vec![],
        jev_disagreement: String::new(),
        kb_refs: vec!["published/2026-08-14/and-wander-3fa91c".into()],
        memory_refs: vec!["rule/不要把门店活动当产品新闻".into()],
        inputs_hash: format!("demo-{key}"),
    }
}
