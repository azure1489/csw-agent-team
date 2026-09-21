-- +goose Up
-- daily_news v9：01 交给情报收集员工作台。
--
-- **这一版只插 draft，不激活。** 激活是切换清单里单独的一步（wfctl activate daily_news@9），
-- 要在白天、无进行中 run、采集服务已放行验证通过之后人工执行。
-- 迁移里自动激活等于「部署即切换」——那会让回退变成一次紧急迁移，而不是停一个服务。
--
-- 01 改了什么：从「先扫一遍、挑 4–8 条」改成「窗口内每条图文贴文都判、每张图都识别」。
-- 判断按 Van 确认的六个维度，每维要给依据；结论只有四档，没有分数；
-- 五类对照材料缺一不判；没读到实图落待核，待核不是淘汰。
-- 02 / 03 改了什么：优先读新的判断台账，台账为空时才退回 intake-trace——
-- 回退到 Hermes 收集员那天又没有台账了，这条兼容不能省。

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

-- 01 整段重写：工作台的作业标准会被原样注入模型，逐句替换太脆。

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = '## 任务内容（怎么做）
引擎自动派工，派工单含当期窗口与采集要求。窗口按生产约定：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00。**窗口以首次进入来源库的时间为准**，不是以平台的发布时间——同一条贴文晚几天才被抓到，它属于被抓到的那一期。

### 这一阶段的底线
- **窗口内每条图文贴文都判，不设 top K、不抽样。** 只取图文：类型是图或轮播，且媒体里一个视频都没有；夹了视频的轮播不算。
- **每张图都识别。** 没真正读到实图的候选落「待核」，**不算判过，也不因此淘汰**，补齐图后重评。
- **不打数字分、不设权重。** 结论只有四档：推荐 / 备选 / 不推荐 / 待核。
- **来源不加权。** 某个账号粉丝多、某个平台条数多，都不构成优先；判的是这一条本身。
- **点赞、评论、标签、话题是输入，不是维度。** 它们帮你理解热度，不替你下结论。

### 判断框架（Van 确认版）
核心问题：**这件事有没有足够具体的变化，值得 CSW 帮读者解释一次。** 非新品也可因有依据的设计、文化、历史或当下关联成立。**品牌知名度不是标准。**

六个维度，各判 成立 / 不成立 / 不明，**每维都要给依据**（引正文原话，或指明第几张图）。编不出依据就判「不明」，不许空着，更不许编：
- change 变化必须具体——结构或使用方式改了什么、品牌首次做了什么、原产品为什么改。「某品牌发布新品」通常不够。
- use 与真实使用有关——人怎么穿、搭、住、跑、移动；产品为什么这样设计。
- gain 有 CSW 读者能理解的价值——新的使用思路、值得了解的设计逻辑、文化背景。
- compare 最好有比较参照——与上一代、品牌以往做法或同类产品的不同。参照要准确，**不虚构旧款问题**。
- explain 能产生有事实支持的编辑判断——能解释某设计影响什么使用动作。只有「好看」「有趣」不够。
- csw 视角——是否符合 CSW 的报道角度与读者。

**三句话必须答得出**：发生了什么变化？为什么值得户外用户知道？它和以前有什么不一样？答不清就写明是缺资料、角度未成立还是价值不足。

七类优先关注（老产品结构性改款；装备解决具体使用问题；品牌新的空间、社群或服务方式；装备进入新生活场景且有超出造型挪用的信息；小众品类值得解释的新设计；有真实争议或取舍的产品；能从产品解释有证据的消费或生活方式变化）与七类降低优先级（只有新配色或普通 logo 联名；换材质说不出差异；宣称创新无可核实变化；纯明星带货；缺可靠来源先补证；只能说好看；必须强行拔高成趋势）**都是倾向，不是黑名单**。

### 对照材料：五类缺一不判
判每一条之前，先备齐五类对照材料。任何一类取不到，就不要下结论，把这条落「待核」并写明缺哪一类：
1. 正式已发布的条目（`GET /api/v1/ledger/posts?since=90d&brand=品牌`）
2. 知识库范例
3. 生成过文章的贴文
4. 03 阶段的历史决定与原话（`GET /api/v1/ledger/decisions?since=90d`）
5. 上一轮台账

**30 天查重针对同一事实或同一角度，不按品牌名机械去重。** 同一品牌另一件事、同一件事另一个角度，都照常判。03 里被否决过的同事实同角度可以排除，但要写明依据，且能被人捞回来。

### 要上报的三样
1. **候选条目** `PUT /runs/{run_id}/items`：条目键 = 品牌英文或拼音小写 + 短横 + 原文链接 sha256 前 6 位（如 hxo-3fa91c，生成后不再改）。每条写 discovered_via、fetched_at、evidence_url、dedup_note、origin（dispatch / van_link / supplement）、first_seen_at。
2. **采集轮** `PUT /runs/{run_id}/sweeps`：每个采集器一条，tool 填 `csw_api`（直接调接口，不经 MCP）。found（接口返回，含重复）、fetched_unique（去重后）、reviewed（真读过正文或看过图并形成判断的）、unreviewed（只加载未展开的）、in_window、registered、result。**每条都判意味着 unreviewed 必须是 0**；不是 0 就说明这一步没跑完，如实写，不要凑。
3. **判断台账** `PUT /runs/{run_id}/intake-judgements`：窗口内每条候选一行，含四档结论、六维（各带依据）、三句话、对照结论、热度说明、gaps、image_seen。昨天已判、今天仍在窗口内的写 carried=true，**出现在台账里但不重判**。

### 先交首批
凑够一批推荐（一般 4–8 条）就先提交，等主编校准，再把其余判完的以**补件**追加，不重新提交整包。补件必须挂在已通过的交付物上。

### 交付物
文件夹 = index.md（按四档分组的逐条台账 + 采集轮次表 + 对照结论）+ images/（每条一张预览）+ trace/（sweeps.jsonl、items.jsonl、judgements.jsonl，字段与接口同名）。',
  self_check_criteria = '## 提交前自检（全过才交）
- 先调 `GET /runs/{run_id}/intake-check`，八条判据没有红的再提交。判红就修，不要带着红交上去让主编替你发现。
- **窗口内每条图文贴文都判了**：台账行数不少于采集轮上报的去重候选数，unreviewed 为 0。没判完就别说判完了。
- **每张图都识别了**；没读到实图的都落了「待核」，没有因此被淘汰。
- 六个维度齐全，**每维都有依据**，依据引的是正文原话或某张图，没有一条是编的。编不出来的判了「不明」。
- 三句话每条都答得出；答不清的写明是缺资料、角度未成立还是价值不足。
- 五类对照材料齐备；缺任何一类的条目落了「待核」并写明缺哪一类。
- 查重针对的是同一事实或同一角度，不是同一品牌名；同一品牌的另一件事没有被误杀。
- 没有给任何条目打数字分，没有按来源或粉丝数加权。
- 采集轮 tool 填的是 csw_api，found / fetched_unique / reviewed / unreviewed / in_window / registered 各有数且对得上。
- 每条条目都填了 published_at 与 first_seen_at；拿不到确切披露日期的落「待核」并写明缺的就是日期。
- 首批已成形就交了，没有为个别对象无限深挖；其余判完的走补件追加。
- 交付物结构完整：index.md 按四档分组，预览图在 images/，三份 jsonl 在 trace/。',
  acceptance = '## 验收标准（主编首批校准）
- 先看 `GET /runs/{run_id}/intake-check`：每条都判是否通过（台账行数盖住去重候选数、未审为 0）、待核是否一致（没读到实图的都落了待核）、必扫来源是否都有轮次、窗口是否合规。
- **审阅覆盖是代码算出来的，不是自我声明。** 采集轮的 unreviewed 不为 0 就是没跑完，不接受「其余的看过但没记」。
- 每条的六维都有依据，依据能在正文或图里核到。**发现编造的依据直接退回**，这比漏判更严重。
- 待核的条目写明了缺什么，且没有被当成淘汰处理——补齐后要能重评。
- 首批条数够判断方向（一般 4–8 条），是按判断结论排出来的推荐，不是单一对象的深挖包。
- 查重结论具体：指出最相关的一条已发内容及本次差别；未检到时写「在已同步记录中未发现」并注明覆盖范围。
- 没有出现分数、权重、按来源加权的排序。
- 主编在本闸给出方向：哪些继续、哪些停止、还缺哪类来源；退回时写明方向与位置，不是笼统的「再多找几条」。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'intake';
-- +goose StatementEnd

-- 02 / 03 只补「先读台账、没有再退回」这一句，其余不动。

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = replace(instructions, '01 的采集过程可以直接读：GET /runs/{run_id}/intake-trace 给出采集轮次', '01 的判断台账可以直接读：`GET /runs/{run_id}/intake-judgements` 给出窗口内**每条**候选的结论档、六个维度各自成立与否与依据、三句话、对照结论。台账里「不推荐」的那些也在，你能看见 01 判了什么、凭什么判——不必只看它挑出来的那几条。**台账为空时**（旧期，或收集员没走工作台）再退回 `GET /runs/{run_id}/intake-trace`。

`GET /runs/{run_id}/intake-trace` 给出采集轮次')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'shortlist';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = replace(instructions, '读 GET /runs/{run_id}/intake-trace 或 adminctl intake-check 的覆盖结论', '先读 `GET /runs/{run_id}/intake-judgements`（有台账时它最全：每条候选判了什么、凭什么判），台账为空再退回 `GET /runs/{run_id}/intake-trace`；覆盖结论看 `GET /runs/{run_id}/intake-check`')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'topic';
-- +goose StatementEnd


-- 刻意没有 UPDATE workflows SET status='active' —— 见文件开头。

-- +goose Down

-- +goose StatementBegin
DELETE FROM workflows WHERE wf_key='daily_news' AND status='draft'
  AND id = (SELECT MAX(id) FROM workflows WHERE wf_key='daily_news' AND status='draft');
-- +goose StatementEnd
