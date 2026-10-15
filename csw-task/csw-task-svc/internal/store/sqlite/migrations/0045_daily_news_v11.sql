-- +goose Up
-- daily_news v11：去掉 02 价值初筛与 03 选题方案（2026-10-16 用户确认，见 docs/选题流程简化_去掉02与03_方案_20261016.md）。
--
-- **这一版只插 draft，不激活。** 激活由用户在后台或 adminctl 确认（与 v9 / v10 相同的做法）。
-- 复制源是「最新版本」：生产上 v10 已激活，新库上 v10 还是 draft，两边都该从 v10 复制。
--
-- 结构：shortlist / topic 两段及其闸不复制；04 写作、05 配图、06 版式改为依赖 01；
-- 01 在「主编格式核对」之后加「Van选题」（主编代录）——末闸，条目批准随它录入（topic_approve + items）。
-- 阶段名与序号不动：02、03 空号，存储路径（按阶段名派生）不变。
-- 01：一次交齐（01 补件会把已开工的 04 / 05 / 06 全标待返工）、条目状态由 01 落定、给 Van 的选题单、
-- 缺量说明与补证停止条件从 03 搬来、查近发与引用反馈从 02 搬来。

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
FROM workflow_stages WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news') - 1)
  AND code NOT IN ('shortlist', 'topic');
-- +goose StatementEnd

-- 依赖：照抄不涉及 02 / 03 的边；原来依赖 03 的（写作、配图、版式）改依赖 01。
-- +goose StatementBegin
INSERT INTO workflow_stage_deps (workflow_id, stage_id, depends_on_id)
SELECT ns.workflow_id, ns.id, nd.id FROM workflow_stage_deps d
JOIN workflow_stages os ON os.id = d.stage_id
JOIN workflow_stages od ON od.id = d.depends_on_id
JOIN workflow_stages ns ON ns.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND ns.code = os.code
JOIN workflow_stages nd ON nd.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND nd.code = CASE WHEN od.code = 'topic' THEN 'intake' ELSE od.code END
WHERE os.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news') - 1)
  AND os.code NOT IN ('shortlist', 'topic');
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), ns.id, g.gate_order, g.reviewer_role, g.relayed_by_hub, g.name
FROM workflow_gates g JOIN workflow_stages os ON os.id = g.stage_id
JOIN workflow_stages ns ON ns.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND ns.code = os.code
WHERE g.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news') - 1)
  AND os.code NOT IN ('shortlist', 'topic');
-- +goose StatementEnd

-- 01 加 Van 选题闸（主编代录、末闸）。
-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT s.workflow_id, s.id, 2, 'van', 1, 'Van选题'
FROM workflow_stages s
WHERE s.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND s.code = 'intake';
-- +goose StatementEnd

-- 通用验收：阶段序号清单去掉 02 / 03。
-- +goose StatementBegin
UPDATE workflows SET common_acceptance = REPLACE(common_acceptance, '02-价值初筛 / 03-选题方案 / ', '')
WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news');
-- +goose StatementEnd

-- 01 作业内容：对照材料、交付方式、交付物。
-- +goose StatementBegin
UPDATE workflow_stages SET instructions = REPLACE(REPLACE(REPLACE(instructions,
  '4. 03 阶段的历史决定',
  '4. 历史选题决定（v11 起在 01 的 Van 选题闸录入，此前在 03；按条目决定读，`recent_rejected` 已随任务详情附上）'),
  '### 先交首批
凑够一批推荐选题（一般 4–8 个）就提交，交主编**核对格式与材料**（五项清单）。主编对内容的意见写在通过意见里，随 02 派工单交选题研究员采用；**工作台不按内容意见返工**，只在格式或材料不全被退回时补材料、重新导出。',
  '### 条目状态由你落定（v11 起，原 02 的活）
登记条目时按判断结论写状态，Van 在本阶段的闸上直接按它选题：
- 推荐 → `shortlisted`，rank = `primary`（主选）
- 备选 → `shortlisted`，rank = `alt`（备选）
- 待核 → `pending_check`，必须有「影响判断」级的缺口；备选里还挂着影响判断的缺口的（备选·待补证）也登记成待核，不算成熟
- 不推荐 → **不登记成条目**：逐帖结论与理由在判断台账里，那是「都判过了」的证据。之前登记过、这一版改判不推荐或被并入别的选题的，标 `dropped` 并给 reason_code（no_value 价值不足 / not_new 非新披露只是转发 / dup_published 近期已发同角度 / dup_recent_rejected 近期已被否决或暂缓 / out_of_window 不在窗口 / evidence_missing 证据或图片不足 / aesthetic_mismatch 审美或价值不符 / superseded 被同期更好的取代 / other 须写明）与一句理由
不留 `candidate`——那是「还没判断」的意思。Van 已批准的条目重新登记不会被改回。

### 原 02 的三项，写进交付物
- **查近发**：主推条目对照 `GET /api/v1/ledger/posts?since=90d&brand=品牌`（含 Van 人工发布与拆条）；查不到写「在已同步记录中未发现」并注明覆盖范围。最近几期被否决或暂缓的同对象（任务详情 `recent_rejected`）要标出来。
- **引用编辑反馈**：任务详情随附的 feedback 与候选相关的，写明引用了哪条、符合还是触碰；一次否决不推成永久禁选，推断的偏好不当成 Van 原话。
- **缺量归因**：某个平台整轮没扫或整轮失败，写进缺量说明，这是缺量归因的依据。

### 待核在本阶段闭环
**有明确报道潜力的待核**，提交前定点深核完：补到了就重评定档，补不到决定选题的关键资料的定为「备选·待补证」，缺口交主编。其余待核不逐条深核，如实留在待核栏、写明缺什么——不因此拖住一次交齐。补证有停止条件：先回答「查清这个问题是否可能改变采用决定」，只是多几个参数或图片的不追加；同一核心问题一轮仍解决不了就停，不拖住其他候选；因价值低被否决的题不换说法重新包装。提交时仍待核的，Van 不选，由主编之后标继续研究或留到下期。

### 缺量说明
推荐不足生产约定的数量时，在选题单后单列：差多少（主选、备选分别已有几条、各缺几条）；为什么缺（按实际候选归类：有价值资讯偏少 / 采集漏项 / 近期已发布 / 审美或价值不符 / 证据或图片不足，不写笼统的「素材不足」；写「采集漏项」要有 `intake-check` 的覆盖事实）。怎样补、补不齐怎么办由主编在通过意见里给 Van。不扩大窗口、不降低标准、不用旧闻或弱题凑数。06:00 以后出现的资讯不进当期主选。

### 一次交齐（v11 起）
窗口内全部判完、有报道潜力的待核深核完再提交，**一次交齐，不分首批、不用补件追加批次**：本阶段通过后写作、配图、版式直接开工，这时交补件会把它们全部标成「补件待返工」。补件只用来改错，并在摘要里写明影响哪几条。Van 选题之后要加题，由主编走条目补批（`POST /runs/{run_id}/items/{条目键}/decision`），不经本阶段。

提交后两道闸：**主编格式核对**（五项清单，只查格式与材料；主编的内容意见写在通过意见里，写给 Van 看）→ **Van选题**（主编代录）。**工作台不按内容意见返工**，只在格式或材料不全被退回时补材料、重新导出。'),
  '### 交付物
文件夹 = index.md（选题一览 + 按四档分组的逐帖台账 + 采集轮次表）',
  '### 给 Van 的选题单（index.md 最前面）
Van 不看附件、不等补资料也能做决定：
- **推荐选题逐张列**：标题「具体对象｜一句推荐理由」、三句话（是什么 / 为什么值得看 / 依据）、原始披露时间与主要来源、查重结论（最相关的已发内容及本次差别）、影响成稿的缺口、一张代表图。主选在前，备选在后。
- **待核**：一个短列表，每条一句缺什么。
- **不推荐**：只给条数与理由码分布，不展开（逐帖在台账附录里）。
- 末尾写五栏条数：线索 / 待核 / 成熟主选 / 成熟备选 / 淘汰，与引擎登记一致；推荐贴文数与独立选题数分开给出。

### 交付物
文件夹 = index.md（给 Van 的选题单 + 缺量说明 + 按四档分组的逐帖台账（附录）+ 采集轮次表）')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND code = 'intake';
-- +goose StatementEnd

-- 01 自检：追加 v11 的几项。
-- +goose StatementBegin
UPDATE workflow_stages SET self_check_criteria = self_check_criteria || '
- **条目状态已落定**：推荐为 shortlisted + primary、备选为 shortlisted + alt、待核为 pending_check 并有「影响判断」级缺口；不推荐不登记（逐帖在台账里），此前登记过又改判的标了 dropped 并有 reason_code 与理由；没有留在 candidate 的。
- **有报道潜力的待核已深核**：补得到的已重评定档；补不到的写明缺什么、谁处理、已尝试什么；其余待核写明缺什么，没有为它们拖住提交。
- 主推条目查过近发并写了结论；相关编辑反馈写明引用了哪条；最近被否决或暂缓的同对象已标出。
- **给 Van 的选题单在 index.md 最前面**：推荐逐张有标题、三句话、来源与时间、查重、缺口、代表图；待核与不推荐只列简表；五栏条数与引擎一致。
- 推荐不足数量时写了缺量说明（差多少、为什么缺）；归因到采集漏项的有覆盖事实。
- **一次交齐**：窗口内全部判完再提交，没有留待补件追加的批次。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND code = 'intake';
-- +goose StatementEnd

-- 01 验收：两道闸各一节。
-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = '## 验收标准（主编格式核对）
**本闸只查格式与材料是否齐全，不改贴文的判断与去向**（2026-10-10 分工调整）。五项都符合就通过，哪怕对内容有不同看法。
1. **包结构**：index.md（选题单在最前面）、images/、trace/ 下的必备文件齐全，首页链接可打开。
2. **窗口与采集**：窗口起止与派工一致；采集轮已上报；接口翻到最后一页；补采有记录。先看 `GET /runs/{run_id}/intake-check`。
3. **全部判过**：窗口内每条候选都有判断；没看到实图的标待核；条目状态已落定，没有留在 candidate。
4. **材料齐全**：选题单的推荐与备选附原图；原始发布时间与首次入库时间都写了；原文与译文都在。
5. **对账一致**：包与引擎逐键一致（五栏条数、主选 / 备选与登记一致）；intake-check 无红灯。

- **只有不符合这五项才退回**，退回意见写明是第几项、哪个文件或条目；工作台返工只补材料、重新导出，不改判断。
- **对内容的意见写在通过意见里，写给 Van 看**：你会选哪几个、哪几条是同一件事、代表图、译名、缺量怎样补与补不齐的建议。不为内容意见退回 01。

## 验收标准（Van选题 · 主编代录）
- 交给 Van 的是选题单这份实物（推荐卡、代表图、查重、缺口）和你的通过意见，不是 zip 路径、任务版本号或审计条数。
- Van 的决定由你代录：comment 写 Van 原话，并逐条记录采用（可写）、继续研究、暂缓、否决。「保留」「就这条」按采用；「再看看」「继续研究」按继续研究；措辞不明时只问一次受影响的范围。
- 录入时 decision_type 写 topic_approve，items 写批准可写的条目键：引擎为每条生成写作与配图任务并自动派工，版式准备同时就绪。继续研究、暂缓、否决的条目用条目决定（`POST /runs/{run_id}/items/{条目键}/decision`）逐条录，同样附 Van 原话。之后补批或撤回单条也走条目决定。
- 整单不可用（窗口错、来源大面积漏扫）才退回给收集员，写明方向与位置；只选一部分是常态，不退回。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND code = 'intake';
-- +goose StatementEnd

-- 04 / 07：选题方案与短名单的引用改成 01 的选题单与 Van 选题。
-- +goose StatementBegin
UPDATE workflow_stages SET instructions = REPLACE(instructions, '选题卡与短名单作写作依据', '01 的选题单与判断台账作写作依据')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND code = 'write';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = REPLACE(instructions, '（按选题方案顺序）', '（按 Van 选题的顺序，Van 没排序的主选在前）'),
  self_check_criteria = REPLACE(self_check_criteria, '条目顺序与选题方案一致', '条目顺序与 Van 选题一致')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND code = 'fulltext';
-- +goose StatementEnd

-- 刻意没有 UPDATE workflows SET status='active' —— 见文件开头。

-- +goose Down

-- 按内容认出 v11（没有 topic 阶段、01 带 Van 选题闸），只删 draft；已激活的走后台回滚。
-- +goose StatementBegin
DELETE FROM workflows WHERE wf_key='daily_news' AND status='draft'
  AND id IN (SELECT workflow_id FROM workflow_stages
             WHERE code='intake' AND instructions LIKE '%一次交齐（v11 起）%');
-- +goose StatementEnd
