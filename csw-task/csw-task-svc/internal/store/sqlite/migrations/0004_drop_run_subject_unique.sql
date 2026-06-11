-- 放宽 runs 的 (workflow_id, subject) 唯一约束：允许同一工作流同 subject（如同一天）触发多个实例，
-- 靠 run_id 区分（交付物路径含 run_id 作任务ID）。sqlite 不支持 DROP CONSTRAINT，需重建表。
-- 用 NO TRANSACTION 以便临时关闭外键（tasks/events 外键引用 runs）。

-- +goose NO TRANSACTION

-- +goose Up
PRAGMA foreign_keys=OFF;
CREATE TABLE runs_new (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  workflow_id  INTEGER NOT NULL REFERENCES workflows(id),
  workflow_ver INTEGER NOT NULL,
  subject      TEXT NOT NULL,
  title        TEXT,
  status       TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','done','paused','aborted')),
  created_by   INTEGER REFERENCES agents(id),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
INSERT INTO runs_new (id, workflow_id, workflow_ver, subject, title, status, created_by, created_at, updated_at)
  SELECT id, workflow_id, workflow_ver, subject, title, status, created_by, created_at, updated_at FROM runs;
DROP TABLE runs;
ALTER TABLE runs_new RENAME TO runs;
PRAGMA foreign_keys=ON;

-- +goose Down
PRAGMA foreign_keys=OFF;
CREATE TABLE runs_old (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  workflow_id  INTEGER NOT NULL REFERENCES workflows(id),
  workflow_ver INTEGER NOT NULL,
  subject      TEXT NOT NULL,
  title        TEXT,
  status       TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','done','paused','aborted')),
  created_by   INTEGER REFERENCES agents(id),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(workflow_id, subject)
);
INSERT INTO runs_old (id, workflow_id, workflow_ver, subject, title, status, created_by, created_at, updated_at)
  SELECT id, workflow_id, workflow_ver, subject, title, status, created_by, created_at, updated_at FROM runs;
DROP TABLE runs;
ALTER TABLE runs_old RENAME TO runs;
PRAGMA foreign_keys=ON;
