//! 阶段 0.2 的依赖自检：在目标主机上把每个重依赖真正跑一遍。
//!
//! 交叉编译「编得过」不等于「跑得动」——SQLite 的 bundled C、ring 的汇编、
//! lance 的 SIMD 与 mmap、rustls 的根证书读取，都要在 x86_64 glibc 2.34 的
//! 真实主机上验证。这个子命令就是那份验收单。
//!
//! 用法：`csw-collector xcheck [--http-url URL] [--dim 2048] [--rows 200]`

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use futures::TryStreamExt;
use lancedb::arrow::arrow_array::{
    FixedSizeListArray, Int32Array, RecordBatch, RecordBatchIterator, StringArray,
    types::Float32Type,
};
use lancedb::arrow::arrow_schema::{DataType, Field, Schema};
use lancedb::index::Index;
use lancedb::index::scalar::FtsIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};

pub struct Opts {
    pub http_url: String,
    pub dim: i32,
    pub rows: usize,
}

pub async fn run(opts: Opts) -> Result<()> {
    let workdir = std::env::temp_dir().join(format!("csw-collector-xcheck-{}", std::process::id()));
    std::fs::create_dir_all(&workdir)?;
    println!("工作目录 {}", workdir.display());
    println!("目标三元组 {}", env!("TARGET_TRIPLE"));

    let mut failures = Vec::new();
    for (name, result) in [
        ("rusqlite (bundled)", check_sqlite(&workdir)),
        ("blake3 / sha2 / jiff", check_hash_time()),
    ] {
        report(name, result, &mut failures);
    }
    report(
        "lancedb (向量 + FTS)",
        check_lancedb(&workdir, opts.dim, opts.rows).await,
        &mut failures,
    );
    report("axum + tokio (本地回环)", check_axum().await, &mut failures);
    report(
        "reqwest + rustls/ring (TLS)",
        check_tls(&opts.http_url).await,
        &mut failures,
    );

    println!("常驻内存峰值 {}", peak_rss());
    let _ = std::fs::remove_dir_all(&workdir);

    if failures.is_empty() {
        println!("\n全部通过");
        Ok(())
    } else {
        anyhow::bail!("{} 项未通过：{}", failures.len(), failures.join("、"))
    }
}

fn report(name: &str, result: Result<String>, failures: &mut Vec<String>) {
    match result {
        Ok(detail) => println!("  通过  {name} — {detail}"),
        Err(e) => {
            println!("  失败  {name} — {e:#}");
            failures.push(name.to_string());
        }
    }
}

fn check_sqlite(workdir: &std::path::Path) -> Result<String> {
    let path = workdir.join("probe.db");
    let conn = rusqlite::Connection::open(&path).context("打开 SQLite")?;
    // 本地状态库实际会用的 pragma：WAL + 外键 + busy_timeout
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE rounds(id INTEGER PRIMARY KEY, kind TEXT NOT NULL);
         CREATE TABLE steps(id INTEGER PRIMARY KEY, round_id INTEGER NOT NULL
             REFERENCES rounds(id), status TEXT NOT NULL);
         INSERT INTO rounds(kind) VALUES ('prefetch');
         INSERT INTO steps(round_id, status) VALUES (1, 'succeeded');",
    )?;
    let joined: String = conn.query_row(
        "SELECT r.kind || '/' || s.status FROM rounds r JOIN steps s ON s.round_id = r.id",
        [],
        |row| row.get(0),
    )?;
    let version: String = conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    anyhow::ensure!(joined == "prefetch/succeeded", "查询结果不符：{joined}");
    anyhow::ensure!(mode == "wal", "WAL 未生效，journal_mode={mode}");
    Ok(format!("SQLite {version}，WAL 与外键生效"))
}

fn check_hash_time() -> Result<String> {
    use sha2::Digest;
    let blake = blake3::hash(b"csw-collector").to_hex();
    let sha = hex(&sha2::Sha256::digest(b"csw-collector"));
    let now = jiff::Timestamp::now();
    anyhow::ensure!(blake.len() == 64 && sha.len() == 64, "摘要长度不对");
    Ok(format!(
        "blake3 {}… sha256 {}… 现在 {now}",
        &blake[..8],
        &sha[..8]
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn check_lancedb(workdir: &std::path::Path, dim: i32, rows: usize) -> Result<String> {
    let uri = workdir.join("lance").to_string_lossy().into_owned();
    let db = lancedb::connect(&uri).execute().await.context("connect")?;

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("doc", DataType::Utf8, true),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
            true,
        ),
    ]));
    // 造一批可判别的向量：第 i 行在第 i 维上更大，最近邻应当唯一确定
    let vectors = (0..rows).map(|i| {
        let mut v = vec![Some(0.01_f32); dim as usize];
        v[i % dim as usize] = Some(1.0);
        Some(v)
    });
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from_iter_values(0..rows as i32)),
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|i| format!("户外 露营 灯具 第{i}条")),
            )),
            Arc::new(FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(vectors, dim)),
        ],
    )?;

    let t0 = Instant::now();
    let tbl = db
        .create_table(
            "probe",
            Box::new(RecordBatchIterator::new(
                vec![Ok(batch)].into_iter(),
                schema,
            )) as Box<dyn lancedb::arrow::arrow_array::RecordBatchReader + Send>,
        )
        .execute()
        .await
        .context("create_table")?;
    let write_ms = t0.elapsed().as_millis();

    // 向量检索：查第 7 行的形状，期望它排第一
    let mut query = vec![0.01_f32; dim as usize];
    query[7] = 1.0;
    let t1 = Instant::now();
    let hits = tbl
        .query()
        .nearest_to(query.as_slice())?
        .limit(5)
        .execute()
        .await
        .context("向量检索")?
        .try_collect::<Vec<_>>()
        .await?;
    let search_ms = t1.elapsed().as_millis();
    let top = hits
        .first()
        .and_then(|b| b.column_by_name("id").cloned())
        .and_then(|c| c.as_any().downcast_ref::<Int32Array>().map(|a| a.value(0)))
        .context("检索结果里没有 id 列")?;
    anyhow::ensure!(top == 7, "最近邻应为 7，实际 {top}");

    // 倒排索引（阶段 0.3 要换成 jieba 分词器，这里先验证索引路径能建）
    let t2 = Instant::now();
    tbl.create_index(&["doc"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await
        .context("建 FTS 索引")?;
    let fts_ms = t2.elapsed().as_millis();

    let count = tbl.count_rows(None).await?;
    anyhow::ensure!(count == rows, "行数应为 {rows}，实际 {count}");
    Ok(format!(
        "{rows} 行 × {dim} 维：写入 {write_ms}ms，检索 {search_ms}ms（top1=id{top}），建 FTS 索引 {fts_ms}ms"
    ))
}

async fn check_axum() -> Result<String> {
    let app = axum::Router::new().route("/healthz", axum::routing::get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("绑定回环")?;
    let addr = listener.local_addr()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
    });

    let body = reqwest::Client::new()
        .get(format!("http://{addr}/healthz"))
        .send()
        .await?
        .text()
        .await?;
    let _ = tx.send(());
    server.await?.context("axum 退出异常")?;
    anyhow::ensure!(body == "ok", "回环响应不对：{body}");
    Ok(format!("监听 {addr}，自请求与优雅退出均正常"))
}

async fn check_tls(url: &str) -> Result<String> {
    let t0 = Instant::now();
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("请求 {url}"))?;
    // 任何 HTTP 状态都算通过——要验的是 TLS 握手与根证书链，不是对端的业务
    Ok(format!(
        "{url} → {} （{}ms）",
        resp.status(),
        t0.elapsed().as_millis()
    ))
}

fn peak_rss() -> String {
    // Linux 才有 VmHWM；开发机 macOS 上跳过，反正要看的是目标主机的数
    match std::fs::read_to_string("/proc/self/status") {
        Ok(s) => s
            .lines()
            .find(|l| l.starts_with("VmHWM:"))
            .map(|l| l.trim_start_matches("VmHWM:").trim().to_string())
            .unwrap_or_else(|| "未知".into()),
        Err(_) => "不可用（非 Linux）".into(),
    }
}
