# Agent 部署装配（bootstrap + 安装清单 · v3）

> 每个编辑部 agent = 独立运行环境（独立飞书 bot、独立 token）。**`csw-task` skill 只有一份实现**，装到所有 agent；
> 角色差异由 ① token（= 引擎 role_code）② 引擎按阶段下发的作业手册 ③ 本目录的身份 bootstrap 承载。
> bootstrap 是**零标准**的：任何「做什么 / 自检 / 验收」都不写在这里——运行期一律以 `task <id>` 引擎返回为准。

## 装配公式（每个 agent 环境）

```
csw-task skill（同一份，见 ../SKILL.md，只读分发）
+ lark-im / lark-event skill（被 @ 唤醒；兜底时人工发群消息）
+ 角色工具 skill 子集（下表）
+ 本目录对应的 bootstrap（拼进该 agent 的 system prompt 或 CLAUDE.md）
+ env：CSW_TASK_TOKEN（各不同，=角色）/ CSW_TASK_BASE_URL（只指向生产引擎）/ CSW_TASK_ROSTER（指向 ../roster.json）
```

## 各角色安装清单

| bootstrap | role_code | 群 bot | 阶段（daily_news v3） | 除公共底座外的工具 skill |
|---|---|---|---|---|
| `editor.md` | editor | 主编 | 03 选题方案、07 内容整合稿（合流）+ 各主编闸 + 代录 Van（03 / 08 / 12） | `csw-workflow`（可选：按 Van 意愿改流程定义，激活须人在终端确认；需后台 operator 账号） |
| `collector.md` | collector | 情报收集员 | 01 情报逐条、05 配图与素材核、11 小红书选图包 | 营事编集室 csw MCP · opencli（xiaohongshu）· WebFetch |
| `researcher.md` | researcher | 选题研究员 | 02 价值初筛 | —（发布记录接口上线后查近发） |
| `writer.md` | writer | 深度内容创作者 | 04 公众号写作 | — |
| `xhswriter.md` | xhswriter | 小红书图文作者 | 10 小红书改编 | — |
| `designer.md` | designer | 视觉设计师 | 06 版式与模板准备 | generate-images（仅指定生成时） |
| `publisher.md` | publisher | 发布运营员 | 08 公众号完整审核稿（组版）、09 草稿保存与回读、12 小红书图文包、13 小红书草稿与发布 | mp-helper · opencli（xiaohongshu） |
| `reviewer.md` | reviewer | 合规版权审查员 | 无固定阶段：主编点名的专项检查 | — |
| `analyst.md` | analyst | 数据复盘师 | 不在日更 run 内：发布记录与复盘 | 数据子系统（syncer / ledger） |

## 分发与维护规则

- **skill 只读分发**：各 profile 里的 `csw-task` SKILL.md 以本仓库版本为准，禁止在 profile 内改写（包括 curator 自动整理、追加私有条款）。要改先改仓库、升 `version`，再统一下发。
- **版本自检**：引擎在 `task` / `my-tasks` 响应里回显 `skill_min_version`，低于则 agent 不开工（SKILL.md §0.1）。
- **只连生产引擎**：`CSW_TASK_BASE_URL` 只能指向生产引擎；agent 不得构建、运行或连接引擎副本。演练用生产引擎里只带「本地演练」授权的 run。
- **群播报由引擎单写**：agent 不发流转播报；监听被 @ 后回引擎核实。

## 引导步骤（每个 agent 一次性）

1. 管理侧签发 token：`./bin/adminctl token issue <role_code>`（明文仅显示一次）。xhswriter / reviewer / analyst 为 v3 新接入角色，需要各签一次。
2. 该 agent 环境配置 env 三件套；把对应 bootstrap 并入其 system prompt / CLAUDE.md；安装上表 skill。
3. 启动 lark 监听（`lark-event` consume「CSW编辑部」群，被 @ 即唤醒进入工作循环）。
4. 验证：`curl -fsS $CSW_TASK_BASE_URL/api/v1/me/tasks -H "Authorization: Bearer $CSW_TASK_TOKEN"` 返回 200，且响应里的 `skill_min_version` 不高于本地 SKILL.md 的 `version`。

> 兜底：给每个 agent 配低频 cron（如每 30 分钟）扫一遍 `my-tasks?status=open`——通知只是提示，引擎才是队列。
