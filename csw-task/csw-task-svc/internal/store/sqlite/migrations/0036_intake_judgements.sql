-- +goose Up
-- 情报收集员工作台：把「每条候选的判断结论」搬进引擎。
--
-- 为什么要这张表：01 现在只留结果不留判断过程——哪条被淘汰、凭什么淘汰，只写在交付物正文里，
-- 02 / 03 读不到，也没法回头核。0028 的 item_traces 记的是「状态怎么变的」，
-- 这张表记的是「为什么这么判」：四档结论 + Van 的六个维度各自成立与否 + 每维的依据。
--
-- 刻意不设分数列，也不设权重列。口径是「不打数字分、无权重」，
-- 留了列就一定会有人去填、去排序，然后口径就名存实亡了。

CREATE TABLE intake_judgements (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id        INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  -- 候选键：品牌小写 + '-' + 链接 sha256 前 6 位。与交付物里的条目键同一个。
  candidate_key TEXT NOT NULL,
  -- 进了 run_items 的才有 item_key；被判不推荐的候选不进 run_items，这里为空。
  item_key      TEXT,
  platform      TEXT NOT NULL DEFAULT 'instagram' CHECK (platform IN ('instagram','xhs','web','other')),
  post_ref      TEXT NOT NULL DEFAULT '',
  source_url    TEXT,
  tier          TEXT NOT NULL CHECK (tier IN ('recommend','alternate','not_recommend','pending_check')),
  -- 六维：{"change":{"verdict":"yes|no|unclear","basis":"..."}, ...}
  dims_json     TEXT NOT NULL DEFAULT '{}',
  three_sentences_json TEXT NOT NULL DEFAULT '{}',
  comparison_json TEXT NOT NULL DEFAULT '{}',
  heat_note     TEXT NOT NULL DEFAULT '',
  gaps_json     TEXT NOT NULL DEFAULT '[]',
  -- 没真正读到实图就是 0，且 tier 必须是 pending_check。这是口径里的硬规则，用 CHECK 兜住。
  image_seen    INTEGER NOT NULL DEFAULT 0 CHECK (image_seen IN (0,1)),
  -- 命中的七类优先关注 / 七类降低优先级。是倾向，不是黑名单。
  hits_json     TEXT NOT NULL DEFAULT '{}',
  -- Jev 的初评与核对留痕。初评概率只用于排序，不参与结论。
  jev_json      TEXT NOT NULL DEFAULT '{}',
  rubric_version TEXT NOT NULL DEFAULT '',
  -- 昨天已判、今天仍在窗口内：台账要出现这一行，但不重判。
  carried       INTEGER NOT NULL DEFAULT 0 CHECK (carried IN (0,1)),
  actor_id      INTEGER REFERENCES agents(id),
  role_code     TEXT NOT NULL DEFAULT '',
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE(run_id, candidate_key),
  CHECK (image_seen = 1 OR tier = 'pending_check')
);
CREATE INDEX idx_intake_judgements_run ON intake_judgements(run_id, tier);
CREATE INDEX idx_intake_judgements_item ON intake_judgements(run_id, item_key);

-- 采集轮的 tool 白名单要加 csw_api：新的采集服务直接调 csw 的 HTTP 接口，
-- 不再经 MCP，登记时若只能填 other 就分不清「谁扫的」。
-- SQLite 改不了 CHECK，只能整表重建（0027 / 0028 同法）。
PRAGMA foreign_keys=OFF;

CREATE TABLE intake_sweeps_new (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id       INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  task_id      INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  sweep_key    TEXT NOT NULL,
  platform     TEXT NOT NULL CHECK (platform IN ('instagram','xhs','web','other')),
  source_key   TEXT NOT NULL DEFAULT '',
  tool         TEXT NOT NULL DEFAULT 'other' CHECK (tool IN ('csw_mcp','csw_api','opencli','webfetch','other')),
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
  fetched_unique INTEGER NOT NULL DEFAULT 0,
  reviewed     INTEGER NOT NULL DEFAULT 0,
  unreviewed   INTEGER NOT NULL DEFAULT 0,
  corroborated INTEGER NOT NULL DEFAULT 0,
  UNIQUE(run_id, sweep_key)
);

INSERT INTO intake_sweeps_new (id, run_id, task_id, sweep_key, platform, source_key, tool, query,
  started_at, ended_at, window_from, window_to, found, in_window, registered, result, error,
  paged_to_end, actor_id, role_code, created_at, updated_at,
  fetched_unique, reviewed, unreviewed, corroborated)
SELECT id, run_id, task_id, sweep_key, platform, source_key, tool, query,
  started_at, ended_at, window_from, window_to, found, in_window, registered, result, error,
  paged_to_end, actor_id, role_code, created_at, updated_at,
  fetched_unique, reviewed, unreviewed, corroborated
FROM intake_sweeps;

DROP TABLE intake_sweeps;
ALTER TABLE intake_sweeps_new RENAME TO intake_sweeps;
CREATE INDEX idx_intake_sweeps_run ON intake_sweeps(run_id, id);
PRAGMA foreign_keys=ON;

-- +goose Down
PRAGMA foreign_keys=OFF;

CREATE TABLE intake_sweeps_old (
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
  fetched_unique INTEGER NOT NULL DEFAULT 0,
  reviewed     INTEGER NOT NULL DEFAULT 0,
  unreviewed   INTEGER NOT NULL DEFAULT 0,
  corroborated INTEGER NOT NULL DEFAULT 0,
  UNIQUE(run_id, sweep_key)
);

-- 回退时把 csw_api 归到 other：这条路只在回滚演练里走，不丢行。
INSERT INTO intake_sweeps_old (id, run_id, task_id, sweep_key, platform, source_key, tool, query,
  started_at, ended_at, window_from, window_to, found, in_window, registered, result, error,
  paged_to_end, actor_id, role_code, created_at, updated_at,
  fetched_unique, reviewed, unreviewed, corroborated)
SELECT id, run_id, task_id, sweep_key, platform, source_key,
  CASE tool WHEN 'csw_api' THEN 'other' ELSE tool END, query,
  started_at, ended_at, window_from, window_to, found, in_window, registered, result, error,
  paged_to_end, actor_id, role_code, created_at, updated_at,
  fetched_unique, reviewed, unreviewed, corroborated
FROM intake_sweeps;

DROP TABLE intake_sweeps;
ALTER TABLE intake_sweeps_old RENAME TO intake_sweeps;
CREATE INDEX idx_intake_sweeps_run ON intake_sweeps(run_id, id);
PRAGMA foreign_keys=ON;

DROP INDEX IF EXISTS idx_intake_judgements_item;
DROP INDEX IF EXISTS idx_intake_judgements_run;
DROP TABLE IF EXISTS intake_judgements;
