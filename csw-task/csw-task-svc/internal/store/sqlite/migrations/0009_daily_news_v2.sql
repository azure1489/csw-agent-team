-- +goose Up
-- daily_news v2（新流程改造）：十三阶段；Van 只在 03-选题方案 / 07-完整审核稿 / 10-小红书改编 三处出现。
-- 新增阶段属性：dispatch_mode（阶段级派工模式覆盖，NULL=继承工作流）、action_class（read | platform_write:<scope>，
-- 平台写操作受 run 授权护栏约束）、sla_minutes（派工后时限）、per_item（按条目生成任务，条目级推进启用后生效）。
-- 任务快照 dispatch_mode / action_class / sla_minutes（触发时解析，运行实例不受此后定义修改影响）。
-- v2 不设默认闸：0 闸阶段只能靠「无默认闸 + 无覆盖闸」表达，所以每个有闸阶段都显式声明。
-- 旧 active 版本归档；已触发的 run 按各自快照继续。

ALTER TABLE workflow_stages ADD COLUMN dispatch_mode TEXT CHECK (dispatch_mode IS NULL OR dispatch_mode IN ('manual','auto'));
ALTER TABLE workflow_stages ADD COLUMN action_class TEXT NOT NULL DEFAULT 'read';
ALTER TABLE workflow_stages ADD COLUMN sla_minutes INTEGER;
ALTER TABLE workflow_stages ADD COLUMN per_item INTEGER NOT NULL DEFAULT 0 CHECK (per_item IN (0,1));
ALTER TABLE tasks ADD COLUMN dispatch_mode TEXT;
ALTER TABLE tasks ADD COLUMN action_class TEXT NOT NULL DEFAULT 'read';
ALTER TABLE tasks ADD COLUMN sla_minutes INTEGER;

-- 三个新角色（群内原 reserved bot 转正）。后台可能已手工建过，均按「不存在才插」写。
INSERT OR IGNORE INTO roles (code, name, is_management, is_human) VALUES
  ('xhswriter', '小红书图文作者', 0, 0),
  ('reviewer',  '合规版权审查员', 0, 0),
  ('analyst',   '数据复盘师',     0, 0);
INSERT INTO agents (role_code, name) SELECT 'xhswriter', '小红书图文作者' WHERE NOT EXISTS (SELECT 1 FROM agents WHERE role_code='xhswriter');
INSERT INTO agents (role_code, name) SELECT 'reviewer',  '合规版权审查员' WHERE NOT EXISTS (SELECT 1 FROM agents WHERE role_code='reviewer');
INSERT INTO agents (role_code, name) SELECT 'analyst',   '数据复盘师'     WHERE NOT EXISTS (SELECT 1 FROM agents WHERE role_code='analyst');

-- 花名册：三条 reserved 按 open_id 转为 mapped。
UPDATE chat_members SET kind='mapped', role_code='xhswriter', updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
  WHERE kind='reserved' AND open_id='ou_003deac0bece81514fca9cd9ba081139';
UPDATE chat_members SET kind='mapped', role_code='reviewer', updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
  WHERE kind='reserved' AND open_id='ou_5e4a9f33f31701a5dd674db4c9daa2dc';
UPDATE chat_members SET kind='mapped', role_code='analyst', updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
  WHERE kind='reserved' AND open_id='ou_7b9ba1d4d09cfbee3ba3f89005db434f';

-- 归档旧 active（部分唯一索引 uq_wf_active 要求先归档再插入新 active）。
UPDATE workflows SET status='archived' WHERE wf_key='daily_news' AND status='active';

-- +goose StatementBegin
INSERT INTO workflows
  (wf_key, name, version, hub_role_code, dispatch_mode, trigger_roles, common_instructions, common_acceptance, status)
VALUES (
  'daily_news', '资讯日更',
  (SELECT COALESCE(MAX(version),0)+1 FROM workflows WHERE wf_key='daily_news'),
  'editor', 'auto', 'editor,scheduler',
  '## 交付通用约定
- 交付物形态：一个文件夹 = index.md（固定入口，正文内联）+ 附件子目录（images/ source/ html/ xiaohongshu/），相对链接引用。
- 流转：文件夹打包成同名 zip → 一步式提交本服务（服务端自动命名、按规范路径归档、每版留底）→ 角色之间只传完整 https 链接（不能只写路径）。
- 命名与路径由服务端派生，无需手拼：资讯日更_责任方_日期_r{run_id}_v{版本}.zip，路径 csw/资讯日更/{日期}/r{run_id}/{阶段序号-阶段}/。
- index.md 顶部必带 YAML 元信息头：任务 / 类型 / 交付 Agent / 阶段 / 版本 / 时间 / 上游来源 / 状态 / 自检；按条目交付时加「条目」（品牌｜产品或事件）。上游来源逐条列：上游交付物写「来源方 · 完整 https 链接」；派工单是引擎记录，写「主编 · r{run_id} 任务#{task_id} 派工单」。
- 开工先读：《资讯日更 · 生产约定》有效版本（窗口、数量、时间表、授权）、本任务派工单、最新审核意见、上游当前版本。岗位记忆不能替代这些记录。
- 接到派工先在引擎确认接单；确实做不了就报告失败并写明原因，不沉默等待。遇到一处缺口，继续其他不依赖它的工作。
- 已批准内容按版本保持；改动涉及事实、选题或编辑意思时标明受影响范围，不覆盖已批准版本。换图、换资产、补事实走补件，不重跑已批正文。
- 状态以引擎为准：ready=具备派工条件；dispatched=已派工待接单；in_progress=已接单；review=等待某道闸；returned=需返工；passed=已交付；failed=报告无法完成；cancelled=已取消。群消息只是提示。
- 本地演练、后台草稿、公开发布是三种不同授权。任务解锁不等于可以进后台；平台写操作以引擎里本期 run 的授权为准。',
  '## 通用合规项（每份交付物都查）
- 文件名与路径由服务端派生（资讯日更_责任方_日期_r{run_id}_v{版本}.zip；csw/资讯日更/{日期}/r{run_id}/{阶段序号-阶段}/），阶段序号正确：01-情报逐条 / 02-价值初筛 / 03-选题方案 / 04-公众号写作 / 05-配图与素材核 / 06-版式与模板准备 / 07-完整审核稿 / 08-公众号组版打包 / 09-公众号草稿保存 / 10-小红书改编 / 11-小红书视觉 / 12-小红书组包 / 13-小红书草稿与发布。
- 交付的是完整 https 链接，zip 可下载、解压后以 index.md 为入口可读。
- index.md 元信息头字段齐全；按条目交付的写明条目；上游来源逐条列，派工单写引擎引用，不写「无」。
- 附件相对链接全部可打开。
- 返工件版本号已递增（重交自动 +1），原版留底未删；审核针对的是任务当前版本。
- 事实可溯回原文链接；原始披露时间与转载时间分开标注，晚抓到的旧内容不写成当天新闻。',
  'active'
);
-- +goose StatementEnd

-- +goose StatementBegin
INSERT INTO workflow_stages
  (workflow_id, seq, code, name, role_code, output_type, is_merge, dispatch_mode, action_class, sla_minutes, per_item,
   instructions, self_check_criteria, acceptance)
VALUES
-- ───── 01-情报逐条 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 1, 'intake', '01-情报逐条', 'collector', '情报记录', 0, NULL, 'read', 40, 0,
 '## 任务内容（怎么做）
引擎自动派工，派工单含当期窗口与采集要求。按生产约定的窗口采集：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00；以原始披露时间判断是否在窗口内，转载时间不算。

逐条推进，不等整包：一条情报只要有可追溯原文、原始发布时间、一张能判断外观的预览，就可以交给研究员初筛。每条记录：
- 品牌／产品或事件，一句话说清是什么；
- 原文链接（完整 https）、来源平台、原始发布时间（与转载时间分开写）；
- 一张预览图（images/）；
- 已知缺口：缺什么事实、缺什么图。

初筛前不必为每条下载三张图，也不复制历史全包；进入短名单的条目在 05-配图与素材核 再补图。近期发过的同产品同角度内容照样记录并标注，是否采用由研究员和主编判断，不按品牌名机械去重。

每条情报登记为一个条目（PUT /runs/{run_id}/items；条目键 = 品牌英文或拼音小写 + 短横 + 原文链接 sha256 前 6 位，如 hxo-3fa91c，生成后不再改）。先交已成形的一批；窗口内陆续抓到的新条目继续登记，并以补件追加到本任务，不等全部抓完再交。

工具：Instagram 用营事编集室 csw MCP（csw_posts_search → csw_posts_get，无需拟人间隔）；小红书用 opencli xiaohongshu，每步插入随机间隔（搜索后 8–25 秒、读笔记后 5–18 秒、下载后 5–18 秒、切换平台约 45–55 秒）；网页优先 opencli 适配器，没有则 WebFetch。一个来源失败就换已授权的其他来源或同产品其他页面，不把单页失败当成整期停止条件。

交付物：文件夹 = index.md（逐条情报记录）+ images/（每条一张预览）。',
 '## 提交前自检（全过才交）
- 每条都有品牌／产品或事件、原文链接、来源平台、原始发布时间。
- 原始披露时间在当期窗口内；转载时间与披露时间分开写。
- 每条有一张能判断外观的预览图，已放进 images/，相对链接可用。
- 缺口已逐条写明，没有用猜测补齐事实。
- 近期发过的同产品内容已标注，没有擅自删掉。
- 小红书与网页采集按规定插入了随机间隔。',
 '## 验收标准（主编对照）
- 条目信息四要素齐全：是什么、原文链接、来源平台、原始发布时间。
- 窗口判断以原始披露时间为准，没有把转载或晚抓到的旧内容当作当期新闻。
- 每条附一张可判断外观的预览；缺口如实标注。
- 原文链接可达；与本号题材（户外、露营、潮流装备与文化）相关。
- 规范合规：index.md 为逐条清单，预览图在 images/，相对链接可用。'),

-- ───── 02-价值初筛 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 2, 'shortlist', '02-价值初筛', 'researcher', '短名单', 0, NULL, 'read', 40, 0,
 '## 任务内容（怎么做）
与采集交错进行：情报逐条到达就开始判断，不等整包补齐。你判断的是「是否值得继续投入」，不是资料是否齐全。

对每条候选分层判断，不用一个总分把差异抹平：
- 事实与时效：原始披露时间在窗口内；品牌、产品或事件明确；主要事实有来源支持。
- CSW 吸引力：最值得看的变化是什么；外观、设计、品牌文化或使用关系有什么具体看点。必须实际看图、读原文后说清楚。
- 历史与组合：查近期已发内容，说明是否写过同产品或同角度、本次差别在哪；发布记录未同步时写明已查范围。
- 制作可行性：信息能否支撑三段正文；图片是否对应；真正影响成稿的缺口是什么。

结论只有三种：采用、备选、淘汰，每条附一句理由；采用与备选的条目在引擎里标为 shortlisted（淘汰的保持 candidate）。评分只辅助排序，不能压过明确的淘汰理由；资料完整但缺少编辑吸引力的，先淘汰。不强行找前代升级对照，没有前代资料就准确描述本次披露，不编造「升级」「首次」「更好用」。

可以用清楚的工作标题标识选题（正式标题归文案）。情报数量不足时，仍交可用条目并写明缺口，不把整包不足等同于每条都不能研究。

交付物：文件夹 = index.md（短名单：采用 / 备选 / 淘汰三栏，每条含理由、看点、近发对照、缺口、对应情报链接）。',
 '## 提交前自检（全过才交）
- 每条候选都给出采用、备选或淘汰之一，并附一句理由。
- 采用与备选条目都写明具体看点（看过图、读过原文），不是只转述功能参数。
- 已查近期已发内容并写明对照结果或已查范围。
- 没有编造「升级」「首次」等比较；没有前代资料的按本次披露描述。
- 影响成稿的缺口已写明。
- 每条都能对回情报记录与原文链接。',
 '## 验收标准（主编对照）
- 三栏结论清楚，理由具体；淘汰理由没有被总分覆盖。
- 看点落在可见、可核的内容上（设计、审美、文化或使用关系），不是泛泛的「值得关注」。
- 近发对照有依据：写明对照到的已发内容，或写明已查范围。
- 时效与事实判断正确，没有把窗口外的旧闻放进采用栏。
- 规范合规：index.md 三栏齐全，每条可溯回情报记录与原文链接。'),

-- ───── 03-选题方案（合流，主编亲自完成）─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 3, 'topic', '03-选题方案', 'editor', '选题方案', 1, NULL, 'read', 15, 0,
 '## 任务内容（怎么做）
合流阶段，由主编亲自完成：短名单通过后引擎自动派工给你，上游已预填。

1. 亲自看图、读原文、查近期组合，不只转述研究员的结论。审美是否匹配、近期是否重复、整期品牌与品类是否扎堆，由你决定。
2. 按生产约定的数量（主选与备选）排序，形成选题方案。每条一张选题卡：
   - 品牌／产品或事件、原始发布时间、主要来源；
   - 一句话：具体变化，或本次值得报道的内容；
   - 一句话：CSW 读者为什么要看，必要时指出可见的设计特点；
   - 最近最相关的一条已发内容及本次差别；未检到时写明历史覆盖范围；
   - 建议采用或备选，以及真正影响成稿的缺口。
   每条附一张视觉预览。完整证据留在附件，群里不展开长篇打分和流水账。
3. 数量不足时一次说清缺口、已查范围、推荐可用项；已批准的条目照常推进，不用旧闻或弱题凑数。

送审：本阶段两道闸——主编自审、Van 选题。Van 的决定由你代录，comment 写 Van 原话，并逐条记录：采用（可写）、继续研究（只研究不派写作）、暂缓、否决。「保留」「就这条」按采用处理；「再看看」「继续研究」按继续研究处理；措辞不明时只问一次受影响的范围。
录入时 decision_type 写 topic_approve，items 写批准可写的条目键：引擎自动为每个批准的条目生成写作与配图任务；只开研究的条目录为 approve_research，不会派写作。之后补批或撤回单条，用条目决定（POST /runs/{run_id}/items/{条目键}/decision），同样附 Van 原话。

交付物：文件夹 = index.md（选题卡清单 + 整期组合 + 缺口）+ images/（每条一张预览）。',
 '## 提交前自检（全过才交）
- 亲自看过每条主选的图片与原文。
- 每张选题卡五项齐全：来源与时间、具体变化、读者理由、近发对照、建议与缺口。
- 已按生产约定数量排序；备选单独列出。
- 整期组合没有品牌或品类扎堆；如有重复已说明理由。
- 数量不足时缺口、已查范围、推荐可用项已写清。
- 每条附一张视觉预览。',
 '## 验收标准（主编自审 / Van 选题）
- 选题卡五要素齐全，Van 不看附件也能做决定。
- 具体变化与读者理由写得具体，落在可见、可核的内容上。
- 近发对照有依据；没有重复推荐已知近期写过的同角度内容。
- 排序与整期组合合理，主选与备选分开。
- 缺口如实说明，没有用旧闻或弱题凑数。
- 规范合规：index.md 为选题卡清单，预览图在 images/。'),

-- ───── 04-公众号写作 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 4, 'write', '04-公众号写作', 'writer', '文章', 0, 'manual', 'read', 45, 1,
 '## 任务内容（怎么做）
本任务对应一个 Van 已批准可写的条目（条目级：每条一个写作任务，任务详情的 item_key 即条目键）；主编派工单写明条目与批准原话，选题卡与短名单作写作依据。

写作角色：懂户外、懂潮流、懂文化、懂产品、懂行业变化的编辑，不是品牌市场部、百科词条或测评博主。完整保留资讯信息，同时完成筛选、整理与价值判断。

开写前检查素材能否支撑背景、核心事实和具体判断；缺什么就定点核源或向配图与素材环节提出，不把面料、尺寸、重量等整套参数设为每条都必须具备的前提。

每则资讯：
- 标题「品牌名称｜一句话标题」，体现变化、趋势、差异或观察；不堆型号、不直接并列联名双方、不做纯事实陈述。
- 正文三段、300–500 字：第一段品牌背景与资讯总起（2–3 句）；第二段核心信息（是什么、适合谁、使用场景、功能结构、产品特点，信息密度最高）；第三段编辑判断（特别在哪、和什么不同、为什么值得关注）。判断可以引用前文事实，但要增加适用条件、选择依据或具体影响。
- 图位：固定 3 个（标题下 / 第二段前 / 第三段前），写明每个图位需要什么画面；最终配图由 05-配图与素材核提供，07 完整审核稿统一嵌入。

写完即提交，主编逐条实审。另附本条导读行（「品牌｜一句话」）；本条适合作封面主推时，再附 1–2 个推文标题候选（≤64 字符）。总标题、摘要、本期导读由主编在完整审核稿里据这些候选收口。

语言：克制、有判断、有审美；禁止公关稿语言（震撼登场、重磅发布、引爆全网、颠覆行业、重新定义、诚意之作等）。审校说明与免责留在证据记录，不写进读者正文。字数口径以验收标准为准。

交付物：文件夹 = index.md（各条正文 + 图位说明；全部写完时附标题方案、摘要、导读）。',
 '## 提交前自检（全过才交）
- 写的是本任务对应的已批条目。
- 每则标题格式「品牌｜一句话标题」，不是型号堆砌。
- 每则 300–500 字、三段式，第三段有具体的编辑判断。
- 每则写明 3 个图位需要的画面。
- 事实与情报原文一致，关键信息可溯源；没有编造体验或比较。
- 无公关稿语言；正文里没有审校说明。
- 已附本条导读行；适合主推时附了推文标题候选。',
 '## 验收标准（主编对照）
- 写的是本任务对应的已批条目。
- 标题格式统一且有编辑角度。
- 每则 300–500 字、三段式；第二段信息密度足，第三段有判断（差异点、使用场景、产品逻辑或趋势），不只是复述品牌内容。
- 语言保持编辑语气，不像百科，不像广告；无公关稿用语。
- 图位说明与段落内容对应。
- 事实可溯回情报原文链接。
- 本条导读行格式正确（品牌｜一句话）；附了标题候选的，候选有传播力、不超过 64 字符。'),

-- ───── 05-配图与素材核 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 5, 'material', '05-配图与素材核', 'collector', '素材包', 0, NULL, 'read', 45, 1,
 '## 任务内容（怎么做）
选题批准后引擎自动派工。本任务对应一个 Van 已批准可写的条目（条目级：每条一个配图任务），只为这一条补齐最终配图和写作所需的关键事实。

每则资讯提供 3 张最终配图，对应正文三个图位：图1 最有点击力（标题下）、图2 核心信息（第二段前）、图3 细节或使用场景（第三段前）。优先资讯核心原图，其次使用场景图，再次细节图；避免重复角度与构图。

每张注明：归属条目、图位、原图来源链接、使用依据（官方发布 / 媒体转载 / 社区原帖）。使用依据不清的单独标出，交主编判断是否需要专项检查。

文案反馈缺某项事实时，定点补证并附来源；补不到就写明已查范围。一个来源失败就换已授权的其他来源，不停整期。

工具同 01-情报逐条：csw MCP、opencli（按规定插入随机间隔）、WebFetch。

交付物：文件夹 = index.md（按条目列配图清单：图位、来源链接、使用依据；补证事实与来源）+ images/（按「条目-图位」命名）。',
 '## 提交前自检（全过才交）
- 只处理了本任务对应的条目；3 张图对应三个图位。
- 图片清晰、与条目相符，角度不重复。
- 每张都写明来源链接与使用依据；依据不清的已单独标出。
- 补证事实附来源；补不到的写明已查范围。
- 文件按「条目-图位」命名，相对链接可用。',
 '## 验收标准（主编对照）
- 处理的是本任务对应的条目；3 张最终配图，图位分配合理（点击力 / 核心信息 / 细节或场景）。
- 使用原图，没有替换资讯主体。
- 来源链接与使用依据齐全，风险项已标出。
- 补证事实有来源。
- 规范合规：index.md 配图清单完整，images/ 命名与清单一致。'),

-- ───── 06-版式与模板准备 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 6, 'design_prep', '06-版式与模板准备', 'designer', '模板', 0, NULL, 'read', 45, 0,
 '## 任务内容（怎么做）
选题批准后引擎自动派工，与写作并行准备版式，不等全文。

1. 引用已登记的已批资产：顶部横幅、尾部品牌介绍、封面模板、正文模板，逐项写明版本号。资产未登记或需要替换时在 index.md 说明并交主编；首次模板、Van 主动要求的视觉修改、影响表达的重要改版须交 Van 确认。
2. 按已批条目准备公众号封面候选：优先本期传播力最高的资讯原图，按 2.35:1（1920 × 817 px，JPG，sRGB）预排版；主体与标题在中央 60% 安全区，重要元素距边缘不少于 80px。封面上的最终文字等 07 完整审核稿批准后按模板锁定，本阶段可先留位。
3. 日常原图排版、原 LOGO 使用、固定模板替换不需要重新生图；只有派工单明确要求生成的元素才用 generate-images skill，并保留真实产物。

图片使用原则：优先资讯原图；允许裁切、排版、加文字、调亮度与留白、加辅助图形；禁止重绘装备主体、改产品结构或颜色、AI 生成不存在的产品或细节、替换资讯主体。

批准后需要换图或换资产时，提交补件（写明资产、适用位置与预览），由组版负责人接续，不重跑已批正文。

交付物：文件夹 = index.md（资产与模板版本清单、封面候选说明、待确认事项）+ images/（封面候选、模板预览）。',
 '## 提交前自检（全过才交）
- 顶部横幅、尾部品牌介绍、封面模板、正文模板都写明了版本号，或写明待登记。
- 封面候选使用资讯原图，比例 2.35:1、1920 × 817，主体在安全区内。
- 没有重绘装备主体，没有生成不存在的产品。
- 只有派工单要求的元素才生成新图，且保留了真实产物。
- 需要 Van 确认的改版已单独列出。',
 '## 验收标准（主编对照）
- 资产与模板版本清单完整，引用的是已批版本。
- 封面候选对应本期传播力最高的资讯，缩略图状态可识别，安全区合规。
- 原图使用规范：未重绘主体、未改结构颜色、未生成不存在的产品；AI 只用于背景、版式、文字元素。
- 待确认事项写清，没有把首次模板或重要改版当成日常复用。
- 规范合规：index.md 清单齐全，预览图在 images/。'),

-- ───── 07-完整审核稿（合流，主编整期审核）─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 7, 'fulltext', '07-完整审核稿', 'editor', '审核稿', 1, NULL, 'read', 35, 0,
 '## 任务内容（怎么做）
合流阶段，由主编完成：写作与配图都通过后引擎自动派工给你，上游已预填各条正文、配图与素材。

1. 合成完整审核稿：推文总标题（从各条的标题候选中定一个，≤64 字符）+ 摘要（80–120 字符，由你收口）+ 本期导读（汇总各条导读行，逐条「01｜品牌｜一句话」）+ 全部已写成条目的正文（按选题方案顺序）+ 同版配图（每则 3 张按图位嵌入）。条目级任务都通过后本任务才就绪；新条目晚到时本任务会标「补件待返工」，由你决定纳入本期还是留到下期。
2. 整期审核：首次审核一次指出全部影响通过的问题，区分必须修改与可选润色；允许一轮明确修订。同类问题第二次仍未解决时，选择定点编辑、改用已批备选或报告具体缺口，不重复同样的退回。
3. 定点编辑：对有证据支持的局部事实归属、时态、重复、标题与导读关联、句子表达，可以直接改。改动以编辑版本提交（写明基于哪个版本和修改摘要），首稿保留；新增事实必须有来源；改后通读全文连贯性。预算约 10 分钟，是调度目标，不是到时自动放行。
4. 送达形态：OSS 上的单页 HTML 预览（图文内联，浏览器直接可读）为主，同版 zip 留底；不发 localhost 链接。

送审：主编定点编辑 → Van 全文（由你代录，comment 写 Van 原话）。Van 不批准的条目按其原话退回，或替换为已批备选。

交付物：文件夹 = index.md（完整审核稿）+ images/ + html/preview.html（可读预览）。',
 '## 提交前自检（全过才交）
- 总标题、摘要、导读、全部正文、同版配图齐全，条目顺序与选题方案一致。
- 只包含 Van 已批准的条目；缺口已在稿首说明。
- 每则 3 张图按图位嵌入，与段落对应。
- 定点编辑都有版本与修改摘要，首稿保留；新增事实有来源。
- 预览链接为 OSS 上的完整 https 链接，浏览器可直接打开。',
 '## 验收标准（主编定点编辑 / Van 全文）
标题与导读：总标题 ≤64 字符、有编辑角度；摘要 80–120 字符；导读格式统一（01｜品牌｜一句话）。
正文：条目与顺序与批准的选题一致；每则标题「品牌｜一句话标题」、300–500 字、三段式，第三段有具体判断。
语言：编辑语气，无公关稿用语；正文里没有审校说明。
图片：每则 3 张，位置正确、与段落对应；使用原图，未改主体。
事实：关键信息可溯回原文链接；时间表述与原始披露时间一致。
协作留痕：定点编辑版本与首稿都在，修改摘要可读。
规范合规：index.md 为完整审核稿，html/preview.html 可读，配图在 images/。'),

-- ───── 08-公众号组版打包 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 8, 'wx_layout', '08-公众号组版打包', 'publisher', '成品', 0, NULL, 'read', 15, 0,
 '## 任务内容（怎么做）
07 完整审核稿通过 Van 全文审核、06 版式准备完成后，引擎自动派工给你。你是交付执行负责人：按已批准模板完成组版，主编核版。

1. 所有输入绑定当前批准版本：正文、标题、摘要、配图取自 07 已通过的版本；资产与模板取自 06 登记的版本。已批准后又收到补件（换图、换资产）时，按补件替换对应位置，不改动已批正文。
2. 把正文转成适配公众号的 HTML：顶部横幅 + 导读 + 各则正文（图位与审核稿一致）+ 尾部品牌介绍；按封面模板把批准的标题放进封面（最多两行，在安全区内）。
3. 生成可读预览（单页 HTML，本地浏览器直接打开，图片相对链接可用），并整理平台字段清单：标题（≤64 字符）、摘要、作者（按账号填写）、封面图、原文链接（如有）。
4. 本阶段不进任何平台后台，只交本地成品。只有本地演练授权时，这份成品就是当期最终交付。

交付物：文件夹 = index.md（字段清单 + 与批准版本的对应关系 + 预览说明）+ html/article.html + images/（封面与正文图）。',
 '## 提交前自检（全过才交）
- 正文、标题、摘要、配图都取自 07 已通过的版本，版本号已写明。
- 顶部横幅、尾部品牌介绍、封面模板用的是 06 登记的版本。
- HTML 排版正常，图位与审核稿一致，相对链接可用。
- 封面标题取自批准稿，最多两行，在安全区内。
- 字段清单齐全：标题、摘要、作者、封面图。
- 没有进入任何平台后台。',
 '## 验收标准（主编核版）
- 内容与 07 批准版本逐字一致，没有擅自改动正文。
- 资产与模板版本正确；补件已替换到对应位置。
- 可读预览在浏览器中正常显示，图片齐全、位置正确。
- 封面符合模板与安全区要求，缩略图状态可识别。
- 字段清单完整，标题不超过 64 字符。
- 规范合规：index.md 写明版本对应关系，html/article.html 与 images/ 齐全。'),

-- ───── 09-公众号草稿保存（平台写操作）─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 9, 'wx_save', '09-公众号草稿保存', 'publisher', '发布物', 0, NULL, 'platform_write:wx_draft', 10, 0,
 '## 任务内容（怎么做）
本阶段是平台写操作：只有本期 run 的授权包含公众号草稿时，引擎才会派工；没有授权就停在「待授权」。不要自行进入后台，也不要把「可派工」当成「已授权保存」。

1. 保存前确认：目标账号、允许的动作（只存草稿，不群发）、内容版本与 08 已通过的版本一致。
2. 先查询草稿箱里是否已有本期同标题草稿：有就更新或报告，不重复新建。接口响应不确定时先查记录，再决定是否重试。
3. 用 mp-helper skill（先读它的 SKILL.md，严格按用法）：把 HTML 里的图片上传公众号素材库并替换链接，再上传草稿箱；标题、摘要、作者、封面取自 08 的字段清单。
4. 回读：记录草稿 ID、后台截图或回读摘要，附内容版本号。

异常（授权范围不符、字段不一致、出现重复草稿）只报主编，不直接找 Van。

交付物：文件夹 = index.md（草稿 ID、回读摘要、版本号、操作记录）+ html/article.html（素材库地址替换后的正文）+ images/（回读截图）。',
 '## 提交前自检（全过才交）
- 本期 run 的授权包含公众号草稿；没有越权群发。
- 保存前查过草稿箱，没有产生重复草稿。
- HTML 内图片已上传素材库并替换，无外链残留。
- 标题、摘要、作者、封面与 08 字段清单一致。
- 已回读：草稿 ID、截图或摘要、内容版本号都已写明。',
 '## 验收标准（主编对照；本阶段不设审核闸，异常才升级）
- 草稿箱里有且只有一份本期草稿，内容版本与 08 已通过的版本一致。
- 字段齐全正确：标题（≤64 字符）、摘要、作者、封面图。
- 图片均为素材库地址。
- 回读记录可核：草稿 ID、截图或摘要、版本号。
- 规范合规：index.md 写明操作记录，html/article.html 为替换后的正文。'),

-- ───── 10-小红书改编 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 10, 'xhs_text', '10-小红书改编', 'xhswriter', '小红书文本', 0, 'manual', 'read', NULL, 0,
 '## 任务内容（怎么做）
输入：主编派工单，上游是已通过 Van 全文审核的 07 完整审核稿。只有当期纳入小红书时主编才派工；派工单只要导语就只交导语。

小红书不是公众号压缩版：依据传播力、视觉吸引力、认知门槛、讨论价值和信息重要度重新排序和表达；保留准确的产品结构、背景与使用信息，事实与审核稿一致、可溯源。不编造体验，不加评论引导或额外模块。

产出（全部写在 index.md，无附件）：
1. 笔记标题：18 字以内，单一传播点、点击导向；不堆品牌名、不做合集式标题、不直接复制公众号标题。
2. 固定导语（正文开头）：
   📢 户外内容平台 CAMPsomeWHERE
   👇为你播报过去24小时发布的户外潮流资讯
   随后用 🏕️ 🕶️ 👟 ⛺ 等符号列出本期亮点。
3. 笔记正文：保留全部资讯，不减少数量；普通条目 200–300 字，重点条目 300–400 字；排序按传播力，可以不同于公众号顺序。
4. 分页文案（给设计师）：封面主标题 + 副标题 + 封面配图建议 + 推荐原因；各内容卡逐卡列出卡片标题（10–16 字）与卡片正文。

语言：信息优先，少抒情、少空话、少总结；少断行，不留公众号式大面积空白。

送审：主编审 → Van 小红书文本（主编代录）。公众号全文通过不等于小红书文本通过。

交付物：文件夹 = index.md（笔记标题 + 固定导语 + 笔记正文 + 分页文案）。',
 '## 提交前自检（全过才交）
- 笔记标题不超过 18 字，单一传播点，不是公众号标题的直接复制。
- 固定导语正确，并用符号列出本期亮点。
- 保留了全部资讯；普通条目 200–300 字，重点条目不超过 400 字。
- 事实与 07 审核稿一致，没有编造体验或评论引导。
- 分页文案齐全：封面主标题、副标题、配图建议、推荐原因，内容卡逐卡可用。',
 '## 验收标准（主编 / Van 小红书文本）
标题：18 字以内；单一传播点；点击导向；不是公众号标题的直接复制。
导语：使用固定导语，亮点列举与本期内容对应。
正文：保留全部资讯；字数符合普通 200–300、重点 300–400；排序按传播力。
事实：与 07 审核稿一致、可溯源；无杜撰、无夸大。
语言与排版：信息优先，保持小红书阅读节奏；少断行，没有公众号式留白。
分页文案：封面信息与各卡文案齐全，可直接交设计师执行。
规范合规：index.md 四部分齐全，无附件。'),

-- ───── 11-小红书视觉 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 11, 'xhs_visual', '11-小红书视觉', 'designer', '小红书卡片', 0, NULL, 'read', NULL, 0,
 '## 任务内容（怎么做）
10 小红书改编通过 Van 审核后引擎自动派工给你。据已通过的分页文案做封面卡与内容卡。

规格：全部 1242 × 1656 px，3:4 竖版，JPG，sRGB。已登记的小红书卡片模板优先复用。

封面卡：资讯原图 + 主标题（取自分页文案，最多两行，位于上半部分）+ 副标题（最多一行）+ 一句话摘要；图片主体占画面 60% 以上，3 秒内看懂，缩略图状态可识别。封面优先本期传播力最高的资讯。不放大段文字、参数堆砌、品牌堆砌。

内容卡：每则资讯 1–3 张，组成 = 资讯图片 + 卡片标题（10–16 字）+ 卡片正文；文字取自已通过的分页文案，不自行改写。图片区域与文字区域各占 40%–60%，保持留白，适配手机阅读。不做长截图、PPT 风、详情页风、海报风。

图片使用原则：优先资讯原图；禁止重绘装备主体、改产品结构或颜色、AI 生成不存在的产品或细节；只有背景、纹理、图标与版式元素才用 generate-images skill 生成。

全套风格统一、文字可读；不放日期、平台、作者名、水印。数量按派工单。

交付物：文件夹 = index.md（设计意见 + 规格 + 卡片顺序）+ xiaohongshu/（封面卡 + 内容卡）。',
 '## 提交前自检（全过才交）
- 封面卡与内容卡齐全，数量按派工单，全部 1242 × 1656。
- 使用资讯原图；没有重绘主体，没有生成不存在的产品。
- 卡片文字取自已通过的分页文案，没有改写。
- 封面卡主体不少于画面 60%，缩略图可识别。
- 图文比例 40%–60%，全套风格统一；无日期、平台、作者、水印。
- index.md 写明卡片顺序与设计意见。',
 '## 验收标准（主编对照）
- 规格正确：3:4 竖版，1242 × 1656 px。
- 封面卡结构完整（原图、主标题两行内、副标题一行内、一句话摘要），缩略图可识别。
- 内容卡文字与分页文案一致；标题 10–16 字。
- 原图使用规范；AI 只用于背景与版式元素。
- 全套风格统一、文字可读，留白适配手机阅读。
- 规范合规：卡片在 xiaohongshu/，index.md 含设计意见与卡片顺序。'),

-- ───── 12-小红书组包 ─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 12, 'xhs_package', '12-小红书组包', 'publisher', '成品', 0, NULL, 'read', NULL, 0,
 '## 任务内容（怎么做）
10 小红书改编通过 Van 审核、11 卡片通过主编审核后引擎自动派工给你。

1. 用 10 已通过版本的笔记标题与正文，加 11 已通过版本的封面卡和内容卡，组成可直接发布的图文笔记包；卡片顺序与分页文案一致。
2. 核对：图文一一对应、卡片数量正确、文字与审核稿一致、无日期平台作者水印。
3. 本阶段不进平台后台，只交本地成品。

交付物：文件夹 = index.md（笔记标题 + 正文 + 卡片顺序清单 + 与批准版本的对应关系）+ xiaohongshu/（卡片）。',
 '## 提交前自检（全过才交）
- 笔记标题与正文取自 10 已通过的版本，卡片取自 11 已通过的版本，版本号已写明。
- 卡片顺序与分页文案一致，数量正确。
- 图文一一对应，没有改动文字。
- 没有进入平台后台。',
 '## 验收标准（主编核版）
- 内容与批准版本一致，版本对应关系可核。
- 卡片顺序、数量正确，图文对应。
- 笔记包可直接用于发布：标题、正文、卡片齐全。
- 规范合规：index.md 清单完整，卡片在 xiaohongshu/。'),

-- ───── 13-小红书草稿与发布（平台写操作）─────
((SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'),
 13, 'xhs_save', '13-小红书草稿与发布', 'publisher', '发布物', 0, NULL, 'platform_write:xhs_draft', NULL, 0,
 '## 任务内容（怎么做）
本阶段是平台写操作：只有本期 run 的授权包含小红书草稿或小红书发布时，引擎才会派工；没有授权就停在「待授权」。派工单写明本次是入草稿还是公开发布，只做授权范围内的动作。

1. 确认账号、动作、内容版本与 12 已通过的版本一致；先查创作者中心是否已有同标题草稿，避免重复。
2. 用 opencli xiaohongshu publish 按派工单指示入草稿或发布，操作之间按规定插入随机间隔。
3. 回读：记录笔记 ID 或草稿位置、截图，附内容版本号。

异常只报主编，不直接找 Van。

交付物：文件夹 = index.md（笔记 ID 或草稿位置、回读摘要、版本号、操作记录）+ xiaohongshu/笔记文案.md（实发标题 + 正文 + 所用卡片清单）。',
 '## 提交前自检（全过才交）
- 本期 run 的授权覆盖本次动作（入草稿或发布）。
- 操作前查过已有草稿，没有重复。
- 实发内容与 12 已通过的版本一致，没有改写。
- 已回读：笔记 ID 或草稿位置、截图、版本号都已写明。',
 '## 验收标准（主编对照；本阶段不设审核闸，异常才升级）
- 动作与授权一致：只存草稿时没有公开发布。
- 平台上有且只有一份本期笔记或草稿，内容与批准版本一致。
- 回读记录可核：笔记 ID 或草稿位置、截图、版本号。
- 规范合规：index.md 写明操作记录，xiaohongshu/笔记文案.md 齐全。');
-- +goose StatementEnd

-- DAG 依赖边（15 条）：intake→shortlist→topic→{write, material, design_prep}；fulltext←write+material；
-- wx_layout←fulltext+design_prep→wx_save；xhs_text←fulltext→xhs_visual；xhs_package←xhs_text+xhs_visual→xhs_save。
-- +goose StatementBegin
INSERT INTO workflow_stage_deps (workflow_id, stage_id, depends_on_id)
SELECT s.workflow_id, s.id, d.id
FROM workflow_stages s
JOIN workflow_stages d ON d.workflow_id = s.workflow_id
WHERE s.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active')
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
    OR (s.code='xhs_text'    AND d.code='fulltext')
    OR (s.code='xhs_visual'  AND d.code='xhs_text')
    OR (s.code='xhs_package' AND d.code='xhs_text')
    OR (s.code='xhs_package' AND d.code='xhs_visual')
    OR (s.code='xhs_save'    AND d.code='xhs_package')
  );
-- +goose StatementEnd

-- 逐阶段闸（10 行，无默认闸）。未列出的阶段（intake / shortlist / material / design_prep / wx_save / xhs_save）为 0 闸，提交即通过。
-- +goose StatementBegin
INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name)
SELECT s.workflow_id, s.id, g.gate_order, g.reviewer_role, g.relayed, g.name
FROM workflow_stages s
JOIN (
            SELECT 'topic' code, 1 gate_order, 'editor' reviewer_role, 0 relayed, '主编自审' name
  UNION ALL SELECT 'topic',       2, 'van',    1, 'Van选题'
  UNION ALL SELECT 'write',       1, 'editor', 0, '主编审'
  UNION ALL SELECT 'fulltext',    1, 'editor', 0, '主编定点编辑'
  UNION ALL SELECT 'fulltext',    2, 'van',    1, 'Van全文'
  UNION ALL SELECT 'wx_layout',   1, 'editor', 0, '主编核版'
  UNION ALL SELECT 'xhs_text',    1, 'editor', 0, '主编审'
  UNION ALL SELECT 'xhs_text',    2, 'van',    1, 'Van小红书文本'
  UNION ALL SELECT 'xhs_visual',  1, 'editor', 0, '主编审'
  UNION ALL SELECT 'xhs_package', 1, 'editor', 0, '主编核版'
) g ON g.code = s.code
WHERE s.workflow_id = (SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active');
-- +goose StatementEnd

-- +goose Down
-- 删除本迁移建的 v2（识别特征：含 fulltext 阶段；子表 CASCADE），恢复 v1 为 active。
-- 已有 run 引用 v2 时外键会阻止删除——那时应改为前滚修复，而不是回滚。
DELETE FROM workflows WHERE wf_key='daily_news'
  AND id IN (SELECT workflow_id FROM workflow_stages WHERE code='fulltext');
UPDATE workflows SET status='active' WHERE wf_key='daily_news' AND version=1
  AND NOT EXISTS (SELECT 1 FROM workflows WHERE wf_key='daily_news' AND status='active');
UPDATE chat_members SET kind='reserved', role_code=NULL, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
  WHERE role_code IN ('xhswriter','reviewer','analyst');
DELETE FROM agents WHERE role_code IN ('xhswriter','reviewer','analyst');
DELETE FROM roles WHERE code IN ('xhswriter','reviewer','analyst');
ALTER TABLE tasks DROP COLUMN sla_minutes;
ALTER TABLE tasks DROP COLUMN action_class;
ALTER TABLE tasks DROP COLUMN dispatch_mode;
ALTER TABLE workflow_stages DROP COLUMN per_item;
ALTER TABLE workflow_stages DROP COLUMN sla_minutes;
ALTER TABLE workflow_stages DROP COLUMN action_class;
ALTER TABLE workflow_stages DROP COLUMN dispatch_mode;
