//! 采集：取候选 → 下载图 → 识别 → 向量化。
//!
//! 「采集媒体信息」在本系统里是一件事，不是四件：取候选、下载、识别、向量化一体完成，
//! 任何一步缺失都算这一条没采到。
//!
//! 内置采集器五个：csw 贴文窗口、csw 单条、Van 链接、小红书、网页/RSS；
//! 另有自定义采集器三层（外部命令 / HTTP / MCP），**默认关闭**，仅 superadmin 可登记。
//!
//! 只取图文的判据是严格的：`contentType ∈ {Image, Carousel}` **且** `mediaList` 里没有 Video。

pub mod csw;
pub mod download;
pub mod recognize;
