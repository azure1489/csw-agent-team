-- +goose Up
-- daily_news v4：按 2026-09-16 失败复盘（主编《选题交付改进方案》）改三处——
-- · 01-情报逐条加一道主编闸「主编首批校准」：首批交齐即校准方向，避免弱题被做厚之后才否决；
-- · 02-价值初筛改为「A 价值 / B 资格 两道轻检查通过才深入」，新增待核（pending_check）与主选/备选分级；
-- · 03-选题方案加五栏追踪、补证停止条件、检查点主动核结果、交 Van 的是实物。
-- 写法同 0024：复制当前 active 版本的阶段、依赖、闸，再按 code 定点更新；最后归档旧 active、激活新版。

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

-- 01 新增主编首批校准闸（v3 里 01 是 0 闸）。
-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), s.id, 1, 'editor', 0, '主编首批校准'
FROM workflow_stages s WHERE s.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND s.code = 'intake';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
引擎自动派工，派工单含当期窗口与采集要求。按生产约定的窗口采集：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00；以原始披露时间判断是否在窗口内，转载时间不算。

**先交首批，等主编校准，再决定往哪里深挖。** 派工后先用约 20 分钟扫一遍已授权来源，把成形的线索（一般 4–8 条）作为第一版提交：本阶段有一道主编闸，主编按原文、原始时间、预览图判断方向，通过后你再继续补采。没等到校准结论就深挖单一对象，是这条流程要避免的浪费。

每条记录：
- 品牌／产品或事件，一句话说清是什么；
- 原文链接（完整 https）、来源平台、原始发布时间（与转载时间分开写）；
- 一张预览图（images/）；
- 已知缺口：缺什么事实、缺什么图。

首批不必为每条下载三张图，也不复制历史全包；进入短名单的条目在 05-配图与素材核 再补图。近期发过的同产品同角度内容照样记录并标注，是否采用由研究员和主编判断，不按品牌名机械去重。任务详情随附近 30 天已发标题与最近几期被否决、暂缓的条目及理由，登记前先对照，重复或刚被否决的同角度对象直接标注，不重新包装成新题。

每条情报登记为一个条目（PUT /runs/{run_id}/items；条目键 = 品牌英文或拼音小写 + 短横 + 原文链接 sha256 前 6 位，如 hxo-3fa91c，生成后不再改）。关键事实未核实的登记为 pending_check（待核），不要直接标 shortlisted。

主编校准通过后：按校准意见继续补采，新条目继续登记，并以补件追加到本任务，不重新提交整包，也不等全部抓完再交。同一对象的补证由主编按「这个问题能否改变采用决定」派单，没有新的核心依据不自行反复补图补参数。

工具：Instagram 用营事编集室 csw MCP（csw_posts_search → csw_posts_get，无需拟人间隔）；小红书用 opencli xiaohongshu，每步插入随机间隔（搜索后 8–25 秒、读笔记后 5–18 秒、下载后 5–18 秒、切换平台约 45–55 秒）；网页优先 opencli 适配器，没有则 WebFetch。一个来源失败就换已授权的其他来源或同产品其他页面，不把单页失败当成整期停止条件。

交付物：文件夹 = index.md（逐条情报记录，首批与后续补件分别成段）+ images/（每条一张预览）。',
  self_check_criteria='## 提交前自检（全过才交）
- 首批已成形就交，没有为个别对象无限深挖；每条都有品牌／产品或事件、原文链接、来源平台、原始发布时间。
- 原始披露时间在当期窗口内；转载时间与披露时间分开写。
- 每条有一张能判断外观的预览图，已放进 images/，相对链接可用。
- 关键事实未核实的条目已登记为 pending_check，没有直接标 shortlisted。
- 已对照任务详情随附的近 30 天已发与最近被否决条目，重复或刚被否决的同角度对象已标注。
- 缺口已逐条写明，没有用猜测补齐事实。
- 小红书与网页采集按规定插入了随机间隔。',
  acceptance='## 验收标准（主编首批校准）
- 首批在派工后尽快到达，条数够主编判断方向（一般 4–8 条），不是单一对象的深挖包。
- 条目信息四要素齐全：是什么、原文链接、来源平台、原始发布时间；每条附一张可判断外观的预览。
- 窗口判断以原始披露时间为准，没有把转载或晚抓到的旧内容当作当期新闻。
- 与近 30 天已发、最近被否决条目的重复情况已标注，没有换说法重报同一对象。
- 主编在本闸给出方向：哪些对象继续投入、哪些停止、还缺哪类来源；退回时写明方向与位置，不是笼统的「再多找几条」。
- 规范合规：index.md 为逐条清单，预览图在 images/，相对链接可用。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'intake';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
与采集交错进行：情报逐条到达就开始判断，不等整包补齐。你判断的是「是否值得继续投入」，不是资料是否齐全。01 的主编首批校准结论是你的起点，方向已被主编停掉的对象不再投入。

**先过两道轻检查，都成立才深入。**
- A 价值：是什么、这次的变化或设计、文化、使用关系看点是什么、CSW 读者具体得到什么。依据现有官方原文与预览图判断即可，不要求先证明「结构革命」。设计、文化、使用细节都可以成立，但要说清读者得到什么：例如折边带来拆洗便利可以成立；仅有昼夜反差的蓄光不自动成立。
- B 资格：是否新披露或仅为转发；该型号的历史与已发对照（GET /api/v1/ledger/posts?since=90d&brand=品牌，含 Van 人工发布与拆条）；最近几期是否已被否决或暂缓（任务详情随附）；是否在当期窗口内。

两道都过再核会影响采用决定的关键事实，再谈制作准备。A 或 B 不成立就直接淘汰，不用补图、补参数把弱题做厚。

关键事实不明的条目标 pending_check（待核）并写明缺哪一条事实，不计入成熟主选或成熟备选。归属清楚时不要把秒级时间精度当额外门槛；核心时间冲突（窗口归属、首发归属）不得忽略。

结论四种：成熟采用（主选）、成熟备选、待核、淘汰，每条附一句理由。成熟采用与成熟备选在引擎里标 shortlisted 并写明 rank（primary 主选 / alt 备选）；待核标 pending_check；淘汰保持 candidate 并写清淘汰理由。评分只辅助排序，不能压过明确的淘汰理由；资料完整但缺少编辑吸引力的，先淘汰。

相关编辑反馈：任务详情随附的 feedback（按阶段与品牌匹配的 Van 原话、采用 / 打回案例）。与候选相关的写明引用了哪条、符合还是触碰；一次否决不推成永久禁选，推断的偏好不当成 Van 原话。

不强行找前代升级对照，没有前代资料就准确描述本次披露，不编造「升级」「首次」「更好用」。可以用清楚的工作标题标识选题（正式标题归文案）。情报数量不足时，仍交可用条目并写明缺口，不把整包不足等同于每条都不能研究。

交付物：文件夹 = index.md（成熟采用 / 成熟备选 / 待核 / 淘汰四栏，每条含 A 价值与 B 资格的结论、理由、查重结论、引用的反馈、缺口、对应情报链接）。',
  self_check_criteria='## 提交前自检（全过才交）
- 每条候选都写了 A 价值与 B 资格两道检查的结论，都成立才进入深核。
- 结论落在四栏之一：成熟采用（主选）、成熟备选、待核、淘汰，各附一句理由。
- 成熟条目已标 shortlisted 并写明 rank；待核条目已标 pending_check 并写明缺哪一条事实。
- 每条都写了查重结论：对照到的已发内容及差别，或「在已同步记录中未发现」加覆盖范围；最近几期被否决、暂缓的同对象已标注。
- 与候选相关的编辑反馈已写明引用了哪条；没有把推断当成 Van 原话。
- 没有编造「升级」「首次」等比较；没有前代资料的按本次披露描述。
- 没有为淘汰候选补图、补参数；影响成稿的缺口已写明。',
  acceptance='## 验收标准（主编对照）
- 四栏结论清楚：成熟采用、成熟备选、待核、淘汰；待核不被算作成熟数量。
- A 价值写清读者具体得到什么，落在可见、可核的内容上；没有用参数堆砌代替看点。
- B 资格写清新披露与否、已发对照、最近是否被否决、窗口归属；核心时间冲突已处理。
- 查重有依据；没有推荐已知近期写过的同角度内容，也没有把被否决对象换说法重报。
- 淘汰理由具体，没有被总分覆盖；弱题没有靠补证做厚。
- 每条都能对回情报记录与原文链接。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'shortlist';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
合流阶段，由主编亲自完成：短名单通过后引擎自动派工给你，上游已预填。送审时方案必须已经成熟：成熟候选、预览图、查重结论与推荐理由都已备齐，Van 审题时不现场等研究员补资料。

1. 亲自看图、读原文、核来源与时间、查近期发布记录与相关编辑反馈，不只转述研究员的结论。审美是否匹配、近期是否重复、整期品牌与品类是否扎堆，由你决定。
2. 按生产约定的数量（主选与备选）排序，形成选题方案。每条一张选题卡：
   - 品牌／产品或事件、原始发布时间、主要来源；
   - 一句话：具体变化，或本次值得报道的内容；
   - 一句话：CSW 读者为什么要看，必要时指出可见的设计特点；
   - 查重：最相关的一条已发内容及本次差别；未检到时写「在已同步记录中未发现」并注明覆盖范围；
   - 推荐理由（引用了哪条编辑反馈就写明）、建议采用或备选、真正影响成稿的缺口。
   每条附一张视觉预览。完整证据留在附件，群里不展开长篇打分和流水账。
3. 缺量时单列「缺量说明」，四项写全：差多少（主选、备选分别已有几条合格、各缺几条）；为什么缺（按实际候选归类：有价值资讯偏少 / 采集漏项 / 近期已发布 / 审美或价值不符 / 证据或图片不足，不写笼统的「素材不足」）；怎样补（补查来源或方向、责任人、完成时间，能否在窗口与质量要求内补齐）；补不齐怎么办（已查范围、剩余缺口、明确建议，由 Van 决定是否接受减量）。未经 Van 确认不扩大时间窗口、不降低标准；已批条目照常生产。07:00 以后出现的资讯不进当期主选，确有必要时单列一条「追加建议」。
5. 整期交付按五栏追踪，不把在研究的算成熟：线索（candidate）、待核（pending_check）、成熟主选（shortlisted + rank=primary）、成熟备选（shortlisted + rank=alt）、Van 已批准（approved_write / approved_research）。方案里逐栏写清条数，淘汰另记理由。首批提交与后续补采完成分开记录，不用「首批已交」代表整期完成。
6. 补证有停止条件：每次追加研究前，先写清「查清这个问题是否可能改变采用决定」。能改变的，给准确问题、来源、预算与截止；只是多几个参数或图片的，不追加。同一核心问题一轮仍解决不了就停止该对象，不拖住其他候选；因价值低被否决的题不换文案重新包装。
7. 到检查点由你主动核结果：没有进展先判断失败原因（来源不足 / 价值尺度偏差 / 技术阻塞），再决定换来源、纠正筛法，或向 Van 提出交付取舍。不默认再顺延一轮，也不等 Van 追问才处理。需要改窗口、数量或交付安排时，一次说清决定、建议与影响。
8. 交给 Van 的是可直接判断的实物：预览图、可读的选题卡、主选与备选排序、具体查重结论与真实缺口；不是 zip 路径、任务版本号或审计条数。过程细节留在交付物与引擎里。
4. 效果记录（写在 index.md 末尾，每期都填）：首轮主选推荐数、备选数；采集启动到送审的耗时，并把耗时拆成实际作业、等主编、等 Van、技术阻塞四段。Van 批准后补记采用数、补选轮次、被打回条目及原因（已发布 / 审美不符 / 变化不足 / 证据不足 / 其他），以及 Van 实际审题用时（等待回复的时间单列）。

送审：主编自审 → Van 选题。Van 的决定由你代录，comment 写 Van 原话，并逐条记录：采用（可写）、继续研究（只研究不派写作）、暂缓、否决。「保留」「就这条」按采用处理；「再看看」「继续研究」按继续研究处理；措辞不明时只问一次受影响的范围。
录入时 decision_type 写 topic_approve，items 写批准可写的条目键：引擎为每个批准的条目生成写作与配图任务并自动派工（派工单写明条目与 Van 原话）；只开研究的条目录为 approve_research，不会派写作。之后补批或撤回单条，用条目决定（POST /runs/{run_id}/items/{条目键}/decision），同样附 Van 原话。

交付物：文件夹 = index.md（选题卡清单 + 整期组合 + 缺量说明 + 效果记录）+ images/（每条一张预览）。',
  self_check_criteria='## 提交前自检（全过才交）
- 亲自看过每条主选的图片与原文，来源与原始披露时间已核。
- 每张选题卡齐全：来源与时间、具体变化、读者理由、查重结论、推荐理由与缺口。
- 查重结论有依据；查不到的写了覆盖范围，没有写成「从未发布」。
- 已按生产约定数量排序；备选单独列出；整期没有品牌或品类扎堆，如有重复已说明理由。
- 缺量时「缺量说明」四项写全，给了具体的补查办法、责任人与完成时间。
- 效果记录已填首轮推荐数与送审耗时（作业 / 等主编 / 等 Van / 技术阻塞分开）。
- 五栏条数已写清，待核没有算进成熟数量；补证停止条件已写明。
- 每条附一张视觉预览。',
  acceptance='## 验收标准（主编自审 / Van 选题）
- 方案成熟：Van 不看附件、不等补资料也能做决定。
- 具体变化与读者理由写得具体，落在可见、可核的内容上；推荐理由用上了相关编辑反馈。
- 查重有依据；没有推荐已知近期写过的同角度内容。
- 排序与整期组合合理，主选与备选分开。
- 缺量说明四项齐全：差多少、为什么缺、怎样补、补不齐怎么办；没有用旧闻或弱题凑数，没有只写「保留缺口」。
- 效果记录字段齐全，数字与方案一致，耗时已按四段拆分。
- 五栏（线索 / 待核 / 成熟主选 / 成熟备选 / 已批准）条数清楚；没有把在研究的算成熟。
- 规范合规：index.md 为选题卡清单，预览图在 images/。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'topic';
-- +goose StatementEnd

-- 归档旧 active、激活 v4（部分唯一索引 uq_wf_active 要求先归档）。
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
