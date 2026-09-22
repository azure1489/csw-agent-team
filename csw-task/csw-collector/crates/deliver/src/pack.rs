//! 确定性打包。**同样的内容，打两次，字节一模一样。**
//!
//! 为什么要这样：提交是幂等的，幂等键从 zip 的 sha256 派生。如果同样的内容
//! 每次打出来的哈希都不同，重试就会变成「换了个键重新提交一遍」——
//! 引擎看到的是两份不同的交付物，而人看到的是两条一模一样的记录。
//!
//! 所以下面每一个非确定性的来源都被钉死了：
//!
//! | 来源 | 怎么钉的 |
//! |---|---|
//! | 目录遍历顺序 | 按路径**字节序**排，不按文件系统给的顺序 |
//! | 修改时间 | 固定为 ZIP 纪元 1980-01-01，不用 `now()` |
//! | 权限位 | 一律 0o644 |
//! | 压缩级别 | 显式指定，不用「默认」——默认值会随版本变 |
//! | 图片压缩 | 用 Stored 不压缩。JPEG/PNG 本来就压过了，再压既慢又可能随版本变 |
//! | `index.md` 里的时间 | 由调用方给，不许写 `now()` |
//!
//! 最后一条不在这个文件里，但它同样关键，所以记在这里：**任何进 zip 的内容
//! 都不能含当前时间**，否则上面六条全白做。

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use sha2::Digest;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

/// zip 上限。引擎是 64 MiB，留出余量。
pub const MAX_ZIP_BYTES: u64 = 48 * 1024 * 1024;

/// 固定的压缩级别。**不要用 None（「默认」）**——默认值会随 flate2 版本变，
/// 那会让同一份内容在升级依赖之后打出不同的哈希。
const DEFLATE_LEVEL: i64 = 6;

/// zip 里的一份文件。`path` 是**相对交付物根目录**的路径，一律用 `/` 分隔。
pub struct Entry {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl Entry {
    pub fn text(path: &str, s: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            bytes: s.into().into_bytes(),
        }
    }
    pub fn binary(path: &str, bytes: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub path: std::path::PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

impl Built {
    /// 提交用的幂等键。**一旦写下就不许换**——引擎的 `Idempotency-Key`
    /// 只对 POST 生效，请求中途崩溃则同键永久 409，换键会造成重复提交。
    pub fn idem_key(&self, task_id: i64) -> String {
        csw_collector_engineapi::client::submit_idem_key(task_id, &self.sha256)
    }
}

/// 打包。`root` 是 zip 里那个唯一的顶层目录名（与 zip 文件同名）。
pub fn build(root: &str, entries: &[Entry], out: &Path) -> Result<Built> {
    anyhow::ensure!(!root.trim().is_empty(), "交付物根目录名不能空");
    for e in entries {
        anyhow::ensure!(!e.path.starts_with('/'), "{} 不能是绝对路径", e.path);
        anyhow::ensure!(
            !e.path.split('/').any(|seg| seg == ".." || seg.is_empty()),
            "{} 里有 .. 或空路径段",
            e.path
        );
    }
    // 按路径的**字节序**排。文件系统给的顺序不确定，语言环境相关的排序也不确定。
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    if let Some(dup) = sorted.windows(2).find(|w| w[0].path == w[1].path) {
        anyhow::bail!("重复的路径 {}", dup[0].path);
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = Vec::new();
    {
        let mut zw = ZipWriter::new(std::io::Cursor::new(&mut buf));
        // ZIP 的时间字段从 1980 起算，给不了更早的
        let epoch = DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
            .map_err(|e| anyhow::anyhow!("固定时间戳：{e:?}"))?;
        for e in &sorted {
            let opts = SimpleFileOptions::default()
                .last_modified_time(epoch)
                .unix_permissions(0o644)
                .large_file(false)
                .compression_method(method_for(&e.path))
                .compression_level(if method_for(&e.path) == CompressionMethod::Deflated {
                    Some(DEFLATE_LEVEL)
                } else {
                    None
                });
            zw.start_file(format!("{root}/{}", e.path), opts)
                .with_context(|| format!("写 {}", e.path))?;
            zw.write_all(&e.bytes)?;
        }
        zw.finish().context("收尾 zip")?;
    }

    let size = buf.len() as u64;
    anyhow::ensure!(
        size <= MAX_ZIP_BYTES,
        "交付物 {size} 字节，超过上限 {MAX_ZIP_BYTES}",
    );
    let sha = hex(&sha2::Sha256::digest(&buf));
    std::fs::write(out, &buf).with_context(|| format!("落盘 {}", out.display()))?;
    Ok(Built {
        path: out.to_path_buf(),
        sha256: sha,
        bytes: size,
    })
}

/// 已经压过的格式用 Stored。再压一遍既慢，压缩结果还可能随 flate2 版本变。
fn method_for(path: &str) -> CompressionMethod {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "zip" | "mp4" | "woff2"
    ) {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Entry> {
        vec![
            Entry::text("index.md", "# 标题\n正文\n"),
            Entry::binary("images/a.jpg", vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]),
            Entry::text("trace/items.jsonl", "{\"a\":1}\n{\"a\":2}\n"),
        ]
    }

    #[test]
    fn 打两次哈希一模一样() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        let a = build("交付物", &sample(), &dir.path().join("a.zip")).unwrap();
        // 中间隔一会儿：如果哪里偷偷用了 now()，这里就会露馅
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = build("交付物", &sample(), &dir.path().join("b.zip")).unwrap();
        assert_eq!(a.sha256, b.sha256, "同样的内容打两次必须字节一致");
        assert_eq!(a.bytes, b.bytes);
    }

    #[test]
    fn 给进去的顺序不影响结果() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        let mut reversed = sample();
        reversed.reverse();
        let a = build("交付物", &sample(), &dir.path().join("a.zip")).unwrap();
        let b = build("交付物", &reversed, &dir.path().join("b.zip")).unwrap();
        // 文件系统给的遍历顺序不确定，所以内部要按字节序排
        assert_eq!(a.sha256, b.sha256);
    }

    #[test]
    fn 内容变了哈希就变() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        let a = build("交付物", &sample(), &dir.path().join("a.zip")).unwrap();
        let mut other = sample();
        other[0] = Entry::text("index.md", "# 标题\n改过的正文\n");
        let b = build("交付物", &other, &dir.path().join("b.zip")).unwrap();
        assert_ne!(a.sha256, b.sha256);
    }

    #[test]
    fn 根目录名进哈希() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        let a = build("甲", &sample(), &dir.path().join("a.zip")).unwrap();
        let b = build("乙", &sample(), &dir.path().join("b.zip")).unwrap();
        assert_ne!(a.sha256, b.sha256);
    }

    #[test]
    fn 图片不压文本压() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        // 一段高度可压缩的内容：文本该被压小，图片该原样存
        let big = "a".repeat(10000);
        let text_zip = build(
            "r",
            &[Entry::text("x.md", big.clone())],
            &dir.path().join("t.zip"),
        )
        .unwrap();
        let img_zip = build(
            "r",
            &[Entry::binary("x.jpg", big.into_bytes())],
            &dir.path().join("i.zip"),
        )
        .unwrap();
        assert!(text_zip.bytes < 1000, "文本该被压：{}", text_zip.bytes);
        assert!(img_zip.bytes > 10000, "图片该原样存：{}", img_zip.bytes);
    }

    #[test]
    fn 路径穿越直接拒绝() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        for bad in [
            "../外面.md",
            "/绝对路径.md",
            "images//空段.jpg",
            "a/../../b",
        ] {
            let r = build("r", &[Entry::text(bad, "x")], &dir.path().join("x.zip"));
            assert!(r.is_err(), "{bad} 该被拒绝");
        }
    }

    #[test]
    fn 重复路径直接拒绝() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        let r = build(
            "r",
            &[Entry::text("a.md", "1"), Entry::text("a.md", "2")],
            &dir.path().join("x.zip"),
        );
        // 静默留下后写的那份，会让「交付物里到底是哪一版」无从查起
        assert!(r.is_err());
    }

    #[test]
    fn 超过上限要报错而不是发出去() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        // 不可压缩的随机字节，且用 Stored
        let big: Vec<u8> = (0..MAX_ZIP_BYTES + 1024)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        let r = build(
            "r",
            &[Entry::binary("big.jpg", big)],
            &dir.path().join("x.zip"),
        );
        assert!(r.unwrap_err().to_string().contains("超过上限"));
        assert!(!dir.path().join("x.zip").exists(), "超限就不该落盘");
    }

    #[test]
    fn 幂等键从哈希派生() {
        let b = Built {
            path: "x".into(),
            sha256: "0123456789abcdef0123456789abcdef".into(),
            bytes: 1,
        };
        assert_eq!(b.idem_key(42), "submit-42-0123456789abcdef");
    }

    #[test]
    fn 打出来的zip能被读回来() {
        let dir = tempdir::TempDir::new("pack").unwrap();
        let out = dir.path().join("a.zip");
        build("交付物", &sample(), &out).unwrap();
        let f = std::fs::File::open(&out).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "交付物/images/a.jpg",
                "交付物/index.md",
                "交付物/trace/items.jsonl"
            ]
        );
    }
}
