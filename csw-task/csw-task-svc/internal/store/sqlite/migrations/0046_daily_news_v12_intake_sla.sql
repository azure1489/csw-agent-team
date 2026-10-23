-- +goose Up
-- daily_news v12：01 情报逐条时限 40 → 75 分钟（2026-10-24 用户确认取消工作台预取轮，
-- 见 docs/取消预取轮_方案_20261024.md）。01 改为 06:00 接单后从零跑，40 分钟的逾期提醒会成为
-- 每天的假警报（10-23 r86 主编 06:41 就开始催）。其余与 v11 完全相同。
--
-- **这一版只插 draft，不激活。** 激活用 adminctl（与 v9–v11 相同的做法）。

-- +goose StatementBegin
INSERT INTO workflows (wf_key, name, version, hub_role_code, dispatch_mode, trigger_roles, common_instructions, common_acceptance, status)
SELECT wf_key, name, (SELECT MAX(version) + 1 FROM workflows WHERE wf_key='daily_news'), hub_role_code, dispatch_mode, trigger_roles,
       common_instructions, common_acceptance, 'draft'
FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news');
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_stages (workflow_id, seq, code, name, role_code, output_type, is_merge, instructions, self_check_criteria, acceptance,
  dispatch_mode, action_class, sla_minutes, per_item, ack_minutes, idle_minutes)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), seq, code, name, role_code, output_type, is_merge, instructions, self_check_criteria, acceptance,
       dispatch_mode, action_class, CASE WHEN code = 'intake' THEN 75 ELSE sla_minutes END, per_item, ack_minutes, idle_minutes
FROM workflow_stages WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news') - 1);
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_stage_deps (workflow_id, stage_id, depends_on_id)
SELECT ns.workflow_id, ns.id, nd.id FROM workflow_stage_deps d
JOIN workflow_stages os ON os.id = d.stage_id
JOIN workflow_stages od ON od.id = d.depends_on_id
JOIN workflow_stages ns ON ns.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND ns.code = os.code
JOIN workflow_stages nd ON nd.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND nd.code = od.code
WHERE os.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news') - 1);
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), ns.id, g.gate_order, g.reviewer_role, g.relayed_by_hub, g.name
FROM workflow_gates g JOIN workflow_stages os ON os.id = g.stage_id
JOIN workflow_stages ns ON ns.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND ns.code = os.code
WHERE g.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news') - 1);
-- +goose StatementEnd

-- 刻意没有 UPDATE workflows SET status='active' —— 见文件开头。

-- +goose Down

-- 只删这一版的 draft（01 时限 75 且是最新版本）；已激活的走后台回滚到 v11。
-- +goose StatementBegin
DELETE FROM workflows WHERE wf_key='daily_news' AND status='draft'
  AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')
  AND id IN (SELECT workflow_id FROM workflow_stages WHERE code='intake' AND sla_minutes=75);
-- +goose StatementEnd
