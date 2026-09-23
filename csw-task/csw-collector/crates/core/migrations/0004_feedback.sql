-- 09-22 判断台账反馈八项（van-rubric/v2）。只加表加列，旧二进制照样能读。

-- 判断：标题、看点类型、制作条件、是否因口径违例重判过
ALTER TABLE judgements ADD COLUMN headline TEXT NOT NULL DEFAULT '';
ALTER TABLE judgements ADD COLUMN novelty_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE judgements ADD COLUMN readiness_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE judgements ADD COLUMN rejudged INTEGER NOT NULL DEFAULT 0;

-- 选题层：同产品、同事件的多条贴文合成一个选题。逐帖台账不动。
-- （0001 的 events 表以 event_key 作全局主键，跨轮会撞，另起一张按轮的）
CREATE TABLE topics (
  id             INTEGER PRIMARY KEY,
  round_id       INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  topic_key      TEXT NOT NULL,              -- = 主帖的 candidate_key
  primary_key    TEXT NOT NULL,
  members_json   TEXT NOT NULL DEFAULT '[]', -- 含主帖
  merge_note     TEXT NOT NULL DEFAULT '',
  tier           TEXT NOT NULL DEFAULT '',   -- 组内最高的有效档
  headline       TEXT NOT NULL DEFAULT '',
  synthesis_json TEXT NOT NULL DEFAULT '{}', -- 共同事实、各帖新增、拆出去的
  created_at     TEXT NOT NULL,
  UNIQUE(round_id, topic_key)
);
CREATE INDEX topics_round ON topics(round_id);

-- 定点补读：外链正文。text 只留本机，进判断时过不可信边界。
CREATE TABLE refetches (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL,
  url           TEXT NOT NULL,
  status        TEXT NOT NULL,               -- ok | blocked | http_error | too_large | bad_type | failed
  http_status   INTEGER,
  bytes         INTEGER NOT NULL DEFAULT 0,
  content_hash  TEXT NOT NULL DEFAULT '',
  text          TEXT NOT NULL DEFAULT '',
  error         TEXT NOT NULL DEFAULT '',
  attempted_at  TEXT NOT NULL,
  UNIQUE(round_id, candidate_key, url)
);
CREATE INDEX refetches_key ON refetches(candidate_key);

-- 选题记忆：同步时丢掉的几列补回来（判断要按品牌挑案例、准则卡要按类别挑）
ALTER TABLE memory_rules ADD COLUMN category TEXT NOT NULL DEFAULT '';
ALTER TABLE memory_rules ADD COLUMN confirmed_at TEXT;
ALTER TABLE memory_cases ADD COLUMN brand TEXT NOT NULL DEFAULT '';
ALTER TABLE memory_cases ADD COLUMN title TEXT NOT NULL DEFAULT '';
ALTER TABLE memory_cases ADD COLUMN judged_tier TEXT NOT NULL DEFAULT '';
