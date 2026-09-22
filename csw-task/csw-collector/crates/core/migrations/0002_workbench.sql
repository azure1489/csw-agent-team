-- 工作台的写操作要落的两样东西。
--
-- 一、手动的活排队给常驻循环做。工作台那一侧只有一个只读连接、没有模型与
--     向量客户端，也不该在一个 HTTP 请求里跑四十分钟的活。请求只排队，
--     循环下一次 tick 把它捡起来——**页面看得到排到哪了，也看得到失败原因**。
--
-- 二、主编指定的深核首批。自动挑法是「推荐优先、其次待核」，
--     但主编看完台账可能要换几条；指定的那几条优先，不够再按自动挑法补。

CREATE TABLE work_queue (
  id           INTEGER PRIMARY KEY,
  kind         TEXT NOT NULL,            -- manual_round | rerun
  round_id     INTEGER REFERENCES rounds(id) ON DELETE CASCADE,  -- rerun 用
  payload_json TEXT NOT NULL DEFAULT '{}',
  actor        TEXT NOT NULL,
  status       TEXT NOT NULL,            -- queued | running | done | failed
  note         TEXT NOT NULL DEFAULT '', -- 做完写一句账，失败写原因
  created_at   TEXT NOT NULL,
  started_at   TEXT,
  ended_at     TEXT
);
CREATE INDEX work_queue_status ON work_queue(status, id);

ALTER TABLE round_candidates ADD COLUMN first_batch INTEGER NOT NULL DEFAULT 0;
