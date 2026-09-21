-- +goose Up
-- run_items 加两个溯源列。
--
-- origin：这条是从哪个入口进来的。派单跑出来的、Van 自己补的链接、补件追加的，
--   三者在台账上看着一样，但意义完全不同——Van 当期采用的 7 篇全部来自她自己补的链接，
--   这件事现在在库里查不出来。
-- first_seen_at：这条候选第一次被看到的时间。窗口按首次入库时间算，
--   同一条贴文跨天仍在窗口内时要能认出「昨天已判」，靠的就是它。
--
-- 只加列不改 CHECK，所以用 ALTER 就够，不必整表重建。
-- 新列一律追加在末尾：itemCols 与 scanItem 的顺序必须严格一致（见 repo_items.go 开头的注释）。
-- 旧行留 NULL，不回填假数据——分不清的就让它分不清，别编。
ALTER TABLE run_items ADD COLUMN origin TEXT;
ALTER TABLE run_items ADD COLUMN first_seen_at TEXT;
CREATE INDEX idx_run_items_origin ON run_items(run_id, origin);

-- +goose Down
DROP INDEX IF EXISTS idx_run_items_origin;
ALTER TABLE run_items DROP COLUMN first_seen_at;
ALTER TABLE run_items DROP COLUMN origin;
