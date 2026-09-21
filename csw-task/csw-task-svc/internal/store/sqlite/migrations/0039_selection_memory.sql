-- +goose Up
-- 选题记忆：准则卡与案例库。
--
-- 为什么单独建表而不是塞进工作流定义：准则是 Van 的口味，会随她的决定慢慢变；
-- 工作流定义是流程形态，改一次要走激活流程。两者变化的频率与授权方式都不一样。
--
-- 案例库只存**她说过的原话**与当时的决定，不存我们的归纳。归纳放准则卡，
-- 且必须标明是否经她确认过——没确认过的归纳不能当依据用。

CREATE TABLE selection_rules (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  rule_key    TEXT NOT NULL UNIQUE,
  -- prefer 优先关注 / lower 降低优先级 / frame 判断框架 / dedup 查重口径
  category    TEXT NOT NULL DEFAULT 'frame' CHECK (category IN ('frame','prefer','lower','dedup','other')),
  text        TEXT NOT NULL,
  -- 归纳自哪些案例，便于回头核
  derived_from TEXT,
  version     TEXT NOT NULL DEFAULT '',
  -- 经 Van 确认过才算数。未确认的只能展示，不能当判断依据。
  confirmed_by_van INTEGER NOT NULL DEFAULT 0 CHECK (confirmed_by_van IN (0,1)),
  confirmed_at TEXT,
  enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_selection_rules_cat ON selection_rules(category, enabled);

CREATE TABLE selection_cases (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  case_key    TEXT NOT NULL UNIQUE,
  run_id      INTEGER REFERENCES runs(id) ON DELETE SET NULL,
  item_key    TEXT,
  brand       TEXT,
  title       TEXT NOT NULL DEFAULT '',
  source_url  TEXT,
  decision    TEXT NOT NULL CHECK (decision IN ('adopted','rejected','deferred','pending_check')),
  -- Van 的原话。代录也要原样附，不许转述。
  quote       TEXT NOT NULL DEFAULT '',
  quote_ref   TEXT,
  decided_at  TEXT,
  -- 当时系统给的结论档，用来算「我们判得准不准」
  judged_tier TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_selection_cases_decision ON selection_cases(decision, decided_at);
CREATE INDEX idx_selection_cases_brand ON selection_cases(brand);

-- +goose Down
DROP INDEX IF EXISTS idx_selection_cases_brand;
DROP INDEX IF EXISTS idx_selection_cases_decision;
DROP TABLE IF EXISTS selection_cases;
DROP INDEX IF EXISTS idx_selection_rules_cat;
DROP TABLE IF EXISTS selection_rules;
