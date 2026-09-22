//! 深核客户端自检：对**真的** codex app-server 跑通握手 → 建线程 → 一个回合。
//!
//! 协议要点是从实测记录抄来的，客户端是新写的——两者之间的缝只有对真二进制
//! 跑一遍才能发现。`contract_live` 对引擎做过同样的事，当场抓到过一个
//! 「返回是扁平的 `{"run_id": N}` 不是 `{"run": {"id": N}}`」的真错。
//!
//! 用法：`csw-collector codex-probe --bin /path/to/codex --home /path/to/codex-home`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use csw_collector_deepcheck::codex::{Codex, CodexConfig, Input};

pub struct Opts {
    pub bin: PathBuf,
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub model: String,
    pub budget_secs: u64,
    /// 附加一张本地图片，验证 localImage 那条路
    pub image: Option<PathBuf>,
    /// 放行给 codex 的环境变量名
    pub env: Vec<String>,
}

pub async fn run(o: Opts) -> Result<()> {
    anyhow::ensure!(o.bin.exists(), "找不到 codex：{}", o.bin.display());
    anyhow::ensure!(o.home.exists(), "找不到 CODEX_HOME：{}", o.home.display());

    println!("codex     {}", o.bin.display());
    println!("HOME      {}", o.home.display());
    println!("模型      {}", o.model);
    println!("放行环境  {}\n", o.env.join("、"));

    let t0 = Instant::now();
    let codex = Codex::start(CodexConfig {
        bin: o.bin.clone(),
        home: o.home.clone(),
        cwd: o.cwd.clone(),
        model: o.model.clone(),
        turn_budget: Duration::from_secs(o.budget_secs),
        env_passthrough: o.env.clone(),
    })
    .await
    .context("拉起并握手")?;
    println!(
        "  通过  握手 initialize + initialized（{} ms）",
        t0.elapsed().as_millis()
    );

    let t1 = Instant::now();
    let thread = codex
        .start_thread("你是核查员。只做核查，不写文章。回答要短。不要执行任何命令。")
        .await
        .context("建线程")?;
    println!(
        "  通过  thread/start → {thread}（{} ms，沙箱 read-only、审批 never）",
        t1.elapsed().as_millis()
    );

    let mut input = vec![Input::Text(
        "只回复两个字：收到。不要解释，不要调用任何工具。".into(),
    )];
    if let Some(img) = &o.image {
        anyhow::ensure!(img.exists(), "找不到图片：{}", img.display());
        input.push(Input::LocalImage(img.clone()));
        input[0] = Input::Text("用一句话说这张图里有什么。不要调用任何工具。".into());
    }

    let t2 = Instant::now();
    let out = codex.run_turn(&thread, &input).await.context("跑回合")?;
    let ms = t2.elapsed().as_millis();
    if out.ok() {
        println!("  通过  turn/completed（{ms} ms，turn {}）", out.turn_id);
    } else {
        println!(
            "  失败  回合状态 {}（{ms} ms）{}",
            out.status,
            if out.error.is_empty() {
                String::new()
            } else {
                format!("：{}", out.error)
            }
        );
    }
    println!("  条目流 {} 条（增量已滤掉）", out.items.len());
    let text = out.text.trim();
    if text.is_empty() {
        println!(
            "  提醒  没收到助手文字——`item/completed` 里 agentMessage 的字段名可能与我们假设的不同"
        );
        if let Some(first) = out.items.iter().find(|p| {
            p.get("item")
                .and_then(|i| i.get("type"))
                .and_then(|t| t.as_str())
                == Some("agentMessage")
        }) {
            println!("  实际形状：{}", head(&first.to_string()));
        }
    } else {
        println!("  助手说：{}", head(text));
    }

    codex.shutdown().await?;
    println!("\n总用时 {:.1}s", t0.elapsed().as_secs_f64());
    anyhow::ensure!(out.ok(), "回合没有正常完成");
    Ok(())
}

fn head(s: &str) -> String {
    s.chars().take(300).collect()
}
