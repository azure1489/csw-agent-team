# csw-collector 工作台接口契约

阶段 1 冻结。**改这里要同步 `crates/core/src/types.rs` 与前端类型**，
并重跑 `csw-collector schema -o ../csw-collector-web/src/contract.json`。

数据类型的权威是 `core::types`；本文只定端点、鉴权与语义。

## 通则

- 前缀 `/api`，JSON 进 JSON 出。时间一律 RFC3339 UTC（`2026-09-22T01:40:00Z`），
  **界面负责换算北京时间**，接口不返回本地时间。
- 鉴权走服务端会话：`csw_collector_sid`（HttpOnly、SameSite=Lax、生产带 Secure）。
  引擎的 access 与 refresh **只留在服务端**，浏览器一个都拿不到。
- **`/api` 下的每一个接口都要会话**，读接口也不例外，没有就 401。
  读得到的东西没有一样能匿名给出去：整期台账、谁把哪一条改了档、Van 说过的原话。
  服务挂在公网域名上，前端那层 `Guard` 只是体验，拦人的是路由上的守卫。
- 所有**写**接口另外要回填 `X-CSW-CSRF` 头，值取自 `/api/auth/me` 的 `csrf_token`。
  `SameSite=Lax` 挡不住表单式跨站 POST，这层不能省。读接口不校验 CSRF：
  它不改状态，而跨站发起的读，攻击者也读不到响应。
- 错误统一 `{"error": "一句人话"}` + 恰当的 HTTP 码。引擎的原文错误只进日志，不透给浏览器。
- 角色：`superadmin` > `operator` > `viewer`，另有 `van`（配置里列出的 viewer 映射而来，
  引擎侧仍是 viewer）。下表的「角色」是**最低**要求。
  写接口按两档判：`superadmin` / `operator` 能改档、指定首批、开轮、重跑、代录勾选；
  `van` 只能勾选；`viewer` 一个写都不行。**`van` 改不了档**——
  进不进评选由主编在引擎上代录并附原话，不是工作台能替她做的。

## 登录

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| POST | `/api/auth/login` | —— | 转发引擎 `/admin/login`。返回 `{username, display_name, role, engine_role, csrf_token}` 并下发会话 cookie。口令错一律 401，**不区分「用户不存在」与「口令错」** |
| GET | `/api/auth/me` | viewer | 角色以引擎当下的回答为准（后台改了角色或停用立刻生效）。access 快过期时服务端自动续，并接住引擎轮换出的新 refresh |
| POST | `/api/auth/logout` | viewer | 204，会话从服务端删除 |

## 轮次

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rounds` | viewer | 列表。`?kind=&status=&since=&limit=` |
| GET | `/api/rounds/:id` | viewer | 详情：窗口、方案版本、准则版本、知识库快照、十步状态与计数 |
| GET | `/api/rounds/:id/steps` | viewer | 每步的全部 attempt，含 `input_hash` 与失败清单 |
| POST | `/api/rounds` | operator | 手动开启一轮。`{days?}` 或 `{window_start, window_end}`（RFC3339，解析不了当场 400）。**不关联引擎任务、不写引擎**，只落本地。**只排队不当场跑**，返回 `{work_id, queued_ahead}` |
| POST | `/api/rounds/:id/steps/:step/rerun` | operator | 重跑一步。`:step` 只认 `harvest`（连识别一起重来）与 `judge`（只重判）。同样只排队，返回 `{work_id, queued_ahead}` |
| POST | `/api/rounds/:id/cancel` | operator | 停止本轮（打断深核、停心跳）。**未实现** |
| GET | `/api/rounds/:id/outbox` | operator | 写引擎的队列。`conflict` 的要人核实，**不要换幂等键重试** |
| GET | `/api/work` | viewer | 排队的活排到哪了、做成没有、没成是为什么。`?limit=` |

**写接口只排队，干活的是常驻循环。** 一轮四十分钟，HTTP 请求等不了；
排队的那一行就是页面上「排到哪了 / 为什么没成」的唯一来源。

**重跑不碰登记与提交。** 引擎那边已经收到的台账要改，只能走补件——
那是人的决定，不该是重跑的副作用。重跑一轮带任务号的，用**那一轮当时**
下发的作业标准，不是现在最新的：换一份标准重判等于换了依据。

## 判断台账

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rounds/:id/judgements` | viewer | 台账。`?tier=&has_gap=&brand=&q=&cursor=&limit=`。按档分组，六维带依据，含 Jev 核对标记 |
| GET | `/api/rounds/:id/judgements/:key` | viewer | 单条全貌**一次给全**：候选原文、每张图与其描述、判断、改档历史、Van 勾选、深核结论、这一轮的模型调用。分七个接口去拿，页面上就是七个各自转圈的小方块 |
| POST | `/api/rounds/:id/judgements/:key/override` | operator | 改档。`{to_tier, reason}`，理由必填。**另存不覆盖**，台账上两者都看得见。返回 `{id, effective_tier}` |
| POST | `/api/rounds/:id/judgements/:key/recover` | operator | 把被 03 决定硬性排除的条目捞回来。**未实现**：硬性排除本身还没做，现在「捞回」就是改档（`override` 到 `alternate`），做出来之后再加这个端点 |
| POST | `/api/rounds/:id/first-batch` | operator | 指定首批要深核的条目。`{keys: []}`，传空数组＝清空指定、退回自动挑法。不在这一轮里的键忽略，返回 `{marked}` |

台账每行同时给**原判**（`tier`）与**现在生效的那一档**（`effective_tier`）
以及改档的理由与人。筛选与排序按生效档——主编把一条捞回成备选之后，
台账还把它排在不推荐那一组里的话，那次捞回等于没发生。

**接口里没有「分数」字段，也不会有。** 结论只有四档 + 六维成立与否 + 依据。
Jev 初评的概率只在服务端决定「先判哪条」，不出现在任何响应里。

## 采集与覆盖

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rounds/:id/coverage` | viewer | 每个采集器一行。**数字来自我们自己发出去的那一份**（outbox 里的字节），不是现场再算——页面上看到的要与引擎收到的是同一份。另附采集那一步的耗时账 |
| GET | `/api/rounds/:id/media?state=failed` | viewer | 这一轮的图；`state=failed` 只看没下到或没识别成的 |
| GET | `/api/sources` | viewer | 来源台账与待补录账号清单（P1） |
| GET | `/api/sources/export` | viewer | 待补录账号导出 CSV |

## 待核结转

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/pending-check` | viewer | 跨轮未结的 `pending_check`。**待核不是淘汰**，补齐材料后重评 |
| POST | `/api/pending-check/:key/recheck` | operator | 补齐后重评单条 |

## 知识库与选题记忆

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/kb/search` | viewer | `?q=&limit=`。向量 top-K ∪ 品牌命中 ∪ 全文 → 合并 → 截断。**这条路上不重排**——重排占 GPU，而 GPU 是全进程串行的；有人在页面上连搜几下，正式轮的向量化就堵住了 |
| GET | `/api/kb/similar-selected` | viewer | `?q=<candidate_key>`。用候选正文现算一次查询向量（候选的融合向量是一次性的，没存），把它自己排除掉 |
| GET | `/api/kb/brands/:brand` | viewer | 某品牌的历史覆盖。**纯查库，不碰 GPU**——这一页是拿来翻的 |
| GET | `/api/kb/backfill` | viewer | 待补录清单：进过选题（决定 / 已发 / 范例）却对不上 csw 在册别名的品牌，按被选中次数排，每行附最近 5 条依据（标题、日期、链接、结论码）。**不带决定正文**（里面有 Van 的原话）。别名表每次现读 |
| GET | `/api/kb/status` | viewer | 各来源**同步到哪天了**与条数。不是「库里有多少」：前者好看，后者才回答「今天的判断有没有拿到昨天的已发条目」 |

知识库那三个检索接口在客户端还没建起来时回 **503**，不回空结果——
「还没起来」与「库里什么都没有」是两件完全不同的事。
| POST | `/api/kb/sync` | operator | 手动触发增量同步 |
| GET | `/api/memory/rules` \| `/cases` | viewer | 准则卡与案例库（含 Van 原话） |
| POST | `/api/memory/rules/:key/confirm` | operator | 记录 Van 的校准结论 |

## Van 模式

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/van/today` | van | 手机优先的当期视图：**只给推荐与备选**（按生效档，改过档的算改过的），每条三句话、热度说明与她已勾的标记。手机上翻三百条不是在帮她；要看全部去判断台账。还没开工时回 `{round_id: null, items: []}`，不是错误 |
| POST | `/api/van/marks` | van | 勾选。`{candidate_key, mark: like\|doubt\|note, note?, round_id?, remove?}`。不给 `round_id` 就落到当期那一轮（最近开的**派单轮**，不含手动轮与预取轮）。`note` 这一种必须有内容 |
| GET | `/api/rounds/:id/van-marks` | viewer | 这一轮的全部勾选 |

**勾选只写本地，不回写引擎。** 进不进评选由主编在引擎上代录，并附 Van 原话。

## 判断框架与指标

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rubric` | viewer | 当前准则版本、核心问题、三问、七类优先、七类降低、六维锚点、**四项「不是维度」**。从代码里的常量来，不是从库里——它要跟着提示词一起走 |
| GET | `/api/rubric/backtest` | operator | 回测结果 |
| GET | `/api/metrics/selection` | viewer | **全是计数，没有一个是分数**。「模型判了什么」与「人改成了什么」分开算（被捞回的 / 被压下的）：两者重合得越少，说明判断框架离 Van 的口味越远——那正是要看的 |

## 运行与设置

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/settings` | operator | 采集方案、开关、服务状态（引擎 / csw / 向量 / 网关 / codex）。**密钥只返回「已配置 / 缺」，不返回值** |
| GET | `/api/plans` \| POST `/api/plans` | superadmin | 采集方案的版本与发布 |
| POST | `/api/collectors/custom` | superadmin | 登记自定义采集器。**默认总开关关闭**，固定目录、不经 shell、清空环境变量 |
| GET | `/api/replay/fixtures` | operator | 夹具清单 |
| POST | `/api/replay` | operator | 用夹具回放一轮，不出网 |
| GET | `/api/audit` | operator | 改档、改方案、手动开轮、登记采集器的留痕。`?limit=` |

## 运维

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/healthz` | —— | 无鉴权。依赖分项状态：`sqlite`（读不了才是真不健康）、`disk`（**到告警线就标不 ok**，别等它到拒开新轮那条线） |
| GET | `/metrics` | —— | 仅回环可达。轮次数、判断数、未结待核、outbox 冲突、存活秒数、磁盘百分比（`-1` = 问不出来）、排队中的手动活 |

**磁盘到 `disk_block_pct`（默认 92%）就不开新轮**，并向引擎报失败、往开发群发一条。
盘满时的表现极难看懂——下载一半失败、SQLite 写不进去、zip 打到一半断掉，
每一处报的都是别的错；到线就停，比让它们轮流出怪错强。

## 不做的事

- **不提供任何改引擎流程状态的接口。** 派工、审核、提交属运行面，由采集服务按引擎派单自己走；
  工作台不给人在这里操作流程的口子。
- **不提供按分数排序的接口。** 排序键在服务端，不外露。
- **不回写图片描述到 csw。**
