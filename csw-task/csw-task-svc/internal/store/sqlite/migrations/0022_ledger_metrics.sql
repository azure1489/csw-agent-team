-- +goose Up
-- 数据子系统 D3：指标快照与运营改进。只存平台原值（raw_json）与异常标记（flags_json），派生指标在查询时计算；
-- 同龄比较按 age_bucket（2h / 24h / 72h / 7d / 30d）；异常原值（曝光 0 而观看 >0、净涨粉与新增减取消不符）原样保留并标记。
CREATE TABLE ledger_metrics_snapshots (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  post_ref     INTEGER NOT NULL REFERENCES ledger_published_posts(id) ON DELETE CASCADE,
  platform     TEXT NOT NULL,
  age_bucket   TEXT NOT NULL CHECK (age_bucket IN ('2h','24h','72h','7d','30d','adhoc')),
  collected_at TEXT NOT NULL,
  window_from  TEXT,
  window_to    TEXT,
  raw_json     TEXT NOT NULL,
  flags_json   TEXT,
  UNIQUE(post_ref, age_bucket, collected_at)
);
CREATE TABLE ledger_account_snapshots (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  platform     TEXT NOT NULL,
  account      TEXT NOT NULL DEFAULT '',
  window_from  TEXT,
  window_to    TEXT,
  collected_at TEXT NOT NULL,
  raw_json     TEXT NOT NULL,
  flags_json   TEXT,
  UNIQUE(platform, account, window_from, window_to, collected_at)
);
CREATE TABLE ledger_improvement_items (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  finding         TEXT NOT NULL,
  evidence_ref    TEXT,
  hypothesis      TEXT,
  owner_role      TEXT REFERENCES roles(code),
  content_version TEXT,
  due_at          TEXT,
  recheck_at      TEXT,
  metric          TEXT,
  status          TEXT NOT NULL DEFAULT 'proposed'
                  CHECK (status IN ('proposed','adopted','executed','supported','unsupported','insufficient','withdrawn')),
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- +goose Down
DROP TABLE IF EXISTS ledger_improvement_items;
DROP TABLE IF EXISTS ledger_account_snapshots;
DROP TABLE IF EXISTS ledger_metrics_snapshots;
