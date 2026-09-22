//! 合并、对照、逐条判断。
//!
//! 口径（总方案已定，改前先改总方案）：
//! - **每条都判，每张图都识别**；`image_seen=false` 一律落 `pending_check`，不算判过。
//! - 对照材料五类**缺一不判**：正式已发布、范例、生成过文章的贴文、03 决定、上一轮台账。
//! - **不打数字分、不设权重**。输出 = 结论档（推荐 / 备选 / 不推荐 / 待核）
//!   + 六维各自「成立 / 不成立 / 不明」配依据 + 三句话 + 对照结论。
//! - 点赞、评论、标签、话题是**输入**，不是维度。
//!
//! Jev 只做初评、核对与窄判断；深核交给 `deepcheck`。

pub mod check;
pub mod materials;
pub mod merge;
pub mod rubric;
pub mod triage;
pub mod verdict;
