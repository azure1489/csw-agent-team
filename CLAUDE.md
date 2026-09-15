# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目性质

本仓库有两个性质截然不同的部分：

1. **流程规范文档（`docs/`）** — 「营事编集室」多 Agent 内容生产团队的规范文档（无构建命令）。产出物是微信公众号文章 + 小红书图文笔记，由九个角色按**十三阶段**流水线协作完成（v3，2026-09 起；v1 十阶段已归档）。
2. **配套服务子工程（`csw-task/`，Monorepo）** — 把上述流程抽象成一个**通用工作流引擎**（任意多阶段、多角色、带审核闸的协作流程做成可编程状态机），有真实构建/测试。`docs/任务流转服务_设计.md` + `docs/管理后台_功能与线框.md` 是其权威蓝图。

仓库布局：

```
CLAUDE.md
docs/                         流程规范 + 服务设计蓝图（详见下「文件结构与权威性」）
skill/                        csw-task skill（SKILL.md v3，只读分发）+ roster.json 花名册兜底 + bootstraps/ 九角色部署模板
csw-task/                     配套服务 Monorepo
  csw-task-svc/               Go + SQLite 后端（运行面 + 管理后台 + 运维 CLI）
  csw-task-web/               React + Vite 管理后台前端
```

## 命令（仅 `csw-task/` 子工程有构建/测试；`docs/` 是纯文档）

后端 `csw-task/csw-task-svc/`（Go 1.23）：

```bash
cd csw-task/csw-task-svc
make build                                   # 产出 bin/{server,adminsrv,adminctl}
make test                                    # = go test ./...
go test ./internal/engine/                   # 单个包
go test ./internal/engine/ -run TestTrigger  # 单个测试（-run 接正则）
go test ./internal/engine/ -run TestTrigger -v
make tidy                                    # go mod tidy

# 初始化 + 引导（首次）
./bin/adminctl migrate up
./bin/adminctl workflow validate daily_news
./bin/adminctl token issue editor                                  # 运行面 agent token（明文仅一次）
./bin/adminctl user create admin --role superadmin --password ***  # 首个后台用户（argon2id）

# 启动两进程（共享同一 sqlite，分凭证）
CSW_ADDR=:8080 ./bin/server                               # 运行面（bearer token）
CSW_JWT_SECRET=*** CSW_ADMIN_ADDR=:8081 ./bin/adminsrv    # 管理后台（JWT）
```

前端 `csw-task/csw-task-web/`（React 18 + Vite 5 + TS）：

```bash
cd csw-task/csw-task-web
pnpm install
pnpm dev      # Vite :5173（端口固定，对齐后端 CORS）；真连 :8081/admin/*
pnpm build    # tsc 类型检查 + vite 生产打包 → dist/
pnpm lint     # 仅 tsc --noEmit（无 ESLint）
```

配置优先级 **env > `config.yaml` > 内置默认**。完整字段、联调步骤、各包测试覆盖见 `csw-task/csw-task-svc/README.md` 与 `csw-task/csw-task-web/README.md`。

生产一键部署（幂等；本机构建 → 上传 → 远端 systemd×2 + docker nginx 反代 + 引导）：

```bash
./csw-task/deploy.sh [user@host] [domain]   # 默认 root@8.138.43.109 tasks.aworld.ltd
# 布局：/ 前端 SPA · /admin/* → adminsrv:8081 · /api/v1/* → server:8080（同源免 CORS）
# 角色 token 一次性引导不在脚本内：ssh <host> /opt/csw-task/bin/adminctl token issue <role>
```

---

# 第一部分：流程规范（`docs/`）

## 文件结构与权威性

> 注意：流程文档与作业手册过去在仓库根目录，现已全部移入 `docs/`。引用路径时以 `docs/` 为准。

- `docs/资讯日更_全流程.md` — **当前生效的流程定义**（v3 十三阶段：采集与研究逐条交错 → 选题 → 写作 / 配图 / 版式三路并行 → 完整审核稿 → 公众号组版与草稿保存；小红书支线直接接完整审核稿），流程图为文内 mermaid（`docs/pipeline_wechat_xhs_v2.png` 是 v1 旧图，仅作历史参考）。流程形态与其他文档冲突时，**以此文档为准**。
- `docs/资讯日更_交付与流转规范.md` — **交付与流转协议的权威**（文件夹 / zip / 存储路径 / 命名 / header / 按阶段声明的闸 / 补件 / 定点编辑 / 授权 / 状态词汇），第 8 节逐段给出十三阶段的派工、闸、动作与 header 范式。
- `docs/资讯日更_验收标准.md` — **主编与 Van 的审核尺子**：通用合规项 + 十三阶段逐段清单。
- `docs/资讯日更_生产约定.md` — 窗口、数量、晨间时间表、人审三处、各阶段时限、授权默认值、送达形态、run inputs 固定字段、引擎权威（草案，待 Van 确认）。
- `docs/作业手册_*.md` — 九个角色的作业手册（主编 / 情报收集员 / 选题研究员 / 文案 / 小红书图文作者 / 设计师 / 发布员 / 合规版权审查员 / 数据复盘师）。
- `docs/新流程改造_整体优化方案_20260915.md` — 本轮改造的方案、主机取证记录与待定事项；两份 0914 反馈文档是其输入。
- `docs/任务流转服务_设计.md` + `docs/管理后台_功能与线框.md` — `csw-task/` 后端的权威蓝图（DDL / 状态机 / API / 鉴权 / 护栏 / 后台线框）。
- `docs/任务流转后台_设计稿/` — 管理后台前端 `csw-task-web/` 的**视觉权威源**（像素级复刻对象：`任务流转后台.html` + `csw-admin/*.jsx` 源 + `screenshots/`）。
- `docs/参考/` — 上一个任务「游戏资讯日报」的历史资料，仅作参考范式，**不是当前任务的权威规范**。差异：任务名、采集源（旧版为 X / Reddit）、终审人称呼（旧版为「用户」）、旧版无小红书。

**阶段的作业内容 / 自检 / 验收的权威在引擎定义**（seed 迁移，或管理后台 / wfctl 改定义）。验收标准全文、规范 §8、全流程的阶段表、作业手册的各阶段小节都由 `docs/tools/gen_stage_docs.py` 从迁移后的数据库生成——改定义后重新生成，不要手改这些小节。修改流程形态改定义 + `docs/资讯日更_全流程.md`；修改交付协议改 `docs/资讯日更_交付与流转规范.md` 的手写部分；任一变动都要联动检查作业手册的岗位要点与 `skill/`。

## 核心架构：十三阶段（v3，`daily_news` v2）

```
01-情报逐条(情报收集员 · 0闸) → 02-价值初筛(选题研究员 · 0闸)
  → 03-选题方案(主编合流 · 主编自审 → Van 选题)
      ├→ 04-公众号写作(文案 · 手动派工 · 主编审，按已批条目)   ┐
      ├→ 05-配图与素材核(情报收集员 · 0闸，只为已批条目)        ├→ 07-完整审核稿(主编合流 · 主编定点编辑 → Van 全文)
      └→ 06-版式与模板准备(设计师 · 0闸) ───────────────┐      ┘
07 ─┬→ 08-公众号组版打包(发布员 · 主编核版，依赖 06+07) → 09-公众号草稿保存(发布员 · 平台写，须 run 授权)
    └→ 10-小红书改编(小红书图文作者 · 手动派工 · 主编 → Van) → 11-小红书视觉(设计师 · 主编审)
          → 12-小红书组包(发布员 · 主编核版) → 13-小红书草稿与发布(发布员 · 平台写，须授权)
```

- **Van 日常只做三类决定**：选题（03）、完整文案（07）、小红书文本（10，仅当期纳入小红书）；由主编代录并附 Van 原话。其余阶段主编单闸或 0 闸。
- **主编是中枢**：合成选题方案与完整审核稿；07 的主编闸上有定点编辑权（首稿保留、改动留痕、预算约 10 分钟）；录入授权；处置失败、取消与重开。
- **平台写受授权约束**：09 / 13 只有本期 run 有对应授权（本地演练 / 公众号草稿 / 公众号发布 / 小红书草稿 / 小红书发布）才派工与提交；只有本地演练时 08 的本地成品即交付。
- **补件不重跑已批正文**：换图、换资产、补事实、追加批准条目，挂在已通过的任务上，只让已开工的直接下游返工。
- **群播报由引擎单写**：派工、待审、退回、阶段通过、待授权、补件、失败、逾期都由引擎自动发到编辑部群并真 @；群消息只是提示，引擎才是真相。

## 交付物协议（所有 Agent 共用）

- **形态**：一个文件夹 = `index.md`（固定入口，正文内联）+ 附件子目录（`images/`、`source/`、`html/`、`xiaohongshu/`），相对链接引用。
- **流转**：文件夹打包成同名 zip → **一步式提交**任务流转服务（`POST /tasks/:id/deliverables` multipart，`kind` = 产出 / 补件 / 定点编辑）→ 角色之间只传**完整 https 链接**。
- **命名与路径由服务端派生**：文件名 `用途_责任方_日期_任务ID_版本.zip`（`任务ID`=`r{run_id}`；合流阶段责任方为产出类型「选题方案」「审核稿」；补件为「角色名补件」），路径 `csw/{用途}/{日期}/{任务ID}/{阶段序号-阶段}/{zip}`。定点编辑版本与产出共用版本流。
- **派工单已引擎化**：`editor_note` + `upstreams`（自动预填上游与补件），不打 zip；引用写「主编 · r{run_id} 任务#{task_id} 派工单」。
- **溯源**：每个 `index.md` 顶部带 YAML 元信息头（任务 / 类型 / 交付 Agent / 阶段 / 版本 / 时间 / 上游来源 / 状态 / 自检；按条目交付加「条目」）。
- **状态以引擎为准**：未就绪 → 待派工 → 已派工 → 已接单 → 待审 → 已交付；另有需返工、报告失败、已取消。退回原因从任务详情的 `latest_review` 自助读取。

## 角色工具约定（详见 `docs/作业手册_*.md`）

- **情报收集员**（01、05）：Instagram 用营事编集室 csw MCP；小红书用 `opencli xiaohongshu`（随机间隔防风控）；网页优先 opencli 适配器、否则 WebFetch。逐条交，原始披露时间与转载时间分开写。
- **选题研究员**（02）：采用 / 备选 / 淘汰 + 理由、看点、近发对照、缺口；看图查近发，不虚构升级对照。
- **文案**（04）：只写已批条目，每则「品牌｜一句话标题」+ 300–500 字三段 + 3 个图位；全部写完附标题方案、摘要、导读。
- **小红书图文作者**（10）：从完整审核稿独立重组小红书表达（标题 ≤18 字、固定导语、普通 200–300 / 重点 300–400 字、分页文案）。
- **设计师**（06、11）：维护已批资产与模板版本；日常原图排版不必重新生图，只有指定生成才用 `generate-images`；换图走补件。
- **发布员**（08、09、12、13）：交付执行负责人；`mp-helper` 存公众号草稿、`opencli xiaohongshu publish` 发小红书；无授权不进后台，保存前查重、保存后回读。
- **合规版权审查员**：主编点名的专项检查，结论以补件挂到受影响交付物。**数据复盘师**：不在日更 run 内，负责发布记录与复盘。

---

# 第二部分：工作流引擎（`csw-task/`）

把第一部分的「多阶段、多角色、带审核闸」抽象成**通用**状态机——引擎层与具体流程无关，`daily_news`（v1 十阶段已归档，当前 v2 十三阶段）只是 seed 进去的第一个工作流定义。设计文档 `docs/任务流转服务_设计.md`（§3 DDL / §5 状态机 / §6 API / §7 鉴权 / §8 护栏）是后端的权威蓝图。

## 后端 `csw-task-svc/`：三入口、分进程分凭证

Go module `github.com/azure1489/csw-agent-team/csw-task-svc`，golang-standards/project-layout：

```
cmd/{server,adminsrv,adminctl}/    三入口
internal/
  config/        env 配置（运行面 + JWT/CORS/后台共用）
  app/{server,adminsrv,adminctl}/  gin 装配 + handlers / CLI
  domain/        实体 + 状态枚举 + 纯规则（无框架依赖）
  engine/        工作流引擎（触发/就绪/派工/提交/审核，单事务）  ← 核心
  workflow/      定义校验（DAG/入口/中枢/闸/三段非空）
  store/sqlite/  连接层 + 仓储 + migrations/*.sql（goose embed）
  files/         BlobStore 接口 + 本地内容寻址实现（含 OSS 后端）
  auth/          bearer token + argon2id 密码 + JWT 签发/校验
  middleware/    requestid/logger/recovery/agentauth/idempotency · jwtauth/rbac/cors
```

**两个 HTTP 进程共享同一 sqlite、但分凭证**——这是核心架构约束：

- `cmd/server`（运行面，`:8080`）：agent 经 **bearer token** 调用，`/api/v1/*`。承载流程动作：触发实例、查我的任务/派工单、上传下载交付物、提交产出、中枢派工、按闸审核。
- `cmd/adminsrv`（管理后台，`:8081`）：人类操作员经 **JWT** 登录（access HS256 内存 + refresh httpOnly cookie 轮换 + argon2id 密码 + RBAC：superadmin/operator/viewer），`/admin/*`。只做定义/成员/监控——**绝不操作流程**（派工/审核/提交属运行面），监控只读。
- `cmd/adminctl`（运维 CLI）：迁移、agent token 引导、工作流校验、离线创建首个 superadmin。

技术栈：`gin` · `modernc.org/sqlite`（纯 Go 免 cgo）· `goose/v3`（迁移 embed）· `slog`。并发模型：WAL + `busy_timeout` + `foreign_keys` + `SetMaxOpenConns(1)`（写串行）。

引擎要点（`internal/engine`）：触发即**快照**工作流定义与阶段属性（派工模式 / 动作类别 / 时限），运行实例不受此后定义热改影响；提交后按阶段声明的闸推进（0 闸直通）；越级审核 409；退回必填方向+位置；代录闸须附原话；合流阶段就绪时自动派给中枢。**平台写护栏**：`action_class=platform_write:<scope>` 的任务在派工、自动派工、提交三处检查 run 授权（`run_authorizations`），缺失 409 `authorization_required` 并停在 ready。**生命周期**：接单 / 心跳（ack）、报告失败、取消（级联未开始的下游）、重开；run 完成 = 全部终态且至少一个通过。**交付物 kind**：补件（挂已通过任务、标下游待返工）、定点编辑（中枢在自己的闸上提交编辑版本、首稿保留）。**事件接续**：需要通知的事件与 outbox 同事务写入，server 进程内 notifier 经飞书 Open API 单写群播报（`CSW_NOTIFIER_*` / `CSW_LARK_*`，默认关闭）；`GET /runs/:id/progress` 给出真实卡点。迁移 0009–0014 为本轮增量（0011 / 0012 为表重建，deploy 前自动备份）。所有写操作支持 `Idempotency-Key`（fail-closed：业务前持久占位 + 请求指纹绑定；2xx 后同指纹回放，非 2xx 释放占位可重试，崩溃后同键 409 需人工核实；见设计 §8），错误统一 `{code,message}` + HTTP 码。

## 前端 `csw-task-web/`：管理后台

React 18 + Vite + TS + Tailwind v3 + Radix（弹层底座），**像素级复刻** `docs/任务流转后台_设计稿/`。真连 `:8081/admin/*`：access JWT 存内存 + refresh cookie 401 自动续期重试 + RBAC 路由守卫。DAG 用自绘 SVG（移植设计稿的分层+贝塞尔+环检测，未引入 React Flow）；激活校验前端 `runChecks` 即时预览 + 后端权威（`PUT` 保存→`activate` 归档旧版，非阻断校验）。`lib/adapters.ts` 处理 API↔编辑器形态转换（deps id↔code 等）。详见 `csw-task/csw-task-web/README.md`。

## skill 通道 `skill/SKILL.md`

`csw-task` skill（**v3.0.0，只读分发**）：agent 与运行面之间的唯一通道 + 编辑部群协作协议。两层架构：**引擎是权威**（状态 / 文件 / 版本 / 闸 / 授权），**飞书编辑部群是可见层**；v3 起**群播报由引擎单写**，agent 只调引擎（被 @ 后先 `my-tasks` 核实、处理全部 open 任务）。核心约定：开工前版本自检（`skill_min_version`）；引擎地址只来自 `CSW_TASK_BASE_URL`，禁止自建引擎副本；接单后心跳，做不了报失败；一步式 submit（`kind` = 产出 / 补件 / 定点编辑）；幂等键从内容确定性派生；退回必填方向+位置；代录 Van 决定必附原话与决定类别；授权、取消、重开由主编操作。作业手册 / 自检 / 验收一律以 `task <id>` 引擎返回为准，skill 不复述标准。
