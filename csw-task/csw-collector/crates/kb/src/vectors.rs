//! 向量库（LanceDB）。**只放向量**，一切结构化字段在 SQLite。
//!
//! 两张表：
//! - `docs` —— 一行一个「文本或图文融合」向量：参考库四类 + 候选缓存。
//! - `images` —— 一行一张图的向量，键就是图片内容的 blake3。
//!
//! `images` 刻意不记「这张图属于谁」：同一张图跨账号转载是常态，归属是一对多的，
//! 那份映射在 SQLite 的 `media` / `kb_docs` 里。向量库只回答「最近的是哪几张」，
//! 拿到 blake3 再回 SQLite 问归属。这样查重天然是跨来源的——候选图与已发条目的图
//! 若是同一份内容，连检索都不用做，哈希就撞上了。
//!
//! ## 三条硬约束
//!
//! **一、不建 ANN 索引。** 参考库满打满算 2.2 万行；IVF_PQ 要训出像样的分区，
//! 官方的经验值是每个分区 ≥256 行、分区数取 √n，2.2 万行只够训 150 个分区，
//! 召回率的损失换来的是一个本来就不存在的性能问题——暴力扫 2.2 万 × 2048 维
//! 不过 4500 万次乘加，毫秒级。**等参考库过十万行再谈索引。**
//!
//! **二、单写者。** 本地 LanceDB 用乐观并发：两个写者同时提交，后一个拿到
//! `CommitConflict`。整个采集服务只有主进程写，进程内再用一把锁串起来（见
//! [`VectorStore::write`]）。**别在子进程或第二个实例里写这个目录。**
//!
//! **三、换了向量模型必须重建。** 不同模型的向量不可比，混在一张表里检索结果
//! 会毫无道理地劣化，而且**不会报错**——这是最难发现的一类故障。所以库目录下
//! 钉一个 `EMBED_MODEL` 文件，[`VectorStore::open`] 每次开库都核对，对不上就拒绝
//! 打开并明说要重建。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::TryStreamExt;
use lancedb::DistanceType;
use lancedb::arrow::arrow_array::{
    BooleanArray, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray, types::Float32Type,
};
use lancedb::arrow::arrow_schema::{DataType, Field, Schema, SchemaRef};
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::table::Table;
use lancedb::table::optimize::OptimizeAction;
use tokio::sync::Mutex;

/// 库目录下记着建库时用的向量模型。换模型必须重建，见模块文档第三条。
const MODEL_STAMP: &str = "EMBED_MODEL";

const DOCS: &str = "docs";
const IMAGES: &str = "images";

/// 距离列，LanceDB 固定叫这个名字。
const DIST: &str = "_distance";

/// 一条文档向量。
#[derive(Debug, Clone)]
pub struct DocVector {
    /// 唯一键。参考库用 `kb:{kind}:{ref_id}`，候选用 `cand:{candidate_key}`。
    pub key: String,
    /// published_item | example | generated_post | decision | candidate
    pub kind: String,
    /// SQLite `kb_docs.id`；候选没有，填 -1
    pub doc_id: i64,
    /// 同一贴文的多种身份共用一个 post_id，用来把自己从结果里排掉
    pub post_id: String,
    pub brand: String,
    /// 发布时间或入库时间，RFC3339；不知道就空串
    pub at: String,
    /// Van 标过「可作范例」的已发条目
    pub is_reference: bool,
    pub vector: Vec<f32>,
}

/// 一条图片向量。键就是图片内容的 blake3 十六进制。
#[derive(Debug, Clone)]
pub struct ImageVector {
    pub blake3: String,
    pub vector: Vec<f32>,
}

/// 检索条件。字段全是**前置**过滤——先按条件缩小再算距离，
/// 2.2 万行的量级上这比后置过滤省事，也不会出现「取了 8 条过滤完只剩 1 条」。
#[derive(Debug, Clone, Default)]
pub struct DocFilter {
    /// 只看这几类；空 = 不限
    pub kinds: Vec<String>,
    /// 排掉这条贴文（通常是候选自己）
    pub exclude_post_id: Option<String>,
    /// 只看这个品牌
    pub brand: Option<String>,
    /// 只看这个时间点之后的（RFC3339 字符串按字典序比较，UTC 下与时序一致）
    pub since: Option<String>,
    /// 只看 Van 标过范例的
    pub reference_only: bool,
}

#[derive(Debug, Clone)]
pub struct DocHit {
    pub key: String,
    pub kind: String,
    pub doc_id: i64,
    pub post_id: String,
    pub brand: String,
    /// 余弦相似度（1 = 完全一致）。LanceDB 给的是距离，这里换算过了。
    pub similarity: f32,
}

#[derive(Debug, Clone)]
pub struct ImageHit {
    pub blake3: String,
    pub similarity: f32,
}

pub struct VectorStore {
    docs: Table,
    images: Table,
    dim: i32,
    model: String,
    /// 单写者的那把锁。**所有**写路径都要先拿到它。
    write: Mutex<()>,
}

impl VectorStore {
    /// 打开（或新建）向量库。
    ///
    /// `model` 是产出这些向量的模型标识，会与库里钉的那份核对。
    pub async fn open(dir: &Path, dim: i32, model: &str) -> Result<Self> {
        anyhow::ensure!(dim > 0, "向量维度要是正数，给的是 {dim}");
        anyhow::ensure!(!model.trim().is_empty(), "要给出向量模型标识");
        std::fs::create_dir_all(dir).with_context(|| format!("建向量库目录 {}", dir.display()))?;
        check_model_stamp(dir, model)?;

        let uri = dir.to_string_lossy().into_owned();
        let db = lancedb::connect(&uri)
            .execute()
            .await
            .with_context(|| format!("连接向量库 {uri}"))?;
        let existing = db.table_names().execute().await.context("列出表")?;

        let docs = ensure_table(&db, &existing, DOCS, docs_schema(dim)).await?;
        let images = ensure_table(&db, &existing, IMAGES, images_schema(dim)).await?;
        check_dim(&docs, dim).await.context("docs 表")?;
        check_dim(&images, dim).await.context("images 表")?;

        Ok(Self {
            docs,
            images,
            dim,
            model: model.to_string(),
            write: Mutex::new(()),
        })
    }

    pub fn dim(&self) -> i32 {
        self.dim
    }
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 写文档向量。按 `key` **幂等**——重跑同步不会写重，也不会留下旧行。
    pub async fn put_docs(&self, rows: &[DocVector]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        for r in rows {
            anyhow::ensure!(
                r.vector.len() == self.dim as usize,
                "{} 的向量是 {} 维，库是 {} 维",
                r.key,
                r.vector.len(),
                self.dim
            );
        }
        let schema = docs_schema(self.dim);
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from_iter_values(rows.iter().map(|r| &r.key))),
                Arc::new(StringArray::from_iter_values(rows.iter().map(|r| &r.kind))),
                Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.doc_id))),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|r| &r.post_id),
                )),
                Arc::new(StringArray::from_iter_values(rows.iter().map(|r| &r.brand))),
                Arc::new(StringArray::from_iter_values(rows.iter().map(|r| &r.at))),
                Arc::new(BooleanArray::from_iter(
                    rows.iter().map(|r| Some(r.is_reference)),
                )),
                Arc::new(fsl(rows.iter().map(|r| r.vector.clone()), self.dim)),
            ],
        )
        .context("组 docs 批")?;
        self.upsert(&self.docs, batch, schema).await?;
        Ok(rows.len())
    }

    /// 写图片向量。按 blake3 幂等。
    pub async fn put_images(&self, rows: &[ImageVector]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        for r in rows {
            anyhow::ensure!(
                r.vector.len() == self.dim as usize,
                "{} 的向量是 {} 维，库是 {} 维",
                r.blake3,
                r.vector.len(),
                self.dim
            );
        }
        let schema = images_schema(self.dim);
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|r| &r.blake3),
                )),
                Arc::new(fsl(rows.iter().map(|r| r.vector.clone()), self.dim)),
            ],
        )
        .context("组 images 批")?;
        self.upsert(&self.images, batch, schema).await?;
        Ok(rows.len())
    }

    async fn upsert(&self, tbl: &Table, batch: RecordBatch, schema: SchemaRef) -> Result<()> {
        let key = tbl.schema().await?.field(0).name().clone();
        let _guard = self.write.lock().await;
        let mut merge = tbl.merge_insert(&[key.as_str()]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge
            .execute(Box::new(RecordBatchIterator::new(
                vec![Ok(batch)].into_iter(),
                schema,
            )) as Box<dyn RecordBatchReader + Send>)
            .await
            .context("merge_insert")?;
        Ok(())
    }

    /// 找相似文档。
    pub async fn search_docs(
        &self,
        query: &[f32],
        filter: &DocFilter,
        limit: usize,
    ) -> Result<Vec<DocHit>> {
        anyhow::ensure!(
            query.len() == self.dim as usize,
            "查询向量是 {} 维，库是 {} 维",
            query.len(),
            self.dim
        );
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut q = self
            .docs
            .query()
            .nearest_to(query)?
            .distance_type(DistanceType::Cosine)
            .select(Select::Columns(
                ["key", "kind", "doc_id", "post_id", "brand"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ))
            .limit(limit);
        if let Some(pred) = filter.predicate() {
            q = q.only_if(pred);
        }
        let batches = q
            .execute()
            .await
            .context("向量检索")?
            .try_collect::<Vec<_>>()
            .await
            .context("收检索结果")?;

        let mut out = Vec::new();
        for b in &batches {
            let key = strs(b, "key")?;
            let kind = strs(b, "kind")?;
            let post_id = strs(b, "post_id")?;
            let brand = strs(b, "brand")?;
            let doc_id = b
                .column_by_name("doc_id")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>().cloned())
                .context("结果里没有 doc_id 列")?;
            let dist = f32s(b, DIST)?;
            for i in 0..b.num_rows() {
                out.push(DocHit {
                    key: key.value(i).to_string(),
                    kind: kind.value(i).to_string(),
                    doc_id: doc_id.value(i),
                    post_id: post_id.value(i).to_string(),
                    brand: brand.value(i).to_string(),
                    similarity: 1.0 - dist.value(i),
                });
            }
        }
        Ok(out)
    }

    /// 找相似图片。返回的 blake3 拿回 SQLite 才知道归属。
    pub async fn search_images(&self, query: &[f32], limit: usize) -> Result<Vec<ImageHit>> {
        anyhow::ensure!(
            query.len() == self.dim as usize,
            "查询向量是 {} 维，库是 {} 维",
            query.len(),
            self.dim
        );
        if limit == 0 {
            return Ok(Vec::new());
        }
        let batches = self
            .images
            .query()
            .nearest_to(query)?
            .distance_type(DistanceType::Cosine)
            .select(Select::Columns(vec!["blake3".to_string()]))
            .limit(limit)
            .execute()
            .await
            .context("图片检索")?
            .try_collect::<Vec<_>>()
            .await?;
        let mut out = Vec::new();
        for b in &batches {
            let h = strs(b, "blake3")?;
            let dist = f32s(b, DIST)?;
            for i in 0..b.num_rows() {
                out.push(ImageHit {
                    blake3: h.value(i).to_string(),
                    similarity: 1.0 - dist.value(i),
                });
            }
        }
        Ok(out)
    }

    /// 取回一条已存的文档向量。预取轮算过的候选向量，正式一轮靠这个复用，
    /// 省掉再过一遍 GPU。取不到就是没缓存，照常重算。
    pub async fn doc_vector(&self, key: &str) -> Result<Option<Vec<f32>>> {
        let batches = self
            .docs
            .query()
            .only_if(format!("key = {}", lit(key)?))
            .select(Select::Columns(vec!["vector".to_string()]))
            .limit(1)
            .execute()
            .await
            .context("取向量")?
            .try_collect::<Vec<_>>()
            .await?;
        for b in &batches {
            if b.num_rows() == 0 {
                continue;
            }
            let col = b.column_by_name("vector").context("结果里没有 vector 列")?;
            let list = col
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .context("vector 列不是 FixedSizeList")?;
            let inner = list.value(0);
            let f = inner
                .as_any()
                .downcast_ref::<Float32Array>()
                .context("向量元素不是 f32")?;
            return Ok(Some(f.values().to_vec()));
        }
        Ok(None)
    }

    /// 删掉一批候选缓存。预取轮的窗口滚过去之后清旧的用。
    pub async fn drop_docs(&self, keys: &[String]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let list = keys
            .iter()
            .map(|k| lit(k))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let _guard = self.write.lock().await;
        self.docs
            .delete(format!("key IN ({list})").as_str())
            .await
            .context("删候选缓存")?;
        Ok(())
    }

    /// 删一批图片向量（杂志整本清理 / 对账后已无人引用的 blake3）。
    pub async fn drop_images(&self, blake3s: &[String]) -> Result<()> {
        if blake3s.is_empty() {
            return Ok(());
        }
        let list = blake3s
            .iter()
            .map(|k| lit(k))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let _guard = self.write.lock().await;
        self.images
            .delete(format!("blake3 IN ({list})").as_str())
            .await
            .context("删图片向量")?;
        Ok(())
    }

    /// （文档数，图片数）
    pub async fn counts(&self) -> Result<(usize, usize)> {
        Ok((
            self.docs.count_rows(None).await?,
            self.images.count_rows(None).await?,
        ))
    }

    /// 合并碎文件、清旧版本。**夜间跑**：每次写都生成新文件，白天跑会和检索抢 IO。
    pub async fn optimize(&self) -> Result<()> {
        let _guard = self.write.lock().await;
        for (name, tbl) in [(DOCS, &self.docs), (IMAGES, &self.images)] {
            tbl.optimize(OptimizeAction::All)
                .await
                .with_context(|| format!("optimize {name}"))?;
        }
        Ok(())
    }
}

impl DocFilter {
    /// 拼成 LanceDB 的过滤表达式。没有任何条件就返回 None。
    fn predicate(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if !self.kinds.is_empty() {
            let list = self
                .kinds
                .iter()
                .filter_map(|k| lit(k).ok())
                .collect::<Vec<_>>()
                .join(", ");
            if !list.is_empty() {
                parts.push(format!("kind IN ({list})"));
            }
        }
        if let Some(p) = &self.exclude_post_id
            && let Ok(v) = lit(p)
        {
            parts.push(format!("post_id != {v}"));
        }
        if let Some(b) = &self.brand
            && let Ok(v) = lit(b)
        {
            parts.push(format!("brand = {v}"));
        }
        if let Some(s) = &self.since
            && let Ok(v) = lit(s)
        {
            parts.push(format!("at >= {v}"));
        }
        if self.reference_only {
            parts.push("is_reference = true".to_string());
        }
        (!parts.is_empty()).then(|| parts.join(" AND "))
    }
}

/// 把一个值转成 SQL 字面量。
///
/// 过滤条件里的品牌名、post_id 全是从贴文正文与第三方接口来的，**不是可信输入**。
/// 单引号翻倍是 SQL 标准的转义；控制字符直接拒——它们在合法的品牌名里不出现，
/// 出现了就说明数据有问题，宁可报错也不要猜。
fn lit(s: &str) -> Result<String> {
    anyhow::ensure!(
        !s.chars().any(|c| c.is_control()),
        "过滤条件里有控制字符，拒绝拼进查询"
    );
    Ok(format!("'{}'", s.replace('\'', "''")))
}

fn docs_schema(dim: i32) -> SchemaRef {
    // 第 0 列是主键，`upsert` 靠这个约定取 merge 的键
    Arc::new(Schema::new(vec![
        Field::new("key", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("doc_id", DataType::Int64, false),
        Field::new("post_id", DataType::Utf8, false),
        Field::new("brand", DataType::Utf8, false),
        Field::new("at", DataType::Utf8, false),
        Field::new("is_reference", DataType::Boolean, false),
        vector_field(dim),
    ]))
}

fn images_schema(dim: i32) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("blake3", DataType::Utf8, false),
        vector_field(dim),
    ]))
}

fn vector_field(dim: i32) -> Field {
    Field::new(
        "vector",
        DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
        true,
    )
}

fn fsl(vectors: impl Iterator<Item = Vec<f32>>, dim: i32) -> FixedSizeListArray {
    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        vectors.map(|v| Some(v.into_iter().map(Some).collect::<Vec<_>>())),
        dim,
    )
}

fn strs<'a>(b: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .with_context(|| format!("结果里没有 {name} 列（或不是字符串）"))
}

fn f32s<'a>(b: &'a RecordBatch, name: &str) -> Result<&'a Float32Array> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
        .with_context(|| format!("结果里没有 {name} 列（或不是 f32）"))
}

fn stamp_path(dir: &Path) -> PathBuf {
    dir.join(MODEL_STAMP)
}

/// 核对（或写下）建库用的向量模型。见模块文档第三条。
fn check_model_stamp(dir: &Path, model: &str) -> Result<()> {
    let path = stamp_path(dir);
    match std::fs::read_to_string(&path) {
        Ok(old) if old.trim() == model.trim() => Ok(()),
        Ok(old) => anyhow::bail!(
            "这个向量库是用 {} 建的，现在要用 {}。\
             不同模型的向量不可比，混进去检索会悄悄失准。\
             要换模型请先删掉 {} 重建。",
            old.trim(),
            model.trim(),
            dir.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(&path, model.trim()).with_context(|| format!("写 {}", path.display()))
        }
        Err(e) => Err(e).with_context(|| format!("读 {}", path.display())),
    }
}

async fn ensure_table(
    db: &lancedb::Connection,
    existing: &[String],
    name: &str,
    schema: SchemaRef,
) -> Result<Table> {
    if existing.iter().any(|n| n == name) {
        db.open_table(name)
            .execute()
            .await
            .with_context(|| format!("打开表 {name}"))
    } else {
        db.create_empty_table(name, schema)
            .execute()
            .await
            .with_context(|| format!("建表 {name}"))
    }
}

/// 老库的维度与现在要用的对不上，就直说。模型换了但 `EMBED_MODEL` 被手动改过时
/// 这是第二道闸。
async fn check_dim(tbl: &Table, dim: i32) -> Result<()> {
    let schema = tbl.schema().await.context("读表结构")?;
    let field = schema
        .field_with_name("vector")
        .context("表里没有 vector 列")?;
    match field.data_type() {
        DataType::FixedSizeList(_, n) if *n == dim => Ok(()),
        DataType::FixedSizeList(_, n) => {
            anyhow::bail!("库里是 {n} 维，要用的是 {dim} 维；换维度要重建向量库")
        }
        other => anyhow::bail!("vector 列的类型是 {other}，应当是 FixedSizeList"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIM: i32 = 8;

    /// 第 i 维为 1、其余为 0.01 的向量：彼此可判别，余弦距离不会是 NaN
    fn v(i: usize) -> Vec<f32> {
        let mut x = vec![0.01_f32; DIM as usize];
        x[i % DIM as usize] = 1.0;
        x
    }

    fn doc(key: &str, kind: &str, post: &str, brand: &str, i: usize) -> DocVector {
        DocVector {
            key: key.into(),
            kind: kind.into(),
            doc_id: i as i64,
            post_id: post.into(),
            brand: brand.into(),
            at: "2026-09-18T00:00:00Z".into(),
            is_reference: kind == "example",
            vector: v(i),
        }
    }

    async fn store(dir: &Path) -> VectorStore {
        VectorStore::open(dir, DIM, "qwen3-vl-test").await.unwrap()
    }

    /// `VectorStore` 没有 `Debug`（里面有把锁），`unwrap_err()` 用不了。
    /// 另外要用 `{:#}` 而不是 `to_string()`——后者只给最外层那句 context，
    /// 真正说明原因的那句在链条里面。
    async fn open_err(dir: &Path, dim: i32, model: &str) -> String {
        match VectorStore::open(dir, dim, model).await {
            Ok(_) => panic!("本该拒绝打开"),
            Err(e) => format!("{e:#}"),
        }
    }

    #[test]
    fn 过滤条件里的单引号要转义() {
        // 品牌名带撇号是真事：`笑's -SHO 's`。不转义就是一条注入
        assert_eq!(lit("笑's").unwrap(), "'笑''s'");
        assert_eq!(lit("a' OR 1=1 --").unwrap(), "'a'' OR 1=1 --'");
        assert_eq!(lit("and wander").unwrap(), "'and wander'");
    }

    #[test]
    fn 控制字符直接拒绝不猜() {
        assert!(lit("换\n行").is_err());
        assert!(lit("空\0字节").is_err());
    }

    #[test]
    fn 空条件不拼出where() {
        assert_eq!(DocFilter::default().predicate(), None);
    }

    #[test]
    fn 条件按与拼接() {
        let f = DocFilter {
            kinds: vec!["example".into(), "decision".into()],
            exclude_post_id: Some("abc".into()),
            reference_only: true,
            ..Default::default()
        };
        let p = f.predicate().unwrap();
        assert!(p.contains("kind IN ('example', 'decision')"), "{p}");
        assert!(p.contains("post_id != 'abc'"), "{p}");
        assert!(p.contains("is_reference = true"), "{p}");
        assert_eq!(p.matches(" AND ").count(), 2);
    }

    #[tokio::test]
    async fn 写入检索与幂等() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        let s = store(dir.path()).await;

        let rows = vec![
            doc("kb:example:1", "example", "p1", "山と道", 1),
            doc(
                "kb:published_item:2",
                "published_item",
                "p2",
                "and wander",
                2,
            ),
            doc("cand:snowpeak-ab12cd", "candidate", "p3", "Snow Peak", 3),
        ];
        assert_eq!(s.put_docs(&rows).await.unwrap(), 3);
        assert_eq!(s.counts().await.unwrap().0, 3);

        // 同样的键再写一遍是更新不是追加
        assert_eq!(s.put_docs(&rows).await.unwrap(), 3);
        assert_eq!(s.counts().await.unwrap().0, 3, "merge_insert 应当按键去重");

        // 查第 2 条的形状，它该排第一
        let hits = s
            .search_docs(&v(2), &DocFilter::default(), 3)
            .await
            .unwrap();
        assert_eq!(hits[0].key, "kb:published_item:2");
        assert!(
            hits[0].similarity > 0.99,
            "命中自己应当接近 1：{}",
            hits[0].similarity
        );
        assert_eq!(hits[0].brand, "and wander");
        assert_eq!(hits[0].doc_id, 2);
    }

    #[tokio::test]
    async fn 前置过滤只看指定的几类() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        let s = store(dir.path()).await;
        s.put_docs(&[
            doc("kb:example:1", "example", "p1", "山と道", 1),
            doc("cand:x", "candidate", "p1", "山と道", 1),
        ])
        .await
        .unwrap();

        // 不限类：两条都在
        let all = s
            .search_docs(&v(1), &DocFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        // 只看参考库：候选缓存不该混进来
        let f = DocFilter {
            kinds: vec!["example".into()],
            ..Default::default()
        };
        let only = s.search_docs(&v(1), &f, 10).await.unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].kind, "example");

        // 排掉同一条贴文之后什么都不剩——这正是「别拿自己跟自己比」的用法
        let f = DocFilter {
            exclude_post_id: Some("p1".into()),
            ..Default::default()
        };
        assert!(s.search_docs(&v(1), &f, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn 图片按内容哈希存一份() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        let s = store(dir.path()).await;
        // 同一张图被两条候选转载：哈希相同，向量库里只该有一行
        let h = "b3aaaa".to_string();
        s.put_images(&[ImageVector {
            blake3: h.clone(),
            vector: v(4),
        }])
        .await
        .unwrap();
        s.put_images(&[ImageVector {
            blake3: h.clone(),
            vector: v(4),
        }])
        .await
        .unwrap();
        s.put_images(&[ImageVector {
            blake3: "b3bbbb".into(),
            vector: v(5),
        }])
        .await
        .unwrap();
        assert_eq!(s.counts().await.unwrap().1, 2);

        let hits = s.search_images(&v(4), 1).await.unwrap();
        assert_eq!(hits[0].blake3, h);
        assert!(hits[0].similarity > 0.99);
    }

    #[tokio::test]
    async fn 候选向量能取回来复用() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        let s = store(dir.path()).await;
        s.put_docs(&[doc("cand:k1", "candidate", "p1", "", 3)])
            .await
            .unwrap();

        let got = s.doc_vector("cand:k1").await.unwrap().expect("该取得到");
        assert_eq!(got, v(3), "取回来的要和存进去的一模一样");
        assert!(s.doc_vector("cand:没存过").await.unwrap().is_none());

        s.drop_docs(&["cand:k1".to_string()]).await.unwrap();
        assert!(s.doc_vector("cand:k1").await.unwrap().is_none());
        assert_eq!(s.counts().await.unwrap().0, 0);
    }

    #[tokio::test]
    async fn 维度不符当场报错不写进去() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        let s = store(dir.path()).await;
        let mut bad = doc("kb:example:1", "example", "p1", "", 1);
        bad.vector.truncate(4);
        let err = s.put_docs(&[bad]).await.unwrap_err().to_string();
        assert!(err.contains("4 维"), "{err}");
        assert_eq!(s.counts().await.unwrap().0, 0);

        assert!(
            s.search_docs(&[1.0, 2.0], &DocFilter::default(), 1)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn 换了向量模型必须重建() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        {
            let s = store(dir.path()).await;
            s.put_docs(&[doc("kb:example:1", "example", "p1", "", 1)])
                .await
                .unwrap();
        }
        // 同一个模型：照常打开，数据还在
        let again = store(dir.path()).await;
        assert_eq!(again.counts().await.unwrap().0, 1);
        drop(again);

        // 换了模型：必须拒绝。混进去检索会悄悄失准，这是最难发现的一类故障
        let err = open_err(dir.path(), DIM, "别的模型").await;
        assert!(err.contains("重建"), "{err}");
    }

    #[tokio::test]
    async fn 换了维度也要拒绝() {
        let dir = tempdir::TempDir::new("vec").unwrap();
        drop(store(dir.path()).await);
        // 模型标识没变但维度变了（比如手改过 EMBED_MODEL）：第二道闸要拦住
        let err = open_err(dir.path(), DIM * 2, "qwen3-vl-test").await;
        assert!(err.contains("重建向量库"), "{err}");
    }
}
