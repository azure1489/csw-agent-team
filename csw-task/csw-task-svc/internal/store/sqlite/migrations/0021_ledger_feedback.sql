-- +goose Up
-- 数据子系统 D2：编辑反馈记忆。原话 + 可回看的图 + 适用范围标签（brand: / category: / stage: / kind: …）。
-- explicit=有原话的明确意见；inferred=模型推测，不得覆盖 explicit。temporary 带过期时间，过期不再下发。
-- 审核带 Van 原话时自动沉淀一条（review_id 唯一，重放不重复）。
CREATE TABLE ledger_feedback_records (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  quote           TEXT NOT NULL,
  said_at         TEXT,
  source_ref      TEXT,
  object_ref      TEXT,
  object_version  TEXT,
  image_refs_json TEXT,
  kind            TEXT NOT NULL DEFAULT 'recent' CHECK (kind IN ('long_term','case','temporary','recent')),
  stance          TEXT NOT NULL DEFAULT 'explicit' CHECK (stance IN ('explicit','inferred')),
  status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','superseded','expired')),
  supersedes_id   INTEGER REFERENCES ledger_feedback_records(id),
  expires_at      TEXT,
  curated_by      TEXT,
  review_id       INTEGER UNIQUE REFERENCES reviews(id) ON DELETE SET NULL,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE TABLE ledger_feedback_tags (
  record_id INTEGER NOT NULL REFERENCES ledger_feedback_records(id) ON DELETE CASCADE,
  tag       TEXT NOT NULL,
  PRIMARY KEY (record_id, tag)
);
CREATE INDEX idx_ledger_feedback_tag ON ledger_feedback_tags(tag);

-- +goose Down
DROP TABLE IF EXISTS ledger_feedback_tags;
DROP TABLE IF EXISTS ledger_feedback_records;
