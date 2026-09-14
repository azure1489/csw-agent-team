---
name: csw-task
description: 营事编集室「任务流转服务」客户端 + 编辑部协作协议。当 agent 需要在工作流里干活时使用：被群消息 @ 唤醒后查任务、下载上游交付物、干活并一步提交产出；（主编）触发实例、派工、审核、退回、代录 Van 审，并按协议在飞书编辑部群播报。封装运行面 HTTP API（bearer 鉴权 / 幂等 / 一步式上传提交），agent 不自己拼 HTTP。触发词：触发流程、开批次、我的任务、提交产出、派工、审核、退回、任务进度、编辑部群、CSW 任务流转。
---

# csw-task · 任务流转服务客户端 + 编辑部协作协议

> 服务端 `csw-task-svc` 已实现（运行面 `/api/v1/*`），本文档按真实 API 定稿。
> **两层架构**：**引擎是权威**（状态/文件/版本/闸都在引擎里），**飞书编辑部群是可见层**（人类 Van 全程旁观、随时插话；agent 之间靠群消息 @ 唤醒）。每个流转动作 = 引擎调用 + 群播报，缺一不可。
> 本 skill 是 agent 与引擎之间的唯一通道（同 `lark-im`/`opencli`/`mp-helper` 性质）。**不要绕开本文档的模板自己拼 HTTP**。群消息收发用 `lark-im` / `lark-event` skill，本文档只定协议（§5）。

## 0. 标准从哪里来（最重要的一条）

**你这阶段「做什么、自检什么、按什么验收」不在本 skill 里，在引擎里**——`task <id>` 返回作业手册（instructions）、自检标准（self_check_criteria）、验收标准（acceptance），它们随工作流定义 seed/热改，永远以引擎返回为准。本 skill 只管：怎么调用引擎、怎么交东西、怎么在群里说话。作业手册点名的专业工具（`generate-images` / `mp-helper` / `opencli xiaohongshu` / csw MCP …）按它说的去用对应 skill。

## 1. 前置配置（每个 agent 各一份）

| 环境变量 | 含义 |
|---|---|
| `CSW_TASK_BASE_URL` | 服务地址，如 `http://localhost:8080`（下文 `$BASE`） |
| `CSW_TASK_TOKEN` | 本 agent 的 bearer token（**token 即角色与权限**，不要外泄；下文 `$TOKEN`） |
| `CSW_TASK_ROSTER` | 花名册**离线兜底文件**路径（§5.1）；正常走 `GET /api/v1/roster`，此文件仅 API 不可达时降级用 |

通用调用头：`-H "Authorization: Bearer $TOKEN"`。写操作必须带确定性幂等键（§6）。错误统一 `{code,message}`（§7）。

## 2. 命令 ↔ 端点（按真实 API）

| 动作 | 谁用 | 端点 |
|---|---|---|
| trigger | 主编/调度器 | `POST /api/v1/workflows/{key}/runs` `{subject,title?,inputs?}` |
| workflows | 主编/调度器 | `GET /api/v1/workflows` |
| inbox | 主编 | `GET /api/v1/me/inbox` → `{dispatch_queue, review_queue, my_tasks}` |
| dispatch | 主编 | `POST /api/v1/tasks/{id}/dispatch` `{editor_note, upstreams?}` |
| review | 主编 | `POST /api/v1/deliverables/{id}/reviews` `{verdict, comment?, return_direction?, return_location?}` |
| my-tasks | 各 agent | `GET /api/v1/me/tasks?status=open\|all` |
| task | 各 agent | `GET /api/v1/tasks/{id}` → 作业手册+自检+验收+派工单(dispatch)+各版本产出(含 latest_review)+闸 |
| fetch | 各 agent | 见 §2.1（双后端） |
| **submit（一步式）** | 各 agent | `POST /api/v1/tasks/{id}/deliverables`（**multipart**）——见 §2.2 |
| run / timeline | 各 agent | `GET /api/v1/runs/{id}` / `…/timeline` |
| **roster（花名册）** | 各 agent | `GET /api/v1/roster[?chat_id=]` → `{chat_id, roster{role_code:{open_id,name,bot}}, reserved_bots{}}`（群播报查 open_id 用，见 §5.1） |

> 没有任何「建/改工作流」动作——定义只在管理后台，本 skill 不碰。

### 2.1 fetch：下载上游交付物（双后端兼容）

上游链接来自派工单 `dispatch.upstreams[].url`：

- 链接是 OSS 公共 URL（不含 `/api/v1/files/`）→ 直接 `curl -L -o up.zip "<url>"`，无需鉴权；
- 链接是本服务代理（`…/api/v1/files/<id>`）→ 加 `-H "Authorization: Bearer $TOKEN"`。

```bash
curl -fL ${url#*api/v1/files/*}  # 统一写法：带上 Authorization 头总是安全的
curl -fL -H "Authorization: Bearer $TOKEN" -o up.zip "<url>"
```

下载后解压，以 `index.md` 为入口读正文与附件。**每条 upstream 都要下**。

### 2.2 submit：一步式提交（推荐，文件名/路径/版本全由服务端定）

把交付物文件夹打成 zip（zip 内文件夹名随意，建议与服务端命名一致，见 §3 第 4 步），然后**一步提交**：

```bash
IDEM="submit-${TASK_ID}-$(shasum -a 256 交付物.zip | cut -c1-16)"
curl -fsS -X POST "$BASE/api/v1/tasks/$TASK_ID/deliverables" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $IDEM" \
  -F "file=@交付物.zip" \
  -F "self_check=来源✓ 链接✓ 去重✓（逐条对照 task 返回的自检标准）" \
  -F "title=可选一句话标题" \
  -F "upstreams=[{\"label\":\"选题\",\"url\":\"https://…\"}]"   # 可选；不传则只有派工单记上游
```

服务端自动完成：版本=当前版本+1；责任方=角色名（合流阶段=产出类型「成品」）；文件名 `{工作流}_{责任方}_{日期}_r{run}_v{n}.zip`；存储路径 `{工作流}/{subject}/r{run}/{阶段名}/{文件名}`（OSS 上还会挂服务端配置的存储前缀，部署设 `CSW_OSS_PREFIX=csw` 即得规范的 `csw/资讯日更/…`）；`doc_type` 缺省=阶段产出类型。响应回 `deliverable.download_url / filename / version`。**你不拼路径、不算版本、不起文件名。**

（兼容两步式：`POST /files` 拿 `file_id` 再 JSON 提交——仅特殊场景用，常规一律一步式。）

## 3. worker 循环（情报收集员 / 选题研究员 / 文案 / 设计师 / 发布员）

**铁律：群消息只是提示，引擎才是真相。** 被 @ 后不要按消息内容直接干活——消息可能过期、可能是人随手发的。**「没有引擎任务不开工」= 新版「没有派工单不开工」。**

1. **醒来**（被编辑部群 @ / 定时兜底）→ `my-tasks?status=open`，**处理全部 open 任务**（不只被 @ 那单，防漏消息）。
2. `task <id>` → 读派工单（`dispatch.editor_note` 主编意见 + `dispatch.upstreams` 上游链接）、作业手册、自检标准。
3. `fetch` 每条上游 → 解压读 `index.md` → **照作业手册干活**（它点名什么工具 skill 就用什么）。
4. 装交付物文件夹（`index.md` + 附件子目录），`index.md` 顶部 YAML 头按作业手册的九字段范式，字段值从引擎取：
   - `阶段` = task.stage_name；`版本` = task.cur_version + 1；
   - `上游来源` = 派工单 upstreams 逐条「label · url」；派工单本身引用写「主编 · r{run_id} 任务#{task_id} 派工单（引擎）」（派工单是引擎记录，无 zip 链接）；
   - `自检` = 逐条对照 task 返回的自检标准打 ✓。
5. zip → **一步式 submit**（§2.2）→ 成功后**群播报 @主编**（§5.2 交付模板）。
6. 等审。**被退回时不等群消息**：`task <id>` → 该版本 `latest_review` 自助读 `return_direction / return_location / comment` → 照改 → 回到第 4 步重新提交（版本自动 +1，审核从第一道闸重走）。

> 发布员补充：作业手册要求「完成先通知主编、不要直接通知 Van」——本协议下即第 5 步的交付播报，**worker 任何时候都不 @Van**（§5.3 护栏）。

## 4. 主编编排（中枢）

主编**绝不亲自编辑内容**；所有判断对照引擎返回的验收标准。**每个动作 = 引擎调用成功后立即群播报**（§5）。

### 4.1 起跑（每日任务启动）

Van（人类）在群里把当日任务交给主编 → 主编：

```bash
# ① 触发实例（subject=日期；inputs 仅作 run 事件留痕）
curl … -X POST "$BASE/api/v1/workflows/daily_news/runs" -d '{"subject":"2026-06-12","inputs":{"采集要求":"…"}}'
#    响应是扁平结构：{"run_id":17,"subject":"…","status":"active","tasks":[{id,seq,stage_name,status,…}]}
# ② 串联首单：tasks 里找 seq=1 且 status=ready 的任务，立即派工，当日采集要求写进 editor_note
curl … -X POST "$BASE/api/v1/tasks/<首任务id>/dispatch" -d '{"editor_note":"今天采集：…（范围/对象/平台/指定链接）"}'
# ③ 群播报 @情报收集员（§5.2 派工模板）
```

> trigger 的 `inputs` **不会**自动变成首个派工单——②③ 不能省。

### 4.2 审核循环（双闸）

```text
inbox（review_queue 拿 deliverable_id + task_id + 下一闸信息）
 → task <task_id>（拿验收标准 acceptance + 该版本 download_url + 自检申报）
 → fetch 解压，对照验收标准逐条核
 → 判定：
   ├─ 不过 → review reject（必填 direction+location）→ 群播报 @该角色（退回模板）
   ├─ 过（gate1，主编审）→ review pass → 此时下一闸是 Van 闸（relayed_by_hub）→ 群播报 @Van 转审（转审模板，附链接）
   └─ Van 在群里给结论 → 代录 gate2：
        过：review pass --comment "Van：通过"（末闸过即下游就绪）
        不过：review reject --direction/--location（转述 Van 的方向与位置）
 → 末闸过后：inbox 看 dispatch_queue（新就绪任务）→ dispatch 下一棒 + 群播报派工
```

```bash
# reject 示例（reject 必带方向+位置，引擎强制）
IDEM="review-${DELIV_ID}-reject-v${VER}"
curl … -X POST "$BASE/api/v1/deliverables/$DELIV_ID/reviews" -H "Idempotency-Key: $IDEM" \
  -d '{"verdict":"reject","return_direction":"补足到15张","return_location":"第3则仅2张"}'
```

### 4.3 两个特殊点（容易忘，写死成检查单）

- **派 07-小红书文本时必须手加内容上游**：07 的 DAG 依赖是 06（发布物），但内容源是 **05-公众号成品**。dispatch 时必带 `upstreams:[{"label":"公众号成品","url":"<05 的 download_url>"}]`（可再附 06）。漏了文案就会拿错包。
- **05 / 09 合成是你的自产任务**（就绪即自动派给你，inbox 的 `my_tasks` 出现）：fetch 03+04（或 07+08）两份已过审最新版 → 合并成一个文件夹（`index.md` 九字段头：责任方=成品；05 注明**公众号作者名**、09 注明**发布还是入草稿**；上游来源两条链接；附主编意见与下一步）→ 一步式 submit（服务端自动命名 `…_成品_…`）→ 它同样走双闸。

## 5. 编辑部群协作协议（lark）

**双写纪律：恒引擎先、群播报后。** 引擎失败则不发消息（什么都没发生）；引擎成功而发消息失败 → 重试一次，仍失败则在产出/备注里记「通知未达，请补发」。worker 监听被 @ 用 `lark-event`。

**群发送硬性规范（不这样发，@ 不生效、对方 agent 不会被唤醒）：**

1. **直接发新消息**到群（独立消息，不要回复 / 对话线程形式）。
2. **真 mention**：@ 人必须用 at-tag `<at user_id="{open_id}">{名称}</at>`，`user_id` 填花名册（`GET /api/v1/roster`）里该角色的 `open_id`。
3. **发送方式固定**：`lark-cli im +messages-send --chat-id <chat_id> --msg-type text --content '<json>'`，content 形如 `{"text":"<at user_id=\"ou_xxx\">名称</at> 正文…"}`（一条消息可串多个 at-tag）。
4. **禁止**：纯文本「@名字」、Markdown 文本里的 @、普通 `send_message`/markdown 发送——这些只是字符串，飞书不会真正提及，对方收不到 @、不会被唤醒（曾出现「口头说已 @、实际没真 @」就是这原因）。

```bash
# 群内 @ 示例：chat_id 与 open_id 都取自 GET /api/v1/roster；可一条带多个 at-tag
lark-cli im +messages-send --chat-id oc_xxx --msg-type text \
  --content '{"text":"<at user_id=\"ou_xxx\">情报收集员</at> 【r17·01-采集·任务#42】新派工：今天采集…，详情见引擎任务 #42"}'
```

### 5.1 花名册

**花名册以引擎为权威，实时拉取**——开工/播报前 `curl -fsS -H "Authorization: Bearer $TOKEN" "$BASE/api/v1/roster"`（结构见 §2 命令表），后台改人/换 open_id **即时生效、无需重新部署**。获取顺序（降级链）：

1. `GET /api/v1/roster` 返回 200 → 用它；
2. API 不可达/5xx 且 `$CSW_TASK_ROSTER` 指向的离线兜底文件存在 → 读该文件；
3. 都没有 → 该角色 @ 不到人时**回退纯文本提示并报主编**，不静默跳过。

键 = 引擎 `role_code`（taskDTO / inbox 都带 role_code，拿到即查该 @ 谁），值含 `open_id`——@ 时填到 `<at user_id="<open_id>">名字</at>`（lark 的 at 属性名叫 user_id，值用 open_id）。`reserved_bots`（群里有但未接入工作流的 bot）一般不 @。`skill/roster.json` 现为离线兜底快照（与 DB 同源，由 deploy 下发）；人类副本与映射现状见 `docs/CSW编辑部_通讯录.md`，后台可视化增删改见管理后台「通讯录」页。

当前映射（七个角色已全部对齐，花名册闭环）：

| role_code | 群 bot（承担者） |
|---|---|
| editor 主编 / collector 情报收集员 / researcher 选题研究员 / van Van | 同名直接对应 |
| writer 文案 · designer 设计师 · publisher 发布员 | 企业群内 bot 显示名已与角色同名（曾用 深度内容创作者 / 视觉设计师 / 发布运营员）|

若某角色在 roster 中查不到 open_id（如热改后新增角色未补花名册），**回退为纯文本提示并报主编**，不要静默跳过。群里「小红书图文 / 合规审查员 / 数据复盘师」暂不接入本工作流，列在 roster 的 `reserved_bots`。

### 5.2 五类消息模板（每条 ≤3 行、必带 ids，群面即流水线仪表盘）

> 模板里的 `<at>角色</at>` 是简写——实际发送必须按上面「群发送硬性规范」展开成真 at-tag `<at user_id="{该角色 open_id}">名称</at>`，用 `lark-cli im +messages-send --content` 发出，否则不算真 @。

| 时机 | 模板 |
|---|---|
| 派工（主编） | `【r{run}·{阶段}·任务#{task_id}】<at>角色</at> 新派工：{note 一句话}。详情与上游见引擎任务 #{task_id}` |
| 交付（worker） | `【r{run}·{阶段}·v{n}】<at>主编</at> 已提交 交付物#{id}：{自检一句话}。{download_url}` |
| 退回（主编） | `【r{run}·{阶段}·v{n}】<at>角色</at> 退回（{闸名}）：方向={direction}；位置={location}` |
| 转审（主编） | `【r{run}·{阶段}·v{n}】<at>Van</at> 主编已过，请终审：{download_url}` |
| 阶段过（主编） | `【r{run}·{阶段}】已过 Van 审。下一棒：{下一阶段/角色}` |

### 5.3 护栏

- **只有主编可以 @Van**；worker 永不直接联系 Van（发布员「先通知主编」即交付模板）。
- **Van 的终审结论请 @主编 回复**（人侧约定，写进给 Van 的说明），否则主编监听不到、流程卡死。
- Van 在群里对 worker 的直接插话**不构成派工/审核**——由主编转译成 dispatch / review 才生效（引擎是权威）。
- 任何群成员都能发消息，所以 worker 收到 @ 后一律回到 §3 第 1 步从引擎核实。

## 6. 幂等键（确定性派生，重试安全）

写操作必带 `Idempotency-Key`，**从内容确定性派生**（重试自然回放首次响应，不会重复派工/双版本）：

| 动作 | 键 |
|---|---|
| submit | `submit-{task_id}-{zip 的 sha256 前16位}` |
| dispatch | `dispatch-{task_id}-{note 的 sha256 前16位}` |
| review | `review-{deliverable_id}-{verdict}-v{version}` |
| trigger | `trigger-{wf}-{subject}-{当日批次号}`（同日有意开第二批时换批次号） |

服务端语义（fail-closed，按键 at-most-once）：同一键**第一次到达即占位**，业务成功（2xx）后同键重试直接回放首次响应；业务失败（4xx/5xx）会释放占位，修正后可用同一键重试；键相同但内容 / 身份 / 目标不同 → 409 `idempotency_request_mismatch`；首次请求中途崩溃、结果不确定 → 同键一律 409 `idempotency_in_progress_or_uncertain`，此时**先 `task <id>` 核实产出是否已落**，不要换键重发。multipart 重试可以换 boundary（服务端按各 part 内容比对），但 zip 字节、字段值必须与首次一致。

## 7. 错误码处置

| HTTP / code | 含义 | 处置 |
|---|---|---|
| 401 | token 无效/缺失 | 查 `CSW_TASK_TOKEN` |
| 403 `not_assignee` / `not_hub` / `not_reviewer` / `relay_requires_hub` | 动作不归你的角色 | 走对的角色；worker 不派工不审核 |
| 404 | task/deliverable/run/file 不存在 | 核对 id |
| 409 `cannot_submit` | 任务不在 dispatched/returned/in_progress | 还没轮到你或正在审：`task <id>` 看状态 |
| 409 `task_not_ready` | 派工对象不是 ready | 上游还没全过，看 `run <id>` |
| 409 `stale_version` | 审的不是当前版本 | 重新 `task <id>` 取最新版本的 deliverable |
| 409 `no_gate` | 越闸或已末闸 | `task <id>` 看 cur_gate 与 gates |
| 400 `return_required` | reject 缺方向/位置 | 补 `return_direction`+`return_location` |
| 400 `doc_type_required` | doc_type 与阶段产出类型都空 | 显式传 `-F doc_type=…` |
| 409 `idempotency_request_mismatch` | 同一幂等键被用在了不同内容 / 身份 / 目标上 | 检查键的派生是否漏了变量；确认是新请求就换键 |
| 409 `idempotency_in_progress_or_uncertain` | 同键首次请求仍在处理或结果不确定 | `task <id>` 核实产出是否已落；已落则不再重发，未落报主编 |
| 413 `idempotency_body_too_large` | 带键请求体超过上传上限 | 检查 zip 大小，超限拆分附件 |
| 503 `idempotency_store_unavailable` | 幂等存储不可用，业务未执行 | 稍后带同一键重试 |
| 5xx | 服务端错误 | 带同一幂等键重试（非 2xx 会自动释放占位）；持续失败报主编 |

> 完整模型见《任务流转服务 · 设计文档》；流程语义见《资讯日更 · 全流程 / 交付与流转规范 / 验收标准》（已 seed 进引擎，运行期以 `task <id>` 返回为准）。
