-- +goose Up
-- 阶段级接续告警：派工后多少分钟仍未接单提醒执行者（超过 3 倍时升级中枢处置），接单后多少分钟没有心跳或产物提醒一次。
-- NULL = 用引擎默认（接单 5 分钟、无活动 10 分钟）。任务在触发时快照阈值；三类提醒各记一次「已提醒」时间，
-- 同一次停滞只提醒一次（重新派工清接单标记，心跳 / 提交清无活动标记）。
ALTER TABLE workflow_stages ADD COLUMN ack_minutes INTEGER;
ALTER TABLE workflow_stages ADD COLUMN idle_minutes INTEGER;
ALTER TABLE tasks ADD COLUMN ack_minutes INTEGER;
ALTER TABLE tasks ADD COLUMN idle_minutes INTEGER;
ALTER TABLE tasks ADD COLUMN ack_notified_at TEXT;
ALTER TABLE tasks ADD COLUMN ack_escalated_at TEXT;
ALTER TABLE tasks ADD COLUMN idle_notified_at TEXT;

-- +goose Down
ALTER TABLE tasks DROP COLUMN idle_notified_at;
ALTER TABLE tasks DROP COLUMN ack_escalated_at;
ALTER TABLE tasks DROP COLUMN ack_notified_at;
ALTER TABLE tasks DROP COLUMN idle_minutes;
ALTER TABLE tasks DROP COLUMN ack_minutes;
ALTER TABLE workflow_stages DROP COLUMN idle_minutes;
ALTER TABLE workflow_stages DROP COLUMN ack_minutes;
