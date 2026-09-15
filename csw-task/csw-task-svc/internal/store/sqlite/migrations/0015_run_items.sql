-- +goose Up
-- 条目级推进：条目与整期分开推进。
-- · run_items：一期里的每条候选资讯（item_key 由登记方生成后不再变），状态由采集 / 研究登记，由中枢按 Van 原话决定。
-- · 批准可写（approved_write）的条目由引擎为其动态生成逐条阶段（per_item）的任务；
--   依赖逐条阶段的合流任务在触发时快照 wait_item_stages，等「至少一条已批准且其逐条任务全部通过」才就绪。
-- · runs.target_count：整期目标条数；已写成条数不足时 run 保持进行中并显示缺口，由中枢 close 接受缺口。
CREATE TABLE run_items (
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
ALTER TABLE runs ADD COLUMN target_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN wait_item_stages TEXT;

-- +goose Down
ALTER TABLE tasks DROP COLUMN wait_item_stages;
ALTER TABLE runs DROP COLUMN target_count;
DROP TABLE IF EXISTS run_items;
