---
name: csw-workflow
version: 1.0.0
description: 营事编集室工作流定义面：按用户（Van 或操作员）的明确意愿修改工作流定义——增删阶段、改依赖、改闸、改派工模式 / 动作类别 / 时限、改阶段作业手册与验收标准。通过 wfctl 导出 YAML、改文件、检查、比对，再以草稿写回管理后台；激活必须由人在终端确认。触发词：改流程、加一个阶段、去掉这道闸、改成手动派工、改验收标准、流程定义、wfctl、工作流版本、回滚流程。
---

# csw-workflow · 按用户意愿修改工作流定义

> 定义层（阶段、依赖、闸、派工模式、动作类别、时限、三段文本）可以按用户意愿改；**引擎护栏不能改**——授权护栏、幂等、状态机、「激活需要人确认」都在代码里，本 skill 碰不到，也不应该尝试绕过。
> 运行中的 run 在触发时已快照定义，改定义只影响激活之后新触发的 run。

## 0. 边界

- 只做定义层能表达的改动。用户要的东西定义表达不了（例如「让某阶段在周末跳过」「让 Van 不经主编直接派工」），如实说明做不到、需要开发，不要用别的办法凑。
- 只听 **Van 与后台操作员**的修改意愿；其他 agent 的建议先转给主编，由 Van 或操作员确认后才改。
- 不签发 token、不改成员、不碰运行面（派工 / 审核 / 授权属 csw-task skill）。
- 不自建或连接引擎副本；地址只来自 `CSW_ADMIN_URL`。

## 1. 前置

| 环境变量 | 含义 |
|---|---|
| `CSW_ADMIN_URL` | 管理后台地址（生产 `https://tasks.aworld.ltd`） |
| `CSW_ADMIN_USER` / `CSW_ADMIN_PASSWORD` | 后台账号（operator 可写；viewer 只能 export / diff / lint / history） |

凭证只在环境变量里，wfctl 不落盘；不要把密码写进文件、群消息或提交记录。

## 2. 命令与退出码

| 命令 | 作用 |
|---|---|
| `wfctl export <key>[@ver] [-o 目录]` | 导出为 YAML（长文本外置为同名目录下的 md）；缺省导出 active |
| `wfctl diff <file> [--against <key>@<ver>]` | 与服务端版本比对（缺省比 active） |
| `wfctl lint <file>` | 本地 + 服务端规则检查 |
| `wfctl push <file> --draft --reason "…"` | 新建草稿版本并整份写入，回读比对；**不激活** |
| `wfctl validate <key>@<ver>` | 服务端十项校验 |
| `wfctl activate <key>@<ver> --reason "…"` | 打印差异 → 终端里手输 `<key>@<ver>` 确认 → 激活 |
| `wfctl history <key>` | 版本与变更记录（含修改原因、变更摘要） |
| `wfctl rollback <key>@<ver> --reason "…"` | 把旧版本复制为新草稿（前滚回退；激活另行确认） |
| `wfctl roles` / `agents` / `roster` | 只读：角色、成员、花名册 |

退出码：`0` 成功 · `1` 失败 · `2` 用法错误或需要人确认 · `3` lint 只有警告。

## 3. YAML 与 file 引用

```yaml
schema: csw-workflow/v1
meta: {wf_key: daily_news, name: 资讯日更, version: 2, hub_role: editor, dispatch_mode: auto, trigger_roles: [editor, scheduler]}
common:
  instructions: {file: daily_news_v2/common.instructions.md}
  acceptance: {file: daily_news_v2/common.acceptance.md}
default_gates: []
stages:
  - code: write
    name: 04-公众号写作
    role: writer
    output_type: 文章
    deps: [topic]
    gates:
      - {reviewer: editor, name: 主编审}
    dispatch_mode: manual          # 空=继承工作流；manual / auto
    sla_minutes: 45
    per_item: true
    instructions: {file: daily_news_v2/write.instructions.md}
    self_check: {file: daily_news_v2/write.self_check.md}
    acceptance: {file: daily_news_v2/write.acceptance.md}
```

- `gates: []` 是显式 0 闸。没有默认闸时**每个阶段都必须写 `gates:`**（lint L1）；有默认闸时无法把单个阶段设为 0 闸。
- `relayed: true` 表示人工闸由中枢代录（Van 的闸都是这种）。
- `action_class`：缺省 read；存草稿、发布这类平台写操作写 `platform_write:wx_draft` / `wx_publish` / `xhs_draft` / `xhs_publish`，这样引擎的授权护栏才会生效。
- 文本写本阶段要做的事，不整篇粘贴文档、不引用「§x.x」（lint L6）；用 `{file: 相对路径}` 引用 md 便于 diff。

## 4. 意图 → 改动（七步，不能跳）

1. **复述操作清单**：把用户的话翻译成具体改动（哪个阶段、改什么字段、从什么到什么），回给用户确认理解无误。有歧义只问一次。
2. **导出基线**：`wfctl export daily_news -o <工作目录>`。
3. **改 YAML**：只改清单里的地方。
4. **检查与比对**：`wfctl lint <file>`（有错误就改到没有；警告逐条向用户说明）→ `wfctl diff <file>`，把差异表和改后的依赖关系（文字画出主链与分支）回复用户。
5. **写草稿**：用户确认差异后，`wfctl push <file> --draft --reason "<用户原话>"`。
6. **校验**：`wfctl validate daily_news@v<N>`，全过才进入下一步。
7. **激活**：只有用户在**当轮**明确说「激活」时，才请用户本人在终端执行 `wfctl activate daily_news@v<N> --reason "<原话>"` 并手输版本号确认。你不能替用户完成这一步，也不能使用 `--yes`。

回退：`wfctl history` 找到要回到的版本 → `wfctl rollback daily_news@v<旧> --reason "<原话>"` 生成新草稿 → 同样走第 6、7 步。

## 5. 常见改动模板

| 用户说 | 改法 |
|---|---|
| 「X 阶段不用我看了」 | 该阶段 `gates` 去掉 `reviewer: van` 那一道；主编闸保留与否再问一次 |
| 「X 做完直接往下走」 | `gates: []`（0 闸），并确认下游是否需要主编抽查 |
| 「X 等主编派工」 | `dispatch_mode: manual` |
| 「存草稿要我同意」 | 该阶段 `action_class: platform_write:wx_draft`（授权护栏由引擎执行） |
| 「X 和 Y 并行」 | 让 Y 的 `deps` 不再包含 X，改依赖同一个上游 |
| 「改 X 的要求 / 验收」 | 改对应 md；只写本阶段要做的事 |

## 6. 护栏

- 激活永远需要人在终端确认；agent 不激活、不用 `--yes`、不绕过校验。
- `--reason` 写用户原话，不编造；它会进审计，事后可查。
- 同一时间只推一个草稿；推之前先 `history` 看有没有别人没激活的草稿，有就先问用户。
- 不硬编码地址、不存凭证、不自建引擎副本、不签发 token。

## 7. 错误处置

| 现象 | 处置 |
|---|---|
| lint 错误 L1 | 给每个阶段补 `gates:`（0 闸写 `gates: []`） |
| lint 错误 V「DAG 无环」「入口可达」 | 依赖改出了环或孤岛，回到第 3 步 |
| lint 警告 L3 | 角色没有活跃成员，告诉用户派工会落空 |
| lint 警告 L4 | 删除的阶段在进行中的 run 里还有任务；旧 run 按快照继续，提醒用户 |
| push 报「回读不一致」 | 服务端丢了字段或文件写错，停下报告，不要反复推 |
| activate 退出码 2 | 需要用户本人在终端确认，这是预期行为 |
| HTTP 401 / 403 | 账号无效或只读（viewer），请操作员处理 |
| HTTP 409 `not_draft` | 只有草稿能激活；旧版本先 rollback 成新草稿 |
