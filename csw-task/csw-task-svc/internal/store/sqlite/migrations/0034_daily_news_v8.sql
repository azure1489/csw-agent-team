-- +goose Up
-- daily_news v8：r47 查出采集供给不足的真因——收集员把 csw MCP 当成「按账号名猜」的接口。
-- 实测：同一时间窗（9/14–9/17）后台 csw_posts_search 返回 100+ 条，它只报 14 条窗口内；
-- 它试的 klattermusen / minimalworks-official / zanearts 都不在后台 20 个在册账号里，
-- 于是记成「零结果」——不是当天没新品，是查了不存在的账号。
-- 121 次 MCP 调用清一色 {account: 名字, limit: 3}，start/end 一次没用过，csw_accounts_list 一次没调过。
-- 正确用法写进定义：时间窗是采集的唯一入口，按 start/end 取全量贴文；
-- 账号不是入口，而是贴文的附属信息——拿到贴文后需要了解来源背景时，才另用账号接口去查。

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
    '工具：Instagram 用营事编集室 csw MCP；',
    '**Instagram 查 csw MCP：时间窗是唯一入口，不要按账号搜**。
1. 直接 csw_posts_search 按当期窗口取全量：start / end 填窗口起止、order=latest、limit 给足（100 起，不够就翻页取完）。这一步就是本路采集的全部入口。
2. **不要用 account 去逐个搜**，更不要凭记忆猜账号名——猜出来的“零结果”只说明这个账号不在后台，不说明该品牌当天没发；这种零结果不得记为已扫来源。
3. 账号信息是贴文的附属，不是采集起点：拿到贴文之后，**需要了解某条的来源背景时**，再用 csw_accounts_list（可带 keyword）查那个账号。不需要就不查。
另外 csw_user_selections_list 可读 Van 在前端勾选的贴文，开工时先看一眼有没有。

工具：Instagram 用营事编集室 csw MCP；'),
  self_check_criteria=replace(self_check_criteria,
    '- 本次全部采集轮已上报；',
    '- Instagram 这一路是直接用 start/end/order=latest 按时间窗取全量贴文取完的，没有按账号逐个搜、没有凭记忆猜 account，也没有把「账号不在后台」记成「该品牌没发」；账号信息只在需要了解来源背景时才另外查。
- 本次全部采集轮已上报；'),
  acceptance=replace(acceptance,
    '- 采集轮次表能回答「找过哪里」：',
    '- Instagram 走的是「按时间窗取全量贴文」，不是按账号逐个搜；窗口内条数与后台实际量级对得上。
- 采集轮次表能回答「找过哪里」：')
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
