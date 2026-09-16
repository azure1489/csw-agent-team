-- +goose Up
-- run 级停滞检测：现有三类接续告警（未接单 / 未接单升级 / 接单后无活动）都挂在单个任务上，
-- 前提是任务处于 dispatched 或 in_progress。中枢报完失败后，任务是 failed（终态）、下游是 blocked，
-- 整个 run 里一个活动任务都没有——于是 run 还是 active，却无人在动，三类告警一条都不会触发。
-- r44 实测：08:51 报失败后引擎沉默 47 分钟，直到人工介入收口。
-- 这一列用于「同一次停滞只提醒一次」，任务级用 tasks 上的 *_notified_at，run 级用这里。
ALTER TABLE runs ADD COLUMN stalled_notified_at TEXT;

-- +goose Down
ALTER TABLE runs DROP COLUMN stalled_notified_at;
