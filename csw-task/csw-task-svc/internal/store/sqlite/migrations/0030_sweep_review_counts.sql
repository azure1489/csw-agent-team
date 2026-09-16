-- +goose Up
-- r44 暴露的口径问题：found 记的是「接口返回数」，被当成了「审阅量」。
-- 某一轮 csw MCP 返回 100 篇原帖、只有 13 篇登记进引擎，其余 87 篇的逐条处理情况在引擎里查不到——
-- 既不能算「已审后淘汰」（没有审阅记录），也不该算「漏登记」（本就没打算逐条审）。
-- 所以把「拿到多少」和「看了多少」拆开记：获取层与审阅层各自有数，判断层仍以条目与轨迹为准。
ALTER TABLE intake_sweeps ADD COLUMN fetched_unique INTEGER NOT NULL DEFAULT 0;
ALTER TABLE intake_sweeps ADD COLUMN reviewed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE intake_sweeps ADD COLUMN unreviewed INTEGER NOT NULL DEFAULT 0;

-- +goose Down
ALTER TABLE intake_sweeps DROP COLUMN unreviewed;
ALTER TABLE intake_sweeps DROP COLUMN reviewed;
ALTER TABLE intake_sweeps DROP COLUMN fetched_unique;
