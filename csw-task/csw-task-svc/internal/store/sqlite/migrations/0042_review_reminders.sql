-- +goose Up
-- 待审提醒：交付物停在某道闸上没人审，按 15 / 30 / 60 分钟、之后每小时提醒审核方，
-- 提醒三次仍无动作升级到 Van（夜间不升级）。
-- 09-28 r56 #592：v5 17:06 交上去时主编正忙着退回 v4，这条 @ 并进了那一轮被漏掉，
-- 之后八小时没人动——逾期 / 未接单 / 无活动三类告警都不看「待审」，整期停滞一期只报一次。
-- 按「交付物 × 闸」记：交新版本或过了一道闸，就是新的一次等待，从头计。
CREATE TABLE review_reminders (
  deliverable_id   INTEGER NOT NULL REFERENCES deliverables(id) ON DELETE CASCADE,
  gate_order       INTEGER NOT NULL,
  reminded         INTEGER NOT NULL DEFAULT 0,
  last_reminded_at TEXT,
  escalated_at     TEXT,
  PRIMARY KEY (deliverable_id, gate_order)
);

-- +goose Down
DROP TABLE review_reminders;
