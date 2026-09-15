-- tasks 重建（一次带齐后续需要的列，避免反复重建）：
-- · status 增加 failed（执行者报告无法完成）/ cancelled（中枢取消）；
-- · item_key（条目键，非条目级任务为空串），唯一约束改为 (run_id, stage_code, item_key)；
-- · fail_reason、last_activity_at（接单心跳 / 提交）、overdue_notified_at（逾期只提醒一次）、rework_pending（上游补件待返工）。
-- sqlite 改 CHECK / UNIQUE 只能重建表：NO TRANSACTION 以便临时关外键，表重建步骤包在显式事务里，中途失败整体回滚。

-- +goose NO TRANSACTION

-- +goose Up
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE tasks_new (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id        INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  stage_code    TEXT NOT NULL,
  item_key      TEXT NOT NULL DEFAULT '',
  stage_name    TEXT NOT NULL,
  seq           INTEGER NOT NULL,
  role_code     TEXT NOT NULL,
  is_merge      INTEGER NOT NULL DEFAULT 0 CHECK (is_merge IN (0,1)),
  output_type   TEXT,
  instructions        TEXT,
  self_check_criteria TEXT,
  acceptance          TEXT,
  assignee_id   INTEGER REFERENCES agents(id),
  status        TEXT NOT NULL DEFAULT 'blocked'
                CHECK (status IN ('blocked','ready','dispatched','in_progress','review','passed','returned','failed','cancelled')),
  cur_version   INTEGER NOT NULL DEFAULT 0,
  dispatch_mode TEXT,
  action_class  TEXT NOT NULL DEFAULT 'read',
  sla_minutes   INTEGER,
  rework_pending INTEGER NOT NULL DEFAULT 0 CHECK (rework_pending IN (0,1)),
  fail_reason   TEXT,
  dispatched_at TEXT,
  started_at    TEXT,
  completed_at  TEXT,
  due_at        TEXT,
  last_activity_at    TEXT,
  overdue_notified_at TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, stage_code, item_key)
);
INSERT INTO tasks_new
  (id, run_id, stage_code, stage_name, seq, role_code, is_merge, output_type,
   instructions, self_check_criteria, acceptance, assignee_id, status, cur_version,
   dispatch_mode, action_class, sla_minutes, dispatched_at, started_at, completed_at, due_at, created_at, updated_at)
SELECT id, run_id, stage_code, stage_name, seq, role_code, is_merge, output_type,
   instructions, self_check_criteria, acceptance, assignee_id, status, cur_version,
   dispatch_mode, action_class, sla_minutes, dispatched_at, started_at, completed_at, due_at, created_at, updated_at
FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;
CREATE INDEX idx_tasks_assignee_status ON tasks(assignee_id, status);
CREATE INDEX idx_tasks_run             ON tasks(run_id, seq);
CREATE INDEX idx_tasks_due             ON tasks(status, due_at);
COMMIT;
PRAGMA foreign_keys=ON;

-- +goose Down
-- 回到 0010 形态：failed→returned、cancelled→blocked，条目级任务（item_key 非空）丢弃。
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE tasks_old (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id        INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  stage_code    TEXT NOT NULL,
  stage_name    TEXT NOT NULL,
  seq           INTEGER NOT NULL,
  role_code     TEXT NOT NULL,
  is_merge      INTEGER NOT NULL DEFAULT 0 CHECK (is_merge IN (0,1)),
  instructions        TEXT,
  self_check_criteria TEXT,
  acceptance          TEXT,
  assignee_id   INTEGER REFERENCES agents(id),
  status        TEXT NOT NULL DEFAULT 'blocked'
                CHECK (status IN ('blocked','ready','dispatched','in_progress','review','passed','returned')),
  cur_version   INTEGER NOT NULL DEFAULT 0,
  dispatched_at TEXT,
  started_at    TEXT,
  completed_at  TEXT,
  due_at        TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  output_type   TEXT,
  dispatch_mode TEXT,
  action_class  TEXT NOT NULL DEFAULT 'read',
  sla_minutes   INTEGER,
  UNIQUE(run_id, stage_code)
);
INSERT INTO tasks_old
  (id, run_id, stage_code, stage_name, seq, role_code, is_merge, instructions, self_check_criteria, acceptance,
   assignee_id, status, cur_version, dispatched_at, started_at, completed_at, due_at, created_at, updated_at,
   output_type, dispatch_mode, action_class, sla_minutes)
SELECT id, run_id, stage_code, stage_name, seq, role_code, is_merge, instructions, self_check_criteria, acceptance,
   assignee_id, CASE status WHEN 'failed' THEN 'returned' WHEN 'cancelled' THEN 'blocked' ELSE status END,
   cur_version, dispatched_at, started_at, completed_at, due_at, created_at, updated_at,
   output_type, dispatch_mode, action_class, sla_minutes
FROM tasks WHERE item_key = '';
DROP TABLE tasks;
ALTER TABLE tasks_old RENAME TO tasks;
CREATE INDEX idx_tasks_assignee_status ON tasks(assignee_id, status);
CREATE INDEX idx_tasks_run             ON tasks(run_id, seq);
COMMIT;
PRAGMA foreign_keys=ON;
