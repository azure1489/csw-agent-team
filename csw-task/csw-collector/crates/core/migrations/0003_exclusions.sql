-- 硬性排除：Van 在 03 否决过的，同事实同角度的别再端上来。
--
-- 这是**唯一一条会让候选不经模型判断就落定的规则**，所以两道保险都在表里：
-- ① 没有 Van 原话的否决 `active=0`，摆出来等人确认，不自动生效；
-- ② 命中记在 exclusion_hits 上，主编可以逐条捞回（restored=1），
--    也可以把整条规则停掉（active=0）。

CREATE TABLE exclusions (
  id              INTEGER PRIMARY KEY,
  -- 来源决定的稳定标识 `run_id#item_key`。同一条目在不同 run 里可能被决定两次，
  -- 各算各的规则——后一次未必推翻前一次，可能是同一个口味的第二次表态。
  decision_ref    TEXT NOT NULL UNIQUE,
  item_key        TEXT NOT NULL,
  title           TEXT NOT NULL DEFAULT '',
  brand           TEXT NOT NULL DEFAULT '',
  source_url      TEXT NOT NULL DEFAULT '',
  -- Van 的原话。**空不是「没否决」，是「否决了但没留下话」**——见 active。
  quote           TEXT NOT NULL DEFAULT '',
  reason          TEXT NOT NULL DEFAULT '',
  reason_code     TEXT NOT NULL DEFAULT '',
  decided_at      TEXT NOT NULL DEFAULT '',
  actor_role      TEXT NOT NULL DEFAULT '',
  -- 1 = 参与排除。没有原话的进表但默认 0：一条会让候选消失的规则，
  -- 拿不出 Van 的话就没法在台账上解释它，也没法判断否的是这件事还是这个角度。
  active          INTEGER NOT NULL DEFAULT 1,
  inactive_reason TEXT NOT NULL DEFAULT '',
  changed_by      TEXT NOT NULL DEFAULT '',
  changed_at      TEXT NOT NULL DEFAULT '',
  created_at      TEXT NOT NULL
);

CREATE INDEX idx_exclusions_brand ON exclusions(brand) WHERE active = 1;

-- 一轮里哪条候选被哪条规则排除了，以及当时的两个判据值。
-- **留着是为了能解释**：台账上要能回答「它为什么没进判断」。
CREATE TABLE exclusion_hits (
  round_id        INTEGER NOT NULL,
  candidate_key   TEXT NOT NULL,
  exclusion_id    INTEGER NOT NULL REFERENCES exclusions(id),
  same_fact       REAL NOT NULL,
  -- 新料的概率。**越高越不该排**，判据是「< NO_SUBSTANCE 才排」
  new_substance   REAL NOT NULL,
  -- 主编捞回：只对这一轮这一条生效，规则本身还在
  restored        INTEGER NOT NULL DEFAULT 0,
  restored_by     TEXT NOT NULL DEFAULT '',
  restored_reason TEXT NOT NULL DEFAULT '',
  created_at      TEXT NOT NULL,
  PRIMARY KEY (round_id, candidate_key)
);
