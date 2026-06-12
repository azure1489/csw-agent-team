-- +goose Up
-- tasks 增加 output_type 快照列（随 Trigger 从 workflow_stages 快照，与 instructions 等同列性质）。
-- 用途：交付物 doc_type 缺省取阶段产出类型；submit 一步式按「责任方=合流阶段产出类型/角色名」派生文件名。
ALTER TABLE tasks ADD COLUMN output_type TEXT;

-- 回填存量任务：runs.workflow_id 指向具体定义版本行，按 (workflow_id, stage_code) 对回该版本的阶段。
UPDATE tasks SET output_type = (
  SELECT s.output_type
  FROM workflow_stages s
  JOIN runs r ON r.workflow_id = s.workflow_id
  WHERE r.id = tasks.run_id AND s.code = tasks.stage_code
);

-- +goose Down
ALTER TABLE tasks DROP COLUMN output_type;
