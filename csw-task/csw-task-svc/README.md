# csw-task-svc · 任务流转服务（通用工作流引擎后端）

Go + SQLite 实现的**通用工作流引擎**运行面服务：把「任意多阶段、多角色、带审核闸的协作流程」做成可编程状态机，供各 Agent（= 各 skill）经 HTTP 调用。`资讯日更` 十阶段是 seed 进去的第一个工作流。

> 设计依据：`../../任务流转服务_设计.md`（§3 DDL / §5 状态机 / §6 API / §6.1 后台 / §7 鉴权 / §8 护栏 / §9 选型）+ `../../管理后台_功能与线框.md`（后台 §1–§14）。
> 本目录是 Monorepo `csw-task/` 下的 Go 后端；前端 `csw-task-web/` 见「实现轮次」。

## 实现轮次

**第一轮 · 运行面（已完成）**
- ✅ `cmd/server`：运行面 HTTP API（§6 全部端点），bearer token 鉴权
- ✅ `internal/engine`：触发→快照、就绪重算、派工（含上游自动预填）、合流自动派、提交（守卫+版本+0 闸直通）、按闸审核（越级 409 / 人工闸代录 / 退回必填方向位置）
- ✅ 全表迁移（20 表 + 约束硬化）+ `daily_news` 定义 seed（10 阶段内嵌作业手册/自检/验收）
- ✅ `cmd/adminctl`：迁移 / agent·token 引导 / 工作流校验

**第二轮 · 管理后台 API（已完成）**
- ✅ `cmd/adminsrv`：`/admin/*` JSON API（§6.1 + 后台 §1–§14），与运行面**分进程、分凭证**
- ✅ JWT 登录（access HS256 内存 + refresh httpOnly cookie 轮换）+ argon2id 密码 + RBAC（superadmin/operator/viewer）+ CORS（AllowCredentials）
- ✅ 工作流定义 CRUD / 整份 PUT / 校验 / 激活（归档旧版）/ 克隆；角色·成员·token 管理；后台用户（防自锁/保 ≥1 superadmin）；只读监控；审计；概览；设置（只读）
  - 整份 PUT 支持 **draft 与 active**（active 为「就地热改」，仅影响此后新触发的实例——运行实例在 Trigger 时已快照定义，不受改动影响）；归档版本只读（`archived_readonly`）。active 保存后随响应回带一次 `workflow.Validate` 报告（`{ok,validation:{all_ok,checks}}`），**不阻断保存**——未过仅提示「触发前请修复」，由人决定。
- ✅ `adminctl user create/passwd/list`：离线引导首个 superadmin

**第三轮 · 前端（已完成）**
- ✅ `../csw-task-web/`：React 18 + Vite + TS + Tailwind v3 + Radix（弹层底座），像素级复刻 `../../任务流转后台_设计稿/`
- ✅ 全部 10 页面 + 工作流编辑器（5 段 + StageDrawer + DAG 自绘 SVG + 校验/激活）+ JWT 鉴权（access 内存 + refresh cookie，401 自动续期重试）+ RBAC 路由守卫 + 真连 `/admin/*`（字段适配 deps id↔code 等）
- 启动：`cd ../csw-task-web && pnpm install && pnpm dev`（:5173，后端 CORS 已预配该 origin）；详见该目录 `README.md`

## 技术栈

`gin-gonic/gin` · `modernc.org/sqlite`（纯 Go 免 cgo）· `pressly/goose/v3`（迁移，embed）· `google/uuid` · `slog`。
并发：WAL + `busy_timeout` + `foreign_keys` + `synchronous=NORMAL` + `SetMaxOpenConns(1)`（写串行）。

## 目录（golang-standards/project-layout）

```
cmd/{server,adminsrv,adminctl}/    三入口：运行面 / 管理后台 / CLI
internal/
  config/                          env 配置（运行面 + JWT/CORS/后台共用）
  app/{server,adminsrv,adminctl}/  gin 装配 + handlers / CLI
  domain/                          实体 + 状态枚举 + 纯规则（含 admin 实体，无框架依赖）
  engine/                          工作流引擎（触发/就绪/派工/提交/审核，单事务）
  workflow/                        定义校验（DAG/入口/中枢/闸/三段非空）
  store/sqlite/                    连接层 + 仓储（repo_def/run/deliv/admin）+ migrations/*.sql（embed）
  files/                           BlobStore 接口 + 本地内容寻址实现
  auth/                            bearer token + argon2id 密码 + JWT 签发/校验
  middleware/                      requestid/logger/recovery/agentauth/idempotency · jwtauth/rbac/cors
```

## 构建与运行

```bash
make build            # 产出 bin/server bin/adminsrv bin/adminctl
make test             # go test ./...

# 初始化 + 引导
./bin/adminctl migrate up
./bin/adminctl workflow validate daily_news
./bin/adminctl token issue editor                                  # 运行面 agent token（明文仅一次）
./bin/adminctl user create admin --role superadmin --password ***  # 首个后台用户（argon2id）

# 启动两个进程（共享同一 sqlite，分凭证）
CSW_ADDR=:8080 ./bin/server                                        # 运行面（bearer）
CSW_JWT_SECRET=*** CSW_ADMIN_ADDR=:8081 ./bin/adminsrv             # 管理后台（JWT）
```

配置（优先级 **env > `config.yaml` > 内置默认**；把 `configs/config.example.yaml` 复制为 `config.yaml` 即生效，env 仍可覆盖，`CSW_CONFIG` 可指定文件路径。完整字段 + prod / docker-compose 示例见该文件）：
- 通用：`CSW_DATA_DIR` `CSW_DB_PATH` `CSW_BLOB_DIR`
- 运行面：`CSW_ADDR` `CSW_BASE_URL` `CSW_MAX_UPLOAD_BYTES` `CSW_ALLOWED_CONTENT_TYPES`
- 后台：`CSW_ADMIN_ADDR` `CSW_JWT_SECRET`（未设则启动生成临时密钥+warn）`CSW_JWT_ACCESS_TTL`(15m) `CSW_JWT_REFRESH_TTL`(168h) `CSW_ADMIN_CORS_ORIGIN`(默认 http://localhost:5173) `CSW_COOKIE_SECURE`(生产 true)
- 文件存储：`CSW_BLOB_BACKEND`(local|oss，默认 local)；oss 时 `CSW_OSS_ENDPOINT` `CSW_OSS_BUCKET` `CSW_OSS_ACCESS_KEY_ID` `CSW_OSS_ACCESS_KEY_SECRET` `CSW_OSS_PREFIX`(默认 blobs；prefix=本服务在 bucket 内的根目录，语义路径与内容寻址都落它之下——设 `csw` 即得 `csw/资讯日更/…`)。OSS 对象以 public-read 上传，`download_url` 直接是永久公共 URL（无需签名，前端 / agent 均可直接下载），本地后端则 `download_url=/files/:id` 服务端代理 + Range。**语义路径由服务端派生**（见下「提交产出」）：`{工作流名}/{subject}/r{run_id}/{阶段名}/{工作流名}_{责任方}_{subject 压缩}_r{run_id}_v{n}.zip`，每版独立留底；`POST /files` 的可选 `object_key` 字段保留作通用/兼容用途，留空则内容寻址（`<prefix>/<sha 分片>`，同内容去重）。命名模板只用引擎自身数据（工作流名/subject/run/阶段/角色/版本），引擎保持通用。

运行时数据落 `CSW_DATA_DIR`（sqlite；local 后端 blob 落 `blobs/`，oss 后端进对象存储），已 `.gitignore`。

## 运行面 API（`/api/v1`，均需 `Authorization: Bearer <token>`）

| 方法 | 路径 | 用途 |
|---|---|---|
| GET | `/me/tasks?status=open\|all` | 我的任务 |
| GET | `/tasks/:id` | 任务详情：作业手册+自检+验收+派工单+各版本产出（含 `latest_review`：最新审核的 verdict/方向/位置/comment——被退回后自助读原因，不依赖通知原文）+闸；task 带 `output_type` |
| GET | `/runs/:id` · `/runs/:id/timeline` | 流程进度 / 事件流水（`gate_passed`/`gate_returned` 事件 detail 含审核意见与退回方向/位置） |
| POST | `/files`（multipart `file` [+ `object_key`]）· GET `/files/:id` | 通用上传（内容寻址去重 / 或按 `object_key` 语义路径）/ 下载。交付物提交不必再走这里——用下行一步式 |
| POST | `/tasks/:id/deliverables` | 提交产出（version 服务端自增）。**multipart 一步式（推荐）**：`file` + `self_check`/`title`/`summary`/`meta_json`/`upstreams`(JSON)，文件名与存储路径由服务端按 工作流/subject/run/阶段/版本/责任方 派生（合流阶段责任方=产出类型「成品」），agent 不拼路径不算版本；`doc_type` 可省（缺省=阶段 `output_type`）。JSON 两步式（先 `/files` 拿 `file_id`）仍兼容 |
| GET | `/workflows` · POST `/workflows/:key/runs` | 列 active / 触发实例 |
| GET | `/me/inbox` | 中枢：派工队列+审核队列+我的合流任务 |
| POST | `/tasks/:id/dispatch` | 中枢派工（上游自动预填，可覆盖） |
| POST | `/deliverables/:id/reviews` | 审核（pass/reject，按闸推进，人工闸代录） |

写操作支持 `Idempotency-Key`（重复请求回放首次响应）。错误统一 `{code,message}` + HTTP 码（400/401/403/404/409）。

agent 不直接拼 HTTP，经配套 `csw-task` skill（见 `../SKILL.md`）调用。

## 管理后台 API（`/admin/*`，JWT；人类操作员）

| 分组 | 接口 |
|---|---|
| 认证 | `POST /login`·`/refresh`(cookie)·`/logout` · `GET /me` |
| 概览 | `GET /overview` |
| 工作流 | `GET /workflows`·`/workflows/:id` · `POST /workflows`·`.../clone` · `PUT /workflows/:id` · `POST .../validate`·`.../activate`·`.../archive` |
| 角色/成员 | `GET/POST /roles`·`PATCH /roles/:code` · `GET/POST /agents`·`PATCH /agents/:id` · `POST /agents/:id/tokens`·`.../:tid/revoke` |
| 后台用户 | `GET/POST /users`·`PATCH /users/:id`·`POST /users/:id/reset-password`（superadmin） |
| 监控 | `GET /runs`·`/runs/:id`·`/runs/:id/timeline`·`/tasks/:id`·`/deliverables/:id`（只读） |
| 审计/设置 | `GET /audit` · `GET/PUT /settings`（PUT 只读回显） |

鉴权：access JWT（HS256，前端存内存）+ refresh（httpOnly cookie，刷新轮换）。RBAC：写=operator+，后台用户/设置=superadmin，读=viewer+。前后端分离经 CORS（`AllowCredentials`）。后台**不操作流程**（派工/审核/提交属运行面），监控只读。所有写操作落 `admin_audit`。

## 验证

`go test ./...` 含：迁移冒烟 + seed 完整性 + admin 用户/refresh/激活归档旧版（`internal/store/sqlite`）；引擎全流程 + 鉴权护栏（`internal/engine`）；argon2id 往返 + JWT 签发/解析/过期（`internal/auth`）。HTTP 层经 `curl` 端到端：
- 运行面：触发/派工/上传下载/提交/双闸/inbox/幂等/401·403·409。
- 后台：登录/refresh 轮换/logout、工作流 clone→校验→激活归档、整份 PUT、成员 token **跨进程交叉验证**（adminsrv 签发→运行面可用→吊销即失效）、RBAC（operator→/users 403）、防自锁、只读监控、审计、CORS 预检。
