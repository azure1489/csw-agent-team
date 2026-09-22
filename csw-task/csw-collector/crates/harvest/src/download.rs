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

/// 一张图最多收这么多字节。
///
/// Instagram 的原图极少超过 10 MiB，缩略更小。设这个上限不是为了省流量，
/// 是因为**响应体一次读进内存**：并发 8 的时候，一个地址指向一份大文件
/// 就能把服务顶到 `MemoryMax` 上去，而那一轮的其余一千多张图跟着一起没了。
/// 一张图取不到只是这条候选落待核，整轮被拖垮是另一回事。
pub const DEFAULT_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// 按内容哈希存盘的根目录
    pub dir: PathBuf,
    /// 缩略宽度；0 表示取原图（05 / 11 用）
    pub width: u32,
    pub concurrency: usize,
    pub timeout: Duration,
    pub max_attempts: u32,
    /// 单张上限，见 [`DEFAULT_MAX_BYTES`]
    pub max_bytes: u64,
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
    /// 挂上之后回放模式下不再去拉图。见 [`Self::with_recorder`]。
    rec: Option<Arc<csw_collector_core::record::Recorder>>,
}

impl Downloader {
    pub fn new(cfg: DownloadConfig) -> Result<Self> {
        csw_collector_core::ensure_crypto_provider();
        let http = reqwest::Client::builder().timeout(cfg.timeout).build()?;
        let gate = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
        Ok(Self {
            cfg,
            http,
            gate,
            rec: None,
        })
    }

    /// 挂上录制回放层。
    ///
    /// # 夹具里只有 URL → 哈希，图片本身还在 blob 目录里
    ///
    /// 下载是**先取回字节才算得出哈希**的，所以 blob 缓存挡不住出网：
    /// 不问一次就不知道该找哪个文件。夹具补上的正是这一跳。
    ///
    /// 这意味着**夹具和 blob 目录是一对**，blob 被清掉（图片有 120 天清理）
    /// 回放就会失败，报错里会说清是哪张。这比静默去拉图强——
    /// 那样回放就又出网了。
    pub fn with_recorder(mut self, rec: Arc<csw_collector_core::record::Recorder>) -> Self {
        self.rec = Some(rec);
        self
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
        if let Some(rec) = self.rec.clone() {
            return self
                .fetch_recorded(&rec, &url)
                .await
                .map_err(|e| (url.clone(), format!("{e:#}")));
        }
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

    /// 走夹具的那条路。录制时把 URL → 哈希记下来，回放时照着从 blob 读。
    async fn fetch_recorded(
        &self,
        rec: &Arc<csw_collector_core::record::Recorder>,
        url: &str,
    ) -> Result<Downloaded> {
        let req = serde_json::json!({"url": url, "width": self.cfg.width});
        let v = rec
            .wrap("image", &req, || async {
                let target = sized_url(url, self.cfg.width);
                let d = self.try_once(url, &target).await?;
                Ok(serde_json::json!({"blake3": d.blake3, "bytes": d.bytes}))
            })
            .await?;
        let hash = v
            .get("blake3")
            .and_then(|x| x.as_str())
            .context("夹具里没有 blake3")?;
        let path = blob_path(&self.cfg.dir, hash);
        anyhow::ensure!(
            path.exists(),
            "回放要的图不在了：{}（{url}）。夹具与 blob 目录是一对，\
             blob 被清理过就得重录——这里不会去拉图，那样回放就又出网了。",
            path.display()
        );
        Ok(Downloaded {
            url: url.to_string(),
            blake3: hash.to_string(),
            bytes: v.get("bytes").and_then(|x| x.as_u64()).unwrap_or(0),
            path,
            from_cache: true,
        })
    }

    async fn try_once(&self, orig: &str, target: &str) -> Result<Downloaded> {
        let mut resp = self.http.get(target).send().await.context("发起请求")?;
        let status = resp.status();
        anyhow::ensure!(status.is_success(), "返回 {status}");
        let cap = self.cfg.max_bytes;
        // 先看它自报多大，能省掉一次白读
        if let Some(n) = resp.content_length() {
            anyhow::ensure!(n <= cap, "说自己有 {n} 字节，超过单张上限 {cap}");
        }
        // 但 Content-Length 可以撒谎，也可以干脆不给（chunked），所以边读边数。
        // 超了立刻断开：已经读进来的那部分扔掉，不进内存更不落盘。
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await.context("读响应体")? {
            anyhow::ensure!(
                bytes.len() as u64 + chunk.len() as u64 <= cap,
                "读到 {} 字节还没完，超过单张上限 {cap}",
                bytes.len() + chunk.len()
            );
            bytes.extend_from_slice(&chunk);
        }
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

    /// 超大的那张要被挡在内存外面，同一批里其余的照常下完。
    ///
    /// 两种都要挡：老老实实报 Content-Length 的，和什么都不报（chunked）却一直发的。
    /// 后者才是真正危险的那种——只看 Content-Length 会放它进来。
    #[tokio::test]
    async fn 超过上限的那张不会读进内存() {
        let srv = wiremock::MockServer::start().await;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/small.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .mount(&srv)
            .await;
        // 自报 3000 字节，上限 1000
        Mock::given(method("GET"))
            .and(path("/big.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8; 3000]))
            .mount(&srv)
            .await;
        // 不报长度，分块一直发
        Mock::given(method("GET"))
            .and(path("/endless.jpg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![9u8; 5000])
                    .append_header("transfer-encoding", "chunked"),
            )
            .mount(&srv)
            .await;

        let dir = std::env::temp_dir().join(format!("csw-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = Downloader::new(DownloadConfig {
            dir: dir.clone(),
            width: 768,
            concurrency: 4,
            timeout: Duration::from_secs(5),
            max_attempts: 1,
            max_bytes: 1000,
        })
        .unwrap();
        let rep = d
            .fetch_all(&[
                format!("{}/small.jpg", srv.uri()),
                format!("{}/big.jpg", srv.uri()),
                format!("{}/endless.jpg", srv.uri()),
            ])
            .await;
        assert_eq!(rep.ok.len(), 1, "只该下来那张小的");
        assert!(rep.ok[0].url.ends_with("small.jpg"));
        assert_eq!(rep.failed.len(), 2, "两张超限的都该挡下：{:?}", rep.failed);
        for f in &rep.failed {
            assert!(f.error.contains("上限"), "{} 的理由是 {}", f.url, f.error);
        }
        // 超限的一个字节都不该落盘
        assert_eq!(walkdir(&dir).len(), 1, "超限的那两张不该留下文件");
        let _ = std::fs::remove_dir_all(&dir);
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
            max_bytes: DEFAULT_MAX_BYTES,
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
    /// 回放模式下**一次图也不该去拉**。
    ///
    /// 这条测试的做法：录一遍，然后把 mock 服务器关掉再回放。
    /// 真去出网的话连接会被拒，测试就红——比断言「调了几次」结实。
    #[tokio::test]
    async fn 回放下不再去拉图() {
        use csw_collector_core::record::{Mode, Recorder};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let base = std::env::temp_dir().join(format!("csw-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (blob, fixtures) = (base.join("blob"), base.join("fixtures"));
        std::fs::create_dir_all(&blob).unwrap();
        std::fs::create_dir_all(&fixtures).unwrap();
        let body = b"pretend-this-is-a-jpeg".to_vec();

        let url = {
            let srv = wiremock::MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/a.jpg"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
                .mount(&srv)
                .await;
            let url = format!("{}/a.jpg", srv.uri());

            let rec = Arc::new(Recorder::new(Mode::Record, &fixtures));
            let d = dl(&blob).with_recorder(rec);
            let rep = d.fetch_all(std::slice::from_ref(&url)).await;
            assert_eq!(rep.failed.len(), 0, "{:?}", rep.failed);
            assert_eq!(rep.ok[0].blake3, blake3::hash(&body).to_hex().to_string());
            url
            // srv 在这儿析构：下面再请求就是连不上
        };

        let rec = Arc::new(Recorder::new(Mode::Replay, &fixtures));
        let d = dl(&blob).with_recorder(rec);
        let rep = d.fetch_all(std::slice::from_ref(&url)).await;
        assert_eq!(rep.failed.len(), 0, "服务器已关，还能成功才说明真没出网");
        assert_eq!(rep.ok[0].blake3, blake3::hash(&body).to_hex().to_string());
        assert!(rep.ok[0].from_cache);

        // blob 被清掉就必须报错，不能偷偷去拉
        std::fs::remove_file(&rep.ok[0].path).unwrap();
        let rec2 = Arc::new(Recorder::new(Mode::Replay, &fixtures));
        let d2 = dl(&blob).with_recorder(rec2);
        let rep2 = d2.fetch_all(&[url]).await;
        assert_eq!(rep2.ok.len(), 0);
        assert!(
            rep2.failed[0].error.contains("回放要的图不在了"),
            "{}",
            rep2.failed[0].error
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    fn dl(dir: &Path) -> Downloader {
        Downloader::new(DownloadConfig {
            dir: dir.to_path_buf(),
            width: 0,
            concurrency: 2,
            timeout: Duration::from_secs(5),
            max_attempts: 1,
            max_bytes: DEFAULT_MAX_BYTES,
        })
        .unwrap()
    }
}
