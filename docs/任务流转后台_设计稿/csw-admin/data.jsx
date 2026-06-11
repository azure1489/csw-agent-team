// ============================================================
// Seed data — 资讯日更 (daily_news) 十阶段流水线为真实示例
// ============================================================

const ROLES = [
  { code: "editor",     name: "主编",       is_management: true,  is_human: false },
  { code: "van",        name: "Van",        is_management: true,  is_human: true  },
  { code: "collector",  name: "情报收集员",  is_management: false, is_human: false },
  { code: "researcher", name: "选题研究员",  is_management: false, is_human: false },
  { code: "writer",     name: "文案",       is_management: false, is_human: false },
  { code: "designer",   name: "设计师",     is_management: false, is_human: false },
  { code: "publisher",  name: "发布员",     is_management: false, is_human: false },
  { code: "scheduler",  name: "调度器",     is_management: true,  is_human: false },
];

const AGENTS = [
  { id: 1, name: "主编bot",   role_code: "editor",     active: true,
    tokens: [
      { id: 12, label: "生产",   status: "active",  last_used_at: "2026-06-09 10:20", expires_at: "永不",        created_at: "2026-03-01" },
      { id: 9,  label: "轮换-Q1", status: "revoked", last_used_at: "2026-02-28 18:02", expires_at: "永不",        created_at: "2026-01-04" },
    ] },
  { id: 2, name: "情报bot",   role_code: "collector",  active: true,
    tokens: [
      { id: 21, label: "生产",   status: "active",  last_used_at: "2026-06-09 10:42", expires_at: "永不",         created_at: "2026-04-12" },
      { id: 22, label: "轮换-Q2", status: "active",  last_used_at: "—",               expires_at: "2026-09-30",   created_at: "2026-06-01" },
    ] },
  { id: 3, name: "选题bot",   role_code: "researcher", active: true,
    tokens: [ { id: 31, label: "生产", status: "active", last_used_at: "2026-06-09 11:30", expires_at: "永不", created_at: "2026-04-12" } ] },
  { id: 4, name: "文案bot",   role_code: "writer",     active: true,
    tokens: [ { id: 41, label: "生产", status: "active", last_used_at: "2026-06-09 13:55", expires_at: "永不", created_at: "2026-04-12" } ] },
  { id: 5, name: "设计bot",   role_code: "designer",   active: true,
    tokens: [ { id: 51, label: "生产", status: "expired", last_used_at: "2026-05-30 09:12", expires_at: "2026-06-01", created_at: "2026-03-01" } ] },
  { id: 6, name: "发布bot",   role_code: "publisher",  active: false,
    tokens: [ { id: 61, label: "生产", status: "active", last_used_at: "2026-06-05 20:10", expires_at: "永不", created_at: "2026-04-12" } ] },
  { id: 7, name: "scheduler", role_code: "scheduler",  active: true,
    tokens: [ { id: 71, label: "定时任务", status: "active", last_used_at: "2026-06-09 06:00", expires_at: "永不", created_at: "2026-04-12" } ] },
  { id: 8, name: "Van",       role_code: "van",        active: true, human: true, tokens: [] },
];

// ---- Markdown content samples for stages ----
const MD = {
  collect_inst: `# 01-采集 · 作业手册

**任务内容**：抓取当日各平台资讯，整理为结构化资讯包。

## 怎么做
1. 覆盖官方渠道（品牌官网 / 官方公众号 / 官博）
2. 覆盖社区热门（小红书 / 微博话题 / 行业垂媒）
3. 每条记录：标题 · 来源链接 · 摘要 · 配图 ≥ 1

## 工具
- \`opencli fetch\` 抓取网页
- \`mp-helper\` 拉取公众号文章

## 产出要求
- \`资讯包.zip\`：\`index.md\`（YAML 头）+ 原文截图 + 配图
- 不少于 5 条，且每条多图`,
  collect_self: `## 提交前自检（全过才交）
- [ ] 官方渠道覆盖完整
- [ ] 社区热门已纳入
- [ ] 所有来源链接可达
- [ ] 已去重（同事件只保留信息最全一条）
- [ ] 每条配图清晰、数量充足`,
  collect_acc: `## 验收标准（主编 / Van 对照判定）
- 来源平台齐全（官方 + 社区）
- 链接全部可达
- 无重复事件
- 配图清晰、数量满足下游排版`,
  topic_inst: `# 02-选题 · 作业手册

从资讯包中筛选当日选题，排序并补足配图。

## 怎么做
1. 从上游资讯包中选 **5 则** 最具传播价值的选题
2. 按重要性 / 时效性排序
3. 每则补足配图至 **3 张**（共 15 图）

## 产出要求
- \`选题成品.zip\`：5 则 × {标题 / 导语 / 配图×3 / 来源}`,
  topic_self: `## 提交前自检
- [ ] 恰好 5 则
- [ ] 已按优先级排序
- [ ] 每则配图 ≥ 3（共 ≥ 15）
- [ ] 来源链接保留`,
  topic_acc: `## 验收标准
- 选题数量正确（5 则）
- 排序合理
- 配图齐全（15 张）
- 选题与品牌调性相符`,
  generic_inst: `# 作业手册

**任务内容**：（描述本阶段做什么 / 用什么工具 / 产出要求）

## 怎么做
1. …
2. …

## 产出要求
- …`,
  generic_self: `## 提交前自检（全过才交）
- [ ] …
- [ ] …`,
  generic_acc: `## 验收标准（reviewer 对照判定）
- …
- …`,
};

// daily_news v3 stages
const DN_STAGES = [
  { id: 301, seq: 1,  code: "collect",     name: "01-采集",       role_code: "collector",  output_type: "资讯包",   is_merge: false, deps: [],
    instructions: MD.collect_inst, self_check: MD.collect_self, acceptance: MD.collect_acc, gate_override: null },
  { id: 302, seq: 2,  code: "topic",       name: "02-选题",       role_code: "researcher", output_type: "选题成品", is_merge: false, deps: [301],
    instructions: MD.topic_inst, self_check: MD.topic_self, acceptance: MD.topic_acc, gate_override: null },
  { id: 303, seq: 3,  code: "wx_content",  name: "03-公众号内容",  role_code: "writer",     output_type: "文章",     is_merge: false, deps: [302],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 304, seq: 4,  code: "wx_visual",   name: "04-公众号视觉",  role_code: "designer",   output_type: "封面",     is_merge: false, deps: [303],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 305, seq: 5,  code: "wx_final",    name: "05-公众号成品",  role_code: "editor",     output_type: "成品",     is_merge: true,  deps: [303, 304],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc,
    gate_override: [ { gate_order: 1, reviewer_role: "editor", relayed: false, name: "主编自检" }, { gate_order: 2, reviewer_role: "van", relayed: true, name: "Van审" } ] },
  { id: 306, seq: 6,  code: "wx_publish",  name: "06-公众号发布",  role_code: "publisher",  output_type: "发布物",   is_merge: false, deps: [305],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 307, seq: 7,  code: "xhs_text",    name: "07-小红书文本",  role_code: "writer",     output_type: "小红书文本", is_merge: false, deps: [306],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 308, seq: 8,  code: "xhs_visual",  name: "08-小红书视觉",  role_code: "designer",   output_type: "卡片",     is_merge: false, deps: [307],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 309, seq: 9,  code: "xhs_final",   name: "09-小红书成品",  role_code: "editor",     output_type: "成品",     is_merge: true,  deps: [307, 308],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 310, seq: 10, code: "xhs_publish", name: "10-小红书发布",  role_code: "publisher",  output_type: "发布物",   is_merge: false, deps: [309],
    instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
];

// event_recap v1 (draft) — 6 stages, auto dispatch
const ER_STAGES = [
  { id: 401, seq: 1, code: "gather",   name: "01-素材汇集", role_code: "collector",  output_type: "素材包", is_merge: false, deps: [],     instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 402, seq: 2, code: "outline",  name: "02-复盘提纲", role_code: "researcher", output_type: "提纲",   is_merge: false, deps: [401],  instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 403, seq: 3, code: "draft",    name: "03-初稿",     role_code: "writer",     output_type: "初稿",   is_merge: false, deps: [402],  instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 404, seq: 4, code: "visual",   name: "04-配图",     role_code: "designer",   output_type: "配图",   is_merge: false, deps: [403],  instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 405, seq: 5, code: "final",    name: "05-成品",     role_code: "editor",     output_type: "成品",   is_merge: true,  deps: [403,404], instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
  { id: 406, seq: 6, code: "publish",  name: "06-发布",     role_code: "publisher",  output_type: "发布物", is_merge: false, deps: [405],  instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null },
];

const WORKFLOWS = [
  { id: 1, wf_key: "daily_news",  name: "资讯日更", version: 3, status: "active",   hub_role: "editor", dispatch_mode: "manual",
    trigger_roles: ["editor", "scheduler"], stages: DN_STAGES,
    common_instructions: "所有交付物以 zip 提交，内含 index.md（YAML 头：标题/来源/摘要）。Agent 之间只传 download_url。",
    common_acceptance: "通用合规项：① 无敏感/违规内容 ② 来源可追溯 ③ 配图版权合规 ④ 文件命名规范 {工作流}_{角色}_{日期}_v{n}.zip",
    default_gates: [ { gate_order: 1, reviewer_role: "editor", relayed: false, name: "主编审" }, { gate_order: 2, reviewer_role: "van", relayed: true, name: "Van审" } ],
    created_at: "2026-06-09 09:50" },
  { id: 2, wf_key: "daily_news",  name: "资讯日更", version: 2, status: "archived", hub_role: "editor", dispatch_mode: "manual",
    trigger_roles: ["editor", "scheduler"], stages: DN_STAGES,
    common_instructions: "", common_acceptance: "",
    default_gates: [ { gate_order: 1, reviewer_role: "editor", relayed: false, name: "主编审" }, { gate_order: 2, reviewer_role: "van", relayed: true, name: "Van审" } ],
    created_at: "2026-05-20 14:30" },
  { id: 3, wf_key: "event_recap", name: "活动复盘", version: 1, status: "draft",    hub_role: "editor", dispatch_mode: "auto",
    trigger_roles: ["editor"], stages: ER_STAGES,
    common_instructions: "", common_acceptance: "通用合规项：① 数据脱敏 ② 结论可溯源",
    default_gates: [ { gate_order: 1, reviewer_role: "editor", relayed: false, name: "主编审" }, { gate_order: 2, reviewer_role: "van", relayed: true, name: "Van审" } ],
    created_at: "2026-06-08 16:12" },
];

// ---- Runs ----
const RUNS = [
  { id: 11, wf_key: "event_recap", wf_name: "活动复盘", subject: "2026-06-09", status: "active", cur_stage: "02-复盘提纲", trigger: "主编bot", created_at: "2026-06-09 09:10" },
  { id: 10, wf_key: "daily_news",  wf_name: "资讯日更", subject: "2026-06-09", status: "active", cur_stage: "03-公众号内容", trigger: "主编bot", created_at: "2026-06-09 10:00" },
  { id: 9,  wf_key: "daily_news",  wf_name: "资讯日更", subject: "2026-06-08", status: "done",   cur_stage: "—",            trigger: "scheduler", created_at: "2026-06-08 06:00" },
  { id: 8,  wf_key: "daily_news",  wf_name: "资讯日更", subject: "2026-06-07", status: "done",   cur_stage: "—",            trigger: "scheduler", created_at: "2026-06-07 06:00" },
  { id: 7,  wf_key: "daily_news",  wf_name: "资讯日更", subject: "2026-06-06", status: "aborted",cur_stage: "04-公众号视觉", trigger: "主编bot", created_at: "2026-06-06 06:00" },
];

// run #10 detail — stage progress
const RUN10_STAGES = [
  { seq: 1, name: "01-采集",      role: "情报收集员", status: "passed",  cur_v: 1, gate_state: "已通过",       returns: [] },
  { seq: 2, name: "02-选题",      role: "选题研究员", status: "passed",  cur_v: 2, gate_state: "已通过",       returns: [ { v: 1, gate: "②Van审", direction: "补足到15张", location: "第3则仅2张" } ] },
  { seq: 3, name: "03-公众号内容", role: "文案",      status: "review",  cur_v: 2, gate_state: "①主编审✔ ②Van审 待", returns: [ { v: 1, gate: "②Van审", direction: "补足配图", location: "第3则" } ] },
  { seq: 4, name: "04-公众号视觉", role: "设计师",    status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
  { seq: 5, name: "05-公众号成品", role: "主编",      status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
  { seq: 6, name: "06-公众号发布", role: "发布员",    status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
  { seq: 7, name: "07-小红书文本", role: "文案",      status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
  { seq: 8, name: "08-小红书视觉", role: "设计师",    status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
  { seq: 9, name: "09-小红书成品", role: "主编",      status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
  { seq: 10,name: "10-小红书发布", role: "发布员",    status: "blocked", cur_v: 0, gate_state: "—",            returns: [] },
];

const RUN10_TIMELINE = [
  { t: "10:00", actor: "主编bot",   type: "run_created",  text: "触发实例 资讯日更 · 2026-06-09" },
  { t: "10:20", actor: "主编bot",   type: "dispatched",   text: "派工 01-采集 → 情报收集员" },
  { t: "10:42", actor: "情报bot",   type: "file_uploaded",text: "上传 资讯包_v1.zip" },
  { t: "10:43", actor: "情报bot",   type: "submitted",    text: "提交 01-采集 产出 v1" },
  { t: "11:00", actor: "主编bot",   type: "gate_passed",  text: "01-采集 闸1 主编审 通过" },
  { t: "11:05", actor: "主编bot",   type: "stage_passed", text: "01-采集 闸2 Van审 通过 → 阶段完成" },
  { t: "11:30", actor: "主编bot",   type: "dispatched",   text: "派工 02-选题 → 选题研究员（上游：01 资讯包）" },
  { t: "12:40", actor: "选题bot",   type: "submitted",    text: "提交 02-选题 产出 v1" },
  { t: "13:10", actor: "主编bot",   type: "gate_returned",text: "02-选题 v1 在 Van审 被退：补足到15张 · 第3则仅2张" },
  { t: "13:55", actor: "选题bot",   type: "submitted",    text: "提交 02-选题 产出 v2" },
  { t: "14:30", actor: "主编bot",   type: "stage_passed", text: "02-选题 双闸通过 → 阶段完成" },
  { t: "14:35", actor: "主编bot",   type: "dispatched",   text: "派工 03-公众号内容 → 文案（上游：02 + 手加01）" },
  { t: "15:20", actor: "文案bot",   type: "submitted",    text: "提交 03-公众号内容 产出 v1" },
  { t: "15:40", actor: "主编bot",   type: "gate_returned",text: "03-公众号内容 v1 在 Van审 被退：补足配图 · 第3则" },
  { t: "16:10", actor: "文案bot",   type: "submitted",    text: "提交 03-公众号内容 产出 v2" },
  { t: "16:25", actor: "主编bot",   type: "gate_passed",  text: "03-公众号内容 v2 闸1 主编审 通过（待 Van审）" },
];

const ADMIN_USERS = [
  { id: 1, username: "zhang", display_name: "张管理", role: "superadmin", status: "active",   last_login_at: "2026-06-09 10:05" },
  { id: 2, username: "li",    display_name: "李运营", role: "operator",   status: "active",   last_login_at: "2026-06-09 09:40" },
  { id: 3, username: "guest", display_name: "访客",   role: "viewer",     status: "disabled", last_login_at: "—" },
];

const AUDIT = [
  { t: "2026-06-09 10:05", user: "zhang", action: "login",             target: "—",                detail: { ip: "10.0.2.31", ua: "Chrome 124 · macOS" } },
  { t: "2026-06-09 09:50", user: "zhang", action: "workflow_activate", target: "daily_news@v3",    detail: { from: "v2", note: "旧版 v2 自动归档", checks: "全部通过" } },
  { t: "2026-06-09 09:40", user: "li",    action: "login",             target: "—",                detail: { ip: "10.0.2.44", ua: "Chrome 124 · Windows" } },
  { t: "2026-06-09 09:30", user: "li",    action: "token_issue",       target: "agent:主编bot #12",detail: { label: "生产", expires: "永不" } },
  { t: "2026-06-08 16:12", user: "zhang", action: "workflow_create",   target: "event_recap@v1",   detail: { dispatch_mode: "auto", stages: 6 } },
  { t: "2026-06-08 11:20", user: "li",    action: "agent_disable",     target: "agent:发布bot",    detail: { reason: "凭证泄露，临时停用" } },
  { t: "2026-06-08 11:18", user: "li",    action: "token_revoke",      target: "agent:设计bot #51",detail: { reason: "已过期，吊销" } },
  { t: "2026-06-07 18:02", user: "zhang", action: "role_update",       target: "role:van",         detail: { is_human: "0 → 1" } },
  { t: "2026-06-07 09:15", user: "zhang", action: "user_create",       target: "user:li",          detail: { role: "operator" } },
];

const OVERVIEW = {
  active_workflows: 3,
  running: 2,
  done_today: 5,
  alerts: 1,
  running_list: [
    { id: 10, name: "资讯日更", subject: "2026-06-09", progress: "3 / 10", cur: "03-公众号内容", status: "active" },
    { id: 11, name: "活动复盘", subject: "2026-06-09", progress: "2 / 6",  cur: "02-复盘提纲",   status: "active" },
  ],
  recent_changes: [
    { name: "daily_news", ver: "v3", what: "已激活", status: "active", t: "09:50" },
    { name: "event_recap", ver: "v1", what: "草稿创建", status: "draft", t: "昨 16:12" },
    { name: "daily_news", ver: "v2", what: "归档", status: "archived", t: "09:50" },
  ],
  recent_audit: AUDIT.slice(0, 5),
};

const CURRENT_USER = { username: "zhang", display_name: "张管理", role: "superadmin" };

Object.assign(window, {
  ROLES, AGENTS, WORKFLOWS, RUNS, RUN10_STAGES, RUN10_TIMELINE,
  ADMIN_USERS, AUDIT, OVERVIEW, CURRENT_USER, MD,
});
