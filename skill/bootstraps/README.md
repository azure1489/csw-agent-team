# Agent 部署装配（bootstrap + 安装清单）

> 每个编辑部 agent = 独立运行环境（独立飞书 bot、独立 token）。**`csw-task` skill 只有一份实现**，
> 装到所有 agent；角色差异由 ① token（=引擎 role_code）② 引擎按阶段下发的作业手册 ③ 本目录的身份 bootstrap 承载。
> bootstrap 是**零标准**的：任何「做什么/自检/验收」都不写在这里——运行期一律以 `task <id>` 引擎返回为准。

## 装配公式（每个 agent 环境）

```
csw-task skill（同一份，见 ../SKILL.md）
+ lark-im / lark-event skill（群播报 / 被 @ 唤醒，人人都装）
+ 角色工具 skill 子集（下表）
+ 本目录对应的 bootstrap（拼进该 agent 的 system prompt 或 CLAUDE.md）
+ env：CSW_TASK_TOKEN（各不同，=角色）/ CSW_TASK_BASE_URL / CSW_TASK_ROSTER（指向 ../roster.json，全员相同）
```

## 各角色安装清单

| bootstrap | role_code | 群 bot（承担者） | 除公共底座外的工具 skill |
|---|---|---|---|
| `editor.md` | editor | 主编 | —（lark 重度使用） |
| `collector.md` | collector | ⏳ 待补充 | 营事编集室 csw MCP · opencli（xiaohongshu）· WebFetch |
| `researcher.md` | researcher | 选题研究员 | —（纯编辑判断） |
| `writer.md` | writer | 深度内容创作者 | —（写作） |
| `designer.md` | designer | 视觉设计师 | generate-images |
| `publisher.md` | publisher | 发布运营员 | mp-helper · opencli（xiaohongshu） |

## 引导步骤（每个 agent 一次性）

1. 管理侧签发 token：`./bin/adminctl token issue <role_code>`（明文仅显示一次）。
2. 该 agent 环境配置 env 三件套；把对应 bootstrap 内容并入其 system prompt / CLAUDE.md。
3. 安装上表的 skill；启动 lark 监听（`lark-event` consume「CSW编辑部」群，被 @ 即唤醒进入工作循环）。
4. 验证：`curl -fsS $CSW_TASK_BASE_URL/api/v1/me/tasks -H "Authorization: Bearer $CSW_TASK_TOKEN"` 返回 200 即通。

> 服务端部署提醒：生产 OSS 后端须设 `CSW_OSS_PREFIX=csw`，对象路径才是规范的 `csw/资讯日更/…`（默认 `blobs/`）。
> listener 掉线兜底：可给每个 agent 配一个低频 cron（如每 30 分钟）扫一遍 `my-tasks?status=open`——通知只是提示，引擎才是队列。
