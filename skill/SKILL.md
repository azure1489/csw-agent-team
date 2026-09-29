---
name: csw-task
version: 3.11.0
description: 营事编集室「任务流转服务」客户端 + 编辑部协作协议（v3：十三阶段、引擎单写群播报）。当 agent 需要在工作流里干活时使用：被群消息 @ 唤醒后查任务、接单、下载上游、干活并一步提交产出或补件；（主编）触发实例、派工、审核、定点编辑、录入授权、代录 Van 决定、取消与重开。封装运行面 HTTP API（bearer 鉴权 / 幂等 / 一步式上传提交），agent 不自己拼 HTTP。触发词：触发流程、开批次、我的任务、接单、提交产出、补件、派工、审核、退回、定点编辑、授权、任务进度、编辑部群、CSW 任务流转。
---

# csw-task · 任务流转服务客户端 + 编辑部协作协议（v3）

> **两层架构**：**引擎是权威**（状态 / 文件 / 版本 / 闸 / 授权都在引擎里），**飞书编辑部群是可见层**（Van 全程旁观，agent 靠 @ 唤醒）。
> **v3 起群播报由引擎单写**：派工、待审、退回、阶段通过、待派工、待授权、补件、失败、逾期、未接单与无活动提醒，都由引擎的通知组件按事件自动发到群里并真 @ 对应角色。**agent 只调用引擎，不再自己发流转播报**（兜底规则见 §5）。
> 本 skill 是 agent 与引擎之间的唯一通道，**只读分发**：各 profile 不得改写本文件（包括 curator 自动整理），版本以 frontmatter `version` 为准（§0.1）。

## 0. 标准从哪里来（最重要的一条）

**你这阶段「做什么、自检什么、按什么验收」不在本 skill 里，在引擎里**——`task <id>` 返回作业手册（instructions）、自检标准（self_check_criteria）、验收标准（acceptance），随工作流定义维护，永远以引擎返回为准。本 skill 只管：怎么调用引擎、怎么交东西、群里怎么配合。作业手册点名的专业工具（`generate-images` / `mp-helper` / `opencli xiaohongshu` / csw MCP …）按它说的用对应 skill。业务约定（窗口、数量、时间表、授权默认值）见《资讯日更 · 生产约定》。

### 0.1 版本自检（每次开工前）

`my-tasks` 与 `task <id>` 的响应顶层都带 `skill_min_version`。本文件 `version` 低于它 → **不开工**：对手头任务调 `fail`（reason 写「csw-task skill 版本 X 低于要求 Y，待重新分发」），然后停手。不要自己改本文件去「对齐版本」。

## 1. 前置配置（每个 agent 各一份）

| 环境变量 | 含义 |
|---|---|
| `CSW_TASK_BASE_URL` | 引擎地址（生产：`https://tasks.aworld.ltd`；下文 `$BASE`） |
| `CSW_TASK_TOKEN` | 本 agent 的 bearer token（**token 即角色与权限**，不外泄、不借用；下文 `$TOKEN`） |
| `CSW_TASK_ROSTER` | 花名册离线兜底文件（§5.1）；正常走 `GET /api/v1/roster` |

**引擎地址只能来自 `CSW_TASK_BASE_URL`。不得构建、运行或连接任何引擎副本**——包括自己编译的 csw-task-svc、隔离演练端口、临时数据库。演练用生产引擎里只带「本地演练」授权的 run。发现地址不对：停手，报主编。

通用调用头：`-H "Authorization: Bearer $TOKEN"`。写操作带确定性幂等键（§6）。错误统一 `{code,message}`（§7）。

## 2. 命令 ↔ 端点

| 动作 | 谁用 | 端点 |
|---|---|---|
| trigger | 主编 / 调度器 | `POST /api/v1/workflows/{key}/runs` `{subject,title?,inputs?}` → `{run_id,subject,status,tasks[]}` |
| workflows | 主编 / 调度器 | `GET /api/v1/workflows` |
| inbox | 主编 | `GET /api/v1/me/inbox` → `{dispatch_queue, review_queue, failed_queue, my_tasks}` |
| dispatch | 主编 | `POST /api/v1/tasks/{id}/dispatch` `{editor_note, upstreams?}` |
| review | 主编 | `POST /api/v1/deliverables/{id}/reviews` `{verdict, comment?, return_direction?, return_location?, decision_type?, source_quote?, items?, expected_version?}` |
| authorize | 主编 | `POST /api/v1/runs/{id}/authorizations` `{scope, source_quote, expires_at?}` · `DELETE …/authorizations/{scope}` · `GET …/authorizations` |
| cancel / reopen | 主编 | `POST /api/v1/tasks/{id}/cancel` `{reason}` · `POST /api/v1/tasks/{id}/reopen` `{reason}` |
| my-tasks | 各 agent | `GET /api/v1/me/tasks?status=open\|all` |
| task | 各 agent | `GET /api/v1/tasks/{id}` → 作业手册 + 自检 + 验收 + 派工单 + 各版本产出（含 latest_review）+ supplements + 闸 + skill_min_version |
| ack | 执行者 | `POST /api/v1/tasks/{id}/ack`（接单；已接单时再调 = 心跳） |
| fail | 执行者 | `POST /api/v1/tasks/{id}/fail` `{reason}` |
| fetch | 各 agent | 见 §2.1 |
| submit | 执行者 / 主编 | `POST /api/v1/tasks/{id}/deliverables`（multipart；`kind` = output 缺省 / supplement / edit）见 §2.2、§2.3 |
| run / timeline / progress | 各 agent | `GET /api/v1/runs/{id}` · `…/timeline` · `…/progress`（每个任务的真实卡点与下一步、条目与缺口） |
| items | 参与角色 / 主编 | `GET /api/v1/runs/{id}/items` · `PUT /api/v1/runs/{id}/items` `{items:[{item_key,title,brand?,product?,source_url?,published_at?,status?,rank?,discovered_via?,fetched_at?,evidence_url?,dedup_note?,reason_code?,reason?}]}`（登记只能写 candidate 线索 / pending_check 待核 / shortlisted 成熟 / dropped 淘汰；成熟可带 rank=primary 主选 \| alt 备选；淘汰必须带 reason_code 与 reason） |
| sweeps | 参与角色 / 主编 | `PUT /api/v1/runs/{id}/sweeps` `{sweeps:[{sweep_key,platform,source_key?,tool?,query?,started_at?,ended_at?,window_from?,window_to?,found,fetched_unique,reviewed,corroborated,unreviewed,in_window,registered,result,error?,paged_to_end?}]}`（上报采集轮；按 sweep_key 幂等，同键重报即更新；result=failed 必须写 error；单次最多 200 条。**found 是接口返回数，不是审阅量**：拿到多少条记 found / fetched_unique，真正读过正文或看过图的记 reviewed，只加载未展开的记 unreviewed——未审的如实记为未审，不得事后补成 dropped。**为核实线索去读的官方页、原始出处记 corroborated**（它们算已审但不作为候选登记，判据用「已审 − 佐证」与登记数对账）。每条条目必须填 published_at，拿不到确切日期的标 pending_check 并写明缺日期，不要留空） |
| intake-trace | 参与角色 / 主编 | `GET /api/v1/runs/{id}/intake-trace`（采集轮 + 条目溯源 + 判断轨迹 + 来源覆盖；02 / 03 据此判断采集是否合理，不必解压交付物） |
| decide | 主编 | `POST /api/v1/runs/{id}/items/{item_key}/decision` `{decision, source_quote}`（approve_write / approve_research / defer / reject） |
| close | 主编 | `POST /api/v1/runs/{id}/close` `{reason}`（接受整期缺口结束 run；要求全部任务终态且**至少一个通过**） |
| abort | 主编 | `POST /api/v1/runs/{id}/abort` `{reason}`（作废 run：全部任务终态但**一个都没通过**时的收尾路径，状态落 `aborted` 而非 `done`，不计入完成统计） |
| ledger | 各 agent | `GET /api/v1/ledger/posts?since=30d&brand=&q=`（发布记录查重，含合集拆条；`verdict=not_found_in_synced_records` 只表示已同步记录里没有，**不是**「从未发布」） |
| feedback | 各 agent / 主编 | `GET /api/v1/memory/feedback?tags=brand:nanga,stage:topic`（有效编辑反馈）· `POST …`（主编录入原话）；`task <id>` 已随附与本阶段、本条目相关的至多 5 条 |
| roster | 各 agent | `GET /api/v1/roster` |

> 没有「建 / 改工作流」动作——定义只经管理后台或 `csw-workflow` skill（wfctl），本 skill 不碰。

### 2.1 fetch：下载上游交付物

上游链接来自派工单 `dispatch.upstreams[].url`（含「补件#n」）。OSS 公共链接直接下载；本服务代理链接（`…/api/v1/files/<id>`）要带鉴权头——统一带上总是安全的：

```bash
curl -fL -H "Authorization: Bearer $TOKEN" -o up.zip "<url>"
```

解压后以 `index.md` 为入口读正文与附件。**每条 upstream 都要下**。

### 2.2 submit：一步式提交产出

```bash
IDEM="submit-${TASK_ID}-$(shasum -a 256 交付物.zip | cut -c1-16)"
curl -fsS -X POST "$BASE/api/v1/tasks/$TASK_ID/deliverables" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $IDEM" \
  -F "file=@交付物.zip" \
  -F "self_check=逐条对照 task 返回的自检标准打 ✓" \
  -F "title=可选一句话标题"
```

服务端完成：版本 = 当前 +1；文件名 `{工作流}_{责任方}_{日期}_r{run}_v{n}.zip`；存储路径按阶段归档；`doc_type` 缺省 = 阶段产出类型。**你不拼路径、不算版本、不起文件名。**

### 2.3 补件与定点编辑（同一端点，用 kind 区分）

- **补件 `kind=supplement`**：挂在**已通过**的任务上（换图、换资产、补事实），必带 `affects_deliverable_id`（受影响的那份产出 id）。不重开审核、不动已批准版本；直接下游里已经开工的任务会被标「补件待返工」，没开工的派工时自动带上「补件#n」上游。

```bash
curl -fsS -X POST "$BASE/api/v1/tasks/$TASK_ID/deliverables" -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: supplement-${TASK_ID}-$(shasum -a 256 补件.zip | cut -c1-16)" \
  -F kind=supplement -F affects_deliverable_id=$DELIV_ID -F "summary=换头图：原因与适用位置" -F "file=@补件.zip"
```

- **定点编辑 `kind=edit`**：仅主编，见 §4.5。

## 3. 执行者循环（情报收集员 / 选题研究员 / 文案 / 小红书图文作者 / 设计师 / 发布员）

**铁律：群消息只是提示，引擎才是真相。没有引擎任务不开工。**

1. **醒来**（被群 @ / 30 分钟兜底 cron）→ 版本自检（§0.1）→ `my-tasks?status=open`，处理**全部** open 任务。
2. 对每个 `dispatched` 任务先 **`ack` 接单**（进度才显示「已接单」）。派工后 5 分钟未接单会提醒你、15 分钟升级主编（阶段可另设）。长任务每 5–8 分钟再 `ack` 一次作心跳——超过阶段的无活动阈值（默认 10 分钟，组版与保存类 5 分钟）没有心跳也没有产物，会提醒一次并在进度上标「无活动」。
3. `task <id>` → 派工单（`editor_note` + `upstreams`）、作业手册、自检标准。`rework_pending=true` 时先看补件（`supplements` 与派工单里的「补件#n」），按新资产返工。
4. `fetch` 每条上游 → 照作业手册干活。
5. 装交付物文件夹（`index.md` 头按《交付与流转规范》§5；逐条任务的 `item_key` 就是条目，头里写「条目」字段）→ zip → `submit`。**不用再发群消息**：引擎会 @ 下一道闸的审核人；0 闸阶段提交即通过。
6. **被退回**：`task <id>` 该版本的 `latest_review` 读方向 / 位置 / 意见 → 改 → 重交（版本自动 +1，从第一道闸重走）。
7. **做不了就报失败**：`fail` 写清原因（来源全部失效 / 缺必要权限 / 版本过低…），不要沉默等待；主编会重开或取消。**中枢侧（主编）收到失败或退回后，须在同一轮里接出下一步**——重开 / 重派具体任务，或明确宣布本期停止；只发状态播报就结束这一轮会让整期静默停摆（引擎事件驱动，没有新事件就不会再唤醒你）。平台写阶段返回 `authorization_required` = 本期没有这项授权——停手报主编，不找别的办法进后台。

**登记条目（情报收集员 / 选题研究员）**：每条情报在引擎登记为一个条目——`PUT /runs/{id}/items`，条目键 = 品牌英文或拼音小写 + 短横 + 原文链接 sha256 前 6 位（如 `hxo-3fa91c`），生成后不再改；研究员把采用与备选的条目标 `shortlisted`。批准、暂缓、否决只由主编按 Van 原话决定。

**Instagram 查 csw MCP（情报收集员）**：时间窗是唯一入口——直接 `csw_posts_search` 用 `start`/`end`/`order=latest`/`limit` 按当期窗口取全量贴文并翻页取完。不要按 `account` 逐个搜、不要凭记忆猜账号名（猜出来的「零结果」只说明账号不在后台，不说明该品牌没发，不得记为已扫来源）。账号信息是贴文的附属：拿到贴文后需要了解来源背景时，才用 `csw_accounts_list` 查对应账号。

**留采集与判断的痕迹（情报收集员 / 选题研究员）**：提交交付物前用 `PUT /runs/{id}/sweeps` 把本次全部采集轮一次报上去（每扫一个来源一条，含找到几条、窗口内几条、登记几条、成败与失败原因）——没有这条记录，主编无法区分「没找到」与「没去找」。每条条目写 `discovered_via`（对应的 sweep_key）、`fetched_at`、`evidence_url`、`dedup_note`。**判断过就不要留在 candidate**：确定不做的标 `dropped` 并给 `reason_code` 与一句理由（`no_value` / `not_new` / `dup_published` / `dup_recent_rejected` / `out_of_window` / `evidence_missing` / `aesthetic_mismatch` / `superseded` / `other`），引擎会自动落一条判断轨迹；反复上报同样内容不会重复记录。

### 3.1 状态词汇（全员统一，汇报进度只用这些词）

| 状态 | 含义 |
|---|---|
| `blocked` 未就绪 | 上游还没通过 |
| `ready` 待派工 | 具备派工条件；平台写阶段还需授权，否则进度显示「待授权」 |
| `dispatched` 已派工 | 已正式派工，等执行者接单 |
| `in_progress` 已接单 | 执行者已接单在做 |
| `review` 待审 | 等某道闸（看 task 的 `gates` 与交付物 `cur_gate`） |
| `returned` 需返工 | 被退回，按意见重交 |
| `passed` 已交付 | 末闸通过 |
| `failed` 报告失败 | 执行者报告无法完成，等主编处理 |
| `cancelled` 已取消 | 主编取消 |

「解锁」「可派工」不等于「已派工」；「已派工」不等于「已接单」。整期进度以 `GET /runs/{id}/progress` 为准，不要凭记忆汇报。

## 4. 主编编排（中枢）

对照引擎返回的验收标准审核；定点编辑有权限、有预算、有留痕（§4.5）；Van 的决定由你代录，**必须附 Van 原话**。

### 4.0 推进原则：一轮做到底（最容易出问题的一条）

引擎是事件驱动的：**你这一轮一结束，就只有下一条群消息或定时推进才能再叫醒你**。所以每次醒来（被 @、定时推进、提醒）都按这四条做：

1. **先看全部待办**：`inbox` 的 `review_queue` / `dispatch_queue` / `failed_queue` / `my_tasks` 全看一遍，不只看叫醒你的那一条。忙的时候进来的 @ 可能被并进了上一轮——所以**每轮收尾前再查一次 `inbox`**，有新的接着做。
2. **能做完的在这一轮里做完**：待审的给出结论（通过或退回），就绪的派工，失败的重开或取消，自产任务（03 / 07）连续做到 `submit`。
3. **不以心跳或进度汇报结束一轮**：心跳只证明你还在，不是进展。只有三种情况可以停：交出去了；需要 Van 的决定（已按 §4.3 代录请求）；做不了（已 `fail` 写明原因）。
4. **活太大一轮做不完时**，先交能交的最小完整版本（例如 03 按选题代表帖看图出选题卡），不要一小步一小步挪——每次被叫醒只挪一步，一个阶段会拖上几个小时（09-29 r56 #594）。

### 4.1 起跑（每日任务启动）

Van 在群里下达当日任务 → 你：

```bash
# ① 触发（inputs 按《生产约定》§9 固定字段）；v3 起 01-情报逐条自动派工，不用手动派首单
curl -fsS -X POST "$BASE/api/v1/workflows/daily_news/runs" -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: trigger-daily_news-2026-09-16-1" \
  -d '{"subject":"2026-09-16","inputs":{"约定版本":"v1.0","目标":{"主选":6,"备选":2},"授权":["local_drill","wx_draft"],"小红书":false}}'
# ② 录入授权：首期为本地演练 + 公众号草稿（Van 9/15 已定，全文终审通过后保存并回读）；其余按 Van 当日原话
curl -fsS -X POST "$BASE/api/v1/runs/$RUN/authorizations" -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: auth-$RUN-local_drill" -d '{"scope":"local_drill","source_quote":"Van：今天先本地演练"}'
curl -fsS -X POST "$BASE/api/v1/runs/$RUN/authorizations" -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: auth-$RUN-wx_draft" -d '{"scope":"wx_draft","source_quote":"Van 9/15：允许保存 CAMPsomeWHERE 公众号草稿，全文终审获批后保存并回读"}'
# ③ 本期不纳入小红书：取消 10-小红书改编与 11-小红书选图包（12、13 随之取消），run 才能在公众号交付后结束
for T in $XHS_TEXT_TASK $XHS_PICK_TASK; do
  curl -fsS -X POST "$BASE/api/v1/tasks/$T/cancel" -H "Authorization: Bearer $TOKEN" \
    -H "Idempotency-Key: cancel-$T-noxhs" -d '{"reason":"本期不纳入小红书"}'
done
```

### 4.2 审核循环（按阶段声明的闸）

闸以 `task` 返回的 `gates` 为准。v4 下：**Van 只在 03-选题方案、08-公众号完整审核稿、12-小红书图文包三处**（12 在接入小红书后才出现）：03 先主编自审、08 先主编核版、12 先主编审图文，再到 Van；01（主编首批校准）/ 04 / 07 / 10 是主编单闸；02 / 05 / 06 / 09 / 11 / 13 是 0 闸（提交即通过，你按验收标准抽查，发现问题用补件或重开处理）。

```text
inbox.review_queue → task <task_id>（验收标准 + 下载链接 + 自检申报）→ fetch → 对照验收标准
 ├─ 主编闸：pass；或 reject（方向 + 位置必填）；07 的主编闸可以定点编辑（§4.5）
 └─ Van 闸（relayed_by_hub）：主编闸通过时引擎已 @Van；Van 在群里给结论后你代录——
      必带 source_quote（Van 原话）、decision_type、expected_version（你看到的版本号）
```

```bash
curl -fsS -X POST "$BASE/api/v1/deliverables/$D/reviews" -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: review-$D-pass-v$V" \
  -d '{"verdict":"pass","decision_type":"fulltext_approve","source_quote":"Van：这期可以","expected_version":'"$V"'}'
```

`decision_type`：`research_ok` / `topic_approve` / `fulltext_approve` / `template_approve` / `local_verify` / `draft_save_ok` / `publish_ok`。

### 4.3 选题决定（03 Van 闸，按条目）

- 六栏：线索（candidate，还没判断）、待核（pending_check，关键事实未核实，不计成熟数量）、成熟主选（shortlisted + rank=primary）、成熟备选（shortlisted + rank=alt）、淘汰（dropped，判断过且有理由码——与「还没判断」的线索区分开）、Van 已批准（approve_write / approve_research）。
- `items` 写 Van 批准的条目键，**只随 Van 闸录入**（主编自审闸带条目会被拒绝，防止写作在 Van 批准前开工）；「保留」「就这条」= 批准可写；「再看看」「继续研究」= 只开研究、不派写作；措辞不明时只问一次受影响的范围。
- 引擎为每个批准可写的条目生成 04-公众号写作与 05-配图与素材核任务并自动派工（派工单由引擎写明条目、来源与 Van 原话）；07-内容整合稿等这些条目的两项都通过才就绪。
- 补批或撤回单条：`POST /runs/{id}/items/{item_key}/decision {decision, source_quote}`。撤回只取消该条尚未完成的任务，不影响内容整合稿；已写成的条目不能再改决定。批准晚到时，已开工的内容整合稿会标「补件待返工」，由你决定纳入本期还是留到下期。
- 数量不足：已批准的照常推进，缺量说明四要素（差多少、为什么缺、怎样补、补不齐怎么办）写进选题方案；不用旧闻或弱题凑数；改数量或范围由 Van 一次决定。整期目标来自触发 inputs（`目标.主选`）；其余任务都结束而已写成仍不足时，run 保持进行中，Van 接受缺口后 `POST /runs/{id}/close {reason}` 结束。若整期一个任务都没通过（全部取消或失败），`close` 会返回 `run_nothing_passed`——这种颗粒无收的实例用 `POST /runs/{id}/abort {reason}` 作废收尾，状态落 `aborted`，不会被记成完成；不作废它会一直挂在 active 干扰后续批次。

### 4.4 手动派工的两个阶段

- **10-小红书改编**、**11-小红书选图包**：只有当期纳入小红书才派（公众号连续 3 个工作日达标后接入）；不纳入按 §4.1③ 取消。两者并行，12-小红书图文包等两者都交齐。
- 04-公众号写作已改为自动派工：条目决定录入即派，派工单由引擎写明条目与 Van 原话；写作重点写进 03 选题方案的选题卡。

### 4.5 定点编辑（07-内容整合稿的主编闸）

条件：任务在待审、当前闸是你的主编闸。首审一次指出全部影响通过的问题，允许一轮修订；同类问题第二次仍在 → 定点编辑、换已批备选或报缺口，不无限退回。预算约 10 分钟，是调度目标，不是到时自动放行。

```bash
curl -fsS -X POST "$BASE/api/v1/tasks/$TASK_ID/deliverables" -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: edit-$TASK_ID-v$CUR-$(shasum -a 256 协作稿.zip | cut -c1-16)" \
  -F kind=edit -F edit_of=$CUR -F "diff_summary=第2则时态改为已发布；导读第3条与正文对齐" -F "file=@协作稿.zip"
```

效果：首稿标「已被取代」原样保留，编辑版本过主编闸并自动留痕；07 只有这一道闸，随即派 08 组版。编辑后你仍要在 08 核版（检查最终全文与图文一致），再送 Van。新增事实必须有来源。

### 4.6 授权（平台写阶段）

09-公众号草稿保存、13-小红书草稿与发布是平台写操作：run 有对应授权才会派工、才能提交；发布授权涵盖同平台草稿。授权必须带 Van 原话；群里一句「先别存」不构成护栏——要撤销就 `DELETE /runs/{id}/authorizations/{scope}`。录入后，停在「待授权」的阶段自动续派。首期 `wx_draft` 已由 Van 定：08 经 Van 终审通过后自动派 09 保存并回读，不再逐日追加。

`scope`：`local_drill`（本地演练）/ `wx_draft` / `wx_publish` / `xhs_draft` / `xhs_publish`。

### 4.7 失败、重开、取消

- **失败**：`inbox.failed_queue` → 看 `fail_reason` → `reopen`（reason 写清换什么办法，任务回 ready，由你重新派工）或 `cancel`。
- **重开已通过的任务**（如 Van 事后改主意）：`reopen` → 回 ready → 重新派工；已完成的 run 自动回到进行中。
- **取消**：`cancel` 连带取消尚未开始的下游，用于本期不做的支线。

### 4.8 自产任务（03-选题方案 / 07-内容整合稿）

合流就绪即自动派给你（`my_tasks` 出现）。按作业手册做完 `submit`：03 走你自己的闸 → Van 选题；07 只有你自己的闸，通过后派 08 组版，Van 终审在 08。

**接单后在同一轮里连续做到 `submit`**（§4.0）。03 的选题卡按选题看代表帖实图即可，不必每帖都看；缺证据的写进缺口，不要为补一张图停下。引擎的「接单后无活动」提醒对自产任务不接受心跳作答。

## 5. 编辑部群（v3：引擎单写）

- 流转播报全部由引擎通知组件发送：真 at-tag、同一事件只发一次、失败自动重试。agent **不再**发派工 / 交付 / 退回 / 转审 / 阶段过这类播报。
- 被 @ 后一律回引擎核实——群消息可能过期，也可能是人随手发的。
- 引擎只接管**流转播报**。正常沟通照常：主编答疑、解释编辑判断，主动报告关键故障、授权缺口与交付风险；执行者对派工单有疑问直接在群里问主编。
- **兜底**：`GET $BASE/healthz` 的 `notifier` 不是 running，或某个动作 5 分钟后群里仍没有对应播报 → 由主编按 §5.2 人工补发一条；执行者只在需要唤醒主编时发。

### 5.1 花名册

以引擎为权威、实时拉取：`GET /api/v1/roster`；API 不可达时读 `$CSW_TASK_ROSTER`；都没有就回退纯文本并报主编。键 = 引擎 `role_code`，值含 `open_id`。十个角色均已映射：editor 主编 · collector 情报收集员 · researcher 选题研究员 · writer 文案（群名 深度内容创作者）· xhswriter 小红书图文作者 · designer 设计师（视觉设计师）· publisher 发布员（发布运营员）· reviewer 合规版权审查员 · analyst 数据复盘师 · van Van（人类）。

### 5.2 人工兜底发送规范（仅兜底时用）

1. 直接发新消息到群（不回复、不开话题）。
2. 真 mention：`<at user_id="{open_id}">{名称}</at>`，open_id 取花名册。
3. 发送方式固定：`lark-cli im +messages-send --chat-id <chat_id> --msg-type text --content '{"text":"<at user_id=\"ou_xxx\">名称</at> 正文…"}'`。
4. 禁止纯文本「@名字」、Markdown 里的 @、普通 send_message——飞书不会真正提及，对方不会被唤醒。

### 5.3 护栏

- **只有主编可以 @Van**（引擎也只在 Van 闸待审时 @Van）；执行者任何时候不直接联系 Van。
- Van 的结论请 @主编 回复；Van 在群里对执行者的直接插话**不构成派工 / 审核 / 授权**——由主编录进引擎才生效。

## 6. 幂等键（确定性派生，重试安全）

| 动作 | 键 |
|---|---|
| submit | `submit-{task_id}-{zip sha256 前16}` |
| supplement | `supplement-{task_id}-{zip sha256 前16}` |
| edit | `edit-{task_id}-v{edit_of}-{zip sha256 前16}` |
| dispatch | `dispatch-{task_id}-{note sha256 前16}` |
| review | `review-{deliverable_id}-{verdict}-v{version}` |
| authorize | `auth-{run_id}-{scope}`（同一授权重复录入会返回已有记录） |
| fail / cancel / reopen | `{fail\|cancel\|reopen}-{task_id}-{reason sha256 前8}` |
| decide | `decision-{run_id}-{item_key}-{decision}` |
| close | `close-{run_id}` |
| trigger | `trigger-{wf}-{subject}-{当日批次号}` |
| ack | 不带键（重复调用只刷新活动时间） |

服务端语义（fail-closed，按键 at-most-once）：同键第一次到达即占位；业务成功后同键重试回放首次响应；业务失败释放占位，修正后可同键重试；键相同但内容 / 身份 / 目标不同 → 409 `idempotency_request_mismatch`；首次请求中途崩溃 → 同键一律 409 `idempotency_in_progress_or_uncertain`，此时先 `task <id>` 核实是否已落，不要换键重发。multipart 重试可换 boundary，但文件字节与字段值必须一致。

## 7. 错误码处置

| HTTP / code | 含义 | 处置 |
|---|---|---|
| 401 | token 无效 / 缺失 | 查 `CSW_TASK_TOKEN` |
| 403 `not_assignee` / `not_hub` / `not_reviewer` / `relay_requires_hub` | 动作不归你的角色 | 走对的角色 |
| 404 | task / deliverable / run / file 不存在 | 核对 id |
| 409 `cannot_submit` | 任务不在 已派工 / 已接单 / 需返工 | `task <id>` 看状态 |
| 409 `cannot_ack` / `cannot_fail` | 状态不对（已交付、待审中等） | `task <id>` 看状态 |
| 409 `authorization_required` | 平台写阶段本期没有对应授权 | 停手报主编；主编录入 Van 授权后自动续派 |
| 409 `task_not_ready` | 派工对象不是 ready | 上游没过或已派出，看 `progress` |
| 409 `stale_version` / `expected_version_mismatch` | 审的不是当前版本 | 重新 `task <id>` 取最新版本 |
| 409 `not_in_review` / `no_gate` | 不在待审或已到末闸 | `task <id>` 看 gates 与 cur_gate |
| 409 `supplement_requires_passed` | 补件只能挂在已通过的任务上 | 未通过时直接交新版本 |
| 409 `cannot_edit` / `edit_gate_not_yours` / `edit_of_mismatch` | 定点编辑条件不满足 | 只能在自己的待审闸、基于当前版本 |
| 409 `cancel_forbidden_state` / `cannot_reopen` / `run_not_active` | 取消 / 重开 / 授权的状态不对 | 看 `progress` |
| 400 `return_required` / `reason_required` | 缺退回方向位置 / 缺原因 | 补齐 |
| 400 `source_quote_required` | Van 闸或授权缺 Van 原话 | 补 `source_quote` |
| 400 `bad_decision_type` / `bad_items_json` / `bad_scope` / `bad_kind` | 取值不合法 | 按 §4 取值 |
| 400 `items_require_van_gate` | 在主编自审等中间闸带了条目 | 条目只随 Van 闸（或末闸）录入；中间闸不带 items |
| 400 `affects_required` / `bad_affects` / `edit_of_required` / `diff_summary_required` | 补件或定点编辑缺字段 | 按 §2.3 / §4.5 补齐 |
| 400 `doc_type_required` | doc_type 与阶段产出类型都空 | 显式传 `-F doc_type=…` |
| 400 `bad_item_key` / `bad_item_status` / `bad_decision` | 条目键或状态、决定取值不合法 | 条目键小写字母数字与短横；登记只写 candidate / pending_check / shortlisted / dropped |
| 400 `drop_reason_required` / `bad_reason_code` | 淘汰没写理由，或理由码不在词表里 | 标 dropped 必须同时给 reason_code 与一句理由 |
| 400 `bad_sweep_key` / `bad_sweep_platform` / `bad_sweep_tool` / `bad_sweep_result` / `sweep_error_required` / `too_many_sweeps` | 采集轮上报字段不合法 | sweep_key 不能空；platform 取 instagram / xhs / web / other；失败的轮次必须写 error；单次最多 200 条 |
| 403 `not_participant` | 不是本期 run 的参与角色 | 登记条目只由参与角色或主编做 |
| 404 `item_not_found` / 409 `item_already_written` | 没有这个条目 / 该条已写成 | 先登记；已写成的要改请重开相关任务 |
| 409 `run_has_open_tasks` / `run_nothing_passed` | 结束 run 时还有未完成任务 / 没有交付 | 先处理未完成的任务 |
| 409 `idempotency_request_mismatch` | 同键用于不同内容 | 检查键的派生；确属新请求就换键 |
| 409 `idempotency_in_progress_or_uncertain` | 同键首次请求结果不确定 | `task <id>` 核实，已落则不再发 |
| 413 `idempotency_body_too_large` | 请求体超上传上限 | 检查 zip 大小 |
| 503 `idempotency_store_unavailable` | 幂等存储不可用，业务未执行 | 稍后同键重试 |
| 5xx | 服务端错误 | 同键重试；持续失败报主编 |
| （客户端）版本过低 | 本文件 version < `skill_min_version` | 按 §0.1 fail 并停手 |

> 完整模型见《任务流转服务 · 设计文档》；流程语义见《资讯日更 · 全流程 / 交付与流转规范 / 验收标准 / 生产约定》（运行期以 `task <id>` 返回为准）。
