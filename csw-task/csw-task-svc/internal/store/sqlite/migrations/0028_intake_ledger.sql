-- +goose Up
-- 采集与判断过程可核记录（01-情报逐条只留结果、不留过程，无法验证采集是否合理）：
-- · intake_sources 来源台账——本期「应该扫哪些」的基准，是覆盖率判据的分母；
-- · intake_sweeps 采集轮——回答「找过哪里」，让「没找到」与「没去找」可区分；
-- · item_traces 判断轨迹——淘汰理由落库，不再只写在交付物正文里、别的 agent 读不到。
-- run_items 同时加 dropped 状态与四个溯源列：status 的 CHECK 要改，SQLite 只能整表重建（0027 同法）。

CREATE TABLE intake_sources (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  platform   TEXT NOT NULL CHECK (platform IN ('instagram','xhs','web','other')),
  source_key TEXT NOT NULL,
  name       TEXT NOT NULL DEFAULT '',
  entry_url  TEXT,
  enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  required   INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0,1)),
  last_ok_at TEXT,
  added_by   TEXT,
  source_ref TEXT,
  note       TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(platform, source_key)
);
CREATE INDEX idx_intake_sources_enabled ON intake_sources(enabled, required);

-- seed 通道级三行：账号级清单此前从未在任何文档里枚举过（「已授权来源」只存在于收集员的临场判断里），
-- 但空表会让覆盖率判据第一天就失效。三个通道出自 01 定义原文，账号级留给 source add / backfill 人工确认。
INSERT INTO intake_sources (platform, source_key, name, enabled, required, source_ref, note) VALUES
  ('instagram','channel','Instagram（营事编集室 csw MCP）',1,1,'daily_news 01 定义原文','通道级占位；账号级来源待人工登记'),
  ('xhs','channel','小红书（opencli xiaohongshu）',1,1,'daily_news 01 定义原文','通道级占位；关键词与账号待人工登记'),
  ('web','channel','品牌官网与媒体（opencli 适配器 / WebFetch）',1,1,'daily_news 01 定义原文','通道级占位；站点待人工登记');

CREATE TABLE intake_sweeps (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id       INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  task_id      INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  sweep_key    TEXT NOT NULL,
  platform     TEXT NOT NULL CHECK (platform IN ('instagram','xhs','web','other')),
  source_key   TEXT NOT NULL DEFAULT '',
  tool         TEXT NOT NULL DEFAULT 'other' CHECK (tool IN ('csw_mcp','opencli','webfetch','other')),
  query        TEXT,
  started_at   TEXT,
  ended_at     TEXT,
  window_from  TEXT,
  window_to    TEXT,
  found        INTEGER NOT NULL DEFAULT 0,
  in_window    INTEGER NOT NULL DEFAULT 0,
  registered   INTEGER NOT NULL DEFAULT 0,
  result       TEXT NOT NULL DEFAULT 'ok' CHECK (result IN ('ok','failed','partial')),
  error        TEXT,
  paged_to_end INTEGER NOT NULL DEFAULT 0 CHECK (paged_to_end IN (0,1)),
  actor_id     INTEGER REFERENCES agents(id),
  role_code    TEXT NOT NULL DEFAULT '',
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, sweep_key)
);
CREATE INDEX idx_intake_sweeps_run ON intake_sweeps(run_id, id);

CREATE TABLE item_traces (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id      INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  item_key    TEXT NOT NULL,
  from_status TEXT,
  to_status   TEXT NOT NULL,
  reason_code TEXT NOT NULL DEFAULT '',
  reason      TEXT,
  actor_id    INTEGER REFERENCES agents(id),
  actor_role  TEXT NOT NULL DEFAULT '',
  quote_ref   TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_item_traces_run_item ON item_traces(run_id, item_key, id);

PRAGMA foreign_keys=OFF;

CREATE TABLE run_items_new (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id          INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  item_key        TEXT NOT NULL,
  title           TEXT NOT NULL DEFAULT '',
  brand           TEXT,
  product         TEXT,
  source_url      TEXT,
  published_at    TEXT,
  status          TEXT NOT NULL DEFAULT 'candidate'
                  CHECK (status IN ('candidate','pending_check','shortlisted','dropped','approved_write','approved_research','deferred','rejected','written','reviewed','published')),
  rank            TEXT CHECK (rank IS NULL OR rank IN ('primary','alt')),
  decided_by      INTEGER REFERENCES agents(id),
  decided_at      TEXT,
  decision_source TEXT,
  discovered_via  TEXT,
  fetched_at      TEXT,
  evidence_url    TEXT,
  dedup_note      TEXT,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, item_key)
);

INSERT INTO run_items_new (id, run_id, item_key, title, brand, product, source_url, published_at, status, rank,
  decided_by, decided_at, decision_source, created_at, updated_at)
SELECT id, run_id, item_key, title, brand, product, source_url, published_at, status, rank,
  decided_by, decided_at, decision_source, created_at, updated_at
FROM run_items;

DROP TABLE run_items;
ALTER TABLE run_items_new RENAME TO run_items;
PRAGMA foreign_keys=ON;

-- +goose Down
PRAGMA foreign_keys=OFF;

CREATE TABLE run_items_old (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id          INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  item_key        TEXT NOT NULL,
  title           TEXT NOT NULL DEFAULT '',
  brand           TEXT,
  product         TEXT,
  source_url      TEXT,
  published_at    TEXT,
  status          TEXT NOT NULL DEFAULT 'candidate'
                  CHECK (status IN ('candidate','pending_check','shortlisted','approved_write','approved_research','deferred','rejected','written','reviewed','published')),
  rank            TEXT CHECK (rank IS NULL OR rank IN ('primary','alt')),
  decided_by      INTEGER REFERENCES agents(id),
  decided_at      TEXT,
  decision_source TEXT,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, item_key)
);

INSERT INTO run_items_old (id, run_id, item_key, title, brand, product, source_url, published_at, status, rank,
  decided_by, decided_at, decision_source, created_at, updated_at)
SELECT id, run_id, item_key, title, brand, product, source_url, published_at,
  CASE status WHEN 'dropped' THEN 'candidate' ELSE status END, rank,
  decided_by, decided_at, decision_source, created_at, updated_at
FROM run_items;

DROP TABLE run_items;
ALTER TABLE run_items_old RENAME TO run_items;
PRAGMA foreign_keys=ON;

DROP INDEX IF EXISTS idx_item_traces_run_item;
DROP TABLE IF EXISTS item_traces;
DROP INDEX IF EXISTS idx_intake_sweeps_run;
DROP TABLE IF EXISTS intake_sweeps;
DROP INDEX IF EXISTS idx_intake_sources_enabled;
DROP TABLE IF EXISTS intake_sources;
