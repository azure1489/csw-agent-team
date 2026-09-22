//! 磁盘水位。图与 zip 都堆在本地盘上，盘满时的表现极难看懂：
//! 下载一半失败、SQLite 写不进去、zip 打到一半断掉，每一处报的都是别的错。
//!
//! # 为什么用 `df` 而不是引个库
//!
//! 为一个百分比引一棵 libc 绑定树，就多一份「开发机编得过、目标机跑不动」的风险
//! （交叉编译那一关已经吃过几次亏）。`df -Pk` 的输出格式是 POSIX 规定的，
//! Linux 与 macOS 一致，**第五列就是使用率**。
//!
//! # 取不到就当不知道，不拦路
//!
//! `df` 不在、输出变了、容器里没权限——这些都不该让一整轮不跑。
//! 取不到时返回 `None`，上层按「不知道」处理：照常开轮，只在日志里说一声。

use std::path::Path;

/// 这块盘用掉了百分之几。取不到返回 `None`。
pub fn used_pct(path: &Path) -> Option<u8> {
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_df(&String::from_utf8_lossy(&out.stdout))
}

/// 从 `df -Pk` 的输出里切出使用率。
///
/// 只认**第二行**：第一行是表头，再往下是 `df` 一次给多块盘时的事，
/// 而我们只问了一个路径。
fn parse_df(out: &str) -> Option<u8> {
    let line = out.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    // Filesystem 1024-blocks Used Available Capacity Mounted-on
    let pct = cols.get(4)?.trim_end_matches('%');
    pct.parse::<u8>().ok()
}

/// 水位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 够用，或者压根没问出来
    Ok,
    /// 该有人去看了
    Warn,
    /// 不许再开新的轮次
    Block,
}

impl Level {
    pub fn blocked(self) -> bool {
        self == Level::Block
    }
}

/// 按两条水位判。**取不到当 `Ok`**——不知道不等于满了。
pub fn level(pct: Option<u8>, warn_pct: u8, block_pct: u8) -> Level {
    match pct {
        Some(p) if p >= block_pct => Level::Block,
        Some(p) if p >= warn_pct => Level::Warn,
        _ => Level::Ok,
    }
}

/// 一句给日志与 `/healthz` 用的人话。
pub fn note(pct: Option<u8>, warn_pct: u8, block_pct: u8) -> String {
    match (pct, level(pct, warn_pct, block_pct)) {
        (Some(p), Level::Block) => format!("磁盘 {p}%，超过拒开新轮的水位 {block_pct}%"),
        (Some(p), Level::Warn) => format!("磁盘 {p}%，超过告警水位 {warn_pct}%"),
        (Some(p), Level::Ok) => format!("磁盘 {p}%"),
        (None, _) => "磁盘用量问不出来（df 不在或输出不认识），按不知道处理".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 两种系统的df都认得() {
        // macOS
        let mac = "Filesystem   1024-blocks      Used Available Capacity  Mounted on\n\
                   /dev/disk3s5   482797652 422191400  12172980    98%    /System/Volumes/Data\n";
        assert_eq!(parse_df(mac), Some(98));
        // Linux
        let linux = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                     /dev/vda1         41151808  21344256  17890816      55% /\n";
        assert_eq!(parse_df(linux), Some(55));
    }

    #[test]
    fn 认不出来就说不知道不瞎猜() {
        assert_eq!(parse_df(""), None);
        assert_eq!(parse_df("只有表头一行\n"), None);
        assert_eq!(parse_df("头\na b c d 不是数字 e\n"), None);
        // 列数不够
        assert_eq!(parse_df("头\na b c\n"), None);
    }

    #[test]
    fn 水位按两条线判() {
        assert_eq!(level(Some(50), 88, 92), Level::Ok);
        assert_eq!(level(Some(88), 88, 92), Level::Warn, "到线就算");
        assert_eq!(level(Some(91), 88, 92), Level::Warn);
        assert_eq!(level(Some(92), 88, 92), Level::Block, "到线就算");
        assert_eq!(level(Some(99), 88, 92), Level::Block);
        // 不知道不等于满了：df 不在、容器里没权限，都不该让一整轮不跑
        assert_eq!(level(None, 88, 92), Level::Ok);
        assert!(!level(None, 88, 92).blocked());
    }

    #[test]
    fn 那句人话把数字带上() {
        assert!(note(Some(93), 88, 92).contains("93%"));
        assert!(note(Some(93), 88, 92).contains("92%"));
        assert!(note(None, 88, 92).contains("不知道"));
    }

    #[test]
    fn 真问一次不报错() {
        // 问不出来也算过：这个测试只保证它不 panic、不卡住
        let _ = used_pct(std::path::Path::new("."));
    }
}
