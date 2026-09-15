-- +goose Up
-- 数据子系统 D1：真实发布记录（人工与 agent 发布统一入库）+ 合集拆条 + 同步批次（覆盖范围与缺口）。
-- 查重必须回答「在已同步记录中未发现」而不是「从未发布」：覆盖范围由 ledger_sync_runs 给出。
CREATE TABLE ledger_published_posts (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  platform      TEXT NOT NULL CHECK (platform IN ('wechat','xhs')),
  account       TEXT NOT NULL DEFAULT '',
  post_id       TEXT NOT NULL,
  url           TEXT,
  published_at  TEXT,
  title         TEXT NOT NULL DEFAULT '',
  body_text     TEXT,
  template_ver  TEXT,
  source        TEXT NOT NULL DEFAULT 'sync' CHECK (source IN ('agent','manual','sync','import')),
  state         TEXT NOT NULL DEFAULT 'published' CHECK (state IN ('drill','draft','published')),
  run_id        INTEGER REFERENCES runs(id) ON DELETE SET NULL,
  raw_json      TEXT,
  first_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(platform, account, post_id)
);
CREATE INDEX idx_ledger_posts_pub ON ledger_published_posts(platform, published_at);

CREATE TABLE ledger_post_items (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  post_ref    INTEGER NOT NULL REFERENCES ledger_published_posts(id) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,
  item_key    TEXT,
  brand       TEXT,
  product     TEXT,
  angle       TEXT,
  title       TEXT,
  source_url  TEXT,
  split_by    TEXT NOT NULL DEFAULT 'auto' CHECK (split_by IN ('auto','manual')),
  checked_by  TEXT,
  checked_at  TEXT,
  UNIQUE(post_ref, seq)
);
CREATE INDEX idx_ledger_items_brand ON ledger_post_items(brand);

CREATE TABLE ledger_sync_runs (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  platform    TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('probe','backfill','sync','snapshot','import')),
  started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  finished_at TEXT,
  window_from TEXT,
  window_to   TEXT,
  ok          INTEGER NOT NULL DEFAULT 0 CHECK (ok IN (0,1)),
  fetched     INTEGER NOT NULL DEFAULT 0,
  inserted    INTEGER NOT NULL DEFAULT 0,
  updated     INTEGER NOT NULL DEFAULT 0,
  gap_note    TEXT,
  error       TEXT
);
CREATE INDEX idx_ledger_sync_platform ON ledger_sync_runs(platform, ok, id);

-- +goose Down
DROP TABLE IF EXISTS ledger_sync_runs;
DROP TABLE IF EXISTS ledger_post_items;
DROP TABLE IF EXISTS ledger_published_posts;
