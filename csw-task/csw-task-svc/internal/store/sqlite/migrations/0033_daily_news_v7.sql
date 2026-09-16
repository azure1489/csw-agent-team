-- +goose Up
-- daily_news v7：r45 实测暴露两处——
-- · 收集员为核实线索去读官方页（佐证），被计进 reviewed 却不登记成条目，判据二误判为「审过没落状态」；
--   佐证单独记 corroborated（迁移 0032），作业说明里讲清楚。
-- · 6 条条目没写 published_at（它知道日期但没填进字段），窗口判据只能报「无法判断是否属于当期」。
--   自检加硬要求：每条必须填；拿不到确切日期的标 pending_check 并写明缺的就是日期，不留空。
-- 写法同 0031。

-- +goose StatementBegin
INSERT INTO workflows (wf_key, name, version, hub_role_code, dispatch_mode, trigger_roles, common_instructions, common_acceptance, status)
SELECT wf_key, name, (SELECT MAX(version) + 1 FROM workflows WHERE wf_key='daily_news'), hub_role_code, dispatch_mode, trigger_roles,
       common_instructions, common_acceptance, 'draft'
FROM workflows WHERE wf_key='daily_news' AND status='active';
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_stages (workflow_id, seq, code, name, role_code, output_type, is_merge, instructions, self_check_criteria, acceptance,
  dispatch_mode, action_class, sla_minutes, per_item, ack_minutes, idle_minutes)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), seq, code, name, role_code, output_type, is_merge, instructions, self_check_criteria, acceptance,
       dispatch_mode, action_class, sla_minutes, per_item, ack_minutes, idle_minutes
FROM workflow_stages WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active');
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_stage_deps (workflow_id, stage_id, depends_on_id)
SELECT ns.workflow_id, ns.id, nd.id FROM workflow_stage_deps d
JOIN workflow_stages os ON os.id = d.stage_id
JOIN workflow_stages od ON od.id = d.depends_on_id
JOIN workflow_stages ns ON ns.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND ns.code = os.code
JOIN workflow_stages nd ON nd.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND nd.code = od.code
WHERE os.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active');
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), ns.id, g.gate_order, g.reviewer_role, g.relayed_by_hub, g.name
FROM workflow_gates g JOIN workflow_stages os ON os.id = g.stage_id
JOIN workflow_stages ns ON ns.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND ns.code = os.code
WHERE g.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions=replace(instructions,
    'reviewed（你实际读过正文或看过图、形成了判断的几条）、unreviewed（只加载未展开的几条）',
    'reviewed（你实际读过正文或看过图、形成了判断的几条）、corroborated（reviewed 里为核实线索而读的佐证页，如官方页与原始出处，它们不作为候选登记）、unreviewed（只加载未展开的几条）'),
  self_check_criteria=replace(self_check_criteria,
    '- 每条条目都写了 discovered_via、fetched_at、evidence_url、dedup_note。',
    '- 每条条目都写了 discovered_via、fetched_at、evidence_url、dedup_note。
- 每条条目都填了 published_at（原始披露时间）。拿不到确切日期的标 pending_check 并写明缺的就是日期，不要留空——留空会让窗口判据无法判断它是否属于当期。
- 为核实线索去读的官方页、原始出处计入 corroborated，不登记成条目；已审里扣掉佐证后的数，应与登记条目数对得上。'),
  acceptance=replace(acceptance,
    '- 获取与审阅分开：接口返回、去重、已审、未审各有数。已审的都落了状态；未审的如实记为未审，不被伪装成淘汰。',
    '- 获取与审阅分开：接口返回、去重、已审（含佐证）、未审各有数。已审扣除佐证后的数与登记条目对得上；未审的如实记为未审，不被伪装成淘汰。
- 每条条目都有原始披露时间；拿不到的标为待核并写明缺日期，没有空着让窗口无从判断。')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'intake';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflows SET status='archived' WHERE wf_key='daily_news' AND status='active';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflows SET status='active' WHERE id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'));
-- +goose StatementEnd

-- +goose Down
-- +goose StatementBegin
DELETE FROM workflows WHERE wf_key='daily_news' AND status='active'
  AND id = (SELECT MAX(id) FROM workflows WHERE wf_key='daily_news');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflows SET status='active' WHERE id = (SELECT MAX(id) FROM workflows WHERE wf_key='daily_news' AND status='archived');
-- +goose StatementEnd
