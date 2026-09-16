-- +goose Up
-- 条目五栏：线索（candidate）、待核（pending_check）、成熟主选 / 成熟备选（shortlisted + rank）、Van 已批准（approved_*）。
-- status 的 CHECK 要加新值、并新增 rank 列，SQLite 只能整表重建（0011 / 0012 同法）。
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
                  CHECK (status IN ('candidate','pending_check','shortlisted','approved_write','approved_research','deferred','rejected','written','reviewed','published')),
  rank            TEXT CHECK (rank IS NULL OR rank IN ('primary','alt')),
  decided_by      INTEGER REFERENCES agents(id),
  decided_at      TEXT,
  decision_source TEXT,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, item_key)
);

INSERT INTO run_items_new (id, run_id, item_key, title, brand, product, source_url, published_at, status,
  decided_by, decided_at, decision_source, created_at, updated_at)
SELECT id, run_id, item_key, title, brand, product, source_url, published_at, status,
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
                  CHECK (status IN ('candidate','shortlisted','approved_write','approved_research','deferred','rejected','written','reviewed','published')),
  decided_by      INTEGER REFERENCES agents(id),
  decided_at      TEXT,
  decision_source TEXT,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, item_key)
);

INSERT INTO run_items_old (id, run_id, item_key, title, brand, product, source_url, published_at, status,
  decided_by, decided_at, decision_source, created_at, updated_at)
SELECT id, run_id, item_key, title, brand, product, source_url, published_at,
  CASE status WHEN 'pending_check' THEN 'candidate' ELSE status END,
  decided_by, decided_at, decision_source, created_at, updated_at
FROM run_items;

DROP TABLE run_items;
ALTER TABLE run_items_old RENAME TO run_items;
PRAGMA foreign_keys=ON;
