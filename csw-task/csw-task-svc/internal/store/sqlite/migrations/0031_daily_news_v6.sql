-- +goose Up
-- daily_news v6：r44 暴露的口径问题——收集员把「接口返回数」填进 found，被读成「审阅量」；
-- 某一轮返回 100 篇、只有 13 篇进引擎，其余 87 篇的处理情况无法追溯。
-- 收集员自己提出并同意的修法：获取与审阅分开记，未审内容如实记为未审，不得事后补成淘汰。
-- 写法同 0029：复制当前 active 的阶段、依赖、闸，定点更新 intake，再归档旧 active、激活新版。

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
    'found（看到几条）、in_window（其中在窗口内几条）、registered（登记成条目几条）',
    'found（接口返回几条，含重复）、fetched_unique（去重后几条）、reviewed（你实际读过正文或看过图、形成了判断的几条）、unreviewed（只加载未展开的几条）、in_window（其中在窗口内几条）、registered（登记成条目几条）'),
  self_check_criteria=replace(self_check_criteria,
    '- 本次全部采集轮已上报；台账里必扫的来源每个都有一条，失败的写了 error 与替代来源。',
    '- 本次全部采集轮已上报；台账里必扫的来源每个都有一条，失败的写了 error 与替代来源。
- 获取与审阅分开记：接口返回、去重、已审、未审各有数，没有把接口返回量当成审阅量。
- 已审的条目都落了状态（进条目或标 dropped）；只加载未展开的如实记为未审，没有事后补成淘汰、没有倒填理由。'),
  acceptance=replace(acceptance,
    '- 采集轮次表能回答「找过哪里」：必扫来源都有记录，失败的写明原因与替代来源；没有把某个平台整体漏掉而不说明。',
    '- 采集轮次表能回答「找过哪里」：必扫来源都有记录，失败的写明原因与替代来源；没有把某个平台整体漏掉而不说明。
- 获取与审阅分开：接口返回、去重、已审、未审各有数。已审的都落了状态；未审的如实记为未审，不被伪装成淘汰。')
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
