---
name: csw-task
description: 营事编集室「任务流转服务」客户端（草稿）。当 agent 需要在工作流里干活时使用：触发工作流实例、查询自己的任务与进度、下载上游交付物、上传并提交产出；以及（中枢/主编）派工、审核、退回、看派工/审核队列。封装服务的 HTTP API + 每 agent 鉴权 + 文件上传/下载，agent 不必直接拼 HTTP。触发词：触发流程、开批次、我的任务、提交产出、派工、审核、退回、任务进度、CSW 任务流转。
---

# csw-task · 任务流转服务客户端（草稿）

> 状态：**设计草稿**，待服务（`csw-task-svc`）实现后定稿。命令契约依据《任务流转服务 · 设计文档》§6 / §6.2。
> 本 skill 是 agent 与任务流转服务之间的唯一通道——和 `lark-im`、`opencli`、`mp-helper` 同性质，封装底层 API。**不要自己拼 HTTP / curl 调服务**，统一用本 skill。

## 1. 它解决什么

工作流的状态、文件、流转都在「任务流转服务」里。agent 通过本 skill：
- 查到「我现在该干什么」（我的任务 + 派工单 + 上游链接）；
- 下载上游交付物、上传自己的产出、提交回服务；
- 中枢（主编）额外：触发实例、派工、审核、退回。

**三条数据通道**：状态走 JSON、文件走上传/下载、**Agent 之间只传 `download_url`（链接）**（链接放在派工单 `upstreams` 里）。

## 2. 前置配置

环境变量（每个 agent 各自一份）：
- `CSW_TASK_BASE_URL`：服务地址，如 `https://csw-task.internal`
- `CSW_TASK_TOKEN`：本 agent 的 bearer token（决定你的角色与权限；不要外泄）

skill 自动处理：`Authorization: Bearer`、`Idempotency-Key`（每个写命令自动生成、可 `--idem` 指定）、multipart 上传、流式下载、JSON 解析、5xx 重试。

输出：默认人类可读；加 `--json` 输出原始 JSON 供脚本/agent 解析。

## 3. 命令总览（命令 ↔ 端点）

| 命令 | 谁用 | 端点 |
|---|---|---|
| `csw-task trigger <wf> --subject <s> [--inputs <json/文本>]` | 中枢/调度器 | POST `/workflows/{wf}/runs` |
| `csw-task workflows` | 中枢/调度器 | GET `/workflows`（列已激活类型） |
| `csw-task inbox` | 中枢 | GET `/me/inbox` |
| `csw-task dispatch <task> --note <s> [--upstream label=url ...]` | 中枢 | POST `/tasks/{id}/dispatch` |
| `csw-task review <deliverable> --pass` / `--reject --direction <s> --location <s>` | 中枢 | POST `/deliverables/{id}/reviews` |
| `csw-task my-tasks [--status open]` | 各 agent | GET `/me/tasks` |
| `csw-task task <id>` | 各 agent | GET `/tasks/{id}` |
| `csw-task fetch <download_url | file_id> [-o <path>]` | 各 agent | GET `/files/{id}` |
| `csw-task upload <path>` | 各 agent | POST `/files` |
| `csw-task submit <task> --type <类型> --file <file_id> [--meta <json>] [--self-check <s>] [--upstream label=url ...]` | 各 agent | POST `/tasks/{id}/deliverables` |
| `csw-task run <id>` · `csw-task timeline <id>` | 各 agent | GET `/runs/{id}`[`/timeline`] |

> 没有任何「建/改工作流」命令——工作流定义只在管理后台（`adminctl`），本 skill 不碰。

## 4. 按角色的典型流程

### 非管理角色（情报收集员 / 选题研究员 / 文案 / 设计师 / 发布员）
```bash
csw-task my-tasks                       # 1. 看我的任务，拿 task_id + 派工单(含上游 url)
csw-task task 101                        #    看派工单详情：主编意见、下一步、upstreams
csw-task fetch <上游url> -o up.zip       # 2. 下上游交付物（每条 upstream 都下）
# …干活，产出 zip…
csw-task upload 资讯日更_文案_20260609_v1.zip   # 3. 上传 → 得 file_id + download_url
csw-task submit 101 --type 文章 --file 5001 \
  --self-check "三段结构✓ 事实核对✓ 配图对应✓"      # 4. 提交 → task 进入 review
# 等主编/Van 审；被退回则按 review 的 direction/location 改，重新 upload+submit（版本自动 +1）
```

### 中枢（主编）
```bash
# 起跑（按人类指示触发当日实例）
csw-task trigger daily_news --subject 2026-06-09 --inputs "采集6/9露营装备上新"

csw-task inbox                           # 看三件事：派工队列 / 审核队列 / 我的自产任务
csw-task dispatch 102 --note "选5则+排序+15图"        # 派工（upstreams 自动预填，可 --upstream 增改）
csw-task review 1002 --pass                          # 主编审通过 → 转 Van 闸
csw-task review 1002 --pass --comment "Van：通过"     # 代录 Van 审通过 → 该阶段过、下游就绪
csw-task review 1004 --reject \
  --direction "补足到15张" --location "第3则仅2张"    # 退回（必填方向+位置）
```

### 调度器（服务器侧）
```bash
csw-task trigger daily_news --subject "$(date +%F)" --inputs "每日例行采集"
```

## 5. 关键约定与注意

- **只传链接**：把产出交给下游＝主编 `dispatch` 时把上游 `download_url` 放进派工单；下游用 `fetch` 取。不要把文件本体塞进消息。
- **上游自动预填**：`dispatch` 不带 `--upstream` 时，引擎自动填依赖阶段已通过产出的链接；带了 `--upstream` 则以你给的为准（用于手加内容上游，如 07 补 05-成品）。
- **退回必填**：`review --reject` 必须带 `--direction`（改什么方向）和 `--location`（具体位置）；主编不代改。
- **版本与闸重置**：被退回后重新 `submit` 即 v+1，审核从第一道闸重走；原派工单不必重发。
- **合流阶段自动派**：05/09 这类 assignee=主编的合流阶段，就绪时引擎自动派给主编自己（`inbox` 的「自产任务」里出现），主编直接 `submit` 成品即可。
- **提交守卫**：task 不在 dispatched/returned/in_progress 时 `submit` 会被拒（409）；说明还没轮到你或正在审。
- **幂等**：写命令可重试，重复不会重复派工/提交（自动 Idempotency-Key）。

## 6. 错误码

| HTTP | 含义 | 处置 |
|---|---|---|
| 401 | token 无效/缺失 | 检查 `CSW_TASK_TOKEN` |
| 403 | 权限不足（如 worker 触发、非中枢派工） | 该动作不归你；走对的角色 |
| 404 | task/deliverable/run 不存在 | 核对 id |
| 409 | 状态冲突（越闸、提交守卫、版本竞争） | 先 `task <id>` 看当前状态再操作 |
| 5xx | 服务端错误 | skill 自动重试；持续失败上报 |

## 7. 待实现项（草稿遗留）

- 服务 `csw-task-svc` 尚未实现；本 skill 命令为契约草稿，随服务定稿。
- `--inputs` 如何映射到入口阶段派工单内容，待与服务对齐。
- 鉴权/配置载入细节（token 来源、多 run 上下文）待实现时固化。

> 完整模型见《任务流转服务 · 设计文档》；流程语义见《资讯日更 · 全流程 / 交付与流转规范 / 验收标准》。
