//! 高清图与来源库 640 图的**画面校验**。
//!
//! 工作台手里没有来源库那张图的 Instagram 文件名（csw 接口不返回），只能按轮播序号把
//! hires-service 取到的高清图和来源库的 640 图对应起来。序号对应在三处数据上核过是一致的
//! （来源库入库顺序、工作台 media.ordinal、原帖 embed 顺序），但帖子被编辑过（换图、加图）
//! 时序号会错位——所以每一对都再比一次画面：把两张图都缩成 9×8 灰度算差分哈希（dHash），
//! 64 位里差 ≤ [`MAX_DISTANCE`] 位算同一张。
//!
//! Bright Data 的 640 版有时是**居中裁成正方形**的（地址带 `c0.135.1080.1080a`），
//! 这时整图比对不上，再拿高清图的中央正方形比一次。两次都不过才算不匹配。

use image::{DynamicImage, GenericImageView, imageops::FilterType};

/// 64 位差分哈希允许的最大汉明距离。同一张图经过缩放 / 重编码通常在 0–6 位之间，
/// 不同画面一般在 20 位以上；10 留了余量又不会把别的图放进来。
pub const MAX_DISTANCE: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// 整图比对通过
    Same { distance: u32 },
    /// 640 图是居中方裁，用高清图的中央正方形比对通过
    SameCropped { distance: u32 },
    /// 两种比法都不过：很可能帖子被编辑过、顺序变了
    Different { full: u32, cropped: u32 },
    /// 有一张解不出来，无法校验
    #[cfg_attr(not(test), allow(dead_code))]
    Undecodable(String),
}

impl Match {
    pub fn ok(&self) -> bool {
        matches!(self, Match::Same { .. } | Match::SameCropped { .. })
    }

    /// 写进交付包的一句话
    pub fn cn(&self) -> String {
        match self {
            Match::Same { distance } => format!("序号对应，画面校验通过（dHash 差 {distance} 位）"),
            Match::SameCropped { distance } => format!(
                "序号对应，画面校验通过（来源库为居中方裁版，按中央正方形比对，dHash 差 {distance} 位）"
            ),
            Match::Different { full, cropped } => format!(
                "序号对应，但画面校验**未通过**（整图差 {full} 位、方裁差 {cropped} 位，阈值 {MAX_DISTANCE}）"
            ),
            Match::Undecodable(why) => format!("画面校验无法进行：{why}"),
        }
    }
}

/// 一张高清图的指纹：整图与中央正方形各一个 dHash。解一次码，配对时反复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prints {
    pub full: u64,
    pub center: u64,
}

/// 高清图的指纹。
pub fn prints(hi: &[u8]) -> Result<Prints, String> {
    let img = image::load_from_memory(hi).map_err(|e| format!("高清图解码失败：{e}"))?;
    Ok(Prints {
        full: dhash(&img),
        center: dhash(&center_square(&img)),
    })
}

/// 来源库图的指纹（只要整图）。
pub fn lo_print(lo: &[u8]) -> Result<u64, String> {
    image::load_from_memory(lo)
        .map(|i| dhash(&i))
        .map_err(|e| format!("来源库图解码失败：{e}"))
}

/// 用指纹比：规则与 [`verify`] 相同（整图过即同，不过再比中央正方形）。
pub fn compare(hi: &Prints, lo: u64) -> Match {
    let full = distance(hi.full, lo);
    if full <= MAX_DISTANCE {
        return Match::Same { distance: full };
    }
    let cropped = distance(hi.center, lo);
    if cropped <= MAX_DISTANCE {
        return Match::SameCropped { distance: cropped };
    }
    Match::Different { full, cropped }
}

impl Match {
    /// 配对时比远近用：通过的取它的差，不通过的排最后
    pub fn rank(&self) -> u32 {
        match self {
            Match::Same { distance } | Match::SameCropped { distance } => *distance,
            _ => u32::MAX,
        }
    }
}

/// 高清图（`hi`）与来源库图（`lo`）是不是同一张。生产走 [`prints`] + [`compare`]，这里留作测试对照。
#[cfg(test)]
pub fn verify(hi: &[u8], lo: &[u8]) -> Match {
    let hi_img = match image::load_from_memory(hi) {
        Ok(i) => i,
        Err(e) => return Match::Undecodable(format!("高清图解码失败：{e}")),
    };
    let lo_img = match image::load_from_memory(lo) {
        Ok(i) => i,
        Err(e) => return Match::Undecodable(format!("来源库图解码失败：{e}")),
    };
    let lo_hash = dhash(&lo_img);
    let full = distance(dhash(&hi_img), lo_hash);
    if full <= MAX_DISTANCE {
        return Match::Same { distance: full };
    }
    let cropped = distance(dhash(&center_square(&hi_img)), lo_hash);
    if cropped <= MAX_DISTANCE {
        return Match::SameCropped { distance: cropped };
    }
    Match::Different { full, cropped }
}

/// 差分哈希：缩到 9×8 灰度，每行相邻像素比大小，64 位。
pub fn dhash(img: &DynamicImage) -> u64 {
    let g = img.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h <<= 1;
            if g.get_pixel(x, y)[0] > g.get_pixel(x + 1, y)[0] {
                h |= 1;
            }
        }
    }
    h
}

pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

fn center_square(img: &DynamicImage) -> DynamicImage {
    let (w, h) = img.dimensions();
    let side = w.min(h);
    img.crop_imm((w - side) / 2, (h - side) / 2, side, side)
}

/// 解出来的像素尺寸，解不出来是 None。
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::load_from_memory(bytes).ok().map(|i| i.dimensions())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    /// 一张有结构的假图：斜向渐变 + 一个亮块 + 一个暗块，缩小后仍分得出来
    fn picture(w: u32, h: u32, seed: u8) -> DynamicImage {
        let img = ImageBuffer::from_fn(w, h, |x, y| {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let mut v = (fx * 160.0 + fy * 80.0) as u8;
            if fx > 0.2 && fx < 0.45 && fy > 0.3 && fy < 0.6 {
                v = v.saturating_add(90);
            }
            if fx > 0.6 && fx < 0.9 && fy > 0.1 && fy < 0.35 {
                v = v.saturating_sub(90);
            }
            Rgb([v, v.wrapping_add(seed), v / 2])
        });
        DynamicImage::ImageRgb8(img)
    }

    fn jpeg(img: &DynamicImage) -> Vec<u8> {
        let mut b = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Jpeg)
            .unwrap();
        b
    }

    #[test]
    fn 同一张图缩到640再重编码还是同一张() {
        let hi = picture(1440, 1800, 0);
        let lo = hi.resize_exact(512, 640, FilterType::Triangle);
        match verify(&jpeg(&hi), &jpeg(&lo)) {
            Match::Same { distance } => assert!(distance <= 4, "差 {distance} 位"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 来源库是居中方裁版也认得出() {
        let hi = picture(1080, 1350, 0);
        let side = 1080;
        let lo = hi.crop_imm(0, (1350 - side) / 2, side, side).resize_exact(
            640,
            640,
            FilterType::Triangle,
        );
        match verify(&jpeg(&hi), &jpeg(&lo)) {
            Match::SameCropped { .. } | Match::Same { .. } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 不同画面不会被当成同一张() {
        let a = picture(1440, 1800, 0);
        // 左右翻转：同样的色调分布，不同的画面
        let b = a.fliph().resize_exact(512, 640, FilterType::Triangle);
        let m = verify(&jpeg(&a), &jpeg(&b));
        assert!(!m.ok(), "{m:?}");
    }

    #[test]
    fn 指纹比与直接比结论一致() {
        let hi = picture(1440, 1800, 0);
        let lo = hi.resize_exact(512, 640, FilterType::Triangle);
        let other = hi.fliph().resize_exact(512, 640, FilterType::Triangle);
        let p = prints(&jpeg(&hi)).unwrap();
        assert_eq!(
            compare(&p, lo_print(&jpeg(&lo)).unwrap()),
            verify(&jpeg(&hi), &jpeg(&lo))
        );
        assert!(!compare(&p, lo_print(&jpeg(&other)).unwrap()).ok());
        assert!(compare(&p, lo_print(&jpeg(&other)).unwrap()).rank() == u32::MAX);
    }

    #[test]
    fn 解不出来的说清是哪张() {
        let a = picture(100, 100, 0);
        assert!(
            matches!(verify(b"not an image", &jpeg(&a)), Match::Undecodable(s) if s.contains("高清图"))
        );
        assert!(
            matches!(verify(&jpeg(&a), b"nope"), Match::Undecodable(s) if s.contains("来源库"))
        );
        assert_eq!(dimensions(&jpeg(&a)), Some((100, 100)));
        assert_eq!(dimensions(b"nope"), None);
    }
}
