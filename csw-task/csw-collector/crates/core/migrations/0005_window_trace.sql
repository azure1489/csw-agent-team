-- 宽取留痕：接口返回的每一条落到哪儿（窗口内 / 窗口外 / 非图文 / 重复），主编要能逐条复算。
-- 09-28 r56 #592 退回：宽取 1431 条 → 窗口内 128 条，被筛掉的 1303 条当时只计了数、没有逐条落库。
-- source：live = 当轮宽取时记下；refetch = 事后按同一窗口重新取数复算（原宽取未落库）。
CREATE TABLE window_trace (
  round_id      INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  sweep_key     TEXT NOT NULL,
  seq           INTEGER NOT NULL,          -- 接口返回的顺序
  source_id     TEXT NOT NULL,
  candidate_key TEXT NOT NULL,
  account       TEXT NOT NULL DEFAULT '',
  url           TEXT NOT NULL DEFAULT '',
  posted_at     TEXT,
  ingested_at   TEXT,
  media         TEXT NOT NULL DEFAULT '',
  outcome       TEXT NOT NULL,             -- in_window | dup_in_sweep | dup_across | not_image_only | before_window | after_window | no_time
  dup_of        TEXT,
  source        TEXT NOT NULL DEFAULT 'live',
  recorded_at   TEXT NOT NULL,
  PRIMARY KEY (round_id, sweep_key, seq)
);
