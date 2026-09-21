//! 图片下载：按内容哈希存盘、天然去重、并发受限、失败标「未识别」。
//!
//! 两条口径直接体现在这里：
//! - **每张图都识别**——所以下载失败不是「跳过这条候选」，而是这张图标 `failed`，
//!   候选照样往下走，只是它的 `image_seen` 会是 false、落待核。
//! - 缩略宽度走 **OSS 的 `image/resize`**，不在本地解码：少一份图像库依赖，也省带宽。
//!   05 / 11 取原图时把宽度设成 0 即可。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// 按内容哈希存盘的根目录
    pub dir: PathBuf,
    /// 缩略宽度；0 表示取原图（05 / 11 用）
    pub width: u32,
    pub concurrency: usize,
    pub timeout: Duration,
    pub max_attempts: u32,
}

#[derive(Debug, Clone)]
pub struct Downloaded {
    pub url: String,
    pub blake3: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// 这次是真从网上拉的，还是本地已有
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
pub struct DownloadFailure {
    pub url: String,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct DownloadReport {
    pub ok: Vec<Downloaded>,
    pub failed: Vec<DownloadFailure>,
}

impl DownloadReport {
    /// 按 url 查结果，写回候选的 `MediaRef.blake3` 用。
    pub fn by_url(&self) -> HashMap<&str, &Downloaded> {
        self.ok.iter().map(|d| (d.url.as_str(), d)).collect()
    }
}

/// 给 OSS 直链加上缩略参数。
///
/// 只对带 `x-oss-process` 之外的链接加；已经带了处理参数的原样返回，
/// 免得把别人拼好的参数覆盖掉。
pub fn sized_url(url: &str, width: u32) -> String {
    if width == 0 || url.contains("x-oss-process") {
        return url.to_string();
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}x-oss-process=image/resize,w_{width}/format,jpg/quality,q_80")
}

/// 内容哈希的存放路径：`<dir>/ab/cd/<full>.jpg`。
///
/// 分两级目录是因为一期一千多张、三个月就十几万个文件，
/// 平铺在一个目录里 `ls` 都会卡。
pub fn blob_path(dir: &Path, hash: &str) -> PathBuf {
    let (a, b) = (&hash[0..2], &hash[2..4]);
    dir.join(a).join(b).join(format!("{hash}.jpg"))
}

/// 临时文件名的序号。只为区分同进程内的并发写，不需要跨进程唯一。
fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

pub struct Downloader {
    cfg: DownloadConfig,
    http: reqwest::Client,
    gate: Arc<Semaphore>,
}

impl Downloader {
    pub fn new(cfg: DownloadConfig) -> Result<Self> {
        csw_collector_core::ensure_crypto_provider();
        let http = reqwest::Client::builder().timeout(cfg.timeout).build()?;
        let gate = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
        Ok(Self { cfg, http, gate })
    }

    /// 批量下载。**不会因为个别失败而整批失败**——失败的进 `failed`，成功的照常返回。
    pub async fn fetch_all(&self, urls: &[String]) -> DownloadReport {
        // 同一条候选里同一张图可能出现两次，先去重再下
        let mut uniq: Vec<&String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for u in urls {
            if seen.insert(u.as_str()) {
                uniq.push(u);
            }
        }
        let tasks = uniq.into_iter().map(|u| self.fetch_one(u.clone()));
        let results = futures::future::join_all(tasks).await;
        let mut report = DownloadReport::default();
        for r in results {
            match r {
                Ok(d) => report.ok.push(d),
                Err((url, e)) => report.failed.push(DownloadFailure { url, error: e }),
            }
        }
        report
    }

    async fn fetch_one(&self, url: String) -> Result<Downloaded, (String, String)> {
        let _permit = self
            .gate
            .acquire()
            .await
            .map_err(|e| (url.clone(), e.to_string()))?;
        let target = sized_url(&url, self.cfg.width);
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.try_once(&url, &target).await {
                Ok(d) => return Ok(d),
                Err(e) if attempt < self.cfg.max_attempts => {
                    tracing::debug!(url = %url, 第几次 = attempt, 原因 = %e, "下载失败，退避重试");
                    tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt))).await;
                }
                Err(e) => return Err((url, format!("{e:#}"))),
            }
        }
    }

    async fn try_once(&self, orig: &str, target: &str) -> Result<Downloaded> {
        let resp = self.http.get(target).send().await.context("发起请求")?;
        let status = resp.status();
        anyhow::ensure!(status.is_success(), "返回 {status}");
        let bytes = resp.bytes().await.context("读响应体")?;
        anyhow::ensure!(!bytes.is_empty(), "响应体为空");
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let path = blob_path(&self.cfg.dir, &hash);
        // 内容哈希一样就是同一张图，不必重写
        let from_cache = path.exists();
        if !from_cache {
            if let Some(d) = path.parent() {
                tokio::fs::create_dir_all(d).await.ok();
            }
            // 先写临时文件再改名：中途崩了不会留下半张图冒充完整的。
            //
            // 临时名必须**每次唯一**：跨账号转载同一张图很常见（一期一千多张图里不少是转发），
            // 两条候选并发下到同一份内容时哈希相同、目标路径相同；若临时名也相同，
            // 先完成的那个把文件改走，后一个的 rename 就会 ENOENT。
            let tmp = path.with_extension(format!("{}.{}.part", std::process::id(), next_seq()));
            tokio::fs::write(&tmp, &bytes)
                .await
                .with_context(|| format!("写 {}", tmp.display()))?;
            // 目标已存在也没关系：内容哈希相同就是同一张图，rename 覆盖掉即可
            if let Err(e) = tokio::fs::rename(&tmp, &path).await {
                let _ = tokio::fs::remove_file(&tmp).await;
                // 别人已经放好了同一份，就用别人那份
                if !path.exists() {
                    return Err(e).with_context(|| format!("改名到 {}", path.display()));
                }
            }
        }
        Ok(Downloaded {
            url: orig.to_string(),
            blake3: hash,
            path,
            bytes: bytes.len() as u64,
            from_cache,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 递归列出目录下的全部文件。只为断言「没留下临时文件」。
    fn walkdir(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    #[test]
    fn 缩略参数只在没带处理参数时加() {
        assert_eq!(
            sized_url("https://oss/x.jpg", 768),
            "https://oss/x.jpg?x-oss-process=image/resize,w_768/format,jpg/quality,q_80"
        );
        assert_eq!(
            sized_url("https://oss/x.jpg?a=1", 768),
            "https://oss/x.jpg?a=1&x-oss-process=image/resize,w_768/format,jpg/quality,q_80"
        );
        // 已经带了处理参数的不动：别覆盖别人拼好的
        let already = "https://oss/x.jpg?x-oss-process=image/resize,w_200";
        assert_eq!(sized_url(already, 768), already);
        // 宽度 0 = 取原图（05 / 11）
        assert_eq!(sized_url("https://oss/x.jpg", 0), "https://oss/x.jpg");
    }

    #[test]
    fn 存放路径按哈希分两级() {
        let p = blob_path(Path::new("/data/blobs"), "abcdef0123456789");
        assert_eq!(p, Path::new("/data/blobs/ab/cd/abcdef0123456789.jpg"));
    }

    #[tokio::test]
    async fn 个别失败不拖垮整批且内容相同只存一份() {
        let srv = wiremock::MockServer::start().await;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/a.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"IMAGE-ONE".to_vec()))
            .mount(&srv)
            .await;
        // b 与 a 内容相同：哈希一样，只该存一份
        Mock::given(method("GET"))
            .and(path("/b.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"IMAGE-ONE".to_vec()))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .and(path("/bad.jpg"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&srv)
            .await;

        let dir = std::env::temp_dir().join(format!("csw-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = Downloader::new(DownloadConfig {
            dir: dir.clone(),
            width: 768,
            concurrency: 4,
            timeout: Duration::from_secs(5),
            max_attempts: 1,
        })
        .unwrap();

        let urls = vec![
            format!("{}/a.jpg", srv.uri()),
            format!("{}/b.jpg", srv.uri()),
            format!("{}/bad.jpg", srv.uri()),
            format!("{}/a.jpg", srv.uri()), // 重复的 url 先去重
        ];
        let rep = d.fetch_all(&urls).await;
        assert_eq!(rep.ok.len(), 2, "两个成功；失败的是 {:?}", rep.failed);
        assert_eq!(rep.failed.len(), 1, "一个失败不该拖垮整批");
        assert!(rep.failed[0].url.ends_with("bad.jpg"));
        assert_eq!(rep.ok[0].blake3, rep.ok[1].blake3, "内容相同哈希就该相同");
        assert_eq!(rep.ok[0].path, rep.ok[1].path, "同一份内容只存一个文件");

        // 再下一次全是命中本地
        let rep2 = d.fetch_all(&urls[..1]).await;
        assert!(rep2.ok[0].from_cache, "第二次应当命中本地");

        // 目录里只该有一个 .jpg，且没有残留的 .part
        let mut jpgs = 0;
        let mut parts = 0;
        for e in walkdir(&dir) {
            match e.extension().and_then(|s| s.to_str()) {
                Some("jpg") => jpgs += 1,
                Some("part") => parts += 1,
                _ => {}
            }
        }
        assert_eq!(jpgs, 1, "同一份内容只该落一个文件");
        assert_eq!(parts, 0, "不该留下临时文件");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
