# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目性质

本仓库有两个性质截然不同的部分：

1. **流程规范文档（`docs/`）** — 「营事编集室」多 Agent 内容生产团队的规范文档（无构建命令）。产出物是微信公众号文章 + 小红书图文笔记，由多个 Agent 按十阶段流水线协作完成。
2. **配套服务子工程（`csw-task/`，Monorepo）** — 把上述流程抽象成一个**通用工作流引擎**（任意多阶段、多角色、带审核闸的协作流程做成可编程状态机），有真实构建/测试。`docs/任务流转服务_设计.md` + `docs/管理后台_功能与线框.md` 是其权威蓝图。

仓库布局：

```
CLAUDE.md
docs/                         流程规范 + 服务设计蓝图（详见下「文件结构与权威性」）
skill/                        csw-task skill（SKILL.md 定稿）+ roster.json 花名册 + bootstraps/ 六角色部署模板
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

- `docs/资讯日更_全流程.md` — **当前生效的流程定义**（十阶段：公众号先行 → 小红书后接，全程线性链、每段文案→设计师），内联流程图 `docs/pipeline_wechat_xhs_v2.png`。流程形态与其他文档冲突时，**以此文档为准**。
- `docs/资讯日更_交付与流转规范.md` — **交付与流转协议的权威**（文件夹/zip/OSS/命名/header/双闸审核），第 8 节按十阶段逐段给出产出要求、header 范式与自检清单。
- `docs/资讯日更_验收标准.md` — **主编审核每份交付物的尺子**：通用合规项 + 十阶段逐段清单（内容质量 / 规范合规）。条目提炼自规范与各作业手册，**不新增要求**。
- `docs/作业手册_*.md` — 六个角色的工具与产出标准权威（主编 / 情报收集员 / 选题研究员 / 设计师 / 文案 / 发布员）。
- `docs/任务流转服务_设计.md` + `docs/管理后台_功能与线框.md` — `csw-task/` 后端的权威蓝图（DDL / 状态机 / API / 鉴权 / 护栏 / 后台线框）。
- `docs/任务流转后台_设计稿/` — 管理后台前端 `csw-task-web/` 的**视觉权威源**（像素级复刻对象：`任务流转后台.html` + `csw-admin/*.jsx` 源 + `screenshots/`）。
- `docs/参考/` — 上一个任务「游戏资讯日报」的历史资料，仅作参考范式，**不是当前任务的权威规范**。差异：任务名、采集源（旧版为 X / Reddit）、终审人称呼（旧版为「用户」）、旧版无小红书。

修改流程形态改 `docs/资讯日更_全流程.md`，修改交付协议改 `docs/资讯日更_交付与流转规范.md`；任一变动都要联动检查 `docs/作业手册_*.md` 与 `docs/资讯日更_验收标准.md` 是否需要同步更新。

## 核心架构：十阶段流水线（公众号先行 → 小红书后接）

全程一条**线性链**（无并行分叉）；每个生产段内部固定**文案 → 设计师**的先后（设计师要用文案的产出才能动手）：

```
任务启动（Van 交付当日任务）→ 主编出首个派工单
共享前段
  01-采集(情报收集员) → 02-选题(选题研究员：5 则资讯 + 排序 + 15 张配图)

① 公众号生产 · 母内容源（文案 → 设计师）
  03-公众号内容(文案：标题+摘要+导读+正文+图文排版)
   → 04-公众号视觉(设计师：封面，用 03 内容)
   → 05-公众号成品(主编合成) → 06-公众号发布(发布员 → 草稿箱)
  ──── 06 过审后，公众号成品 = 母内容源，进入小红书 ────

② 小红书生产 · 二次拆解（文案 → 设计师，一条 = 封面卡 + 内容卡 + 文本）
  07-小红书文本(文案) → 08-小红书视觉(设计师：封面卡+内容卡，用 07 文本)
   → 09-小红书成品(主编合成) → 10-小红书发布(发布员) → 任务结束
```

**主编是中枢**：唯一对接 Van（人类终审）的角色，只审核、退回、合成、派工，**绝不亲自编辑内容**。每段产出走两道闸：主编审 → Van 审；任一不过则退回原 Agent 升版本重做（v1/v2/v3…，每版各自留底，原版不删）。Van 过后主编出**派工单**转下一阶段——表中「上游」指内容来源，实际每段的直接上游都是主编派工单（内含上游交付物的完整 https 链接）。任务启动同理：01-采集 也由主编出首个派工单（含当日采集要求）才开工，全流程没有「无派工单自行开工」的阶段。

## 交付物协议（所有 Agent 共用）

- **形态**：一个文件夹 = `index.md`（固定入口，正文内联）+ 附件子目录（`images/`、`source/`、`html/`、`xiaohongshu/`），相对链接引用。
- **流转**：文件夹打包成同名 zip → **一步式提交**任务流转服务（`POST /tasks/:id/deliverables` multipart）→ Agent 之间只传**完整 https 链接**（不能只写路径）。
- **命名与路径由服务端派生**（agent 不手拼、不算版本）：文件名 `用途_责任方_日期_任务ID_版本.zip`（如 `资讯日更_文案_20260604_r17_v1.zip`，`任务ID`=`r{run_id}`，同日多次任务互不覆盖；合流成品责任方=「成品」）；路径 `csw/{用途}/{日期}/{任务ID}/{阶段序号-阶段}/{zip}`（`csw/` 为存储前缀配置 `CSW_OSS_PREFIX`）。同日同角色跨阶段的产出（文案 03/07、设计师 04/08、发布员 06/10、成品 05/09）文件名相同，靠任务ID + 阶段路径 + header 的 `阶段` 字段区分，版本号在各阶段内各自从 v1 起算。引擎层 `runs` 已放宽 `(workflow,subject)` 唯一约束（迁移 0004），同一天可多次触发同一工作流。
- **派工单已引擎化**：派工单是引擎里的派工记录（`editor_note` + `upstreams`，不打 zip、不占 OSS 路径）；引用写「主编 · r{run_id} 任务#{task_id} 派工单」。
- **溯源**：每个 `index.md` 顶部必带 YAML 元信息头九字段（任务 / 类型 / 交付 Agent / 阶段 / 版本 / 时间 / 上游来源 / 状态 / 自检），`上游来源` 逐条列：上游交付物写「来源方 · 完整 https 链接」，主编派工单写引擎引用（01-采集 即此写法，不写「无」）。
- **状态流转**：`待主编审核` → `待 Van 审核` → `已通过`；任一不过 → `已退回（附说明）`，被退回原因可从任务详情的 `latest_review`（方向/位置/意见）自助读取。

各阶段的产出要求、header 范式与自检清单详见规范第 8 节；主编派工单的 `index.md` 范式见规范文末附录。

## 角色工具约定（详见 `docs/作业手册_*.md`）

- **情报收集员**（01-采集）：Instagram 用营事编集室 csw MCP（`csw_posts_search` → `csw_posts_get`，无需拟人间隔）；小红书用 `opencli xiaohongshu`（必须随机 sleep 间隔防风控）；网页优先 opencli 适配器、否则 WebFetch。官方/账号情报采完整内容，社区讨论采热门内容，都要带图片和原文链接。
- **选题研究员**（02-选题）：从资讯包精选 **5 则资讯 + 排序 + 15 张配图**（图复制进 `images/`），每则资讯、每张图标注对应情报来源；**标题/摘要不在它这里**（归文案）。
- **文案**（03-公众号内容 + 07-小红书文本）：03 产出标题（≤64 字符，发布员用作公众号标题）+ 摘要（≤120 字符）+ 导读 + 正文 + 图文排版（把 02 的 15 张配图编排进正文），事实可溯回情报原文；07 对公众号成品**二次拆解**出小红书文本（笔记标题 + 正文）。
- **设计师**（04-公众号视觉 + 08-小红书视觉）：所有图必须用 `generate-images` skill 生成；04 据 03 内容做公众号封面，默认 2.35:1（1920×817）；08 据 07 文本做小红书封面卡 + 内容卡，默认 3:4（1242×1656）；派工单若附素材/要求则优先。
- **主编**（05-公众号成品 + 09-小红书成品合成）：合并该段两份过审交付物打包，责任方写阶段名「成品」；05 派工单注明作者名，09 派工单注明小红书发布/入草稿指示。
- **发布员**（06-公众号发布 + 10-小红书发布）：用 `mp-helper` skill 把 HTML 内图片上传公众号素材库并替换链接，再上传草稿箱（标题=文案的标题、作者按账号填写）；小红书用 `opencli xiaohongshu publish` 发布或入草稿（按主编派工单指示）；完成后**先通知主编，不直接通知 Van**。

---

# 第二部分：工作流引擎（`csw-task/`）

把第一部分的「十阶段、多角色、双闸」抽象成**通用**状态机——引擎层与具体流程无关，`daily_news`（资讯日更十阶段）只是 seed 进去的第一个工作流定义。设计文档 `docs/任务流转服务_设计.md`（§3 DDL / §5 状态机 / §6 API / §7 鉴权 / §8 护栏）是后端的权威蓝图。

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

引擎要点（`internal/engine`）：触发即**快照**工作流定义（运行实例不受此后定义热改影响）；提交后按闸推进（主编闸 → Van 闸，0 闸直通）；越级审核 409；退回必填方向+位置；合流阶段（assignee=主编）就绪时自动派给主编。所有写操作支持 `Idempotency-Key`（fail-closed：业务前持久占位 + 请求指纹绑定；2xx 后同指纹回放，非 2xx 释放占位可重试，崩溃后同键 409 需人工核实；见设计 §8），错误统一 `{code,message}` + HTTP 码。

## 前端 `csw-task-web/`：管理后台

React 18 + Vite + TS + Tailwind v3 + Radix（弹层底座），**像素级复刻** `docs/任务流转后台_设计稿/`。真连 `:8081/admin/*`：access JWT 存内存 + refresh cookie 401 自动续期重试 + RBAC 路由守卫。DAG 用自绘 SVG（移植设计稿的分层+贝塞尔+环检测，未引入 React Flow）；激活校验前端 `runChecks` 即时预览 + 后端权威（`PUT` 保存→`activate` 归档旧版，非阻断校验）。`lib/adapters.ts` 处理 API↔编辑器形态转换（deps id↔code 等）。详见 `csw-task/csw-task-web/README.md`。

## skill 通道 `skill/SKILL.md`

`csw-task` skill（**已按真实 API 定稿**）：agent 与运行面之间的唯一通道 + **编辑部群协作协议**。两层架构：**引擎是权威**（状态/文件/版本/闸），**飞书编辑部群是可见层**（Van 旁观、agent 靠 @ 唤醒）——每个流转动作 = 引擎调用 + 群播报（恒引擎先），群消息只是提示、引擎才是真相（worker 被 @ 后必先 `my-tasks` 核实，处理全部 open 任务）。核心约定：一步式 submit（multipart，命名/路径/版本服务端派生，`doc_type` 缺省=阶段 `output_type`）；幂等键从内容确定性派生（如 `submit-{task_id}-{zip sha256 前16}`）；`review --reject` 必填方向+位置，被退回从 `latest_review` 自助读原因；Van 审由主编代录（`comment:"Van：…"`）；派 07 须手加 05-成品上游；花名册 JSON（role_code→lark user_id）配置于 `CSW_TASK_ROSTER`，仅主编可 @Van。作业手册/自检/验收一律以 `task <id>` 引擎返回为准，skill 不复述标准。
