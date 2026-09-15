-- +goose Up
-- daily_news v3（按《0915 改造方案核对反馈》回改）：
-- · 09:00 交 Van 终审的是已套模板、完成排版的完整推文：组版前移到终审之前——07 改「内容整合稿」主编单闸，
--   Van 全文终审移到 08「公众号完整审核稿」（主编核版 → Van），通过后按授权自动派 09 保存并回读。
-- · 小红书只交文案 + 选图包：11 改为 xhs_pick「小红书选图包」（情报收集员，依赖 08，与 10 并行），不做封面卡与内容卡；
--   10 依赖 08、主编单闸；Van 小红书终审移到 12「小红书图文包」。10 / 11 为手动派工，首期不派。
-- · 写作改为自动派工（条目决定即派，派工单由引擎写明条目与 Van 原话）；Van 日常只在 03 / 08 / 12 三处。
-- · 时限按首期倒排时间表重设；接续告警阈值（0023）：未接单 5 分钟，组版 / 保存类无活动 5 分钟、其余 10 分钟。
-- 写法：复制当前 active 版本的阶段，再按 code 定点修改；依赖与闸整表重写。旧版本归档，已触发的 run 按各自快照继续。

-- +goose StatementBegin
INSERT INTO workflows
  (wf_key, name, version, hub_role_code, dispatch_mode, trigger_roles, common_instructions, common_acceptance, status)
SELECT wf_key, name, (SELECT MAX(version) + 1 FROM workflows WHERE wf_key='daily_news'), hub_role_code, dispatch_mode, trigger_roles,
       common_instructions,
       replace(common_acceptance,
               '07-完整审核稿 / 08-公众号组版打包 / 09-公众号草稿保存 / 10-小红书改编 / 11-小红书视觉 / 12-小红书组包',
               '07-内容整合稿 / 08-公众号完整审核稿 / 09-公众号草稿保存 / 10-小红书改编 / 11-小红书选图包 / 12-小红书图文包'),
       'draft'
FROM workflows WHERE wf_key='daily_news' AND status='active';
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_stages
  (workflow_id, seq, code, name, role_code, output_type, instructions, self_check_criteria, acceptance, is_merge,
   dispatch_mode, action_class, sla_minutes, per_item, ack_minutes, idle_minutes)
SELECT (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')), seq, code, name, role_code, output_type, instructions, self_check_criteria, acceptance, is_merge,
       dispatch_mode, action_class, sla_minutes, per_item, ack_minutes, idle_minutes
FROM workflow_stages WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET code='xhs_pick', name='11-小红书选图包', role_code='collector', output_type='选图包', dispatch_mode='manual'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='xhs_visual';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
与采集交错进行：情报逐条到达就开始判断，不等整包补齐。你判断的是「是否值得继续投入」，不是资料是否齐全。

开工先查两样东西，每条候选都要用上：
- 近期发布记录：GET /api/v1/ledger/posts?since=90d&brand=品牌（含 Van 人工发布，合集已拆到单条）。查到就写出对照的那一条及本次差别；查不到只能写「在已同步记录中未发现」，并注明记录覆盖的平台与时间范围，不能写成「从未发布」。
- 相关编辑反馈：任务详情里随附的 feedback（按阶段与品牌匹配的 Van 原话、采用 / 打回案例与图片）。与候选相关的，写明引用了哪条、这条候选符合还是触碰了它；一次否决不推成永久禁选，推断的偏好不当成 Van 原话。

对每条候选分层判断，不用一个总分把差异抹平：
- 事实与时效：原始披露时间在窗口内；品牌、产品或事件明确；主要事实有来源支持。
- CSW 吸引力：最值得看的变化是什么；外观、设计、品牌文化或使用关系有什么具体看点。必须实际看图、读原文后说清楚。
- 历史与组合：上面的查重结论；本期内品牌或品类是否扎堆。
- 制作可行性：信息能否支撑三段正文；图片是否对应；真正影响成稿的缺口是什么。

结论只有三种：采用、备选、淘汰，每条附一句理由；采用与备选的条目在引擎里标为 shortlisted（淘汰的保持 candidate）。评分只辅助排序，不能压过明确的淘汰理由；资料完整但缺少编辑吸引力的，先淘汰。不强行找前代升级对照，没有前代资料就准确描述本次披露，不编造「升级」「首次」「更好用」。

可以用清楚的工作标题标识选题（正式标题归文案）。情报数量不足时，仍交可用条目并写明缺口，不把整包不足等同于每条都不能研究。

交付物：文件夹 = index.md（短名单：采用 / 备选 / 淘汰三栏，每条含理由、看点、查重结论、引用的反馈、缺口、对应情报链接）。',
  self_check_criteria='## 提交前自检（全过才交）
- 每条候选都给出采用、备选或淘汰之一，并附一句理由。
- 采用与备选条目都写明具体看点（看过图、读过原文），不是只转述功能参数。
- 每条都写了查重结论：对照到的已发内容及差别，或「在已同步记录中未发现」加覆盖范围。
- 与候选相关的编辑反馈已写明引用了哪条；没有把推断当成 Van 原话。
- 没有编造「升级」「首次」等比较；没有前代资料的按本次披露描述。
- 影响成稿的缺口已写明。
- 每条都能对回情报记录与原文链接。',
  acceptance='## 验收标准（主编对照）
- 三栏结论清楚，理由具体；淘汰理由没有被总分覆盖。
- 看点落在可见、可核的内容上（设计、审美、文化或使用关系），不是泛泛的「值得关注」。
- 查重有依据：写明对照到的已发内容，或写明「在已同步记录中未发现」与覆盖范围；没有推荐近期写过的同角度内容。
- 相关编辑反馈被实际用上：引用处与候选对得上，没有无视明确的打回理由，也没有把一次否决推成永久禁选。
- 时效与事实判断正确，没有把窗口外的旧闻放进采用栏。
- 规范合规：index.md 三栏齐全，每条可溯回情报记录与原文链接。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='shortlist';
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
4. 效果记录（写在 index.md 末尾，每期都填）：首轮主选推荐数、备选数；采集启动到送审的耗时。Van 批准后补记采用数、补选轮次、被打回条目及原因（已发布 / 审美不符 / 变化不足 / 证据不足 / 其他），以及 Van 实际审题用时（等待回复的时间单列）。

送审：主编自审 → Van 选题。Van 的决定由你代录，comment 写 Van 原话，并逐条记录：采用（可写）、继续研究（只研究不派写作）、暂缓、否决。「保留」「就这条」按采用处理；「再看看」「继续研究」按继续研究处理；措辞不明时只问一次受影响的范围。
录入时 decision_type 写 topic_approve，items 写批准可写的条目键：引擎为每个批准的条目生成写作与配图任务并自动派工（派工单写明条目与 Van 原话）；只开研究的条目录为 approve_research，不会派写作。之后补批或撤回单条，用条目决定（POST /runs/{run_id}/items/{条目键}/decision），同样附 Van 原话。

交付物：文件夹 = index.md（选题卡清单 + 整期组合 + 缺量说明 + 效果记录）+ images/（每条一张预览）。',
  self_check_criteria='## 提交前自检（全过才交）
- 亲自看过每条主选的图片与原文，来源与原始披露时间已核。
- 每张选题卡齐全：来源与时间、具体变化、读者理由、查重结论、推荐理由与缺口。
- 查重结论有依据；查不到的写了覆盖范围，没有写成「从未发布」。
- 已按生产约定数量排序；备选单独列出；整期没有品牌或品类扎堆，如有重复已说明理由。
- 缺量时「缺量说明」四项写全，给了具体的补查办法、责任人与完成时间。
- 效果记录已填首轮推荐数与送审耗时。
- 每条附一张视觉预览。',
  acceptance='## 验收标准（主编自审 / Van 选题）
- 方案成熟：Van 不看附件、不等补资料也能做决定。
- 具体变化与读者理由写得具体，落在可见、可核的内容上；推荐理由用上了相关编辑反馈。
- 查重有依据；没有推荐已知近期写过的同角度内容。
- 排序与整期组合合理，主选与备选分开。
- 缺量说明四项齐全：差多少、为什么缺、怎样补、补不齐怎么办；没有用旧闻或弱题凑数，没有只写「保留缺口」。
- 效果记录字段齐全，数字与方案一致。
- 规范合规：index.md 为选题卡清单，预览图在 images/。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='topic';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  name='07-内容整合稿',
  output_type='整合稿',
  instructions='## 任务内容（怎么做）
合流阶段，由主编完成：全部已批条目的写作与配图都通过后引擎自动派工给你，上游已预填各条正文、配图与素材。本阶段是内容检查与整合，不是 09:00 的审核对象——Van 终审的是 08 组版后的完整推文。

1. 整合内容稿：推文总标题（从各条的标题候选中定一个，≤64 字符）+ 摘要（80–120 字符）+ 本期导读（逐条「01｜品牌｜一句话」）+ 全部已写成条目的正文（按选题方案顺序）+ 同版配图（每则 3 张，写明图位）+ 封面选图建议。新条目晚到时本任务会标「补件待返工」，由你决定纳入本期还是留到下期。
2. 内容检查：首次审核一次指出全部影响通过的问题，区分必须修改与可选润色；允许一轮明确修订。同类问题第二次仍未解决时，选择定点编辑、改用已批备选或报告具体证据缺口，不按同一种方式重复退回。
3. 定点编辑：对有证据支持的局部事实归属、时态、重复、标题与导读关联、句子表达，可以直接改。改动以编辑版本提交（写明基于哪个版本和修改摘要），首稿保留；新增事实必须有来源；改后通读全文连贯性。预算约 10 分钟，是调度目标，不是到时自动放行。
4. 时间：逐条审核在写作期间就做完，本阶段只做整期收口，目标 08:35 前通过，给 08 组版与最终版式检查留出时间。

本阶段只有主编一道闸：通过后引擎自动派 08 组版；你还要在 08 检查最终完整推文，再送 Van。

交付物：文件夹 = index.md（整合稿：总标题、摘要、导读、正文、图位与配图对应、封面选图建议）+ images/。',
  self_check_criteria='## 提交前自检（全过才交）
- 总标题、摘要、导读、全部正文、同版配图齐全，条目顺序与选题方案一致。
- 只包含 Van 已批准的条目；缺口已在稿首说明。
- 每则 3 张图写明图位，与段落对应；封面选图建议已给出。
- 定点编辑都有版本与修改摘要，首稿保留；新增事实有来源。
- 反复出现的问题已改变处理方式或写明证据缺口，没有重复同样的退回。',
  acceptance='## 验收标准（主编整合审）
标题与导读：总标题 ≤64 字符、有编辑角度；摘要 80–120 字符；导读格式统一（01｜品牌｜一句话）。
正文：条目与顺序与批准的选题一致；每则标题「品牌｜一句话标题」、300–500 字、三段式，第三段有具体判断。
语言：编辑语气，无公关稿用语；正文里没有审校说明。
图片：每则 3 张，图位清楚、与段落对应；使用原图，未改主体。
事实：关键信息可溯回原文链接；时间表述与原始披露时间一致。
协作留痕：定点编辑版本与首稿都在，修改摘要可读。
规范合规：index.md 为整合稿，配图在 images/。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='fulltext';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  name='08-公众号完整审核稿',
  output_type='完整推文',
  instructions='## 任务内容（怎么做）
07 内容整合稿通过、06 版式准备完成后，引擎自动派工给你。你的产出就是 09:00 交 Van 终审的完整公众号审核稿：当期内容真正装进 CSW 模板、完成实际图文排版的完整推文，不是正文加图片附件。

1. 输入绑定当前版本：正文、标题、摘要、配图取自 07 已通过的版本；顶部横幅、尾部品牌介绍、封面与正文模板取自 06 登记的版本。收到补件（换图、换资产）时按补件替换对应位置，不改动已批正文。写作期间可以按已过主编审的条目先行预排，整篇到齐后统一收口。
2. 组版：顶部横幅 + 导读 + 各则正文（图位与整合稿一致、图注齐全）+ 尾部品牌介绍；字体字号、层级、间距、分隔按模板；按封面模板把标题放进封面（最多两行，在安全区内），封面选图按整合稿建议。
3. 预览：生成 OSS 上的单页 HTML 预览（图文内联，浏览器直接打开，展示的是拟写入公众号的实际排版效果），同版 zip 留底；不发 localhost 链接，不用 Markdown 拼成的普通网页代替。
4. 平台字段清单：标题（≤64 字符）、摘要、作者（按账号填写）、封面图、原文链接（如有）。
5. 本阶段不进任何平台后台。

送审：主编核版（检查最终全文、版式与图文一致）→ Van 全文终审（主编代录，comment 写 Van 原话，decision_type 写 fulltext_approve）。Van 通过后，引擎按本期授权自动派 09 保存草稿。

交付物：文件夹 = index.md（OSS 预览链接 + 字段清单 + 与批准版本的对应关系）+ html/article.html + html/preview.html + images/（封面与正文图）。',
  self_check_criteria='## 提交前自检（全过才交）
- 正文、标题、摘要、配图都取自 07 已通过的版本，版本号已写明；模板与头尾图用的是 06 登记的版本。
- 整篇已套模板：字体字号、层级、间距、分隔、图注、头尾图都按模板，图位与整合稿一致。
- 封面标题取自整合稿，最多两行，在安全区内；缩略图状态可识别。
- OSS 预览为完整 https 链接，浏览器直接打开就是实际排版效果，图片齐全；同版 zip 已留底。
- 字段清单齐全：标题、摘要、作者、封面图。
- 没有进入任何平台后台。',
  acceptance='## 验收标准（主编核版 / Van 全文终审）
完整推文：整篇标题、摘要、封面选图及效果、导语、全部资讯正文与正式配图、头尾图都在；打开预览就是可直接阅读和判断的推文，不是正文加附件。
标题与导读：总标题 ≤64 字符、有编辑角度；摘要 80–120 字符；导读格式统一（01｜品牌｜一句话）。
正文：条目与顺序与批准的选题一致；每则「品牌｜一句话标题」、300–500 字、三段式，第三段有具体判断；与 07 已通过版本逐字一致。
版式：套用 CSW 模板，字体字号、层级、间距、分隔、图注一致；每则 3 张图位置正确、与段落对应；使用原图，未改主体。
封面：符合模板与安全区，缩略图状态可识别。
事实：关键信息可溯回原文链接；时间表述与原始披露时间一致。
规范合规：index.md 写明预览链接与版本对应关系，html/ 与 images/ 齐全，字段清单完整。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='wx_layout';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
本阶段是平台写操作：08 完整审核稿通过 Van 终审、且本期 run 的授权包含公众号草稿时，引擎才会派工；没有授权就停在「待授权」。不要自行进入后台，也不要把「可派工」当成「已授权保存」。

1. 保存前确认：目标账号、允许的动作（只存草稿，不群发）、内容版本与 08 已通过的版本一致。
2. 先查询草稿箱里是否已有本期同标题草稿：有就更新或报告，不重复新建。接口响应不确定时先查记录，再决定是否重试。
3. 用 mp-helper skill（先读它的 SKILL.md，严格按用法）：把 HTML 里的图片上传公众号素材库并替换链接，再上传草稿箱；标题、摘要、作者、封面取自 08 的字段清单。
4. 回读核对：在后台预览里对照 08 的预览，检查是否丢图、错序、样式变化；记录草稿 ID、后台截图或回读摘要，附内容版本号。
5. 兼容修复只为还原已批准的内容与版式；需要实质改动已批内容或版式的，停下来报主编，由主编交 Van 确认受影响部分。

异常（授权范围不符、字段不一致、出现重复草稿、回读差异无法还原）只报主编，不直接找 Van。

交付物：文件夹 = index.md（草稿 ID、回读核对结论、版本号、操作记录）+ html/article.html（素材库地址替换后的正文）+ images/（回读截图）。',
  self_check_criteria='## 提交前自检（全过才交）
- 本期 run 的授权包含公众号草稿；没有越权群发。
- 保存前查过草稿箱，没有产生重复草稿。
- HTML 内图片已上传素材库并替换，无外链残留。
- 标题、摘要、作者、封面与 08 字段清单一致。
- 已回读核对：没有丢图、错序或样式变化；修复没有改动已批内容。
- 草稿 ID、截图或摘要、内容版本号都已写明。',
  acceptance='## 验收标准（主编对照；本阶段不设审核闸，异常才升级）
- 草稿箱里有且只有一份本期草稿，内容版本与 08 已通过的版本一致。
- 回读与 08 预览一致：图片齐全、顺序正确、样式没有变化。
- 字段齐全正确：标题（≤64 字符）、摘要、作者、封面图。
- 图片均为素材库地址。
- 回读记录可核：草稿 ID、截图或摘要、版本号。
- 规范合规：index.md 写明操作记录与回读结论，html/article.html 为替换后的正文。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='wx_save';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
输入：派工单，上游是已通过 Van 终审的 08 公众号完整审核稿。只有当期纳入小红书时才派工；与 11 选图包并行，二者在 12 合成图文包后由主编先审、Van 终审一次。

小红书不是公众号压缩版：依据传播力、视觉吸引力、认知门槛、讨论价值和信息重要度重新排序和表达；保留准确的产品结构、背景与使用信息，事实与审核稿一致、可溯源。不编造体验，不加评论引导或额外模块。

产出（全部写在 index.md，无附件）：
1. 笔记标题：18 字以内，单一传播点、点击导向；不堆品牌名、不做合集式标题、不直接复制公众号标题。
2. 固定导语（正文开头）：
   📢 户外内容平台 CAMPsomeWHERE
   👇为你播报过去24小时发布的户外潮流资讯
   随后用 🏕️ 🕶️ 👟 ⛺ 等符号列出本期亮点。
3. 长文正文：保留全部资讯，不减少数量，不是摘要或分页提纲；普通条目 200–300 字，重点条目 300–400 字；排序按传播力，可以不同于公众号顺序。每条开头写清品牌与产品，方便与 11 选图包逐条对应。

语言：信息优先，少抒情、少空话、少总结；少断行，不留公众号式大面积空白。

送审：主编审。Van 终审在 12 图文包，文案与选图一起看。

交付物：文件夹 = index.md（笔记标题 + 固定导语 + 长文正文 + 条目顺序）。',
  self_check_criteria='## 提交前自检（全过才交）
- 笔记标题不超过 18 字，单一传播点，不是公众号标题的直接复制。
- 固定导语正确，并用符号列出本期亮点。
- 长文保留了全部资讯；普通条目 200–300 字，重点条目不超过 400 字；不是摘要或提纲。
- 事实与 08 审核稿一致，没有编造体验或评论引导。
- 条目顺序已写明，每条开头有品牌与产品。',
  acceptance='## 验收标准（主编审）
标题：18 字以内；单一传播点；点击导向；不是公众号标题的直接复制。
导语：使用固定导语，亮点列举与本期内容对应。
正文：长文、保留全部资讯；字数符合普通 200–300、重点 300–400；排序按传播力。
事实：与 08 审核稿一致、可溯源；无杜撰、无夸大。
语言与排版：信息优先，保持小红书阅读节奏；少断行，没有公众号式留白。
规范合规：index.md 各部分齐全，无附件。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='xhs_text';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions='## 任务内容（怎么做）
输入：派工单，上游是已通过 Van 终审的 08 公众号完整审核稿（含各则配图与来源记录）。只有当期纳入小红书时才派工；与 10 小红书改编并行。

本阶段只交对应资讯的实际选图包，不做封面卡、内容卡或任何生成图。

1. 按资讯分组：每条资讯一个子目录，放 3–6 张可用原图（优先 05 已核的配图，缺的从原始来源补），不重绘、不改主体、不加字。
2. 建议顺序：每组标出首图，全包标出建议的整体顺序；首组优先本期传播力最高的资讯。
3. 图文对应：每张图写明对应哪条资讯、画面是什么，方便与 10 的长文逐条对应。
4. 来源记录：每张图写原始来源链接与使用依据；来源不明或使用依据不清的不放进包里，写明缺口。

交付物：文件夹 = index.md（分组清单：资讯 → 图片文件 → 建议顺序 → 来源链接）+ xiaohongshu/（按资讯分组的图片）。',
  self_check_criteria='## 提交前自检（全过才交）
- 每条资讯都有一组图，且只用原图；没有做封面卡、内容卡或生成图。
- 每组标了首图，全包写了建议顺序。
- 每张图都写明对应的资讯与画面。
- 每张图都有来源链接与使用依据；缺口已写明。
- 文件可直接打开查看、可直接使用。',
  acceptance='## 验收标准（主编对照）
- 分组与 08 审核稿的资讯一一对应，数量合适，没有漏组。
- 图片为原图，主体未改，没有额外加字或生成内容。
- 建议顺序合理，首组是本期传播力最高的资讯。
- 来源记录完整、可追溯。
- 规范合规：图片在 xiaohongshu/，index.md 含分组清单与来源。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='xhs_pick';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  name='12-小红书图文包',
  output_type='图文包',
  instructions='## 任务内容（怎么做）
10 小红书改编通过主编审、11 选图包提交后，引擎自动派工给你。

1. 用 10 已通过版本的笔记标题、导语与长文，加 11 的选图包，组成可查看、可直接使用的图文包：文案一份，图片按资讯分组、按建议顺序排好，每组与文案里对应的资讯一一对上。
2. 核对：图文对应、图片数量与顺序、文字与 10 已通过版本逐字一致、没有日期平台作者水印。
3. 本阶段不进平台后台，只交本地成品；不做封面卡与内容卡。

送审：主编审同版图文 → Van 小红书终审（主编代录，comment 写 Van 原话）。与获批后的公众号草稿箱交付同时送达；任一侧没完成时分别报告完成项与缺口。

交付物：文件夹 = index.md（笔记标题 + 导语 + 长文 + 图片顺序清单 + 与批准版本的对应关系）+ xiaohongshu/（按资讯分组、排好顺序的图片）。',
  self_check_criteria='## 提交前自检（全过才交）
- 文案取自 10 已通过的版本，图片取自 11 的选图包，版本号已写明。
- 图片按资讯分组、按建议顺序排好，每组与文案里的资讯对上。
- 文案逐字一致，没有改写。
- 没有进入平台后台。',
  acceptance='## 验收标准（主编审图文 / Van 小红书终审）
- 文案：标题 18 字以内、固定导语、长文保留全部资讯，与 10 已通过版本一致。
- 图片：原图、按资讯分组、顺序合理，首组是本期传播力最高的资讯；图文一一对应。
- 图文包可直接查看与使用：文案、图片、顺序清单齐全。
- 规范合规：index.md 清单完整，图片在 xiaohongshu/。'
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='xhs_package';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '最终配图由 05-配图与素材核提供，07 完整审核稿统一嵌入。', '最终配图由 05-配图与素材核提供，08 组版时按图位嵌入。')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='write';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '封面上的最终文字等 07 完整审核稿批准后按模板锁定，本阶段可先留位。', '封面上的最终文字等 07 内容整合稿通过后，在 08 组版时按模板锁定，本阶段可先留位。')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='design_prep';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = replace(instructions, '所用卡片清单', '所用图片清单')
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news')) AND code='xhs_save';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  dispatch_mode = CASE code WHEN 'write' THEN NULL ELSE dispatch_mode END,
  sla_minutes = CASE code
    WHEN 'intake' THEN 40 WHEN 'shortlist' THEN 40 WHEN 'topic' THEN 15 WHEN 'write' THEN 50
    WHEN 'material' THEN 45 WHEN 'design_prep' THEN 30 WHEN 'fulltext' THEN 15 WHEN 'wx_layout' THEN 15
    WHEN 'wx_save' THEN 20 WHEN 'xhs_text' THEN 30 WHEN 'xhs_pick' THEN 20 WHEN 'xhs_package' THEN 10
    ELSE NULL END,
  ack_minutes = 5,
  idle_minutes = CASE WHEN code IN ('wx_layout','wx_save','xhs_package','xhs_save') THEN 5 ELSE 10 END
WHERE workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'));
-- +goose StatementEnd

-- DAG 依赖边（15 条）：intake→shortlist→topic→{write, material, design_prep}；fulltext←write+material；
-- wx_layout←fulltext+design_prep→wx_save；wx_layout→{xhs_text, xhs_pick}→xhs_package→xhs_save。

-- +goose StatementBegin
INSERT INTO workflow_stage_deps (workflow_id, stage_id, depends_on_id)
SELECT s.workflow_id, s.id, d.id
FROM workflow_stages s
JOIN workflow_stages d ON d.workflow_id = s.workflow_id
WHERE s.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'))
  AND (
       (s.code='shortlist'   AND d.code='intake')
    OR (s.code='topic'       AND d.code='shortlist')
    OR (s.code='write'       AND d.code='topic')
    OR (s.code='material'    AND d.code='topic')
    OR (s.code='design_prep' AND d.code='topic')
    OR (s.code='fulltext'    AND d.code='write')
    OR (s.code='fulltext'    AND d.code='material')
    OR (s.code='wx_layout'   AND d.code='fulltext')
    OR (s.code='wx_layout'   AND d.code='design_prep')
    OR (s.code='wx_save'     AND d.code='wx_layout')
    OR (s.code='xhs_text'    AND d.code='wx_layout')
    OR (s.code='xhs_pick'    AND d.code='wx_layout')
    OR (s.code='xhs_package' AND d.code='xhs_text')
    OR (s.code='xhs_package' AND d.code='xhs_pick')
    OR (s.code='xhs_save'    AND d.code='xhs_package')
  );
-- +goose StatementEnd

-- 逐阶段闸（9 行，无默认闸）。Van 只在 03 选题、08 全文终审、12 小红书终审，都由中枢代录。
-- 未列出的阶段（intake / shortlist / material / design_prep / wx_save / xhs_pick / xhs_save）为 0 闸，提交即通过。

-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT s.workflow_id, s.id, g.gate_order, g.reviewer_role, g.relayed, g.name
FROM workflow_stages s
JOIN (
            SELECT 'topic' code, 1 gate_order, 'editor' reviewer_role, 0 relayed, '主编自审' name
  UNION ALL SELECT 'topic',       2, 'van',    1, 'Van选题'
  UNION ALL SELECT 'write',       1, 'editor', 0, '主编审'
  UNION ALL SELECT 'fulltext',    1, 'editor', 0, '主编整合审'
  UNION ALL SELECT 'wx_layout',   1, 'editor', 0, '主编核版'
  UNION ALL SELECT 'wx_layout',   2, 'van',    1, 'Van全文终审'
  UNION ALL SELECT 'xhs_text',    1, 'editor', 0, '主编审'
  UNION ALL SELECT 'xhs_package', 1, 'editor', 0, '主编审图文'
  UNION ALL SELECT 'xhs_package', 2, 'van',    1, 'Van小红书终审'
) g ON g.code = s.code
WHERE s.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'));
-- +goose StatementEnd

-- 归档旧 active、激活 v3（部分唯一索引 uq_wf_active 要求先归档）。

-- +goose StatementBegin
UPDATE workflows SET status='archived' WHERE wf_key='daily_news' AND status='active';
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflows SET status='active' WHERE id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='draft' AND version=(SELECT MAX(version) FROM workflows WHERE wf_key='daily_news'));
-- +goose StatementEnd


-- +goose Down
-- 删除本迁移建的 v3（识别特征：含 xhs_pick 阶段；子表 CASCADE），恢复 v2 为 active。
-- 已有 run 引用 v3 时外键会阻止删除——那时应改为前滚修复，而不是回滚。
DELETE FROM workflows WHERE wf_key='daily_news'
  AND id IN (SELECT workflow_id FROM workflow_stages WHERE code='xhs_pick');
UPDATE workflows SET status='active' WHERE wf_key='daily_news'
  AND id = (SELECT MAX(workflow_id) FROM workflow_stages WHERE code='xhs_visual'
            AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news'))
  AND NOT EXISTS (SELECT 1 FROM workflows WHERE wf_key='daily_news' AND status='active');
