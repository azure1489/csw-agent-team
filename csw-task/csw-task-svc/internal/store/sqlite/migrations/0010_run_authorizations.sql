-- +goose Up
-- run 级授权（平台写操作护栏）：action_class=platform_write:<scope> 的任务，派工、自动派工与提交都要求
-- 本 run 存在未撤销、未过期的对应授权（发布授权涵盖同平台草稿）。授权须带 Van 原话，由中枢录入。
CREATE TABLE run_authorizations (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id       INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  scope        TEXT NOT NULL CHECK (scope IN ('local_drill','wx_draft','wx_publish','xhs_draft','xhs_publish')),
  granted_by   INTEGER REFERENCES agents(id),
  source_quote TEXT NOT NULL,
  granted_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  expires_at   TEXT,
  revoked_at   TEXT,
  revoked_by   INTEGER REFERENCES agents(id)
);
CREATE INDEX idx_run_auth_run ON run_authorizations(run_id, scope, revoked_at);

-- +goose Down
DROP TABLE IF EXISTS run_authorizations;
