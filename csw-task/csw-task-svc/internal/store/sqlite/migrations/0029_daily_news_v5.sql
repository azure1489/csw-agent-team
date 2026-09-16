-- +goose Up
-- daily_news v5：01 的采集与判断过程必须留痕，主编与研究员才能验证采集是否合理。
-- · 01-情报逐条：提交前批量上报采集轮（找过哪里、找到几条、失败与替代），条目写溯源四字段，淘汰落库；
-- · 02-价值初筛：淘汰由「留在 candidate」改为标 dropped 并给理由码，采集过程直接读接口不必解压交付物；
-- · 03-选题方案：缺量说明里的「采集漏项」要有覆盖率依据。
-- 写法同 0026：复制当前 active 版本的阶段、依赖、闸，再按 code 定点更新；最后归档旧 active、激活新版。

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
  instructions='## 任务内容（怎么做）
引擎自动派工，派工单含当期窗口与采集要求。按生产约定的窗口采集：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00；以原始披露时间判断是否在窗口内，转载时间不算。

**先交首批，等主编校准，再决定往哪里深挖。** 派工后先用约 20 分钟扫一遍任务详情随附的来源台账（intake_sources；标了必扫的每轮都要覆盖），把成形的线索（一般 4–8 条）作为第一版提交：本阶段有一道主编闸，主编按原文、原始时间、预览图判断方向，通过后你再继续补采。没等到校准结论就深挖单一对象，是这条流程要避免的浪费。

每条记录：品牌／产品或事件（一句话说清是什么）、原文链接（完整 https）、来源平台、原始发布时间（与转载时间分开写）、一张预览图（images/）、已知缺口。

**采集过程要留痕。** 提交前用 PUT /runs/{run_id}/sweeps 批量上报本次全部采集轮，每扫一个来源一条：sweep_key（你生成，同键重报即更新）、platform、source_key、tool、query（找了什么）、时间窗、found（看到几条）、in_window（其中在窗口内几条）、registered（登记成条目几条）、result（ok / failed / partial），失败写 error 并说明换了哪个来源。台账里必扫的来源即使一条都没找到也要报一条——没有这条记录，主编无法区分「没找到」与「没去找」。

每条情报登记为一个条目（PUT /runs/{run_id}/items；条目键 = 品牌英文或拼音小写 + 短横 + 原文链接 sha256 前 6 位，如 hxo-3fa91c，生成后不再改）。每条写 discovered_via（对应的 sweep_key）、fetched_at（抓取时间）、evidence_url（预览图或正文截图的原始地址）、dedup_note（与随附的近 30 天已发的对照结论）。关键事实未核实的登记为 pending_check（待核），不要直接标 shortlisted；看过就确定不做的标 dropped 并给 reason_code 与一句理由，不要留在 candidate 当作还没判断。

首批不必为每条下载三张图，也不复制历史全包；进入短名单的条目在 05-配图与素材核 再补图。近期发过的同产品同角度内容照样记录并标注，是否采用由研究员和主编判断，不按品牌名机械去重。

主编校准通过后：按校准意见继续补采，新条目继续登记、采集轮补报，并以补件追加到本任务，不重新提交整包。

工具：Instagram 用营事编集室 csw MCP；小红书用 opencli xiaohongshu，每步插入随机间隔（搜索后 8–25 秒、读笔记后 5–18 秒、下载后 5–18 秒、切换平台约 45–55 秒）；网页优先 opencli 适配器，没有则 WebFetch。一个来源失败就换已授权的其他来源，失败那一轮照样上报。

交付物：文件夹 = index.md（逐条情报记录 + 采集轮次表 + 淘汰清单）+ images/（每条一张预览）+ trace/（sweeps.jsonl、items.jsonl，字段与接口同名）。',
  self_check_criteria='## 提交前自检（全过才交）
- 首批已成形就交，没有为个别对象无限深挖；每条都有品牌／产品或事件、原文链接、来源平台、原始发布时间。
- 原始披露时间在当期窗口内；转载时间与披露时间分开写。
- 每条有一张能判断外观的预览图，已放进 images/，相对链接可用。
- 本次全部采集轮已上报；台账里必扫的来源每个都有一条，失败的写了 error 与替代来源。
- 每条条目都写了 discovered_via、fetched_at、evidence_url、dedup_note。
- 关键事实未核实的标 pending_check；确定不做的标 dropped 并给了 reason_code 与理由，没有把判断过的留在 candidate。
- 已对照任务详情随附的近 30 天已发与最近被否决条目，重复或刚被否决的同角度对象已标注。
- 缺口已逐条写明，没有用猜测补齐事实。
- 小红书与网页采集按规定插入了随机间隔。',
  acceptance='## 验收标准（主编首批校准）
- 先看 adminctl intake-check 的结论：必扫来源是否都有轮次、找到与登记的落差是否有淘汰记录兜底、窗口是否合规、淘汰是否都有理由码、条目来源分布是否过于集中。
- 首批在派工后尽快到达，条数够主编判断方向（一般 4–8 条），不是单一对象的深挖包。
- 条目信息四要素齐全：是什么、原文链接、来源平台、原始发布时间；每条附一张可判断外观的预览。
- 采集轮次表能回答「找过哪里」：必扫来源都有记录，失败的写明原因与替代来源；没有把某个平台整体漏掉而不说明。
- 窗口判断以原始披露时间为准，没有把转载或晚抓到的旧内容当作当期新闻。
- 与近 30 天已发、最近被否决条目的重复情况已标注，没有换说法重报同一对象。
- 主编在本闸给出方向：哪些对象继续投入、哪些停止、还缺哪类来源；退回时写明方向与位置，不是笼统的「再多找几条」。
- 规范合规：index.md 为逐条清单，预览图在 images/，采集轮与判断轨迹在 trace/。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'intake';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
与采集交错进行：情报逐条到达就开始判断，不等整包补齐。你判断的是「是否值得继续投入」，不是资料是否齐全。01 的主编首批校准结论是你的起点，方向已被主编停掉的对象不再投入。

01 的采集过程可以直接读：GET /runs/{run_id}/intake-trace 给出采集轮次（找过哪些来源、各找到几条、哪些失败）、每条条目的来源与查重结论、以及判断轨迹，不必解压交付物。发现某个平台整轮没扫或整轮失败，在结论里写明，这是缺量归因的依据。

**先过两道轻检查，都成立才深入。**
- A 价值：是什么、这次的变化或设计、文化、使用关系看点是什么、CSW 读者具体得到什么。依据现有官方原文与预览图判断即可，不要求先证明「结构革命」。设计、文化、使用细节都可以成立，但要说清读者得到什么：例如折边带来拆洗便利可以成立；仅有昼夜反差的蓄光不自动成立。
- B 资格：是否新披露或仅为转发；该型号的历史与已发对照（GET /api/v1/ledger/posts?since=90d&brand=品牌，含 Van 人工发布与拆条）；最近几期是否已被否决或暂缓（任务详情随附）；是否在当期窗口内。

两道都过再核会影响采用决定的关键事实，再谈制作准备。A 或 B 不成立就直接淘汰，不用补图、补参数把弱题做厚。

关键事实不明的条目标 pending_check（待核）并写明缺哪一条事实，不计入成熟主选或成熟备选。归属清楚时不要把秒级时间精度当额外门槛；核心时间冲突（窗口归属、首发归属）不得忽略。

结论四种：成熟采用（主选）、成熟备选、待核、淘汰，每条附一句理由。成熟采用与成熟备选在引擎里标 shortlisted 并写明 rank（primary 主选 / alt 备选）；待核标 pending_check；**淘汰标 dropped**，并给 reason_code 与一句理由：no_value 价值不足 / not_new 非新披露只是转发 / dup_published 近期已发同角度 / dup_recent_rejected 近期已被否决或暂缓 / out_of_window 不在窗口 / evidence_missing 证据或图片不足 / aesthetic_mismatch 审美或价值不符 / superseded 被同期更好的取代 / other（须写明）。淘汰不再留在 candidate——那是「还没判断」的意思，留着会让别人以为你没看过。评分只辅助排序，不能压过明确的淘汰理由；资料完整但缺少编辑吸引力的，先淘汰。

相关编辑反馈：任务详情随附的 feedback（按阶段与品牌匹配的 Van 原话、采用 / 打回案例）。与候选相关的写明引用了哪条、符合还是触碰；一次否决不推成永久禁选，推断的偏好不当成 Van 原话。

不强行找前代升级对照，没有前代资料就准确描述本次披露，不编造「升级」「首次」「更好用」。可以用清楚的工作标题标识选题（正式标题归文案）。情报数量不足时，仍交可用条目并写明缺口，不把整包不足等同于每条都不能研究。

交付物：文件夹 = index.md（成熟采用 / 成熟备选 / 待核 / 淘汰四栏，每条含 A 价值与 B 资格的结论、理由、查重结论、引用的反馈、缺口、对应情报链接）。',
  self_check_criteria='## 提交前自检（全过才交）
- 每条候选都写了 A 价值与 B 资格两道检查的结论，都成立才进入深核。
- 结论落在四栏之一：成熟采用（主选）、成熟备选、待核、淘汰，各附一句理由。
- 成熟条目已标 shortlisted 并写明 rank；待核条目已标 pending_check 并写明缺哪一条事实。
- 淘汰条目已标 dropped 并给了 reason_code 与理由，没有留在 candidate。
- 每条都写了查重结论：对照到的已发内容及差别，或「在已同步记录中未发现」加覆盖范围；最近几期被否决、暂缓的同对象已标注。
- 已看过 intake-trace：某个平台整轮没扫或失败的，已写进缺量归因。
- 与候选相关的编辑反馈已写明引用了哪条；没有把推断当成 Van 原话。
- 没有编造「升级」「首次」等比较；没有前代资料的按本次披露描述。
- 没有为淘汰候选补图、补参数；影响成稿的缺口已写明。',
  acceptance='## 验收标准（主编对照）
- 四栏结论清楚：成熟采用、成熟备选、待核、淘汰；待核不被算作成熟数量。
- 淘汰在引擎里是 dropped 且有理由码，能与「还没判断」的线索区分开。
- A 价值写清读者具体得到什么，落在可见、可核的内容上；没有用参数堆砌代替看点。
- B 资格写清新披露与否、已发对照、最近是否被否决、窗口归属；核心时间冲突已处理。
- 查重有依据；没有推荐已知近期写过的同角度内容，也没有把被否决对象换说法重报。
- 缺量归因用上了采集覆盖事实，不是笼统的「素材不足」。
- 淘汰理由具体，没有被总分覆盖；弱题没有靠补证做厚。
- 每条都能对回情报记录与原文链接。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'shortlist';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions=replace(instructions,
    '为什么缺（按实际候选归类：有价值资讯偏少 / 采集漏项 / 近期已发布 / 审美或价值不符 / 证据或图片不足，不写笼统的「素材不足」）',
    '为什么缺（按实际候选归类：有价值资讯偏少 / 采集漏项 / 近期已发布 / 审美或价值不符 / 证据或图片不足，不写笼统的「素材不足」。写「采集漏项」要有依据：读 GET /runs/{run_id}/intake-trace 或 adminctl intake-check 的覆盖结论，指明哪些来源没扫、哪些整轮失败）'),
  acceptance=replace(acceptance,
    '- 缺量说明四项齐全：差多少、为什么缺、怎样补、补不齐怎么办；没有用旧闻或弱题凑数，没有只写「保留缺口」。',
    '- 缺量说明四项齐全：差多少、为什么缺、怎样补、补不齐怎么办；没有用旧闻或弱题凑数，没有只写「保留缺口」。
- 归因到采集漏项时有覆盖事实支撑（哪些来源没扫、哪些失败），不是凭印象。')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code = 'topic';
-- +goose StatementEnd

-- 归档旧 active、激活 v5（部分唯一索引 uq_wf_active 要求先归档）。
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
