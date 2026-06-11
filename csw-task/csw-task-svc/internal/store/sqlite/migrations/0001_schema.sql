-- +goose Up
-- 通用工作流引擎 schema（对应《任务流转服务·设计》§3 全部 20 表 + §8 约束硬化）。

-- ───────────── 通用层 ─────────────
CREATE TABLE roles (
  code          TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  is_management INTEGER NOT NULL DEFAULT 0 CHECK (is_management IN (0,1)),
  is_human      INTEGER NOT NULL DEFAULT 0 CHECK (is_human IN (0,1))
);

CREATE TABLE agents (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  role_code  TEXT NOT NULL REFERENCES roles(code),
  name       TEXT NOT NULL,
  active     INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE agent_tokens (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  agent_id     INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  token_hash   TEXT NOT NULL UNIQUE,
  label        TEXT,
  last_used_at TEXT,
  expires_at   TEXT,
  revoked      INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0,1)),
  created_by   INTEGER REFERENCES admin_users(id),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_agent_tokens_agent ON agent_tokens(agent_id);

-- ───────────── 定义层 ─────────────
CREATE TABLE workflows (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  wf_key        TEXT NOT NULL,
  name          TEXT NOT NULL,
  version       INTEGER NOT NULL DEFAULT 1,
  hub_role_code TEXT REFERENCES roles(code),
  dispatch_mode TEXT NOT NULL DEFAULT 'manual' CHECK (dispatch_mode IN ('manual','auto')),
  trigger_roles TEXT,
  common_instructions TEXT,
  common_acceptance   TEXT,
  status        TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','active','archived')),
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(wf_key, version)
);

CREATE TABLE workflow_stages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  workflow_id INTEGER NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,
  code        TEXT NOT NULL,
  name        TEXT NOT NULL,
  role_code   TEXT NOT NULL REFERENCES roles(code),
  output_type TEXT,
  instructions        TEXT,
  self_check_criteria TEXT,
  acceptance          TEXT,
  is_merge    INTEGER NOT NULL DEFAULT 0 CHECK (is_merge IN (0,1)),
  UNIQUE(workflow_id, code),
  UNIQUE(workflow_id, seq)
);

CREATE TABLE workflow_stage_deps (
  workflow_id   INTEGER NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
  stage_id      INTEGER NOT NULL REFERENCES workflow_stages(id) ON DELETE CASCADE,
  depends_on_id INTEGER NOT NULL REFERENCES workflow_stages(id) ON DELETE CASCADE,
  PRIMARY KEY (stage_id, depends_on_id)
);

CREATE TABLE workflow_gates (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  workflow_id    INTEGER NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
  stage_id       INTEGER REFERENCES workflow_stages(id) ON DELETE CASCADE,
  gate_order     INTEGER NOT NULL,
  reviewer_role  TEXT NOT NULL REFERENCES roles(code),
  relayed_by_hub INTEGER NOT NULL DEFAULT 0 CHECK (relayed_by_hub IN (0,1)),
  name           TEXT
);

-- ───────────── 实例层 ─────────────
CREATE TABLE runs (
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

CREATE TABLE tasks (
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
  UNIQUE(run_id, stage_code)
);

CREATE TABLE task_deps (
  task_id       INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  depends_on_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  PRIMARY KEY (task_id, depends_on_id)
);

CREATE TABLE task_gates (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id        INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  gate_order     INTEGER NOT NULL,
  reviewer_role  TEXT NOT NULL,
  relayed_by_hub INTEGER NOT NULL DEFAULT 0 CHECK (relayed_by_hub IN (0,1)),
  name           TEXT,
  UNIQUE(task_id, gate_order)
);

CREATE TABLE files (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  sha256       TEXT NOT NULL,
  filename     TEXT NOT NULL,
  byte_size    INTEGER NOT NULL,
  content_type TEXT,
  storage_path TEXT NOT NULL,
  uploaded_by  INTEGER REFERENCES agents(id),
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_files_sha ON files(sha256);

CREATE TABLE deliverables (
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

CREATE TABLE deliverable_upstreams (
  deliverable_id INTEGER NOT NULL REFERENCES deliverables(id) ON DELETE CASCADE,
  label          TEXT,
  upstream_url   TEXT NOT NULL,
  upstream_id    INTEGER REFERENCES deliverables(id)
);

CREATE TABLE reviews (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  deliverable_id   INTEGER NOT NULL REFERENCES deliverables(id) ON DELETE CASCADE,
  task_gate_id     INTEGER NOT NULL REFERENCES task_gates(id),
  reviewer_id      INTEGER REFERENCES agents(id),
  verdict          TEXT NOT NULL CHECK (verdict IN ('pass','reject')),
  comment          TEXT,
  return_direction TEXT,
  return_location  TEXT,
  created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE events (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id         INTEGER REFERENCES runs(id) ON DELETE CASCADE,
  task_id        INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  deliverable_id INTEGER REFERENCES deliverables(id) ON DELETE SET NULL,
  actor_id       INTEGER REFERENCES agents(id),
  type           TEXT NOT NULL,
  detail_json    TEXT,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE idempotency_keys (
  key           TEXT PRIMARY KEY,
  agent_id      INTEGER REFERENCES agents(id),
  endpoint      TEXT,
  response_json TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- ───────────── 后台鉴权层（管理后台人类用户 · JWT；表本轮建好，HTTP API 第二轮再接）─────────────
CREATE TABLE admin_users (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  username      TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  display_name  TEXT,
  role          TEXT NOT NULL DEFAULT 'operator' CHECK (role IN ('superadmin','operator','viewer')),
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','disabled')),
  last_login_at TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE admin_refresh_tokens (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id     INTEGER NOT NULL REFERENCES admin_users(id) ON DELETE CASCADE,
  token_hash  TEXT NOT NULL UNIQUE,
  expires_at  TEXT NOT NULL,
  revoked     INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0,1)),
  user_agent  TEXT,
  ip          TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_admin_refresh_user ON admin_refresh_tokens(user_id);

CREATE TABLE admin_audit (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id     INTEGER REFERENCES admin_users(id),
  action      TEXT NOT NULL,
  target      TEXT,
  detail_json TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- ───────────── 索引 ─────────────
CREATE INDEX idx_tasks_assignee_status ON tasks(assignee_id, status);
CREATE INDEX idx_tasks_run             ON tasks(run_id, seq);
CREATE INDEX idx_deliverables_task     ON deliverables(task_id);
CREATE INDEX idx_reviews_deliverable   ON reviews(deliverable_id);
CREATE INDEX idx_events_run            ON events(run_id, id);

-- ───────────── 约束硬化（§8）─────────────
CREATE UNIQUE INDEX uq_wf_gates_default  ON workflow_gates(workflow_id, gate_order)           WHERE stage_id IS NULL;
CREATE UNIQUE INDEX uq_wf_gates_override ON workflow_gates(workflow_id, stage_id, gate_order) WHERE stage_id IS NOT NULL;
CREATE UNIQUE INDEX uq_wf_active         ON workflows(wf_key)                                 WHERE status = 'active';

-- +goose Down
DROP TABLE IF EXISTS admin_audit;
DROP TABLE IF EXISTS admin_refresh_tokens;
DROP TABLE IF EXISTS idempotency_keys;
DROP TABLE IF EXISTS events;
DROP TABLE IF EXISTS reviews;
DROP TABLE IF EXISTS deliverable_upstreams;
DROP TABLE IF EXISTS deliverables;
DROP TABLE IF EXISTS files;
DROP TABLE IF EXISTS task_gates;
DROP TABLE IF EXISTS task_deps;
DROP TABLE IF EXISTS tasks;
DROP TABLE IF EXISTS runs;
DROP TABLE IF EXISTS workflow_gates;
DROP TABLE IF EXISTS workflow_stage_deps;
DROP TABLE IF EXISTS workflow_stages;
DROP TABLE IF EXISTS workflows;
DROP TABLE IF EXISTS agent_tokens;
DROP TABLE IF EXISTS admin_users;
DROP TABLE IF EXISTS agents;
DROP TABLE IF EXISTS roles;
