//! 对**真引擎二进制**跑一遍的契约测试。
//!
//! wiremock 那套只能证明「我按我以为的样子解析」。字段名拼错、状态枚举对不上、
//! 分页参数名变了——这些 mock 一个都发现不了，因为 mock 是照我的假设写的。
//! 这个测试起真的 `bin/server`，用真的 token 走真的库。
//!
//! 引擎二进制不在就跳过（打印原因）：不是每台机器都会先 `make build` Go 那一侧。
//! 要跑它：`cd ../csw-task-svc && make build`，再 `cargo test -p csw-collector-engineapi`。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use csw_collector_engineapi::{
    DeliverableKind, EngineClient, ItemInput, JudgementInput, SubmitInput, SweepInput,
};

/// 引擎二进制的位置：crates/engineapi → 上三层是 csw-task/，旁边就是 csw-task-svc。
fn engine_bin_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../csw-task-svc/bin")
}

fn engine_bin(name: &str) -> Option<PathBuf> {
    let p = engine_bin_dir().join(name);
    p.exists().then(|| p.canonicalize().unwrap_or(p))
}

struct Engine {
    child: std::process::Child,
    port: u16,
    _dir: tempdir::TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 起一台真引擎：临时库 → 迁移 → 签 collector token → 触发一期 → 跑 server。
///
/// 返回 `Ok(None)` 只代表「引擎二进制不在，跳过」；其余问题一律 `Err`，
/// 免得把「触发失败」误报成「没装引擎」——那会让这个测试形同虚设。
async fn boot() -> Result<Option<(Engine, String, i64, i64)>, String> {
    // 测试里自己也建 reqwest Client（探活、触发），同样要先装提供者
    csw_collector_core::ensure_crypto_provider();
    let (Some(adminctl), Some(server)) = (engine_bin("adminctl"), engine_bin("server")) else {
        return Ok(None);
    };
    let dir = tempdir::TempDir::new("csw-live").map_err(|e| e.to_string())?;
    let db = dir.path().join("live.db");

    let run = |bin: &PathBuf, args: &[&str]| -> Result<String, String> {
        let out = Command::new(bin)
            .args(args)
            .env("CSW_DB_PATH", &db)
            .output()
            .map_err(|e| format!("跑 {} {:?}：{e}", bin.display(), args))?;
        if !out.status.success() {
            return Err(format!(
                "{} {:?} 失败：{}",
                bin.display(),
                args,
                String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(300)
                    .collect::<String>()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let token_from = |s: &str, what: &str| -> Result<String, String> {
        s.split_whitespace()
            .find(|w| w.starts_with("csw_live_"))
            .map(|w| w.to_string())
            .ok_or_else(|| {
                format!(
                    "{what} 的输出里没找到明文 token：{}",
                    s.chars().take(200).collect::<String>()
                )
            })
    };
    run(&adminctl, &["migrate", "up"])?;
    let token = token_from(
        &run(&adminctl, &["token", "issue", "collector"])?,
        "collector token",
    )?;
    let editor = token_from(
        &run(&adminctl, &["token", "issue", "editor"])?,
        "editor token",
    )?;

    // 找一个空闲端口再让引擎占它。有竞态，但比写死端口好。
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| e.to_string())?;
    let child = Command::new(&server)
        .env("CSW_DB_PATH", &db)
        .env("CSW_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("起 server：{e}"))?;
    let eng = Engine {
        child,
        port,
        _dir: dir,
    };

    // 等它起来
    let base = format!("http://127.0.0.1:{}/api/v1", eng.port);
    let probe = reqwest::Client::new();
    for _ in 0..80 {
        if probe
            .get(format!("{base}/me/tasks"))
            .bearer_auth(&token)
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 用主编 token 触发一期，才会有派给 collector 的 01 任务
    let resp = probe
        .post(format!("{base}/workflows/daily_news/runs"))
        .bearer_auth(&editor)
        .json(&serde_json::json!({"subject": "2026-09-22"}))
        .send()
        .await
        .map_err(|e| format!("触发一期：{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "触发返回不是 JSON（{status}）：{e}；{}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    // 触发返回是扁平的 {"run_id": N, "tasks": [...]}，不是 {"run": {"id": N}}
    let run_id = body.get("run_id").and_then(|v| v.as_i64()).ok_or_else(|| {
        format!(
            "触发返回里没有 run_id（{status}）：{}",
            text.chars().take(300).collect::<String>()
        )
    })?;

    let c = EngineClient::new(&base, &token, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let tasks = c.my_tasks().await.map_err(|e| format!("接单：{e}"))?;
    let task_id = tasks
        .tasks
        .first()
        .map(|t| t.task.id)
        .ok_or_else(|| "触发后 collector 没有任何任务——01 应当自动派给它".to_string())?;
    Ok(Some((eng, token, run_id, task_id)))
}

#[tokio::test]
async fn 对真引擎跑通接单登记与自查() {
    let booted = boot().await.unwrap_or_else(|e| panic!("起真引擎失败：{e}"));
    let Some((eng, token, run_id, task_id)) = booted else {
        // 只有「二进制不在」才跳过，其余都在上一行炸了
        println!(
            "跳过「对真引擎跑通」：{} 下没有 adminctl / server。先 cd csw-task/csw-task-svc && make build",
            engine_bin_dir().display()
        );
        return;
    };
    let base = format!("http://127.0.0.1:{}/api/v1", eng.port);
    let c = EngineClient::new(&base, &token, Duration::from_secs(20)).unwrap();

    // 接单：字段名与状态枚举必须真的对得上
    let tasks = c.my_tasks().await.unwrap();
    let t = &tasks.tasks[0].task;
    assert_eq!(t.stage_code, "intake", "01 应当派给 collector");
    assert!(
        t.status.is_actionable(),
        "刚派的单应当是要干活的状态，实为 {:?}",
        t.status
    );
    assert_eq!(t.run_id, run_id);

    // 作业标准要取得到——它会被原样注入模型，取空了判断就没依据
    let detail = c.task_detail(task_id).await.unwrap();
    assert!(!detail.instructions.trim().is_empty(), "作业内容不能是空的");
    assert!(!detail.acceptance.trim().is_empty(), "验收标准不能是空的");
    let h1 = detail.instructions_hash();
    assert_eq!(
        h1,
        c.task_detail(task_id).await.unwrap().instructions_hash(),
        "同一份标准要算出同一个哈希"
    );

    // ack 兼作心跳，连调三次不该报错
    for _ in 0..3 {
        c.ack(task_id).await.unwrap();
    }

    // 条目登记：形态照 `serve::register::item_inputs`。推荐与备选都写 shortlisted——
    // 09-23 演练时写的是 candidate / alternate，引擎 400 bad_item_status
    c.put_items(
        run_id,
        &[ItemInput {
            item_key: "a-000001".into(),
            title: "品牌｜一句话".into(),
            source_url: "https://www.instagram.com/p/AAA/".into(),
            published_at: "2026-09-22T06:43:00Z".into(),
            status: "shortlisted".into(),
            ..Default::default()
        }],
    )
    .await
    .unwrap();

    // 采集轮：tool=csw_api 必须被接受（0036 才加进白名单的）
    c.put_sweeps(
        run_id,
        &[SweepInput {
            sweep_key: "csw-window".into(),
            platform: "instagram".into(),
            source_key: "channel".into(),
            tool: "csw_api".into(),
            found: 438,
            fetched_unique: 2,
            reviewed: 2,
            unreviewed: 0,
            in_window: 2,
            registered: 1,
            result: "ok".into(),
            ..Default::default()
        }],
    )
    .await
    .unwrap();

    // 判断台账：六维齐全、每维有依据
    c.put_judgements(
        run_id,
        &[
            live_judgement("a-000001", "recommend", true),
            live_judgement("b-000002", "not_recommend", true),
        ],
    )
    .await
    .unwrap();

    // 没读到实图却落非待核档 —— 引擎必须拒
    let err = c
        .put_judgements(
            run_id,
            &[live_judgement("c-000003", "not_recommend", false)],
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "image_unseen_must_pend", "实得 {err:?}");

    // 自查：两条判断盖住两条去重候选 → 不该判红
    let check = c.intake_check(run_id).await.unwrap();
    assert!(!check.checks.is_empty());
    let judged = check
        .checks
        .iter()
        .find(|c| c.name.starts_with("每条都判"))
        .expect("应当有「每条都判」这条判据");
    assert!(judged.ok, "每条都判应当通过：{judged:?}");

    // 提交：kind 是英文 output——09-23 送的是中文「产出」，引擎 400 bad_kind
    let zip = std::env::temp_dir().join(format!("csw-live-{}.zip", std::process::id()));
    std::fs::write(&zip, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
    let sub = c
        .submit(&SubmitInput {
            task_id,
            kind: DeliverableKind::Output,
            zip_path: zip.clone(),
            file_name: "情报逐条_情报收集员_test_v1.zip".into(),
            idem_key: format!("submit-{task_id}-live"),
            note: String::new(),
            affects_deliverable_id: None,
            item_key: String::new(),
        })
        .await;
    let _ = std::fs::remove_file(&zip);
    sub.expect("真引擎应当接受 kind=output 的提交");

    // 历史决定：空库也要给空列表而不是报错
    let ds = c.ledger_decisions("365d", 10).await.unwrap();
    assert!(ds.is_empty());
}

fn live_judgement(key: &str, tier: &str, image_seen: bool) -> JudgementInput {
    let dim = serde_json::json!({"verdict": "yes", "basis": "正文「改了结构」"});
    JudgementInput {
        candidate_key: key.into(),
        item_key: String::new(),
        platform: "instagram".into(),
        post_ref: String::new(),
        source_url: String::new(),
        tier: tier.into(),
        dims: serde_json::json!({
            "change": dim, "use": dim, "gain": dim,
            "compare": dim, "explain": dim, "csw": dim
        }),
        three_sentences: serde_json::json!({
            "what": "a", "why_worth": "b", "grounds": "c",
            "headline": "品牌 背包｜侧袋结构值得解释",
            "novelty": {"kind": "explainable_design", "basis": "第 2 张图", "prior_evidence": ""}
        }),
        comparison: serde_json::json!({
            "verdict": "unconfirmed", "against": "", "note": "",
            "hits": [{"ref_no": "M1", "title": "旧文", "url": "", "state": "published",
                      "published_at": "", "body_available": false, "dup_fact": ""}],
            "readiness": {"fact_source": "primary", "usable_images": 2, "material_complete": false, "note": ""}
        }),
        heat_note: String::new(),
        gaps: serde_json::json!([{"level": "decision", "what": "正文缺失", "owner": "editor",
                                  "tried": "对照材料里只有标题", "next": "核对正文"}]),
        hits: serde_json::json!({}),
        jev: serde_json::json!({}),
        rubric_version: "van-rubric/v2".into(),
        image_seen,
        carried: false,
    }
}
