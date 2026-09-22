//! `kb sync` / `kb import`：把四类对照材料同步进本地知识库。
//!
//! 一次完整同步 = **取材料 → 建别名表 → 算向量**。三段都可以单独重跑：
//! 材料按 `content_hash` 幂等，向量按 `embed_model` 挑没算过的，别名表按
//! `alias_lc` 去重。中途断了直接再跑一次，不需要清理。
//!
//! 一次性建库要一两个小时（实测向量化每条 96–118 ms，两万多条），**放夜间**。
//! 日常的两次同步只处理增量，几分钟。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use csw_collector_core::vector::{VectorClient, VectorConfig};
use csw_collector_core::{Config, Secrets};
use csw_collector_engineapi::client::EngineClient;
use csw_collector_harvest::csw::{CswClient, CswConfig};
use csw_collector_kb::brands::{Alias, BrandIndex};
use csw_collector_kb::fts::Tokenizer;
use csw_collector_kb::vectors::VectorStore;
use csw_collector_kb::{docs, fts, sync};

pub struct SyncOpts {
    /// 全量重拉，不用重叠窗口
    pub full: bool,
    /// 这一轮最多算多少条向量。0 = 不设上限。
    /// 夜间建库不限；白天补跑时限一下，免得和正式轮抢 GPU。
    pub embed_limit: usize,
    /// 重建别名表（默认只在表空时建）
    pub refresh_brands: bool,
}

pub async fn sync(cfg: &Config, secrets: &Secrets, o: SyncOpts) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;

    let csw = Arc::new(CswClient::new(CswConfig {
        base_url: cfg.csw.base_url.clone(),
        api_key: secrets.csw_api_key.clone(),
        page_size: cfg.csw.window_page,
        timeout: Duration::from_secs(cfg.csw.timeout_secs),
    })?);
    let engine = EngineClient::new(
        &cfg.engine.base_url,
        &secrets.engine_token,
        Duration::from_secs(60),
    )?;

    // ── 一、别名表 ──
    // 品牌命中的主路。放在最前面：已生成贴文那一类要靠它从账号名认品牌。
    let index = ensure_brands(&conn, &csw, o.refresh_brands).await?;
    println!("别名表 {} 条", index.len());
    let tok = Tokenizer::with_brands(&index.dict_words());

    // ── 二、四类材料 ──
    let mut reports = Vec::new();
    // 引擎那两路失败不该让 csw 那路也不跑：各报各的，最后一起摆出来
    match sync::sync_ledger_posts(&conn, &engine, &tok, o.full).await {
        Ok(r) => reports.push(r),
        Err(e) => println!("  台账发布记录失败：{e:#}"),
    }
    match sync::sync_decisions(&conn, &engine, &tok, o.full).await {
        Ok(r) => reports.push(r),
        Err(e) => println!("  历史决定失败：{e:#}"),
    }
    match sync::sync_generated(&conn, &csw, &tok, &index).await {
        Ok(r) => reports.push(r),
        Err(e) => println!("  已生成贴文失败：{e:#}"),
    }

    // ── 二点五、选题记忆 ──
    // 它不是「对照材料」那五类之一，不进参考库也不算向量——准则与案例是判断的
    // 口径，要的是每条都看一遍，不是检索相似。所以单独报一行。
    match sync::sync_memory(&conn, &engine).await {
        Ok((rules, cases)) => println!("选题记忆：准则 {rules} 条、案例 {cases} 条"),
        Err(e) => println!("  选题记忆失败：{e:#}"),
    }

    println!("\n来源            取到    新增    变了    没变    跳过");
    for r in &reports {
        println!(
            "{:<16}{:>4}{:>8}{:>8}{:>8}{:>8}",
            r.source, r.fetched, r.inserted, r.changed, r.unchanged, r.skipped
        );
    }

    // 全文索引整理一次：contentless 的 FTS5 删行是追加一条反向记录，
    // 不 optimize 的话查询会越来越慢
    fts::optimize(&conn).context("整理全文索引")?;

    // ── 三、向量 ──
    let store = VectorStore::open(&cfg.lance_path(), crate::EMBED_DIM, &cfg.vector.embed_model)
        .await
        .context("打开向量库")?;
    let vector = VectorClient::new(VectorConfig {
        base_url: cfg.vector.base_url.clone(),
        timeout: Duration::from_secs(cfg.vector.timeout_secs),
        batch_weight: cfg.vector.text_batch,
        ..Default::default()
    })?;
    if !vector.healthy().await {
        println!(
            "\n向量服务 {} 不可达，这一轮只同步材料不算向量",
            cfg.vector.base_url
        );
        print_counts(&conn, &store).await?;
        return Ok(());
    }
    let limit = if o.embed_limit == 0 {
        usize::MAX
    } else {
        o.embed_limit
    };
    let t = std::time::Instant::now();
    let rep = sync::embed_pending(&conn, &store, &vector, &cfg.vector.embed_model, limit).await?;
    println!(
        "\n向量：算了 {}、失败 {}、还剩 {}，用时 {:.1}s",
        rep.embedded,
        rep.failed,
        rep.remaining,
        t.elapsed().as_secs_f64()
    );
    if rep.remaining > 0 {
        println!("  （还有没算完的。再跑一次 kb sync 即可，失败的不会被标记成算过）");
    }
    print_counts(&conn, &store).await?;
    Ok(())
}

/// 待补录清单打到终端。只读。
pub fn backfill(cfg: &Config, limit: usize) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())?;
    let index = BrandIndex::load(&conn)?;
    let rows = csw_collector_kb::coverage::backfill_list(&conn, &index)?;
    println!(
        "在册别名 {} 条；待补录品牌 {} 个\n",
        index.len(),
        rows.len()
    );
    println!(
        "{:<28}{:>6}{:>6}  {:<10}  依据",
        "品牌", "选中", "出现", "最近"
    );
    let n = if limit == 0 { rows.len() } else { limit };
    for r in rows.iter().take(n) {
        let ev = r
            .evidence
            .iter()
            .take(2)
            .map(|e| e.title.chars().take(24).collect::<String>())
            .collect::<Vec<_>>()
            .join(" / ");
        println!(
            "{:<28}{:>6}{:>6}  {:<10}  {}",
            r.brand,
            r.adopted,
            r.mentions,
            r.latest
                .as_deref()
                .map(|d| &d[..10.min(d.len())])
                .unwrap_or("——"),
            ev
        );
    }
    Ok(())
}

pub fn import(cfg: &Config, path: &str) -> Result<()> {
    let conn = csw_collector_core::store::open(&cfg.db_path())?;
    let index = BrandIndex::load(&conn)?;
    let tok = Tokenizer::with_brands(&index.dict_words());
    let r = sync::import_examples(&conn, &tok, std::path::Path::new(path))?;
    println!(
        "范例导入：取到 {}、新增 {}、变了 {}、没变 {}、跳过 {}",
        r.fetched, r.inserted, r.changed, r.unchanged, r.skipped
    );
    println!("（向量还没算。跑一次 `kb sync` 会把它们补上）");
    Ok(())
}

/// 别名表：默认只在空的时候建，因为它包含人工加的别名，重建会把那些冲掉。
async fn ensure_brands(
    conn: &rusqlite::Connection,
    csw: &CswClient,
    refresh: bool,
) -> Result<BrandIndex> {
    let existing = BrandIndex::load(conn)?;
    if !existing.is_empty() && !refresh {
        return Ok(existing);
    }
    let accounts = csw.accounts().await.context("拉在册账号")?;
    let aliases: Vec<Alias> = accounts
        .iter()
        .flat_map(|a| {
            // 一个账号给两个别名候选：展示名与账号名。
            // 账号名常是品牌的拉丁写法（`snowpeak_official`），展示名常是中日文写法。
            let brand = if a.account_name.trim().is_empty() {
                a.account.clone()
            } else {
                a.account_name.clone()
            };
            [a.account_name.clone(), a.account.clone()]
                .into_iter()
                .filter_map(move |x| Alias::new(&brand, &x))
        })
        .collect();
    let index = BrandIndex::new(aliases);
    let n = index.save(conn, "accounts")?;
    println!("从 {} 个在册账号建别名表，新增 {} 条", accounts.len(), n);
    // 存回去再读一遍：库里可能还有人工加的别名，那些也要进索引
    BrandIndex::load(conn)
}

async fn print_counts(conn: &rusqlite::Connection, store: &VectorStore) -> Result<()> {
    let by_kind = docs::counts_by_kind(conn)?;
    let total: usize = by_kind.iter().map(|(_, n)| n).sum();
    let (vdocs, vimgs) = store.counts().await?;
    println!(
        "\n知识库现状：文档 {total} 条（{}），向量 {vdocs} 条、图片向量 {vimgs} 条",
        by_kind
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join("、")
    );
    Ok(())
}
