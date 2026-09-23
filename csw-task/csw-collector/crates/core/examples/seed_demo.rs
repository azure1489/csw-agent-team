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
//! | 采集与覆盖 | 两个采集器（一个失败的）+ 八个计数与耗时卡片 |
//! | 待核结转 | 两条待核（一条是没读到实图） |
//! | 指标 | 原判与生效档不一致的两条（捞回 / 压下） |
//! | 判断框架 | 三条排除规则：生效的、没原话的、人工停用的 |
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
        (
            "and_wander",
            "and_wander",
            Tier::Recommend,
            true,
            "秋冬新色上架，配色与去年同款有明确区别",
        ),
        (
            "snowpeak",
            "snow_peak",
            Tier::Recommend,
            true,
            "折叠桌加了一个卡扣，收纳厚度少了两厘米",
        ),
        (
            "goldwin",
            "goldwin",
            Tier::Recommend,
            true,
            "与建筑事务所联名，门店陈列同步更新",
        ),
        (
            "nanga",
            "nanga_official",
            Tier::Recommend,
            true,
            "羽绒睡袋换了新的填充配比，标注温标下调",
        ),
        (
            "helinox",
            "helinox",
            Tier::Alternate,
            true,
            "椅子出了新色，结构没变",
        ),
        (
            "mystery",
            "mysteryranch",
            Tier::Alternate,
            true,
            "背包的肩带调节方式改了，细节图能看出来",
        ),
        (
            "karrimor",
            "karrimor",
            Tier::Alternate,
            true,
            "复刻了九十年代的配色",
        ),
        (
            "montbell",
            "montbell",
            Tier::NotRecommend,
            true,
            "只是门店活动预告，没有产品信息",
        ),
        (
            "patagonia",
            "patagonia",
            Tier::NotRecommend,
            true,
            "转发了一条旧内容，无新事实",
        ),
        (
            "chums",
            "chums",
            Tier::NotRecommend,
            true,
            "同一件事上周已经发过",
        ),
        (
            "norrona",
            "norrona",
            Tier::PendingCheck,
            true,
            "提到发售日期但没写清是哪个市场",
        ),
        (
            "yeti",
            "yeti",
            Tier::PendingCheck,
            false,
            "图全下失败，只有正文",
        ),
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

    // 主编改了两档：一条捞回、一条压下。指标页看的就是这两者的重合度。
    //
    // **忽略「本来就是这一档」那个错**：这个脚本要能在同一个库上反复跑
    // （走查脚本每次起栈都会调它），第二次跑时这两条已经是改过的档了。
    let _ = workbench::put_override(
        &conn,
        round.id,
        "chums",
        Tier::Alternate,
        "这条其实是新配色，上周那条是另一个系列",
        "editor",
    );
    let _ = workbench::put_override(
        &conn,
        round.id,
        "goldwin",
        Tier::Alternate,
        "联名信息还没官宣，先放备选",
        "editor",
    );

    // Van 的勾选：三种都来一个
    workbench::put_van_mark(&conn, round.id, "and_wander", "like", "", "editor")?;
    workbench::put_van_mark(&conn, round.id, "snowpeak", "doubt", "", "editor")?;
    workbench::put_van_mark(
        &conn,
        round.id,
        "nanga",
        "note",
        "温标下调这点要在正文里说清楚",
        "editor",
    )?;

    // 首批深核
    workbench::set_first_batch(
        &conn,
        round.id,
        &["and_wander".into(), "snowpeak".into(), "nanga".into()],
    )?;

    // 一件**已经做完**的活 + 几条留痕。
    //
    // 这里曾经留的是一件「排队中」的活，为的是让总览页有东西看。
    // 后果是：服务一启动，主循环的 `run_queued` 就把它当真活取出来执行了——
    // 开了第二轮、去打了真的 csw 接口（假密钥被 401 拒绝）。
    // **样例数据不能留任何「待执行」的东西**：它会被真的执行。
    let work = workbench::enqueue(
        &conn,
        "manual_round",
        None,
        &serde_json::json!({"days": 2}),
        "editor",
    )?;
    workbench::take_next(&conn)?; // 标 running
    workbench::finish_work(
        &conn,
        work,
        true,
        "第 1 轮：取到 12 条，判了 12（推荐 4 备选 3 待核 2）",
    )?;
    workbench::audit(
        &conn,
        "editor",
        "first_batch",
        &format!("r{}", round.id),
        &serde_json::json!({"要的": 3, "标上的": 3}),
    )?;
    workbench::audit(
        &conn,
        "van",
        "van_mark",
        &format!("r{}/and_wander", round.id),
        &serde_json::json!({"mark": "like"}),
    )?;

    // ── 硬性排除 ──
    // 三条规则铺开三种状态：生效的、没原话所以不自动生效的、人工停用的。
    // 这一页最要紧的就是**把没生效的也摆出来**——看不见的规则最危险。
    use csw_collector_core::exclusion;
    let rules = [
        (
            "41#snowpeak-plain",
            "某品牌秋季新色帐篷",
            "snow peak",
            "这种只换个配色的普通上新没什么好说的",
            "没有看点",
        ),
        // 没有原话：进表但 active=0，等人看过再开
        (
            "42#nanga-restock",
            "某品牌睡袋补货",
            "nanga",
            "",
            "重复报道",
        ),
        (
            "43#coleman-store",
            "某品牌门店活动",
            "coleman",
            "门店活动不是产品新闻",
            "不是产品新闻",
        ),
    ];
    for (r, title, brand, quote, reason) in rules {
        exclusion::upsert(
            &conn,
            &exclusion::Exclusion {
                id: 0,
                decision_ref: r.into(),
                item_key: r.split('#').nth(1).unwrap_or("").into(),
                title: title.into(),
                brand: brand.into(),
                source_url: String::new(),
                quote: quote.into(),
                reason: reason.into(),
                reason_code: "no_value".into(),
                decided_at: "2026-09-15T02:00:00Z".into(),
                actor_role: "Van".into(),
                active: true,
                inactive_reason: String::new(),
                changed_by: String::new(),
                changed_at: String::new(),
            },
        )?;
    }
    // 第三条人工停用：这个角度现在又想要了
    let all = exclusion::all(&conn)?;
    if let Some(e) = all.iter().find(|e| e.decision_ref == "43#coleman-store") {
        exclusion::set_active(&conn, e.id, false, "门店活动现在也想报了", "主编")?;
    }
    // 这一轮挡下两条，其中一条主编捞回了。
    //
    // **被排除的条目照样在台账主表里**——真实流程里 `apply_exclusions` 会产出
    // 一条 `excluded_judgement`（六维全「不明」、依据写「未送判断」）。
    // 样例只造命中不造那一行的话，台账上那句「主表里那几行的六维是不明」
    // 就指着两行不存在的东西。
    if let Some(e) = all.iter().find(|e| e.decision_ref == "41#snowpeak-plain") {
        for (key, text, fact, sub, restored) in [
            (
                "snowpeak-c1d2e3",
                "秋季新色帐篷到店，配色三选一",
                0.94,
                0.06,
                false,
            ),
            (
                "snowpeak-f4a5b6",
                "秋季新色帐篷 10 月 3 日发售，售价 68,000 日元",
                0.89,
                0.11,
                true,
            ),
        ] {
            let c = cand(key, "snow_peak", text, true);
            ledger::upsert_candidate(&conn, &c)?;
            ledger::attach_candidate(&conn, round.id, key, "csw-window", false)?;
            media::put_prepared(
                &conn,
                key,
                &c.media,
                &[desc(&format!("{key}-b0"), 0), desc(&format!("{key}-b1"), 1)],
            )?;
            let why = format!(
                "与 Van 否过的《某品牌秋季新色帐篷》是同一件事，且没有新料\
                 （同一事实 {fact:.2}、新料 {sub:.2}）。原话：这种只换个配色的普通上新没什么好说的"
            );
            ledger::put_judgement(
                &conn,
                round.id,
                &excluded(key, &why),
                &[],
                "规则排除（未经模型）",
                "van-rubric/v1",
            )?;
            exclusion::record_hit(&conn, round.id, key, e.id, fact, sub)?;
            if restored {
                exclusion::restore(&conn, round.id, key, "主编", "这次带了发售日和价格，是新料")?;
            }
        }
    }

    // ── 采集与覆盖那一页要的两份账 ──
    //
    // 覆盖页的八个卡片读 `round_steps.counts_json`，各采集器那张表读的是
    // **我们真发给引擎的那一份字节**（outbox 里 kind=sweeps 的 body）——
    // 不是现场再算一遍，页面上看到的要和引擎收到的是同一份。
    // 少了这两样，那一页就是八个「——」加一句「还没发出去」。
    {
        let step = rounds::begin_step(&conn, round.id, StepCode::Harvest, "demo")?;
        rounds::end_step(
            &conn,
            step.id,
            csw_collector_core::types::StepStatus::Succeeded,
            &serde_json::json!({
                "候选": 14,
                "读到实图": 12,
                "复用识别": 3,
                "新落库描述": 22,
                "下载毫秒": 41_000,
                "识别向量墙钟毫秒": 398_000,
                "网关占用毫秒": 286_000,
                "GPU占用毫秒": 540_000,
            }),
            "",
        )?;
        // 两个采集器，其中网页那个失败了——「失败不兜底，如实报」要看得见
        let sweeps = serde_json::json!({"sweeps": [
            {"sweep_key": "csw_window", "platform": "instagram", "source_key": "csw-window",
             "query": "2026-09-22T05:30Z ~ 2026-09-23T05:30Z", "tool": "csw_api",
             "found": 438, "fetched_unique": 305, "in_window": 14, "reviewed": 14,
             "unreviewed": 0, "registered": 14, "result": "ok", "error": "",
             "paged_to_end": true},
            {"sweep_key": "web_rss", "platform": "web", "source_key": "camphack",
             "query": "https://camphack.example/feed", "tool": "web",
             "found": 0, "fetched_unique": 0, "in_window": 0, "reviewed": 0,
             "unreviewed": 0, "registered": 0, "result": "failed",
             "error": "解析 RSS 失败：响应不是 XML（HTTP 503）", "paged_to_end": false}
        ]});
        let body = serde_json::to_string(&sweeps)?;
        csw_collector_core::outbox::enqueue(
            &conn,
            &csw_collector_core::outbox::NewEntry {
                round_id: round.id,
                kind: OutboxKind::Sweeps,
                idem_key: format!("demo-sweeps-r{}", round.id),
                body_path: String::new(),
                body_json: body.clone(),
                body_sha: blake3::hash(body.as_bytes()).to_hex().to_string(),
                depends_on: None,
            },
        )?;
    }

    // 轮次收尾。不收的话总览页会一直显示「还在跑」，而且重启后的
    // `recover_interrupted` 会把它的步标成中断——样例数据不该看着像出了事
    rounds::finish_round(&conn, round.id, "done", "")?;

    println!(
        "造好了：{} 条候选、{} 条判断、2 条改档、3 个勾选、1 件做完的活、\
         3 条排除规则（挡下 1、捞回 1，两条都在台账里）",
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
        kind: if ordinal == 0 {
            ImageKind::Product
        } else {
            ImageKind::Detail
        },
        usable_as_figure: true,
        model: "gpt-6-astra".into(),
        prompt_version: "v1".into(),
    }
}

/// 被规则挡下的那一行。**不是判断**——六维全「不明」，依据写「未送判断」。
/// 与 `judge::exclude::excluded_judgement` 产出的形状一致。
fn excluded(key: &str, why: &str) -> Judgement {
    let basis = format!("未送判断：{why}");
    Judgement {
        candidate_key: key.into(),
        tier: Tier::NotRecommend,
        dims: Dim::ALL
            .into_iter()
            .map(|d| {
                (
                    d,
                    DimJudgement {
                        verdict: Verdict::Unclear,
                        basis: basis.clone(),
                    },
                )
            })
            .collect(),
        headline: format!("{key}｜被硬性排除"),
        three_sentences: ThreeSentences::default(),
        novelty: Novelty::default(),
        readiness: Readiness::default(),
        unanswered: Unanswered::None,
        comparison: Comparison {
            verdict: ComparisonVerdict::SameFactNoGain,
            against: "某品牌秋季新色帐篷".into(),
            note: why.into(),
            hits: vec![ComparisonHit {
                ref_no: "D1".into(),
                title: "某品牌秋季新色帐篷".into(),
                state: HitState::Decision,
                body_available: true,
                dup_fact: why.into(),
                ..Default::default()
            }],
        },
        heat_note: String::new(),
        look: String::new(),
        image_seen: true,
        gaps: vec![],
        priority_hits: vec![],
        lower_hits: vec![],
        jev_disagreement: String::new(),
        kb_refs: vec![],
        memory_refs: vec![],
        inputs_hash: format!("excluded:demo-{key}"),
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
        headline: format!("{}｜示例推荐理由", &text[..text.len().min(24)]),
        three_sentences: ThreeSentences {
            what: text.into(),
            why_worth: "读者出门前会关心的那一类设计".into(),
            grounds: "正文写明了结构与用途".into(),
        },
        novelty: Novelty {
            kind: NoveltyKind::ExplainableDesign,
            basis: "正文第二句".into(),
            prior_evidence: String::new(),
        },
        readiness: Readiness {
            fact_source: FactSource::Primary,
            usable_images: if image_seen { 2 } else { 0 },
            material_complete: tier == Tier::Recommend,
            note: String::new(),
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
            hits: vec![ComparisonHit {
                ref_no: "M1".into(),
                title: "and wander 秋季系列".into(),
                url: "https://mp.weixin.qq.com/s/demo".into(),
                state: HitState::Published,
                published_at: "2026-08-14".into(),
                body_available: true,
                dup_fact: String::new(),
            }],
        },
        heat_note: "点赞是该账号近 90 天中位的 2.4 倍".into(),
        look: if image_seen {
            "实图上能看清织标位置与面料纹理".into()
        } else {
            String::new()
        },
        image_seen,
        gaps: if image_seen {
            vec![Gap {
                level: GapLevel::Production,
                what: "发售价格未写".into(),
                owner: GapOwner::Collector,
                tried: "正文与图片都没有".into(),
                next: "深核查官网".into(),
            }]
        } else {
            vec![Gap {
                level: GapLevel::Decision,
                what: "两张图都下载失败，没读到实图".into(),
                owner: GapOwner::Collector,
                tried: "下载重试 3 次".into(),
                next: "下一轮重新下载后重判".into(),
            }]
        },
        priority_hits: vec!["具体的产品改动".into()],
        lower_hits: vec![],
        jev_disagreement: String::new(),
        kb_refs: vec!["published/2026-08-14/and-wander-3fa91c".into()],
        memory_refs: vec!["rule/不要把门店活动当产品新闻".into()],
        inputs_hash: format!("demo-{key}"),
    }
}
