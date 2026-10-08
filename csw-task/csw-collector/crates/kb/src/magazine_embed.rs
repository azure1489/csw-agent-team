//! 杂志背景库的向量回填（方案 §6「回填任务」、步骤 5）。
//!
//! 两遍，都只算 `kb_doc_images.vector_eligible = 1` 的图（决策 8）：
//! 1. **融合向量**：条目文字 + 裁图（缩到 `image_side` 宽）→ LanceDB `docs` 表，
//!    键 `kb:magazine_item:{ref_id}`；成功后 `kb_docs.embed_model` 记模型。
//! 2. **纯图向量**：裁图 → `images` 表，键 blake3；成功后按 blake3 把
//!    `kb_doc_images.image_embed_model` 记上（同一张图在几期里重复出现只算一次）。
//!
//! 融合向量全部算完才开始纯图：文字 + 图检索先覆盖全部书，以图搜图随后补上。
//!
//! **不复用 [`crate::sync::embed_pending`]**：那条只送文本，且按设计排除了杂志。
//!
//! # 让路（护栏 5）
//!
//! 调用方给一个 `stop`，每批开始前问一次；为真就在当前批结束后返回，不取新批。
//! 只标成功的批次，没标的下次自然再挑到——中断可续，不需要单独的进度表。
//!
//! # 读不出来的图
//!
//! 按顺序取待算的行，一张坏图若不跳过会永远排在最前面、挡住后面所有书。
//! 读图失败的行记进 [`Backfill`] 的跳过集合（进程内），本进程不再取；
//! 不改 `vector_eligible`，进度里它仍算「应算未算」，看得见。

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use csw_collector_core::vector::{EmbedInput, VectorClient};
use rusqlite::{Connection, params};
use serde_json::Value;

use crate::docs::{self, MAGAZINE};
use crate::magazine::{INGESTED, vector_progress};
use crate::vectors::{DocVector, ImageVector, VectorStore};

/// 回填参数（取自 `[vector].embed_model` 与 `[magazine]`）。
#[derive(Debug, Clone)]
pub struct EmbedOpts {
    pub model: String,
    pub fused_batch: usize,
    pub image_batch: usize,
    /// 送向量服务前图缩到多宽（px）；原图更窄就不放大
    pub image_side: u32,
}

/// 跨调用保留的进程内状态：读图失败、本进程不再取的行。
#[derive(Debug, Default)]
pub struct Backfill {
    skip_docs: HashSet<i64>,
    skip_b3: HashSet<String>,
}

impl Backfill {
    pub fn skipped(&self) -> usize {
        self.skip_docs.len() + self.skip_b3.len()
    }
}

#[derive(Debug, Default)]
pub struct BackfillReport {
    /// 这次算成的融合向量条数
    pub fused: usize,
    /// 这次算成的纯图向量张数
    pub pure: usize,
    /// 读图失败、跳过的
    pub unreadable: usize,
    /// 因 `stop` 为真而停（还有活没干完）
    pub stopped: bool,
    /// 向量服务那一批失败了（这批没标，下次重试）
    pub error: Option<String>,
    /// 花在向量服务上的毫秒数与送出的图数：实测每图耗时用
    pub embed_ms: u128,
    pub images_sent: usize,
    /// 这次动过的书，调用方据此更新 `ingested.json`
    pub books: BTreeSet<String>,
    /// 还剩多少（含跳过的）
    pub fused_left: i64,
    pub pure_left: i64,
}

impl BackfillReport {
    pub fn done(&self) -> usize {
        self.fused + self.pure
    }
    /// 每图毫秒；没送过图为 None
    pub fn ms_per_image(&self) -> Option<u128> {
        (self.images_sent > 0).then(|| self.embed_ms / self.images_sent as u128)
    }
}

/// 待算融合向量的一行
struct FusedRow {
    doc_id: i64,
    book_key: String,
    local_path: String,
}

/// 待算纯图向量的一张
struct PureRow {
    blake3: String,
    book_key: String,
    local_path: String,
}

fn next_fused(
    conn: &Connection,
    model: &str,
    n: usize,
    skip: &HashSet<i64>,
    held: &HashSet<i64>,
) -> Result<Vec<FusedRow>> {
    let mut st = conn.prepare(
        "SELECT i.doc_id, i.book_key, i.local_path
         FROM kb_doc_images i JOIN kb_docs d ON d.id = i.doc_id
         WHERE i.vector_eligible = 1 AND d.embed_model <> ?1
         ORDER BY i.book_key, i.doc_id LIMIT ?2",
    )?;
    let rows = st.query_map(params![model, (n + skip.len() + held.len()) as i64], |r| {
        Ok(FusedRow {
            doc_id: r.get(0)?,
            book_key: r.get(1)?,
            local_path: r.get(2)?,
        })
    })?;
    Ok(rows
        .filter_map(Result::ok)
        .filter(|r| !skip.contains(&r.doc_id) && !held.contains(&r.doc_id))
        .take(n)
        .collect())
}

fn next_pure(
    conn: &Connection,
    model: &str,
    n: usize,
    skip: &HashSet<String>,
    held: &HashSet<String>,
) -> Result<Vec<PureRow>> {
    let mut st = conn.prepare(
        "SELECT blake3, MIN(book_key), MIN(local_path)
         FROM kb_doc_images
         WHERE vector_eligible = 1 AND image_embed_model <> ?1
         GROUP BY blake3 ORDER BY MIN(book_key), MIN(doc_id) LIMIT ?2",
    )?;
    let rows = st.query_map(params![model, (n + skip.len() + held.len()) as i64], |r| {
        Ok(PureRow {
            blake3: r.get(0)?,
            book_key: r.get(1)?,
            local_path: r.get(2)?,
        })
    })?;
    Ok(rows
        .filter_map(Result::ok)
        .filter(|r| !skip.contains(&r.blake3) && !held.contains(&r.blake3))
        .take(n)
        .collect())
}

/// 同一张图在别处已算过纯图向量的，直接标上（向量库按 blake3 存，本来就有）。
fn inherit_pure(conn: &Connection, model: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE kb_doc_images SET image_embed_model = ?1
         WHERE image_embed_model <> ?1
           AND blake3 IN (SELECT blake3 FROM kb_doc_images WHERE image_embed_model = ?1)",
        [model],
    )?)
}

fn left(conn: &Connection, model: &str) -> Result<(i64, i64)> {
    Ok(conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM kb_doc_images i JOIN kb_docs d ON d.id = i.doc_id
             WHERE i.vector_eligible = 1 AND d.embed_model <> ?1),
           (SELECT COUNT(DISTINCT blake3) FROM kb_doc_images
             WHERE vector_eligible = 1 AND image_embed_model <> ?1)",
        [model],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// 读图、缩到 `side` 宽、转 JPEG base64。
pub fn load_b64(path: &Path, side: u32) -> Result<String> {
    use image::ImageEncoder;
    let bytes = std::fs::read(path).with_context(|| format!("读 {}", path.display()))?;
    let img =
        image::load_from_memory(&bytes).with_context(|| format!("解码 {}", path.display()))?;
    let rgb = shrink(img, side).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .context("编码 JPEG")?;
    Ok(csw_collector_harvest::recognize::b64(&out))
}

/// 缩到 `side` 宽，原图更窄就不动。只限宽：杂志裁图有很多竖长条，按长边缩会把字缩没。
fn shrink(img: image::DynamicImage, side: u32) -> image::DynamicImage {
    if side > 0 && img.width() > side {
        img.resize(side, u32::MAX, image::imageops::FilterType::Triangle)
    } else {
        img
    }
}

/// 批量读图（解码与缩放放到阻塞线程，不占异步执行器）。
async fn load_all(paths: Vec<String>, side: u32) -> Vec<Result<String>> {
    tokio::task::spawn_blocking(move || {
        paths
            .iter()
            .map(|p| load_b64(&PathBuf::from(p), side))
            .collect()
    })
    .await
    .unwrap_or_else(|e| vec![Err(anyhow::anyhow!("读图线程失败：{e}"))])
}

/// 攒够这么多行才写一次向量库。LanceDB 每次写都是一次提交、生成新碎片文件，
/// 而 `docs` 表是正式轮判断暴力扫的那张；一批 4 行一提交，两次整理之间会积上万个碎片。
const FLUSH_ROWS: usize = 64;

/// 已算好、还没写进向量库的向量。写进去之后才标记（先写向量库再标记，同 embed_pending）。
#[derive(Default)]
struct Held {
    docs: Vec<(DocVector, String)>,
    doc_ids: HashSet<i64>,
    imgs: Vec<(ImageVector, String)>,
    b3s: HashSet<String>,
}

impl Held {
    fn len(&self) -> usize {
        self.docs.len() + self.imgs.len()
    }

    async fn flush(
        &mut self,
        conn: &Connection,
        store: &VectorStore,
        model: &str,
        rep: &mut BackfillReport,
    ) -> Result<()> {
        if !self.docs.is_empty() {
            let rows: Vec<DocVector> = self.docs.iter().map(|(v, _)| v.clone()).collect();
            store.put_docs(&rows).await?;
            let ids: Vec<i64> = rows.iter().map(|v| v.doc_id).collect();
            docs::mark_embedded(conn, &ids, model)?;
            rep.fused += ids.len();
            rep.books.extend(self.docs.drain(..).map(|(_, b)| b));
            self.doc_ids.clear();
        }
        if !self.imgs.is_empty() {
            let rows: Vec<ImageVector> = self.imgs.iter().map(|(v, _)| v.clone()).collect();
            store.put_images(&rows).await?;
            let tx = conn.unchecked_transaction()?;
            for r in &rows {
                tx.execute(
                    "UPDATE kb_doc_images SET image_embed_model = ?1 WHERE blake3 = ?2",
                    params![model, r.blake3],
                )?;
            }
            tx.commit()?;
            rep.pure += rows.len();
            rep.books.extend(self.imgs.drain(..).map(|(_, b)| b));
            self.b3s.clear();
        }
        Ok(())
    }
}

/// 算一批（或几批），直到没活、`stop` 为真或向量服务失败。
pub async fn embed_magazine(
    conn: &Connection,
    store: &VectorStore,
    vector: &VectorClient,
    o: &EmbedOpts,
    state: &mut Backfill,
    stop: &dyn Fn() -> bool,
) -> Result<BackfillReport> {
    let mut rep = BackfillReport::default();
    let fused_n = o.fused_batch.max(1);
    let image_n = o.image_batch.max(1);
    inherit_pure(conn, &o.model)?;
    let mut held = Held::default();
    loop {
        if held.len() >= FLUSH_ROWS {
            held.flush(conn, store, &o.model, &mut rep).await?;
        }
        if stop() {
            rep.stopped = true;
            break;
        }
        // ── 第一遍：融合向量 ──
        let todo = next_fused(conn, &o.model, fused_n, &state.skip_docs, &held.doc_ids)?;
        if !todo.is_empty() {
            let imgs = load_all(
                todo.iter().map(|r| r.local_path.clone()).collect(),
                o.image_side,
            )
            .await;
            let mut docs_ok = Vec::new();
            let mut inputs = Vec::new();
            for (r, img) in todo.iter().zip(imgs) {
                match (img, docs::get(conn, r.doc_id)?) {
                    (Ok(b64), Some(d)) => {
                        inputs.push(EmbedInput::Fused {
                            text: crate::sync::for_embedding(&d.text()),
                            images_b64: vec![b64],
                        });
                        docs_ok.push((d, r.book_key.clone()));
                    }
                    (Err(e), _) => {
                        tracing::warn!(条目 = r.doc_id, 原因 = %format!("{e:#}"), "杂志裁图读不出来，本进程跳过");
                        state.skip_docs.insert(r.doc_id);
                        rep.unreadable += 1;
                    }
                    (Ok(_), None) => {
                        state.skip_docs.insert(r.doc_id);
                    }
                }
            }
            if inputs.is_empty() {
                continue;
            }
            let t = Instant::now();
            match vector.embed(&inputs).await {
                Ok(vs) if vs.len() == docs_ok.len() => {
                    rep.embed_ms += t.elapsed().as_millis();
                    rep.images_sent += inputs.len();
                    for ((d, b), v) in docs_ok.into_iter().zip(vs) {
                        held.doc_ids.insert(d.id);
                        let row = DocVector {
                            key: d.vector_key(),
                            kind: MAGAZINE.into(),
                            doc_id: d.id,
                            post_id: String::new(),
                            brand: d.brand.clone(),
                            at: d.published_at.clone().unwrap_or_default(),
                            is_reference: false,
                            vector: v,
                        };
                        held.docs.push((row, b));
                    }
                }
                Ok(vs) => {
                    rep.error = Some(format!(
                        "融合向量条数对不上：要 {} 给 {}",
                        inputs.len(),
                        vs.len()
                    ));
                    break;
                }
                Err(e) => {
                    rep.error = Some(format!("融合向量：{e:#}"));
                    break;
                }
            }
            continue;
        }

        // ── 第二遍：纯图向量 ──
        // 不受 `features.image_vectors` 约束：那个开关只管候选逐图向量（正式轮里占卡），
        // 杂志纯图向量在空闲时回填，是以图搜图的底子。
        let todo = next_pure(conn, &o.model, image_n, &state.skip_b3, &held.b3s)?;
        if todo.is_empty() {
            break;
        }
        let imgs = load_all(
            todo.iter().map(|r| r.local_path.clone()).collect(),
            o.image_side,
        )
        .await;
        let mut ok = Vec::new();
        let mut inputs = Vec::new();
        for (r, img) in todo.iter().zip(imgs) {
            match img {
                Ok(b64) => {
                    inputs.push(EmbedInput::Image(b64));
                    ok.push(r);
                }
                Err(e) => {
                    tracing::warn!(图 = %r.blake3, 原因 = %format!("{e:#}"), "杂志裁图读不出来，本进程跳过");
                    state.skip_b3.insert(r.blake3.clone());
                    rep.unreadable += 1;
                }
            }
        }
        if inputs.is_empty() {
            continue;
        }
        let t = Instant::now();
        match vector.embed(&inputs).await {
            Ok(vs) if vs.len() == ok.len() => {
                rep.embed_ms += t.elapsed().as_millis();
                rep.images_sent += inputs.len();
                for (r, v) in ok.iter().zip(vs) {
                    held.b3s.insert(r.blake3.clone());
                    held.imgs.push((
                        ImageVector {
                            blake3: r.blake3.clone(),
                            vector: v,
                        },
                        r.book_key.clone(),
                    ));
                }
            }
            Ok(vs) => {
                rep.error = Some(format!(
                    "纯图向量条数对不上：要 {} 给 {}",
                    inputs.len(),
                    vs.len()
                ));
                break;
            }
            Err(e) => {
                rep.error = Some(format!("纯图向量：{e:#}"));
                break;
            }
        }
    }
    // 每条出口（让路、没活、向量服务失败）都把已算好的写进去
    held.flush(conn, store, &o.model, &mut rep).await?;
    (rep.fused_left, rep.pure_left) = left(conn, &o.model)?;
    Ok(rep)
}

/// 更新 `{book}/ingested.json` 里的向量进度，**其余字段原样保留**。
///
/// 不能用 [`crate::magazine::write_ingested`]：那个整份重写，`manifest_generated_at`
/// 一变，下次同步就会把这本当成「清单变了」重新入库。文件不存在（没经目录同步入库）就不写。
pub fn touch_vector_progress(conn: &Connection, book_dir: &Path, book_key: &str) -> Result<bool> {
    let path = book_dir.join(INGESTED);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let mut v: Value =
        serde_json::from_str(&text).with_context(|| format!("解析 {}", path.display()))?;
    let Some(obj) = v.as_object_mut() else {
        anyhow::bail!("{} 不是对象", path.display());
    };
    let (fd, ft, pd, pt) = vector_progress(conn, book_key)?;
    obj.insert("fused_done".into(), fd.into());
    obj.insert("fused_target".into(), ft.into());
    obj.insert("pure_done".into(), pd.into());
    obj.insert("pure_target".into(), pt.into());
    obj.insert(
        "vectors_at".into(),
        jiff::Timestamp::now().to_string().into(),
    );
    let part = book_dir.join(format!("{INGESTED}.part"));
    std::fs::write(&part, serde_json::to_vec_pretty(&v)?)
        .with_context(|| format!("写 {}", part.display()))?;
    std::fs::rename(&part, &path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fts::Tokenizer;
    use crate::magazine::{ingest_lines, tests::line};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const MODEL: &str = "测试模型";

    struct Echo;
    impl wiremock::Respond for Echo {
        fn respond(&self, req: &wiremock::Request) -> wiremock::ResponseTemplate {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            let n = body["input"].as_array().map(Vec::len).unwrap_or(0);
            let data: Vec<Value> = (0..n)
                .map(|i| serde_json::json!({"embedding": vec![i as f32 + 1.0; 8]}))
                .collect();
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data}))
        }
    }

    fn jpeg(dir: &Path, name: &str, w: u32, h: u32) -> String {
        let p = dir.join(name);
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            w,
            h,
            image::Rgb([90, 120, 150]),
        ))
        .save_with_format(&p, image::ImageFormat::Jpeg)
        .unwrap();
        p.to_string_lossy().into_owned()
    }

    struct Fx {
        conn: Connection,
        store: VectorStore,
        vector: VectorClient,
        dir: tempdir::TempDir,
        _srv: wiremock::MockServer,
    }

    /// 一本书 `n` 张有译文的裁图（blake3 由 `img_of(i)` 给，可重复），各有本地图。
    async fn fixture(status: u16, n: i64, img_of: impl Fn(i64) -> u8) -> Fx {
        let srv = wiremock::MockServer::start().await;
        let m = wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/embeddings"));
        if status == 200 {
            m.respond_with(Echo).mount(&srv).await;
        } else {
            m.respond_with(wiremock::ResponseTemplate::new(status))
                .mount(&srv)
                .await;
        }
        let dir = tempdir::TempDir::new("magemb").unwrap();
        let store = VectorStore::open(&dir.path().join("lance"), 8, MODEL)
            .await
            .unwrap();
        let vector = VectorClient::new(csw_collector_core::vector::VectorConfig {
            base_url: srv.uri(),
            timeout: std::time::Duration::from_secs(5),
            ..Default::default()
        })
        .unwrap();
        let conn = csw_collector_core::store::open_in_memory().unwrap();
        let lines: Vec<_> = (0..n)
            .map(|i| {
                let mut l = line("b1", "T", i, 0, Some("译文"), "TNF", img_of(i));
                l.local_path = jpeg(dir.path(), &format!("{i}.jpg"), 1200, 900);
                l
            })
            .collect();
        ingest_lines(&conn, &Tokenizer::new(), "b1", &lines).unwrap();
        Fx {
            conn,
            store,
            vector,
            dir,
            _srv: srv,
        }
    }

    fn opts() -> EmbedOpts {
        EmbedOpts {
            model: MODEL.into(),
            fused_batch: 4,
            image_batch: 2,
            image_side: 768,
        }
    }

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn 缩到指定宽_窄图不放大() {
        let img = |w, h| image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h));
        let w = shrink(img(1600, 400), 768);
        assert_eq!((w.width(), w.height()), (768, 192));
        let n = shrink(img(300, 900), 768);
        assert_eq!((n.width(), n.height()), (300, 900));
        let d = tempdir::TempDir::new("b64").unwrap();
        let b64 = load_b64(Path::new(&jpeg(d.path(), "w.jpg", 1600, 400)), 768).unwrap();
        assert!(!b64.is_empty() && b64.len().is_multiple_of(4));
        assert!(load_b64(Path::new("/没有/这张.jpg"), 768).is_err());
    }

    #[tokio::test]
    async fn 两遍算完_同图只算一次() {
        // 6 条，第 4、5 条与第 0 条同一张图 → 纯图只有 4 张不同的
        let f = fixture(200, 6, |i| if i >= 4 { 10 } else { 10 + i as u8 }).await;
        let mut st = Backfill::default();
        let rep = embed_magazine(&f.conn, &f.store, &f.vector, &opts(), &mut st, &|| false)
            .await
            .unwrap();
        assert_eq!(
            (rep.fused, rep.pure, rep.fused_left, rep.pure_left),
            (6, 4, 0, 0)
        );
        assert!(rep.error.is_none() && !rep.stopped);
        assert_eq!(f.store.counts().await.unwrap(), (6, 4));
        assert_eq!(
            count(
                &f.conn,
                "SELECT COUNT(*) FROM kb_doc_images WHERE image_embed_model = '测试模型'"
            ),
            6,
            "同一张图的几条都要标上"
        );
        assert_eq!(rep.books.iter().collect::<Vec<_>>(), vec!["b1"]);
        // 再跑没活
        let again = embed_magazine(&f.conn, &f.store, &f.vector, &opts(), &mut st, &|| false)
            .await
            .unwrap();
        assert_eq!(again.done(), 0);
    }

    /// 护栏 5：正式轮开始（stop 变真）后当前批做完即停，不取新批；下次接着做。
    #[tokio::test]
    async fn 让路_当前批做完即停_下次接着做() {
        let f = fixture(200, 10, |i| 10 + i as u8).await;
        let asked = AtomicUsize::new(0);
        // 第一次问：还没开始；第二次问：正式轮来了
        let stop = || asked.fetch_add(1, Ordering::SeqCst) >= 1;
        let mut st = Backfill::default();
        let rep = embed_magazine(&f.conn, &f.store, &f.vector, &opts(), &mut st, &stop)
            .await
            .unwrap();
        assert!(rep.stopped);
        assert_eq!((rep.fused, rep.pure), (4, 0), "只做完已开始的那一批");
        assert_eq!(rep.fused_left, 6);
        assert_eq!(
            f.store.counts().await.unwrap(),
            (4, 0),
            "让路前算好的要写进向量库"
        );
        assert_eq!(
            count(
                &f.conn,
                "SELECT COUNT(*) FROM kb_docs WHERE embed_model = '测试模型'"
            ),
            4
        );
        let rep = embed_magazine(&f.conn, &f.store, &f.vector, &opts(), &mut st, &|| false)
            .await
            .unwrap();
        assert_eq!(
            (rep.fused, rep.pure, rep.fused_left, rep.pure_left),
            (6, 10, 0, 0)
        );
    }

    #[tokio::test]
    async fn 向量服务失败不标记() {
        let f = fixture(500, 3, |i| 10 + i as u8).await;
        let rep = embed_magazine(
            &f.conn,
            &f.store,
            &f.vector,
            &opts(),
            &mut Backfill::default(),
            &|| false,
        )
        .await
        .unwrap();
        assert!(rep.error.is_some());
        assert_eq!((rep.fused, rep.fused_left, rep.pure_left), (0, 3, 3));
        assert_eq!(f.store.counts().await.unwrap(), (0, 0));
    }

    #[tokio::test]
    async fn 中途失败_已算好的照样落库() {
        let f = fixture(500, 10, |i| 10 + i as u8).await;
        // 前 4 次请求成功（第一批 4 条；融合一条一请求），之后才是挂载在先的 500
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(Echo)
            .up_to_n_times(4)
            .with_priority(1)
            .mount(&f._srv)
            .await;
        let rep = embed_magazine(
            &f.conn,
            &f.store,
            &f.vector,
            &opts(),
            &mut Backfill::default(),
            &|| false,
        )
        .await
        .unwrap();
        assert!(rep.error.is_some());
        assert_eq!((rep.fused, rep.fused_left), (4, 6));
        assert_eq!(f.store.counts().await.unwrap(), (4, 0));
    }

    #[tokio::test]
    async fn 坏图跳过不挡后面_装饰图不算() {
        let f = fixture(200, 3, |i| 10 + i as u8).await;
        f.conn
            .execute(
                "UPDATE kb_doc_images SET local_path = '/没有/这张.jpg' WHERE pdf_index = 0",
                [],
            )
            .unwrap();
        f.conn
            .execute(
                "UPDATE kb_doc_images SET vector_eligible = 0 WHERE pdf_index = 2",
                [],
            )
            .unwrap();
        let mut st = Backfill::default();
        let rep = embed_magazine(&f.conn, &f.store, &f.vector, &opts(), &mut st, &|| false)
            .await
            .unwrap();
        assert_eq!((rep.fused, rep.pure), (1, 1));
        assert_eq!(rep.unreadable, 2, "融合、纯图各跳一次");
        assert_eq!((rep.fused_left, rep.pure_left), (1, 1), "坏图仍算应算未算");
        assert_eq!(st.skipped(), 2);
    }

    #[tokio::test]
    async fn 更新进度不动其余字段() {
        let f = fixture(200, 2, |i| 10 + i as u8).await;
        let book = f.dir.path().join("b1");
        std::fs::create_dir_all(&book).unwrap();
        std::fs::write(
            book.join(INGESTED),
            r#"{"manifest_generated_at":"2026-10-08T12:00:00Z","docs":2,"fused_done":0}"#,
        )
        .unwrap();
        embed_magazine(
            &f.conn,
            &f.store,
            &f.vector,
            &opts(),
            &mut Backfill::default(),
            &|| false,
        )
        .await
        .unwrap();
        assert!(touch_vector_progress(&f.conn, &book, "b1").unwrap());
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(book.join(INGESTED)).unwrap()).unwrap();
        assert_eq!(v["manifest_generated_at"], "2026-10-08T12:00:00Z");
        assert_eq!(v["docs"], 2);
        assert_eq!(
            (v["fused_done"].as_i64(), v["pure_target"].as_i64()),
            (Some(2), Some(2))
        );
        assert!(!book.join(format!("{INGESTED}.part")).exists());
        // 没有 ingested.json 的书不写
        assert!(!touch_vector_progress(&f.conn, f.dir.path(), "b1").unwrap());
    }
}
