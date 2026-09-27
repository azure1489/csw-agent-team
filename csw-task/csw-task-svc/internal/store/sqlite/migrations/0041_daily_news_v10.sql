-- +goose Up
-- daily_news v10：01 换成 Van 09-23 认可的新六维与判断台账反馈八项的口径。
--
-- **这一版只插 draft，不激活。** 激活由用户在后台网页点（与 v9 相同的做法）。
-- 复制源是「最新版本」而不是 active：生产上 v9 已激活，新库上 v9 还是 draft，两边都该从 v9 复制。
--
-- 01 改了什么：核心问题换成「值不值得推荐给中国户外潮流读者」；六维换新定义（键名不变）；
-- 三句话换成「是什么 / 为什么值得看 / 依据」并加标题；不许无证据写前后对比；
-- 没读到不等于没价值；查重加「未确认」、生成稿不算已发；缺口分三级；按选题登记。
-- 02 / 03 只补一段「读判断台账（v10 起）」，其余不动。

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

-- 01 整段重写

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = '## 任务内容（怎么做）
引擎自动派工，派工单含当期窗口与采集要求。窗口按生产约定：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00。**窗口以首次进入来源库的时间为准**，不是以平台的发布时间——同一条贴文晚几天才被抓到，它属于被抓到的那一期。

**目标：让系统更准确地找到值得推荐给中国户外潮流读者的内容，减少 Van 重新筛选、补资料、解释和催促的工作。**

### 这一阶段的底线
- **窗口内每条图文贴文都判，不设 top K、不抽样。** 只取图文：类型是图或轮播，且媒体里一个视频都没有；夹了视频的轮播不算。
- **每张图都识别。** 识图步骤逐张看原图、写下看图结果；判断时看的是这些看图结果，另附几张缩略直接看外观——**看图结果齐了就是读到了实图**，不因「只附了几张缩略」落待核。有图下载或识别失败的候选落「待核」，**不算判过，也不因此淘汰**，补齐图后重评。
- **不打数字分、不设权重。** 结论只有四档：推荐 / 备选 / 不推荐 / 待核。
- **来源不加权。** 某个账号粉丝多、某个平台条数多，都不构成优先；判的是这一条本身。
- **点赞、评论、标签、话题是输入，不是维度。** 它们帮你理解热度，不替你下结论。

### 判断框架（Van 确认版，v10 起）
核心问题：**这条值不值得推荐给中国户外潮流读者？凭什么——读者看完能得到什么有用的信息、有趣的认识，或值得了解的设计与文化内容？** 不强求新品；品牌知名或国外发布不自动推荐；参数、价格、发售日期齐全不证明价值。

六个维度，各判 成立 / 不成立 / 不明，**每维都要给依据**（引正文原话，或指明第几张图）。编不出依据就判「不明」：
- change 具体看点——最值得报道的具体事实、变化或角度；可以是改款，也可以是有依据的设计、文化、历史或生活方式内容。
- use 与读者有关——与中国户外潮流爱好者的使用、穿搭、审美、兴趣或生活方式的联系，不限于解决功能问题。
- gain 值得推荐的价值（**核心**）——为什么值得推荐给中国户外潮流读者，看完能获得什么。
- compare 差异与背景——同类、历史、品牌背景或既有做法，不强求上一代产品对比。
- explain 报道角度与依据——CSW 抓哪个具体角度，哪些事实支持，能讲到什么程度；不要求制造趋势。
- csw CSW 适配度——结合实际采用和否决案例说明；不能只因出现露营、户外、旅行等词就判符合。

**六维不是打勾表，不按成立个数定档。推荐必须「值得推荐的价值」与「CSW 适配度」都成立且有依据**；任一不成立，其余几项再齐也最多备选。推荐位是给 Van 选题用的，门槛要高。要分清：跟谁有关（读者关联）、为什么值得看（读者价值）、具体讲什么（报道角度）、为什么适合由 CSW 来讲（适配度）。

**中国读者价值不等于国内买得到、去得了**：国外店铺开业只有地址和营业时间通常价值有限，独特的设计、服务或文化内容可能值得介绍；日本限定装备，限定身份本身不够，具体结构、工艺和审美表达可以成为看点；海外活动，普通报名通知价值有限，值得了解的社群组织方式或文化实践可能有报道价值。

**三句话必须答得出**：是什么？为什么值得中国户外潮流读者看？依据是什么？每条另给一个标题「具体对象｜一句推荐理由」（40 字以内）。

**不要为了满足「变化」制造前后对比。** 分清三种看点：产品现有特点 / 有证据的新变化 / 值得解释的设计或文化内容。**没有旧款或前代的证据，就不写「从……变成……」。** 非新品也可以值得报道。

七类优先关注（老产品结构性改款；装备解决具体使用问题；品牌新的空间、社群或服务方式；装备进入新生活场景且有超出造型挪用的信息；小众品类值得解释的新设计；有真实争议或取舍的产品；能从产品解释有证据的消费或生活方式变化）与七类降低优先级（只有新配色或普通 logo 联名；换材质说不出差异；宣称创新无可核实变化；纯明星带货；缺可靠来源先补证；只能说好看；必须强行拔高成趋势）**都是倾向，不是黑名单**；新品不是前提。

### 对照材料：五类缺一不判，另加选题记忆
1. 正式已发布的条目　2. 知识库范例　3. 生成过文章的贴文　4. 03 阶段的历史决定　5. 上一轮台账；另外按品牌带上 Van 的采用 / 否决案例（选题记忆，只带对象与结论）。

### 查重结论必须与证据一致
- 查重针对同一事实或同一角度，不按品牌名机械去重。结论四种：无重复 / 同品牌有增量 / 同事实无增量 / **查重未确认**。
- 逐条列出命中的历史文章：标题、发布状态（正式发布 / 已推草稿箱 / 仅生成稿）、具体重复了哪条事实。
- **历史文章正文缺失、核不了事实时，只能是「查重未确认」**，不能给确定的无重复结论。
- **生成稿可以帮着复用资料，但不能充当「近期已发」的证据。**

### 区分「价值不足」和「关键资料尚未取得」
已读到关键内容、确认价值不足的，才判不推荐；贴文没有指向别处、本身就是全部内容，只是信息单薄说不出看点（使命文案、补货通知、只有风景、只列参数、合作预告没说做了什么），是价值不足，判不推荐，不落待核。**完整内容在主页外链、正文截断这类「没读到」的，落待核**，并写明补证路径；有明确报道潜力的，工作台会定点补读外链后重评（只抓正文里的地址，内网与云元数据地址一律不碰）。待核的会进深核；**深核过的不再留在待核**：深核后仍缺决定选题的关键资料的，定为「备选·待补证」，缺口交主编。

### 缺口分三级，写清下一步
- **影响选题判断**：产品身份、关键看点或报道价值无法确认。**只有这一级能让条目落待核。**
- **影响成稿**：必要规格、关键图片或时间信息缺失。不影响档位，随条目转给写作。
- **表达边界**：如品牌声称的性能未经独立实测。只是写作提醒，不是补证任务。
每条缺口写：缺什么、由谁处理（收集员 / 主编 / Van）、已尝试什么、下一步。「图片没拍清全部结构，但可靠正文明确说明」不算缺口。事实可靠性、可用配图、资料完整度单独记在制作条件里，不与选题价值混在一起。

### 逐帖之外形成选题
同一产品、同一事件的多条贴文合并成一个选题：汇总互补的原文、图片与证据，标出哪些是重复、哪些是新增；真正不同的报道角度或后续变化单列。**推荐贴文数与独立选题数分开统计，推荐位按选题算。** 条目按选题登记（条目键取代表帖），其余帖子写进去重说明；逐帖判断仍全部写进判断台账。

### 要上报的三样
1. **候选条目** `PUT /runs/{run_id}/items`：按选题登记；条目键 = 代表帖的键（品牌小写 + 短横 + 原文链接 sha256 前 6 位，生成后不再改）；title 用「具体对象｜一句推荐理由」；dedup_note 列同选题的其余帖子。
2. **采集轮** `PUT /runs/{run_id}/sweeps`：每个采集器一条，tool 填 `csw_api`。found、fetched_unique、reviewed、unreviewed、in_window、registered、result 各有数；**unreviewed 必须是 0**，不是 0 就如实写。
3. **判断台账** `PUT /runs/{run_id}/intake-judgements`：窗口内每条候选一行，含四档、六维（各带依据）、三句话（含标题与看点类型）、查重（含命中清单与制作条件）、分级缺口、image_seen；同选题的帖子挂在该选题的条目键上。

### 先交首批
凑够一批推荐选题（一般 4–8 个）就先提交，等主编校准，再把其余判完的以**补件**追加，不重新提交整包。

### 交付物
文件夹 = index.md（选题一览 + 按四档分组的逐帖台账 + 采集轮次表）+ images/（每条一张预览）+ trace/（sweeps.jsonl、items.jsonl，字段与接口同名）。',
  self_check_criteria = '## 提交前自检（全过才交）
- 先调 `GET /runs/{run_id}/intake-check`，判据没有红的再提交。
- **窗口内每条图文贴文都判了**：台账行数不少于采集轮上报的窗口内候选数（in_window），unreviewed 为 0。
- **每张图都识别了**；图下载或识别失败的落了「待核」，没有因此被淘汰；看图结果齐全的没有因「没看全图」落待核。
- 六维齐全、**每维有依据**，没有编造；**推荐的条目「值得推荐的价值」都成立**，没有按成立个数定档。
- 每条有「具体对象｜一句推荐理由」的标题；三句话答得出。
- **没有旧款证据的地方，没有写「从……变成……」**。
- 没读到关键内容的落了待核并写了补证路径，**没有判成不推荐**。
- 查重结论与证据一致：历史正文缺失的是「查重未确认」；**没有拿生成稿当已发证据**；命中的文章都列了状态与重复事实。
- 待核条目都有「影响判断」级的缺口，写了谁处理、已尝试什么、下一步；影响成稿的缺口没有让条目落待核。
- 同一产品、同一事件的多帖合成了一个选题，没有各占一个推荐位。
- 没有打分、没有按来源或粉丝数加权。
- 交付物结构完整：index.md 有选题一览与四档逐帖台账，预览图在 images/，jsonl 在 trace/。',
  acceptance = '## 验收标准（主编首批校准）
- 先看 `GET /runs/{run_id}/intake-check`：每条都判、待核一致、必扫来源有轮次、窗口合规。
- **审阅覆盖是代码算出来的，不是自我声明。**
- 推荐的条目说得清「为什么值得推荐给中国户外潮流读者」，依据能在正文或图里核到。**发现编造的依据或无证据的前后对比直接退回。**
- 查重结论与证据一致：列出命中的文章与状态；正文缺失的写「查重未确认」；生成稿没被当成已发。
- 待核的写明缺什么、谁处理、下一步；没读到的没有被当成没价值。
- **推荐按选题算**：同一产品或事件不占多个推荐位；推荐贴文数与独立选题数分开给出。
- 首批条数够判断方向（一般 4–8 个选题），是按判断结论排出来的推荐，不是单一对象的深挖包。
- 主编在本闸给出方向：哪些继续、哪些停止、还缺哪类来源；退回时写明方向与位置。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'intake';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = instructions || '

### 读判断台账（v10 起）
- **按选题读**：同一选题的多帖挂在同一个条目键上（`intake-judgements` 里的 item_key），推荐位按选题算，不要把同一产品的几帖当成几条。
- 标题「具体对象｜一句推荐理由」在 three_sentences.headline；三句话是「是什么 / 为什么值得看 / 依据」。
- 查重看 comparison：「查重未确认」表示历史正文缺失、核不了；**仅生成稿不算近期已发**。
- 缺口分三级：只有「影响判断」级的会让条目待核；「影响成稿」「表达边界」是写作提醒。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code IN ('shortlist', 'topic');
-- +goose StatementEnd

-- 刻意没有 UPDATE workflows SET status='active' —— 见文件开头。

-- +goose Down

-- 按内容认出 v10，而不是「最新的那个草稿」：部署后有人再建草稿，回滚也不会删错。
-- 已激活（有 run 指着它）的不删——那要走后台回滚，不是迁移回滚。
-- +goose StatementBegin
DELETE FROM workflows WHERE wf_key='daily_news' AND status='draft'
  AND id IN (SELECT workflow_id FROM workflow_stages
             WHERE code='intake' AND instructions LIKE '%判断框架（Van 确认版，v10 起）%');
-- +goose StatementEnd
