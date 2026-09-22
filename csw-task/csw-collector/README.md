# csw-collector · 情报收集员工作台采集服务

Rust 工作区。接管资讯日更流程的 **01 情报逐条**、**05 配图与素材核**、**11 小红书选图包**，
替换现有的 Hermes 情报收集员 agent。

- 做成什么样：`docs/情报收集员工作台_总方案_20260921.md`
- 怎么做、什么顺序：`docs/情报收集员工作台_实施计划_20260922.md`
- 现在做到哪：`docs/情报收集员工作台_实施进度.md` ← **接续先读这页**

## 九个 crate

| crate | 职责 |
|---|---|
| `core` | 共享类型、配置、本地 SQLite 状态库、录制回放层。不依赖任何业务 crate |
| `engineapi` | 任务流转引擎运行面客户端（接单、心跳、登记、提交） |
| `harvest` | 采集：取候选 → 下载 → 识别 → 向量化（一体，缺一步算没采到） |
| `kb` | 知识库：参考库四类 + 检索（向量 ∪ 品牌命中 ∪ 全文 → rerank） |
| `judge` | 合并、对照、逐条判断（结论档 + 六维 + 三句话，不打分） |
| `deepcheck` | codex app-server 薄 stdio 客户端 + 深核编排 |
| `deliver` | 交付物生成与提交（zip 只构建一次，确定性生成） |
| `mcpsrv` | 本地 MCP 只读服务（kb_search / kb_similar_selected / memory_lookup） |
| `collector` | 可执行文件：HTTP 工作台 + 编排 + CLI |

数据放置：**结构化数据全在 SQLite，LanceDB 只放向量。**

## 命令

```bash
make check        # 提交前必过 = fmt --check + clippy -D warnings + test
make build        # 本机调试构建
make build-linux  # 交叉编译生产二进制（x86_64 glibc 2.34）
```

交叉编译需要 `cargo-zigbuild` + `zig`，构建 LanceDB 需要 `protoc`。工具链由
`rust-toolchain.toml` 钉死——换版本会换 codegen，交付前必须与验证时一致。

## 配置

优先级 **env > 配置文件 > 内置默认**（与 `csw-task-svc` 同口径）。
**密钥只从 env 取**，不写入配置文件、不进日志。本地实际配置 `collector.toml` /
`collector.env` 已在 `.gitignore` 里。

模型网关只用 `csw-subapi.833233.xyz`。

`CSW_COLLECTOR_ALERT_WEBHOOK` 也只从 env 走：它是一个**谁拿到都能往群里发消息**
的地址。不填＝告警关着，只进日志。这个群是给运维看的，不是编辑部群——
流程上的事由引擎播报，这里只发这个服务自己的毛病（磁盘、outbox 冲突、
定时的活没跑成）。
