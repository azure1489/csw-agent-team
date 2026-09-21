# csw-collector 工作台接口契约

阶段 1 冻结。**改这里要同步 `crates/core/src/types.rs` 与前端类型**，
并重跑 `csw-collector schema -o ../csw-collector-web/src/contract.json`。

数据类型的权威是 `core::types`；本文只定端点、鉴权与语义。

## 通则

- 前缀 `/api`，JSON 进 JSON 出。时间一律 RFC3339 UTC（`2026-09-22T01:40:00Z`），
  **界面负责换算北京时间**，接口不返回本地时间。
- 鉴权走服务端会话：`csw_collector_sid`（HttpOnly、SameSite=Lax、生产带 Secure）。
  引擎的 access 与 refresh **只留在服务端**，浏览器一个都拿不到。
- 所有**写**接口要回填 `X-CSW-CSRF` 头，值取自 `/api/auth/me` 的 `csrf_token`。
  `SameSite=Lax` 挡不住表单式跨站 POST，这层不能省。
- 错误统一 `{"error": "一句人话"}` + 恰当的 HTTP 码。引擎的原文错误只进日志，不透给浏览器。
- 角色：`superadmin` > `operator` > `viewer`，另有 `van`（配置里列出的 viewer 映射而来，
  引擎侧仍是 viewer）。下表的「角色」是**最低**要求。

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
| POST | `/api/rounds` | operator | 手动开启一轮。`{stage_code, window_start, window_end, plan_version?}`。**不关联引擎任务**，只写本地 |
| POST | `/api/rounds/:id/steps/:step/rerun` | operator | 重跑一步：新建 attempt，并把下游置 `stale` |
| POST | `/api/rounds/:id/cancel` | operator | 停止本轮（打断深核、停心跳） |
| GET | `/api/rounds/:id/outbox` | operator | 写引擎的队列。`conflict` 的要人核实，**不要换幂等键重试** |

## 判断台账

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rounds/:id/judgements` | viewer | 台账。`?tier=&has_gap=&brand=&q=&cursor=&limit=`。按档分组，六维带依据，含 Jev 核对标记 |
| GET | `/api/rounds/:id/judgements/:key` | viewer | 单条：候选原文、每张图与其描述、五类对照材料、深核结论、模型调用记录 |
| POST | `/api/rounds/:id/judgements/:key/override` | operator | 改档。`{to_tier, reason}`。**另存不覆盖**，台账上两者都看得见 |
| POST | `/api/rounds/:id/judgements/:key/recover` | operator | 把被 03 决定硬性排除的条目捞回来 |
| POST | `/api/rounds/:id/first-batch` | operator | 指定首批要深核的条目 |

**接口里没有「分数」字段，也不会有。** 结论只有四档 + 六维成立与否 + 依据。
Jev 初评的概率只在服务端决定「先判哪条」，不出现在任何响应里。

## 采集与覆盖

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rounds/:id/coverage` | viewer | 每个采集器一行：取到、只图文、图片数、已识别、耗时、结果 |
| GET | `/api/rounds/:id/media?state=failed` | viewer | 下载或识别失败的清单 |
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
| GET | `/api/kb/search` | viewer | `?q=&kind=&limit=`。向量 top-K ∪ 品牌命中 ∪ 全文 → rerank ≤8 → 分组 |
| GET | `/api/kb/similar-selected` | viewer | 与某条候选相似的已采用条目 |
| GET | `/api/kb/brands/:brand` | viewer | 某品牌的历史覆盖 |
| GET | `/api/kb/status` | viewer | 各来源的同步水位与条数 |
| POST | `/api/kb/sync` | operator | 手动触发增量同步 |
| GET | `/api/memory/rules` \| `/cases` | viewer | 准则卡与案例库（含 Van 原话） |
| POST | `/api/memory/rules/:key/confirm` | operator | 记录 Van 的校准结论 |

## Van 模式

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/van/today` | van | 手机优先的当期视图：推荐与备选，每条三句话 + 代表图 |
| POST | `/api/van/marks` | van | 勾选。`{candidate_key, mark: like\|doubt\|note, note?}` |

**勾选只写本地，不回写引擎。** 进不进评选由主编在引擎上代录，并附 Van 原话。

## 判断框架与指标

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/rubric` | viewer | 当前准则版本、六维定义与锚点 |
| GET | `/api/rubric/backtest` | operator | 回测结果 |
| GET | `/api/metrics/selection` | viewer | 采用率、首批命中、待核结转等 |

## 运行与设置

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/api/settings` | operator | 采集方案、开关、服务状态（引擎 / csw / 向量 / 网关 / codex）。**密钥只返回「已配置 / 缺」，不返回值** |
| GET | `/api/plans` \| POST `/api/plans` | superadmin | 采集方案的版本与发布 |
| POST | `/api/collectors/custom` | superadmin | 登记自定义采集器。**默认总开关关闭**，固定目录、不经 shell、清空环境变量 |
| GET | `/api/replay/fixtures` | operator | 夹具清单 |
| POST | `/api/replay` | operator | 用夹具回放一轮，不出网 |
| GET | `/api/audit` | operator | 改档、改方案、手动开轮、登记采集器的留痕 |

## 运维

| 方法 | 路径 | 角色 | 说明 |
|---|---|---|---|
| GET | `/healthz` | —— | 无鉴权。依赖分项状态 |
| GET | `/metrics` | —— | 仅回环可达 |

## 不做的事

- **不提供任何改引擎流程状态的接口。** 派工、审核、提交属运行面，由采集服务按引擎派单自己走；
  工作台不给人在这里操作流程的口子。
- **不提供按分数排序的接口。** 排序键在服务端，不外露。
- **不回写图片描述到 csw。**
