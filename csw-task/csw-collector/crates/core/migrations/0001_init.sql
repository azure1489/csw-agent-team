-- 本地状态库 v1。结构化数据全在这里，LanceDB 只放向量。
--
-- 三条贯穿全库的约定：
--   1. 时间一律存 RFC3339 UTC 字符串（`2026-09-22T01:40:00Z`），好查也好人肉核对。
--   2. 凡是「能重跑」的地方都靠 input_hash 对账，不靠时间戳猜。
--   3. 凡是要写引擎的动作，先进 engine_outbox 再发；崩溃后照 outbox 续，不重建。

-- ───────────────────────────── 轮次与步骤 ─────────────────────────────

CREATE TABLE rounds (
  id              INTEGER PRIMARY KEY,
  kind            TEXT NOT NULL,          -- task | manual | prefetch | replay
  trigger         TEXT NOT NULL,          -- dispatch | returned | supplement | manual
  run_id          INTEGER,                -- 引擎的 run；manual / prefetch 轮为空
  task_id         INTEGER,
  stage_code      TEXT,                   -- intake | material | xhs_pick
  -- 引擎任务的目标版本：退回返工会产生同一 task 的新一轮，用它区分
  target_version  INTEGER NOT NULL DEFAULT 1,
  parent_round_id INTEGER REFERENCES rounds(id),
  -- 窗口按首次入库时间算。注意 csw 的 posts/window 只按发布时间过滤，
  -- 所以采集器取更宽的发布时间窗口，再在本地按 ingested_at 收口。
  window_start    TEXT NOT NULL,
  window_end      TEXT NOT NULL,
  plan_version    INTEGER NOT NULL,       -- 采集方案版本
  rubric_version  TEXT NOT NULL,          -- 判断框架版本
  kb_snapshot     TEXT NOT NULL,          -- 本轮用的知识库快照标识
  -- 任务下发的作业标准原样注入模型，哈希存这里，变了就不能复用判断
  instructions_hash TEXT NOT NULL DEFAULT '',
  status          TEXT NOT NULL,          -- running | awaiting_review | done | failed | cancelled
  note            TEXT NOT NULL DEFAULT '',
  created_at      TEXT NOT NULL,
  ended_at        TEXT
);
-- 同一个任务的同一目标版本、同一触发原因只许开一轮。
-- 轮询接口偶尔会把同一条派单读到两次，这条约束是唯一的防线。
CREATE UNIQUE INDEX rounds_task_version_trigger
  ON rounds(task_id, target_version, trigger) WHERE task_id IS NOT NULL;
CREATE INDEX rounds_kind_created ON rounds(kind, created_at DESC);

CREATE TABLE round_steps (
  id          INTEGER PRIMARY KEY,
  round_id    INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  step        TEXT NOT NULL,              -- StepCode
  attempt     INTEGER NOT NULL DEFAULT 1, -- 重跑一步 = 新 attempt，旧的留着看
  status      TEXT NOT NULL,              -- StepStatus
  -- 这一步的输入指纹。与上一次相同就可以复用产物——预取轮与正式轮靠它对账。
  input_hash  TEXT NOT NULL DEFAULT '',
  counts_json TEXT NOT NULL DEFAULT '{}', -- 取到/只图文/已识别/已判 等计数
  error       TEXT NOT NULL DEFAULT '',
  started_at  TEXT,
  ended_at    TEXT,
  UNIQUE(round_id, step, attempt)
);
CREATE INDEX round_steps_round ON round_steps(round_id, step);

-- ───────────────────────────── 候选与媒体 ─────────────────────────────

-- 候选是跨轮共享的：同一条贴文昨天判过、今天还在窗口里，只存一份。
CREATE TABLE candidates (
  candidate_key TEXT PRIMARY KEY,
  platform      TEXT NOT NULL,
  source_id     TEXT NOT NULL,
  account       TEXT NOT NULL,
  url           TEXT NOT NULL,
  text          TEXT NOT NULL,
  translated    TEXT NOT NULL DEFAULT '',
  posted_at     TEXT,
  ingested_at   TEXT,
  likes         INTEGER,
  comments      INTEGER,
  followers     INTEGER,
  heat_ratio    REAL,
  content_type  TEXT NOT NULL DEFAULT '',
  image_only    INTEGER NOT NULL DEFAULT 0,
  tags_json     TEXT NOT NULL DEFAULT '[]',
  hashtags_json TEXT NOT NULL DEFAULT '[]',
  first_seen_at TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  UNIQUE(platform, source_id)
);
CREATE INDEX candidates_ingested ON candidates(ingested_at);
CREATE INDEX candidates_account ON candidates(account);

-- 候选与轮的多对多：一条候选可能出现在预取轮和正式轮里
CREATE TABLE round_candidates (
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL REFERENCES candidates(candidate_key),
  collector     TEXT NOT NULL,            -- 哪个采集器取到的
  in_window     INTEGER NOT NULL DEFAULT 1,
  -- 昨天已判、今天仍在窗口内的条目：台账要出现「已判结转」这一行，不重判
  carried       INTEGER NOT NULL DEFAULT 0,
  event_key     TEXT,                     -- 合并后归到哪件事
  PRIMARY KEY(round_id, candidate_key)
);
CREATE INDEX round_candidates_event ON round_candidates(round_id, event_key);

CREATE TABLE media (
  blake3        TEXT PRIMARY KEY,         -- 按内容存盘，天然去重
  candidate_key TEXT NOT NULL REFERENCES candidates(candidate_key),
  ordinal       INTEGER NOT NULL,
  kind          TEXT NOT NULL,            -- photo | video
  source_hash   TEXT NOT NULL DEFAULT '', -- 来源侧哈希；跨账号转载对不上，只能去同条内重复
  url           TEXT NOT NULL,
  bytes         INTEGER,
  width         INTEGER,
  height        INTEGER,
  path          TEXT NOT NULL DEFAULT '',
  downloaded_at TEXT,
  -- 下载或识别失败标「未识别」：不算看过，但不阻塞下游
  failed        INTEGER NOT NULL DEFAULT 0,
  UNIQUE(candidate_key, ordinal)
);

CREATE TABLE media_descriptions (
  id                INTEGER PRIMARY KEY,
  blake3            TEXT NOT NULL REFERENCES media(blake3),
  candidate_key     TEXT NOT NULL REFERENCES candidates(candidate_key),
  matches_text      TEXT NOT NULL,
  content           TEXT NOT NULL,
  missing_from_text TEXT NOT NULL,
  kind              TEXT NOT NULL,
  usable_as_figure  INTEGER NOT NULL,
  model             TEXT NOT NULL,
  prompt_version    TEXT NOT NULL,
  created_at        TEXT NOT NULL,
  -- 同一张图在同一提示词版本下只识别一次，换版本才重算
  UNIQUE(blake3, prompt_version)
);

-- LanceDB 里的向量按这张表重建。向量本身不在 SQLite 里。
CREATE TABLE embeddings_log (
  id            INTEGER PRIMARY KEY,
  kind          TEXT NOT NULL,            -- fused | image | kb_doc
  ref_key       TEXT NOT NULL,            -- candidate_key 或 blake3 或 kb_doc id
  lance_table   TEXT NOT NULL,
  embed_model   TEXT NOT NULL,
  dim           INTEGER NOT NULL,
  content_hash  TEXT NOT NULL,            -- 入模内容的哈希，变了才重算
  created_at    TEXT NOT NULL,
  UNIQUE(kind, ref_key, embed_model)
);

-- ───────────────────────────── 合并、材料、判断 ───────────────────────

CREATE TABLE events (
  event_key   TEXT PRIMARY KEY,
  round_id    INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  primary_key TEXT NOT NULL REFERENCES candidates(candidate_key),
  members_json TEXT NOT NULL DEFAULT '[]',
  -- 合并依据要能复核：向量相似度多少、Jev 的同一事件概率多少（阈值 0.5）
  merge_note  TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL
);

-- 五类对照材料，缺一不判。这张表记的是「本轮给某条候选看了什么」。
CREATE TABLE materials (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL,
  kind          TEXT NOT NULL,            -- published | example | generated_post | decision | prior_ledger
  ref_id        TEXT NOT NULL,
  title         TEXT NOT NULL DEFAULT '',
  date          TEXT,
  source        TEXT NOT NULL DEFAULT '',
  quote         TEXT NOT NULL DEFAULT '', -- Van 的决定类必须带原话
  publish_state TEXT NOT NULL DEFAULT '',
  rank          INTEGER NOT NULL DEFAULT 0,
  UNIQUE(round_id, candidate_key, kind, ref_id)
);
CREATE INDEX materials_lookup ON materials(round_id, candidate_key);

-- Jev 初评。只用来决定「先判哪条」，不进台账、不显示成分数。
CREATE TABLE triages (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL,
  dims_json     TEXT NOT NULL,            -- 每维「成立」的概率
  worth         REAL NOT NULL,
  lower_json    TEXT NOT NULL DEFAULT '[]', -- 七类降低优先级，一类一个概率
  priority      REAL NOT NULL,            -- explain + csw + use，排序用
  model         TEXT NOT NULL,
  created_at    TEXT NOT NULL,
  UNIQUE(round_id, candidate_key)
);

CREATE TABLE judgements (
  id                 INTEGER PRIMARY KEY,
  round_id           INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key      TEXT NOT NULL,
  tier               TEXT NOT NULL,       -- recommend | alternate | not_recommend | pending_check
  dims_json          TEXT NOT NULL,       -- 六维，每维 verdict + basis
  three_json         TEXT NOT NULL,
  unanswered         TEXT NOT NULL DEFAULT 'none',
  comparison_json    TEXT NOT NULL,
  heat_note          TEXT NOT NULL DEFAULT '',
  look               TEXT NOT NULL DEFAULT '',
  -- false 时 tier 必须是 pending_check，见下面的 CHECK
  image_seen         INTEGER NOT NULL,
  gaps_json          TEXT NOT NULL DEFAULT '[]',
  priority_hits_json TEXT NOT NULL DEFAULT '[]',
  lower_hits_json    TEXT NOT NULL DEFAULT '[]',
  jev_disagreement   TEXT NOT NULL DEFAULT '',
  -- Jev 核对：哪几条依据被判「材料不支持」。标出来给人看，不自动淘汰。
  check_flags_json   TEXT NOT NULL DEFAULT '[]',
  kb_refs_json       TEXT NOT NULL DEFAULT '[]',
  memory_refs_json   TEXT NOT NULL DEFAULT '[]',
  -- 复用的钥匙：正文、描述版本、对照材料、准则版本、作业标准、模型与提示词版本
  inputs_hash        TEXT NOT NULL,
  model              TEXT NOT NULL,
  rubric_version     TEXT NOT NULL,
  created_at         TEXT NOT NULL,
  UNIQUE(round_id, candidate_key),
  CHECK (image_seen = 1 OR tier = 'pending_check')
);
CREATE INDEX judgements_tier ON judgements(round_id, tier);
-- 按输入指纹找可复用的判断：预取轮判过的，正式轮直接拿
CREATE INDEX judgements_reuse ON judgements(candidate_key, inputs_hash);

-- 人工改档另存，不覆盖模型的原判——台账上两者都要看得见。
CREATE TABLE judgement_overrides (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL,
  from_tier     TEXT NOT NULL,
  to_tier       TEXT NOT NULL,
  reason        TEXT NOT NULL,
  actor         TEXT NOT NULL,            -- 后台用户名
  created_at    TEXT NOT NULL
);

CREATE TABLE deepchecks (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL,
  thread_id     TEXT NOT NULL DEFAULT '',
  turn_id       TEXT NOT NULL DEFAULT '', -- 打断要 threadId + turnId，两个都得留
  status        TEXT NOT NULL,            -- running | completed | interrupted | failed
  result_json   TEXT NOT NULL DEFAULT '{}',
  events_path   TEXT NOT NULL DEFAULT '', -- item/completed 事件流落盘位置（delta 已过滤）
  started_at    TEXT NOT NULL,
  ended_at      TEXT,
  UNIQUE(round_id, candidate_key)
);

-- 每次模型调用都记一笔：对账 token 预算、复盘延迟、熔断判据都靠它
CREATE TABLE model_calls (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER REFERENCES rounds(id) ON DELETE CASCADE,
  purpose       TEXT NOT NULL,            -- recognize | judge | deepcheck | triage | verify | embed | rerank
  provider      TEXT NOT NULL,
  model         TEXT NOT NULL,
  input_tokens  INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  latency_ms    INTEGER NOT NULL DEFAULT 0,
  attempts      INTEGER NOT NULL DEFAULT 1,
  status        TEXT NOT NULL,            -- ok | retried | failed
  error         TEXT NOT NULL DEFAULT '',
  created_at    TEXT NOT NULL
);
CREATE INDEX model_calls_round ON model_calls(round_id, purpose);
CREATE INDEX model_calls_day ON model_calls(created_at);

-- ───────────────────────────── 写引擎 ─────────────────────────────────

-- 所有写引擎的动作先落这里再发。
-- 引擎的 Idempotency-Key 只对 POST 生效，且请求中途崩溃后同键永久 409，
-- 所以 idem_key 一旦写下就**不许换**；遇冲突要人核实，不是自动换键重试。
CREATE TABLE engine_outbox (
  seq         INTEGER PRIMARY KEY,        -- 发送顺序
  round_id    INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,              -- OutboxKind
  idem_key    TEXT NOT NULL,
  body_path   TEXT NOT NULL DEFAULT '',   -- 大体积（zip）落盘，只存路径
  body_json   TEXT NOT NULL DEFAULT '',
  body_sha    TEXT NOT NULL,              -- 重试必须发同样的字节
  depends_on  INTEGER REFERENCES engine_outbox(seq),  -- items → sweeps → judgements → submit
  status      TEXT NOT NULL,              -- pending | sending | confirmed | conflict | dead
  attempts    INTEGER NOT NULL DEFAULT 0,
  last_error  TEXT NOT NULL DEFAULT '',
  response_json TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  sent_at     TEXT,
  UNIQUE(round_id, kind, idem_key)
);
CREATE INDEX engine_outbox_pending ON engine_outbox(status, seq);

-- 交付物：zip 只构建一次，字节、sha、幂等键先落盘再发
CREATE TABLE deliverables_local (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  deliv_kind    TEXT NOT NULL,            -- 产出 | 补件 | 定点编辑
  item_key      TEXT NOT NULL DEFAULT '', -- 逐条阶段的条目
  affects_deliverable_id INTEGER,         -- 补件必须带
  zip_path      TEXT NOT NULL,
  zip_sha256    TEXT NOT NULL,
  zip_bytes     INTEGER NOT NULL,
  idem_key      TEXT NOT NULL,
  engine_id     INTEGER,                  -- 引擎回的交付物 id
  created_at    TEXT NOT NULL,
  UNIQUE(round_id, deliv_kind, item_key, zip_sha256)
);

-- 引擎任务状态的本地镜像。引擎是真相，这里只是为了少打接口、能离线看。
CREATE TABLE task_mirror (
  task_id       INTEGER PRIMARY KEY,
  run_id        INTEGER NOT NULL,
  stage_code    TEXT NOT NULL,
  status        TEXT NOT NULL,            -- ready | dispatched | in_progress | review | returned | passed | failed | cancelled
  item_key      TEXT NOT NULL DEFAULT '',
  editor_note   TEXT NOT NULL DEFAULT '',
  upstreams_json TEXT NOT NULL DEFAULT '[]',
  instructions_json TEXT NOT NULL DEFAULT '{}',  -- 作业内容 / 自检 / 验收，原样注入模型
  latest_review_json TEXT NOT NULL DEFAULT '{}', -- 退回原因从这里自助读
  deadline_at   TEXT,
  last_ack_at   TEXT,                     -- 引擎无独立心跳，重复 ack 就是心跳
  fetched_at    TEXT NOT NULL
);

-- ───────────────────────────── 工作台 ────────────────────────────────

-- 登录会话。引擎的 access 与 refresh 只留在这里，浏览器只拿不透明 id。
CREATE TABLE sessions (
  sid             TEXT PRIMARY KEY,
  username        TEXT NOT NULL,
  engine_role     TEXT NOT NULL,          -- superadmin | operator | viewer
  role            TEXT NOT NULL,          -- 工作台角色，可能是引擎没有的 van
  access_token    TEXT NOT NULL,
  access_expires_at TEXT NOT NULL,
  refresh_cookie  TEXT NOT NULL,          -- 引擎每次续期都轮换，必须覆盖写
  csrf            TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  last_seen_at    TEXT NOT NULL
);

-- Van 在工作台上的勾选。只写本地，不回写引擎——进不进评选由主编代录。
CREATE TABLE van_marks (
  id            INTEGER PRIMARY KEY,
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  candidate_key TEXT NOT NULL,
  mark          TEXT NOT NULL,            -- like | doubt | note
  note          TEXT NOT NULL DEFAULT '',
  actor         TEXT NOT NULL,
  created_at    TEXT NOT NULL,
  UNIQUE(round_id, candidate_key, mark, actor)
);

-- 采集方案。改方案要留版本，轮次钉住自己用的那一版。
CREATE TABLE plan_versions (
  version    INTEGER PRIMARY KEY,
  stage_code TEXT NOT NULL,
  body_json  TEXT NOT NULL,
  note       TEXT NOT NULL DEFAULT '',
  actor      TEXT NOT NULL,
  active     INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

-- 谁在什么时候改了什么。改档、改方案、手动开轮、登记自定义采集器都要留痕。
CREATE TABLE audit (
  id         INTEGER PRIMARY KEY,
  actor      TEXT NOT NULL,
  action     TEXT NOT NULL,
  target     TEXT NOT NULL DEFAULT '',
  detail_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);
CREATE INDEX audit_created ON audit(created_at DESC);

-- ───────────────────────────── 知识库 ────────────────────────────────

-- 参考库四类。向量在 LanceDB，结构化字段与全文在这里。
CREATE TABLE kb_docs (
  id           INTEGER PRIMARY KEY,
  kind         TEXT NOT NULL,             -- published_item | example | generated_post | decision
  ref_id       TEXT NOT NULL,
  platform     TEXT NOT NULL DEFAULT '',
  post_id      TEXT NOT NULL DEFAULT '',  -- 同一贴文的多种身份合并到一行
  title        TEXT NOT NULL DEFAULT '',
  body         TEXT NOT NULL,
  url          TEXT NOT NULL DEFAULT '',
  brand        TEXT NOT NULL DEFAULT '',
  published_at TEXT,
  publish_state TEXT NOT NULL DEFAULT '',
  is_reference INTEGER NOT NULL DEFAULT 0,
  content_hash TEXT NOT NULL,
  embed_model  TEXT NOT NULL DEFAULT '',
  created_at   TEXT NOT NULL,
  UNIQUE(kind, ref_id)
);
CREATE INDEX kb_docs_post ON kb_docs(post_id);
CREATE INDEX kb_docs_brand ON kb_docs(brand);

-- 全文辅路。阶段 0.3 实测定的：unicode61 + jieba-rs 预分词 + 短语查询，
-- 中文语料上召回与精确都是 100%，比 LanceDB 的 jieba 路径快约一千倍。
-- tokens 列存预分词结果（空格分隔），查询侧用同一个分词器切好再当短语查。
CREATE VIRTUAL TABLE kb_fts USING fts5(
  tokens,
  content='',                             -- 不重复存正文，正文在 kb_docs
  tokenize='unicode61 remove_diacritics 2'
);

-- 品牌命中的主路：别名表 + 代码精确匹配。
-- 品牌名多是拉丁文、假名、两字中文，分词器都切不稳，所以精确匹配才是主路。
CREATE TABLE brands (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  note       TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE TABLE brand_aliases (
  id       INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id) ON DELETE CASCADE,
  alias    TEXT NOT NULL,
  -- 小写归一后的形态，匹配时用它
  alias_lc TEXT NOT NULL,
  -- 含空格的别名进不了 jieba 用户词典（词典按空白分列），这里标出来，
  -- 建词典时跳过；精确匹配不受影响。
  has_space INTEGER NOT NULL DEFAULT 0,
  source   TEXT NOT NULL DEFAULT '',      -- accounts | manual | kb
  UNIQUE(alias_lc)
);

-- 增量同步的水位
CREATE TABLE kb_cursors (
  source     TEXT PRIMARY KEY,            -- ledger_posts | ledger_decisions | csw_generated | examples
  cursor     TEXT NOT NULL DEFAULT '',
  synced_at  TEXT NOT NULL
);

-- 选题记忆：准则卡与案例库（引擎 0039 的本地镜像）
CREATE TABLE memory_rules (
  id         INTEGER PRIMARY KEY,
  rule_key   TEXT NOT NULL UNIQUE,
  text       TEXT NOT NULL,
  version    TEXT NOT NULL DEFAULT '',
  confirmed_by_van INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL
);
CREATE TABLE memory_cases (
  id         INTEGER PRIMARY KEY,
  case_key   TEXT NOT NULL UNIQUE,
  decision   TEXT NOT NULL,               -- 采用 | 否决 | 待核
  quote      TEXT NOT NULL,               -- Van 原话
  source_url TEXT NOT NULL DEFAULT '',
  decided_at TEXT,
  updated_at TEXT NOT NULL
);
