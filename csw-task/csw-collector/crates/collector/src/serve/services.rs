//! 一次建好、整个进程复用的客户端。
//!
//! **为什么要集中建**：模型网关的并发上限（8）与 GPU 的串行队列都藏在客户端里，
//! 每个地方各建一个就等于把闸门复制了好几份——8 变成 24，实测直接吃 429；
//! GPU 那边更糟，两条队列互相抢卡会把双方都拖到四倍延迟。
//!
//! 所以：**一个进程一套客户端**，谁要用谁借。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use csw_collector_core::jev::{JevClient, JevConfig};
use csw_collector_core::model::{ModelClient, ModelConfig};
use csw_collector_core::vector::{VectorClient, VectorConfig};
use csw_collector_core::{Config, Secrets};
use csw_collector_harvest::csw::{CswClient, CswConfig};
use csw_collector_harvest::download::Downloader;
use csw_collector_kb::brands::BrandIndex;
use csw_collector_kb::fts::Tokenizer;
use csw_collector_kb::vectors::VectorStore;

pub struct Services {
    pub csw: Arc<CswClient>,
    pub downloader: Downloader,
    pub model: ModelClient,
    pub vector: VectorClient,
    /// Jev 不可用时是 None：初评、合并、核对都跳过，**判断照跑**。
    pub jev: Option<JevClient>,
    pub store: VectorStore,
    pub brands: BrandIndex,
    pub tok: Tokenizer,
}

impl Services {
    pub async fn build(
        cfg: &Config,
        secrets: &Secrets,
        conn: &rusqlite::Connection,
    ) -> Result<Self> {
        let csw = Arc::new(CswClient::new(CswConfig {
            base_url: cfg.csw.base_url.clone(),
            api_key: secrets.csw_api_key.clone(),
            page_size: cfg.csw.window_page,
            timeout: Duration::from_secs(cfg.csw.timeout_secs),
        })?);
        let model = ModelClient::new(ModelConfig {
            base_url: cfg.model.base_url.clone(),
            api_key: secrets.sub2api_key.clone(),
            model: cfg.model.model.clone(),
            fallback_model: cfg.model.fallback_model.clone(),
            concurrency: cfg.model.concurrency,
            timeout: Duration::from_secs(cfg.model.timeout_secs),
            max_attempts: 3,
        })?;
        let vector = VectorClient::new(VectorConfig {
            base_url: cfg.vector.base_url.clone(),
            timeout: Duration::from_secs(cfg.vector.timeout_secs),
            batch_weight: cfg.vector.text_batch,
            ..Default::default()
        })?;
        // 密钥不在就自动关：初评跳过并在台账标注，比半开着强
        let jev = (cfg.jev.enabled && !secrets.typesafe_key.trim().is_empty())
            .then(|| {
                JevClient::new(
                    JevConfig {
                        base_url: cfg.jev.base_url.clone(),
                        model: cfg.jev.model.clone(),
                        concurrency: cfg.jev.concurrency,
                        ..Default::default()
                    },
                    &secrets.typesafe_key,
                )
            })
            .transpose()?;
        if jev.is_none() {
            tracing::warn!("Jev 没开：初评、合并与核对都会跳过，判断照跑");
        }

        let brands = BrandIndex::load(conn).context("读别名表")?;
        if brands.is_empty() {
            tracing::warn!("别名表是空的，品牌那一路召回会全空。先跑一次 `kb sync`。");
        }
        let tok = Tokenizer::with_brands(&brands.dict_words());
        let store = VectorStore::open(&cfg.lance_path(), crate::EMBED_DIM, &cfg.vector.embed_model)
            .await
            .context("打开向量库")?;

        Ok(Self {
            csw,
            downloader: super::round::downloader(cfg)?,
            model,
            vector,
            jev,
            store,
            brands,
            tok,
        })
    }
}
