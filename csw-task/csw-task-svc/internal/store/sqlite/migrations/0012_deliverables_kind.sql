-- deliverables 重建：is_dispatch 布尔升级为 kind 四类（保留 is_dispatch 兼容读）：
-- · dispatch 派工单 · output 执行者产出 · supplement 补件（挂在已通过任务上，不重开审核）· edit 主编定点编辑版本；
-- · 新增 affects_deliverable_id（补件影响的产出）、edit_of / diff_summary / collab（定点编辑留痕）；
-- · status 加 superseded（被定点编辑版本取代的首稿，原样保留）；
-- · 表级 UNIQUE(task_id,is_dispatch,version) 改为按版本流拆分的三条部分唯一索引（产出与编辑共用一条版本流）。

-- +goose NO TRANSACTION

-- +goose Up
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE deliverables_new (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id      INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL DEFAULT 'output' CHECK (kind IN ('dispatch','output','supplement','edit')),
  is_dispatch  INTEGER NOT NULL DEFAULT 0 CHECK (is_dispatch IN (0,1)),
  version      INTEGER NOT NULL,
  doc_type     TEXT NOT NULL,
  producer_id  INTEGER REFERENCES agents(id),
  file_id      INTEGER REFERENCES files(id),
  download_url TEXT,
  filename     TEXT,
  title        TEXT,
  summary      TEXT,
  meta_json    TEXT,
  self_check   TEXT,
  editor_note  TEXT,
  affects_deliverable_id INTEGER REFERENCES deliverables(id),
  edit_of      INTEGER,
  diff_summary TEXT,
  collab       INTEGER NOT NULL DEFAULT 0 CHECK (collab IN (0,1)),
  cur_gate         INTEGER NOT NULL DEFAULT 0,
  returned_at_gate INTEGER,
  status       TEXT NOT NULL DEFAULT 'submitted'
               CHECK (status IN ('issued','submitted','in_review','passed','returned','superseded')),
  submitted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
INSERT INTO deliverables_new
  (id, task_id, kind, is_dispatch, version, doc_type, producer_id, file_id, download_url, filename, title, summary,
   meta_json, self_check, editor_note, cur_gate, returned_at_gate, status, submitted_at, created_at)
SELECT id, task_id, CASE is_dispatch WHEN 1 THEN 'dispatch' ELSE 'output' END, is_dispatch, version, doc_type,
   producer_id, file_id, download_url, filename, title, summary,
   meta_json, self_check, editor_note, cur_gate, returned_at_gate, status, submitted_at, created_at
FROM deliverables;
DROP TABLE deliverables;
ALTER TABLE deliverables_new RENAME TO deliverables;
CREATE INDEX idx_deliverables_task ON deliverables(task_id);
CREATE INDEX idx_deliverables_affects ON deliverables(affects_deliverable_id) WHERE affects_deliverable_id IS NOT NULL;
CREATE UNIQUE INDEX uq_deliv_dispatch   ON deliverables(task_id, version) WHERE kind = 'dispatch';
CREATE UNIQUE INDEX uq_deliv_output     ON deliverables(task_id, version) WHERE kind IN ('output','edit');
CREATE UNIQUE INDEX uq_deliv_supplement ON deliverables(task_id, version) WHERE kind = 'supplement';
COMMIT;
PRAGMA foreign_keys=ON;

-- +goose Down
-- 回到 0011 形态：补件行删除，编辑版本当作产出，被取代的首稿当作退回。
PRAGMA foreign_keys=OFF;
BEGIN;
DELETE FROM deliverable_upstreams WHERE deliverable_id IN (SELECT id FROM deliverables WHERE kind='supplement');
DELETE FROM deliverables WHERE kind='supplement';
CREATE TABLE deliverables_old (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id      INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  is_dispatch  INTEGER NOT NULL DEFAULT 0 CHECK (is_dispatch IN (0,1)),
  version      INTEGER NOT NULL,
  doc_type     TEXT NOT NULL,
  producer_id  INTEGER REFERENCES agents(id),
  file_id      INTEGER REFERENCES files(id),
  download_url TEXT,
  filename     TEXT,
  title        TEXT,
  summary      TEXT,
  meta_json    TEXT,
  self_check   TEXT,
  editor_note  TEXT,
  cur_gate         INTEGER NOT NULL DEFAULT 0,
  returned_at_gate INTEGER,
  status       TEXT NOT NULL DEFAULT 'submitted'
               CHECK (status IN ('issued','submitted','in_review','passed','returned')),
  submitted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(task_id, is_dispatch, version)
);
INSERT INTO deliverables_old
  (id, task_id, is_dispatch, version, doc_type, producer_id, file_id, download_url, filename, title, summary,
   meta_json, self_check, editor_note, cur_gate, returned_at_gate, status, submitted_at, created_at)
SELECT id, task_id, is_dispatch, version, doc_type, producer_id, file_id, download_url, filename, title, summary,
   meta_json, self_check, editor_note, cur_gate, returned_at_gate,
   CASE status WHEN 'superseded' THEN 'returned' ELSE status END, submitted_at, created_at
FROM deliverables;
DROP TABLE deliverables;
ALTER TABLE deliverables_old RENAME TO deliverables;
CREATE INDEX idx_deliverables_task ON deliverables(task_id);
COMMIT;
PRAGMA foreign_keys=ON;
