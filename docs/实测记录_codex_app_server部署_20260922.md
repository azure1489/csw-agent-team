# 实测记录：codex app-server 在 agent 主机的部署验证

日期：2026-09-22｜执行：开发｜主机：agent 主机 centos9 8.138.23.218｜对应：总方案 M0 第一项

**结论：十项验证全部通过，可以继续。** codex 0.155.1 在 agent 主机上跑通了深核需要的全部能力——建线程、真实回合、结构化输出、本地图片输入、只读沙箱、MCP 只读白名单工具调用。同时纠正了方案里的六处事实错误，其中两处会直接导致实现失败（模型网关配对、异步回合语义）。

---

## 1. 装了什么

| 项 | 值 |
|---|---|
| 版本 | `codex-cli 0.155.1`（npm 最新稳定版；开发机是 0.149.0，太旧） |
| 安装方式 | `npm install -g @openai/codex@0.155.1`，34 秒 |
| 落地路径 | `/root/.hermes/node/bin/codex`（**与 Hermes 共用 node 环境**，见第 4 节风险） |
| 占用 | 二进制包 354 MB；`CODEX_HOME` 99 MB（含 sessions 与内置 skills） |
| CODEX_HOME | `/opt/csw-collector/codex-home`（独立，不碰 Hermes） |
| 主机余量 | 磁盘 135 GB 可用（85% 已用，测试前后无变化）；内存 8 GB 可用 |

Hermes 的 16 个进程（主网关 PID 2199 与八个 profile 网关）全程未受影响。

## 2. 验证结果

| # | 项目 | 结果 | 用时 | 说明 |
|---|---|---|---|---|
| 1 | 握手 `initialize` | 通过 | 即时 | 返回 userAgent 与 codexHome |
| 2 | MCP 挂载 | 通过 | 首次约 60 秒 | `csw-claude-plugin` v0.1.0，经 npx 从 GitHub 拉起 |
| 3 | 只读白名单 | 通过 | —— | `enabled_tools` 生效，只暴露 9 个只读工具，推送类（`csw_articles_push_wechat`）完全不可见 |
| 4 | 建线程 `thread/start` | 通过 | 即时 | `sandbox=read-only` + `approvalPolicy=never` |
| 5 | 真实回合（模型通路） | 通过 | 6.9 秒 | 返回「收到」 |
| 6 | 无模型元数据警告 | 通过 | —— | `gpt-6-astra` 是 codex 原生支持的模型 |
| 7 | 结构化输出 `outputSchema` | 通过 | 4.8 秒 | 返回合 schema 的 JSON，见下 |
| 8 | 本地图片输入 | 通过 | 4.2 秒 | 准确描述了测试图内容 |
| 9 | 只读沙箱 | 通过 | 12.1 秒 | `touch` 被拒：`code: 30, kind: ReadOnlyFilesystem` |
| 10 | MCP 工具真实调用 | 通过 | 18.8 秒 | 查到真实数据：前 3 个账号 omnium、adidas Outdoor、miyagen，**账号总数 543** |

**结构化输出的实际返回**（判断步骤要用的形态）：

```json
{"reason":"仅更新配色，缺乏产品创新或其他实质信息，情报价值有限。","tier":"not_recommend"}
```

判断本身也对：「只有新配色」正是 Van 的七类降低优先级的第一条。

**只读沙箱的实际拦截**：

```
code: 30, kind: ReadOnlyFilesystem, message: "Read-only file system"
touch: 无法创建 '/tmp/codex_sandbox_probe_file': 只读文件系统
```

模型如实回报「被拒绝：文件系统只读，无法创建文件」。**bwrap 0.4.1 在 root 下工作正常，没有崩溃**——方案第 12 节里「root 下 bwrap 不可用时会直接崩溃」的担忧不成立。

## 3. 生效的配置

`/opt/csw-collector/codex-home/config.toml`（密钥不写进文件，从进程环境取）：

```toml
model = "gpt-6-astra"
model_provider = "sub2api"
web_search = "disabled"

[model_providers.sub2api]
name = "sub2api"
base_url = "https://csw-subapi.833233.xyz/v1"
env_key = "SUB2API_API_KEY"
wire_api = "responses"

[mcp_servers.csw]
command = "/root/.hermes/node/bin/npx"
args = ["-y", "--allow-git=all", "github:azure1489/csw-claude-plugin"]
env_vars = ["CSW_API_KEY", "CSW_API_URL"]
default_tools_approval_mode = "approve"
enabled_tools = ["csw_posts_search", "csw_posts_get", "csw_accounts_list", "csw_posts_window",
  "csw_user_selections_list", "csw_user_selections_count",
  "csw_references_list", "csw_references_get", "csw_articles_list", "csw_articles_get"]
```

凭据来源：`/root/.hermes/profiles/csw-agent/.env` 的 `SUB2API_API_KEY`、`CSW_API_KEY`、`CSW_API_URL`。测试脚本 `/tmp/appserver_full.py`、日志 `/tmp/appserver_full.log` 留在主机上作凭证。

## 4. 纠正方案里的六处事实

### 4.1 模型网关有两套，不可混搭（会直接 401）

| 网关 | 密钥来源 | 模型数 | 有 gpt-6 系列 |
|---|---|---|---|
| **`csw-subapi.833233.xyz`** | `profiles/csw-agent/.env` | 25 | **有**：`gpt-6`、`gpt-6-astra` |
| `subapi.833233.xyz` | `/root/.hermes/.env`（主） | 20 | 无 |

两套各自都能用（单独测均 HTTP 200），但域名与密钥必须配对。测试中用主网关的域名配 profile 的密钥，得到 `401 INVALID_API_KEY`。

**用户 9/22 指定：只用 `csw-subapi.833233.xyz`**，不用另一个。采集服务的环境文件只放这一套。

### 4.2 `gpt-6-astra` 可用，且是 codex 原生支持的模型

方案写「`gpt-6-astra` 在 0.149.0 上被拒，须装较新的固定版本」——0.155.1 上完全正常，且因为是 codex 内置清单里的模型，元数据齐全、**没有警告**。反倒是 `gpt-5.6` 会报 `Model metadata for gpt-5.6 not found. Defaulting to fallback metadata`（功能仍正常，只是退化为默认元数据）。

结论：深核固定用 `gpt-6-astra` + codex 0.155.1。

### 4.3 `turn/start` 是异步的

`turn/start` 的响应立刻返回 `status: "inProgress"`，**不是回合结果**。真正的完成信号是 `turn/completed` 通知。Rust 客户端必须等通知，不能等响应。

连带的约束：**同一线程上一个回合没结束时，起一个带不同 `outputSchema` 的新回合会被拒**，报 `ActiveTurnOutputSchemaMismatch`。判断步骤若要并行，必须一条线程一个回合，或改用直连接口。

### 4.4 `threadId` 在 `result.thread.id`

不是 `result.threadId`。返回的 `thread` 对象还带 `sessionId`、`historyMode`、`modelProvider`、`model`、`status`、`path` 等。

### 4.5 分页历史模式不再阻碍建线程

方案写「分页历史模式目前不能创建线程，用它做审计的写法作废」。0.155.1 上线程默认就是 `historyMode: "paginated"` 且创建正常。这一条限制已解除，但审计仍按原方案落盘 `item/completed` 事件流。

### 4.6 csw 后端域名与账号数

- API 域名是 **`agent-api.campsomewhere.com`**（方案多处写成 `agent.campsomewhere.com`，那是前端域名）。
- 在册账号 **543 个**（方案写 460，已过时）。

## 5. 实测的性能与成本底数

| 项 | 数值 |
|---|---|
| 每回合底座上下文（gpt-6-astra，挂 9 个 MCP 工具） | 约 18.7k token |
| 每回合底座上下文（gpt-5.6，不挂 MCP） | 约 11k token |
| 带命令执行的回合 | 增量约 38k token |
| 带 MCP 工具调用的回合 | 增量约 61k token |
| 纯文本回合 | 4–7 秒 |
| 图片回合 | 4.2 秒 |
| 带工具的回合 | 12–19 秒 |

**挂 MCP 工具会让每回合底座从 11k 涨到 18.7k**（9 个工具的 schema）。深核线程要控制挂载的工具数量，只挂当前线程真正需要的。

深核首批 4–8 条、10 分钟的时间盒，按每条 15–20 秒算是够的，但要留出 MCP 首次拉起的约 60 秒。

## 6. 协议要点（给 Rust 客户端）

```
initialize                        → 响应即可用
initialized（通知）
mcpServerStatus/list              → data[].tools 是工具字典
thread/start {model, modelProvider, cwd, sandbox, approvalPolicy,
              developerInstructions, config}
                                  → result.thread.id
turn/start {threadId, input[], outputSchema?}
                                  → 立刻返回 inProgress，等 turn/completed 通知
turn/interrupt {threadId}         → 打断
```

- 图片输入：`{"type": "localImage", "path": "/绝对路径"}`，与文本项同列在 `input` 数组里。
- 事件流：`thread/started` → `turn/started` → `item/started` / `item/completed`（userMessage、reasoning、agentMessage、commandExecution）→ `thread/tokenUsage/updated` → `thread/status/changed` → `turn/completed`。增量事件 `item/agentMessage/delta`、`item/reasoning/summaryTextDelta` 量很大，落盘时要过滤。
- 服务端请求（审批类 `*RequestApproval`、`elicitation`、`RequestUserInput`）必须有处理器并回 `decline`，否则请求挂住。实测 `approvalPolicy=never` + `sandbox=read-only` + MCP 侧 `default_tools_approval_mode=approve` 的组合下，命令被沙箱直接拒绝，不产生审批请求。
- `thread/start` 可传 `config` 覆盖模型元数据（如 `model_context_window`），但只对建线程生效，每个 turn 仍会按全局配置检查。

## 7. 待办与风险

| # | 事项 | 处理 |
|---|---|---|
| 1 | **codex 装在 `/root/.hermes/node/bin/`**，与 Hermes 共用 node | 生产部署要独立安装到 `/opt/csw-collector/`，避免 Hermes 升级 node 时连带影响；切换后停 Hermes 也不能影响 codex |
| 2 | MCP 经 `npx` 从 GitHub 拉，首次约 60 秒且依赖外网 | 生产按原方案本地固定安装到 `/opt/csw-collector/csw-mcp`，锁定提交 |
| 3 | 每回合底座 18.7k token | 深核只挂必要工具；按线程数与回合数估算费用 |
| 4 | 同线程不能并发带不同 schema 的回合 | 判断步骤若走 app-server 要一条线程一个回合；M0 的「两种接口对照」仍要做直连 Responses 接口的对比 |
| 5 | `CODEX_HOME` 已占 99 MB，含 sessions | 设保留期，随采集服务的清理任务一起处理 |
| 6 | 测试脚本与日志留在主机 `/tmp` | 已保留作凭证，可随时删 |

## 8. 下一步

M0 的这一项已完成。余下项：

1. sub2api × Codex 的兼容性——**已顺带验证通过**（`wire_api = "responses"`，真实回合、工具调用、结构化输出都通）。
2. 逐条判断两种接口的对照实验（直连 Responses 接口 vs app-server 线程）——受第 7 节第 4 条影响，直连很可能更适合批量判断。
3. Rust + LanceDB 四项：codex client crate 跑通深核线程、jieba 切中文品牌名命中率、Qwen3-VL 向量批量入库速度、LanceDB 目录放 OSS。
4. Jev 初评在更多样本上回测。
