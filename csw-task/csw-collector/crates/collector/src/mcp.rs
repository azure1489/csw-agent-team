//! `mcp`：以 stdio 跑本地 MCP 服务。
//!
//! 给两类客户端用：
//! - **深核线程**（codex 子进程按 `CODEX_HOME/config.toml` 挂载它）；
//! - **Hermes**（默认**不**挂——`features.mcp_for_hermes` 要显式打开）。
//!
//! 三个工具全是只读的。`fetch_page` 与 `xhs_search` 不在这里，
//! 它们不进深核白名单（总方案的高风险项清单）。
//!
//! **不要往标准输出打任何别的东西。** stdio 是协议通道，一行杂音就把它冲垮；
//! 日志一律走 stderr（`tracing_subscriber` 默认就是）。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use csw_collector_core::Config;
use csw_collector_core::vector::{VectorClient, VectorConfig};
use csw_collector_kb::brands::BrandIndex;
use csw_collector_kb::fts::Tokenizer;
use csw_collector_kb::vectors::VectorStore;
use csw_collector_mcpsrv::server::Server;
use csw_collector_mcpsrv::tools::{KbContext, KbSearch, KbSimilarSelected, MemoryLookup};

pub async fn run(cfg: &Config) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let brands = BrandIndex::load(&conn).context("读别名表")?;
    let tok = Tokenizer::with_brands(&brands.dict_words());
    let store = VectorStore::open(&cfg.lance_path(), crate::EMBED_DIM, &cfg.vector.embed_model)
        .await
        .context("打开向量库")?;

    // 向量服务不可达时照样起：品牌与全文两路还能查，比整个工具挂掉强
    let vector = VectorClient::new(VectorConfig {
        base_url: cfg.vector.base_url.clone(),
        timeout: Duration::from_secs(cfg.vector.timeout_secs),
        batch_weight: cfg.vector.text_batch,
        ..Default::default()
    })
    .ok();
    let vector = match vector {
        Some(v) if v.healthy().await => Some(v),
        _ => {
            tracing::warn!(地址 = %cfg.vector.base_url, "向量服务不可达，检索只走品牌与全文两路");
            None
        }
    };

    let ctx = Arc::new(KbContext {
        conn: Mutex::new(conn),
        store,
        brands,
        tok,
        vector,
        send_van_quotes: cfg.features.send_van_quotes_to_model,
    });

    let server = Server::new("csw-kb", env!("CARGO_PKG_VERSION"))
        .with(Box::new(KbSearch(ctx.clone())))
        .with(Box::new(KbSimilarSelected(ctx.clone())))
        .with(Box::new(MemoryLookup(ctx)));
    tracing::info!(工具 = ?server.tool_names(), "本地 MCP 起来了（stdio）");
    server.serve_stdio().await
}
