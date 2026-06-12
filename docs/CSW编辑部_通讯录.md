# CSW编辑部 · 群与花名册

> 飞书「CSW编辑部」群的真实成员通讯录，供 `csw-task` skill 的群协作播报（SKILL.md §5）使用。
> 机读副本：`skill/roster.json`（部署时 `$CSW_TASK_ROSTER` 指向它）。
> 飞书 mention 时 open_id 填到 `<at user_id="<open_id>">名字</at>`（lark-im / lark-cli 的 at 属性名为 user_id，值用 open_id）。

## 群

| 项 | 值 |
|---|---|
| 群名称 | CSW编辑部 |
| chat_id | `oc_c7484358fbf366ba1c806e9d65db3103` |
| 群主 | Van（`ou_1fca25427af132224142dd3abdda906d`） |
| 创建 | 2026-05-19T09:13:28Z |
| 模式 / 状态 | DEFAULT / normal |

## 人类用户

| 角色 | user_id | open_id | 说明 |
|---|---|---|---|
| Van | `ag83acf1` | `ou_1fca25427af132224142dd3abdda906d` | 人类终审、群主；**仅主编可 @** |

## 机器人（群内 8 个 bot）

| 群内名称 | open_id |
|---|---|
| 主编 | `ou_e8b7e1e4076dbfbaf88c55078794b5eb` |
| 选题研究员 | `ou_7d2e525dff82f92ab89741f92613353c` |
| 深度内容创作者 | `ou_98ef567d4e26d4d2ef9279da5304a758` |
| 小红书图文作者 | `ou_003deac0bece81514fca9cd9ba081139` |
| 合规版权审查员 | `ou_5e4a9f33f31701a5dd674db4c9daa2dc` |
| 发布运营员 | `ou_2a362b7337a9a96717ef3c1f4ecd51fe` |
| 数据复盘师 | `ou_7b9ba1d4d09cfbee3ba3f89005db434f` |
| 视觉设计师 | `ou_0d21381c313e0fa02be7e68d45c6f96e` |

## → 引擎「资讯日更」role_code 映射现状

⚠️ **群成员是按一套与当前 seed「资讯日更」六角色不同的角色集建的**，并非一一对应：

| 引擎角色 (role_code) | 群 bot | open_id | 状态 |
|---|---|---|---|
| `editor` 主编 | 主编 | `ou_e8b7…` | ✅ 直接对应 |
| `researcher` 选题研究员 | 选题研究员 | `ou_7d2e…` | ✅ 直接对应 |
| `writer` 文案 | 深度内容创作者 | `ou_98ef…` | ✅ 指定承担（bot 群名「深度内容创作者」，担任 writer：03 公众号正文 + 07 小红书文本） |
| `designer` 设计师 | 视觉设计师 | `ou_0d21…` | ✅ 指定承担（视觉设计师＝设计师） |
| `publisher` 发布员 | 发布运营员 | `ou_2a36…` | ✅ 指定承担（发布运营员＝发布员） |
| `van` Van | Van（人类） | `ou_1fca…` | ✅ 直接对应 |
| `collector` 情报收集员 | ——（待补充） | —— | ⏳ 用户稍后补充承接 bot；补齐前 01-采集 无人 |
| `scheduler` 调度器 | ——（可不在群） | —— | — |

**预留（暂不接入本工作流的 bot）**：小红书图文作者、合规版权审查员、数据复盘师。

> 对齐决策（2026-06）：`writer` ← 深度内容创作者、`designer` ← 视觉设计师、`publisher` ← 发布运营员；`collector` 待补充承接 bot；其余预留（见 `skill/roster.json` 的 `reserved_bots`）。**补齐 `collector` 前，01-采集 的派工/交付播报 @ 不到人。**
> 注：以上为花名册（引擎角色↔bot）映射；如需把飞书群里 bot 的**显示名**也改为「文案 / 设计师」等，需在飞书开放平台改 bot 名称（非本仓库改动）。
