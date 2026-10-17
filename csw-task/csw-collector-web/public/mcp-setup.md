# 营事编集室知识库 MCP（csw-kb）接入说明

> 这份说明写给**智能体**（codex、Claude Code 等）照着执行。读完按第 2–5 步装好，
> 然后向用户要令牌，填进环境变量 `CSW_KB_TOKEN` 即可使用。
> 地址：`https://collector.aworld.ltd/mcp`（Streamable HTTP，Bearer 令牌，只读）。

## 0. 执行规则（必须遵守）

- **令牌是密钥。** 只从用户那里要，只写进环境变量 `CSW_KB_TOKEN`。
  - 不要写进 `config.toml`、`.mcp.json` 或任何会提交到仓库的文件。
  - 不要在输出里回显令牌，只说「已配置」。
- 令牌形如 `cswkb_` 加 32 位字母数字。用户还没给，就停下来问用户要，不要猜，也不要自己造。
- 改配置文件之前先读一遍：已经有 `csw-kb` 就更新，不要重复追加。
- 只读服务：没有任何会改数据的工具。

## 1. 它能查什么

| 工具 | 用途 | 主要参数 |
|---|---|---|
| `kb_magazine_search` | 杂志背景库：GO OUT 等日文户外杂志的中文译文、商品、裁图，按文字检索 | `query`（必填），`book_key`、`limit`（≤12） |
| `kb_image_search` | 以图搜图：在杂志裁图里找外观相近的 | 图三选一：`image_url`（公网 https）、`upload_id`（本机图片，见第 8 节）、`image_base64`（只适合很小的图）；`limit` |
| `kb_search` | 判断参考库：已发布条目、范例、成文贴文、03 决定 | `query`，`limit` |
| `kb_similar_selected` | 只看被采用过的相似条目 | `query`，`limit` |

令牌的权限决定能看到哪几件。只有「杂志背景」权限的令牌只列前两件，这是正常的。
杂志条目是日文杂志的译文，**只作背景参照，不是编辑部的选题口径**。

## 2. 前置检查

```bash
codex --version        # 需要 0.149 或更新（支持 --url 与 --bearer-token-env-var）
curl --version
```

Claude Code 或其他 MCP 客户端跳到第 4 步。

## 3. codex 的配置

先执行：

```bash
codex mcp add csw-kb --url https://collector.aworld.ltd/mcp --bearer-token-env-var CSW_KB_TOKEN
```

再在 `~/.codex/config.toml` 里补上免确认那几行。用了 `CODEX_HOME` 就改 `$CODEX_HOME/config.toml`。
codex 默认每次调 MCP 工具都要人点同意，非交互运行（`codex exec`）时会直接失败。这几件都只读，可以放行：

```toml
[mcp_servers.csw-kb]
url = "https://collector.aworld.ltd/mcp"
bearer_token_env_var = "CSW_KB_TOKEN"

[mcp_servers.csw-kb.tools.kb_magazine_search]
approval_mode = "approve"
[mcp_servers.csw-kb.tools.kb_image_search]
approval_mode = "approve"
[mcp_servers.csw-kb.tools.kb_search]
approval_mode = "approve"
[mcp_servers.csw-kb.tools.kb_similar_selected]
approval_mode = "approve"
```

前三行 `codex mcp add` 已经写好了，不要重复写同名的表，否则 TOML 会报重复键。

## 4. 其他客户端

**Claude Code**（`--scope user` 对所有项目生效）：

```bash
claude mcp add --scope user --transport http csw-kb https://collector.aworld.ltd/mcp \
  --header 'Authorization: Bearer ${CSW_KB_TOKEN}'
```

注意 `${CSW_KB_TOKEN}` 两边用**单引号**，这样写进配置的是变量名，不是令牌本身，运行时才展开。

**通用 MCP 客户端**：
- 传输：Streamable HTTP。
- URL：`https://collector.aworld.ltd/mcp`。
- 请求头：`Authorization: Bearer <令牌>`，令牌从环境变量 `CSW_KB_TOKEN` 读。
- 服务端不发会话 ID，只回 JSON，不提供 GET 推送流。

## 5. 设置令牌（向用户要到令牌之后）

让令牌在**启动客户端的那个环境**里可见。

macOS / Linux：先看用户的 shell（`echo $SHELL`），zsh 写 `~/.zshrc`，bash 写 `~/.bashrc`。已经有这一行就替换，没有就追加：

```bash
export CSW_KB_TOKEN='cswkb_…'
```

Windows（PowerShell），设好后要新开终端才生效：

```powershell
setx CSW_KB_TOKEN "cswkb_…"
```

然后**重开终端，再重启 codex 或 Claude Code**。已经在跑的进程读不到新的环境变量。

## 6. 验证

```bash
curl -sS https://collector.aworld.ltd/mcp \
  -H "Authorization: Bearer $CSW_KB_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

- 返回 JSON 且 `result.tools` 里有工具名，就是通了。
- 再跑 `codex mcp list`（或 `claude mcp list`），应能看到 `csw-kb`。
- 最后在客户端里试一次：「用 csw-kb 的 kb_magazine_search 查 snow peak 焚き火台」。

## 7. 出错对照

| 现象 | 原因与处理 |
|---|---|
| HTTP 401 | 令牌没设、设错、已吊销或已过期。先确认 `echo ${CSW_KB_TOKEN:0:6}` 输出 `cswkb_`，再找用户确认令牌 |
| 工具调用报「requires approval」 | 第 3 步的 `approval_mode` 没配 |
| 工具列表里少了几件 | 令牌权限只开了一部分，属正常；要更多权限请用户找管理员重签 |
| HTTP 429 | 超出限速：每枚令牌每分钟 60 次，其中以图搜图 6 次。按 `Retry-After` 等一会儿再试 |
| 以图搜图回「正式轮时段暂停」 | 每天北京时间约 05:55–07:00 让 GPU 给编辑部正式轮，过了这段再试；文字检索照常可用 |
| `image_url 被拒` | 只收 https 公网地址，内网和本机地址会被拦；改用第 8 节的上传 |
| `upload_id … 不存在或已过期` | 上传 10 分钟有效，且只认同一枚令牌上传的；重新上传一次 |
| 上传回 400 | 请求体不是 jpg / png / webp，或忘了 `--data-binary @` 的 `@` |
| HTTP 403 | 请求带了浏览器的 `Origin` 头；从命令行或客户端调用不会带 |

## 8. 用本机图片以图搜图

MCP 的工具参数是模型写出来的，没法把本机文件直接塞进去。所以分两步走，**第一步用 shell 执行**：

```bash
# 1) 上传：请求体就是图片本身（jpg / png / webp，≤10MB）
curl -sS -X POST https://collector.aworld.ltd/mcp/upload \
  -H "Authorization: Bearer $CSW_KB_TOKEN" \
  -H 'Content-Type: application/octet-stream' \
  --data-binary @/path/to/图片.jpg
# 返回 {"upload_id":"up_…","bytes":…,"format":"jpeg","expires_in_secs":600}
```

```text
2) 调 csw-kb 的 kb_image_search，参数 {"upload_id": "up_…", "limit": 6}
```

- `upload_id` 10 分钟内有效，只能用同一枚令牌；同一枚令牌同时最多留 5 张。
- 图片有公网 https 地址的话，直接传 `image_url`，不用上传。
- 不要把图片转成 base64 填进工具参数：图稍大一点就要写几十万个字符。
