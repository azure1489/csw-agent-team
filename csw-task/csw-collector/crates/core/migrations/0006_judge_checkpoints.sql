-- 判断断点：每判完一批立刻存下模型刚判出的结论（还没过口径兜底），进程中途被杀、
-- 重跑同一轮时按（条目键, 输入指纹）接上，不再从头判。
-- 09-30 r58：505 条判到 2 小时 45 分钟被 OOM 杀掉，重启后从头判。
-- 只是加速用的缓存：丢了、清了都不影响正确性；按时间清旧的。
CREATE TABLE judge_checkpoints (
  candidate_key  TEXT NOT NULL,
  inputs_hash    TEXT NOT NULL,
  judgement_json TEXT NOT NULL,
  created_at     TEXT NOT NULL,
  PRIMARY KEY (candidate_key, inputs_hash)
);
CREATE INDEX judge_checkpoints_created ON judge_checkpoints(created_at);
