//! 交付物生成与提交。
//!
//! 形态：`index.md`（YAML 元信息头 + 正文内联）+ `images/`（每条一张预览）
//! + `trace/`（`sweeps.jsonl`、`items.jsonl`），打包成同名 zip 一步式提交。
//!
//! **zip 只构建一次。** 字节、sha、幂等键、表单字段先落 `deliverables_local` 与 outbox 再发，
//! 重试只发已落盘的那一份。遇 `idempotency_in_progress_or_uncertain` 不换键，先查任务状态对账。
//!
//! 确定性生成是第二道防线：路径按字节序、mtime 与权限固定、JPEG 用 Stored、
//! 文本固定 Deflate 级别、JSONL 键序固定、`index.md` 不写当前时间。
//! 有一条测试专门断言「构建两次哈希相同」。
