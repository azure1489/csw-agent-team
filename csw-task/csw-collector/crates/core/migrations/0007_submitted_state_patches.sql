-- 1) 上次交出去的内容指纹单独存：以前记在 note 里，被「内容相同未重交」的标记一盖就没了，
--    主编重开再派时拿不到指纹，把同样的包又交了一遍（09-30 r58 v9–v17 往复）。
ALTER TABLE rounds ADD COLUMN submitted_state TEXT;

-- 2) 补采记录：返工补采窗口缺的那一段，0 条也要留下「几点补的、宽取多少、窗口内多少」。
--    不留的话交付包只能改 window_to，拿不出覆盖证据（09-30 r58 v2 退回）。
CREATE TABLE window_patches (
  round_id       INTEGER NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
  window_from    TEXT NOT NULL,
  window_to      TEXT NOT NULL,
  fetched_at     TEXT NOT NULL,
  found          INTEGER NOT NULL,
  fetched_unique INTEGER NOT NULL,
  in_window      INTEGER NOT NULL,
  query          TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (round_id, window_from, window_to, fetched_at)
);
