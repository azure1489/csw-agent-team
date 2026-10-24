-- +goose Up
-- daily_news v13：图片加工一律交设计师（2026-10-25 用户定）。
-- 起因：r87 的 09 卡在封面裁切——封面以「原图 + 裁切坐标」进到 09，发布员的工具不支持裁切参数。
-- 改动只在作业内容 / 自检 / 验收文字里加要求：05、11 只交原图；06 设计师交成品文件并承接所有图片加工；
-- 07、08 封面只认成品；09、12 不加工图片。阶段、依赖、闸、时限与 v12 完全相同。
--
-- **这一版只插 draft，不激活。** 激活用 adminctl（与 v9–v12 相同的做法）。

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
       dispatch_mode, action_class, sla_minutes, per_item, ack_minutes, idle_minutes
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

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '只交原图：不裁切、不改尺寸、不加字。哪张图需要加工（比例不合用、要出成品图等）在 index.md 写明，由主编派设计师。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'material';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET self_check_criteria = self_check_criteria || char(10) || '- 交的都是原图，没有裁切、改尺寸或加字；需要加工的已写明。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'material';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = acceptance || char(10) || '- 没有自行加工图片（裁切、改尺寸、加字）；需要加工的已写明，交主编派设计师。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'material';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '图片加工都归你（2026-10-25 用户定）：封面候选交可直接上传的成品文件（2.35:1、1920 × 817 px，JPG，sRGB），不交「原图 + 裁切坐标」。其他阶段需要裁切、改尺寸、转格式、出封面成品或按平台回读坐标出缩略图时，主编会派工或以补件交给你；交成品文件，写明输入来源与处理方式；按平台回读坐标裁出的图标明来源，不冒充平台截图。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'design_prep';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET self_check_criteria = self_check_criteria || char(10) || '- 封面候选是可直接上传的成品文件，没有交「原图 + 裁切坐标」。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'design_prep';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = acceptance || char(10) || '- 封面候选为成品文件（2.35:1、1920 × 817），不是原图加裁切参数。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'design_prep';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '封面以 06 设计师的成品文件为准，不接受「原图 + 裁切坐标」；需要换图或改图时派设计师（补件），不交给发布员。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'fulltext';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = acceptance || char(10) || '封面：采用 06 设计师的成品文件，不是原图加裁切参数。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'fulltext';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '图片只用成品：封面用 06 设计师交的成品文件；你不裁切、不改尺寸、不出封面或缩略图。缺成品图或需要改图时报主编，由主编派设计师补件。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'wx_layout';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET self_check_criteria = self_check_criteria || char(10) || '- 封面是设计师交的成品文件，没有自行裁切或改尺寸，没有用「原图 + 裁切坐标」代替。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'wx_layout';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = acceptance || char(10) || '封面成品：封面为设计师交的成品文件（2.35:1、1920 × 817），不是原图加裁切参数。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'wx_layout';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '不加工图片：封面就是 08 字段清单里的成品文件，原样上传；不裁切、不改尺寸、不出缩略图。平台拿不到的缩略图截图（如方形），把回读到的图片地址和坐标交主编，由主编派设计师出图，不自行加工、不冒充截图。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'wx_save';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET self_check_criteria = self_check_criteria || char(10) || '- 没有自行加工图片；封面是成品文件，原样上传。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'wx_save';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '只交原图：不裁切、不改尺寸；需要加工的图写明，由主编派设计师。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'xhs_pick';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '交付物：', '图片原样使用：不裁切、不改尺寸、不加字；需要加工的报主编，由主编派设计师。' || char(10) || char(10) || '交付物：')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'xhs_package';
-- +goose StatementEnd

-- 刻意没有 UPDATE workflows SET status='active' —— 见文件开头。

-- +goose Down

-- 只删这一版的 draft（06 作业内容里有「图片加工都归你」且是最新版本）；已激活的走后台回滚到 v12。
-- +goose StatementBegin
DELETE FROM workflows WHERE wf_key='daily_news' AND status='draft'
  AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')
  AND id IN (SELECT workflow_id FROM workflow_stages WHERE code='design_prep' AND instructions LIKE '%图片加工都归你%');
-- +goose StatementEnd
