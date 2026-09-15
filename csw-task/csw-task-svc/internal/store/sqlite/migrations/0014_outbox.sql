-- +goose Up
-- 事件接续：引擎在写事件的同一事务里写 outbox 行（需要通知的事件才写），notifier 轮询发送到编辑部群。
-- event_id 唯一 → 同一事件最多一行、只发一次；飞书侧再用 uuid=csw-outbox-{event_id} 去重兜底。
-- target_role / cc_roles 由引擎按闸与角色推导，请求方不能指定。
CREATE TABLE outbox (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id       INTEGER NOT NULL UNIQUE REFERENCES events(id) ON DELETE CASCADE,
  event_type     TEXT NOT NULL,
  run_id         INTEGER REFERENCES runs(id) ON DELETE CASCADE,
  task_id        INTEGER,
  deliverable_id INTEGER,
  channel        TEXT NOT NULL DEFAULT 'lark_group',
  target_role    TEXT,
  cc_roles       TEXT,
  payload_json   TEXT NOT NULL DEFAULT '{}',
  status         TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','sent','dead')),
  attempts       INTEGER NOT NULL DEFAULT 0,
  next_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  claimed_at     TEXT,
  sent_at        TEXT,
  message_id     TEXT,
  last_error     TEXT,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_outbox_due ON outbox(status, next_at);

-- 进度卡去重：只在状态类字段变化时发进度卡。
ALTER TABLE runs ADD COLUMN progress_digest TEXT;

-- +goose Down
ALTER TABLE runs DROP COLUMN progress_digest;
DROP TABLE IF EXISTS outbox;
