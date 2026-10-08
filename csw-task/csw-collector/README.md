# csw-collector · 情报收集员工作台采集服务

Rust 工作区。接管资讯日更流程的 **01 情报逐条**、**05 配图与素材核**、**11 小红书选图包**，
替换现有的 Hermes 情报收集员 agent。

- 做成什么样：`docs/情报收集员工作台_总方案_20260921.md`
- 怎么做、什么顺序：`docs/情报收集员工作台_实施计划_20260922.md`
- 现在做到哪：`docs/情报收集员工作台_实施进度.md` ← **接续先读这页**
- 上线后怎么盯着它跑：`docs/情报收集员工作台_运维手册.md`

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

## 跑一轮看看（不写引擎）

```bash
csw-collector run --manual --days 1     # 当场跑完就退出，结果只写本地库
```

**常驻服务在跑的时候它会拒绝**：两个进程同时对着一个库干活会互相拆台
（CLI 一启动就会把 serve 正在跑的步标成中断，而 SQLite 是单写者）。
那时候用工作台的「开始一轮」——它排进队列，由常驻循环去做。

## 配置

优先级 **env > 配置文件 > 内置默认**（与 `csw-task-svc` 同口径）。
**密钥只从 env 取**，不写入配置文件、不进日志。本地实际配置 `collector.toml` /
`collector.env` 已在 `.gitignore` 里。

模型网关只用 `csw-subapi.833233.xyz`。

**高清原图（`[hires]`）**：来源库（Bright Data）2026-06 起只存 Instagram 图片的 640px 档。
05 / 11 开工时按 shortcode 调 `hires-service`（默认 `https://hires.aworld.ltd:9877`，
token 走 env `HIRES_TOKEN`），拿原帖每张图的最大一档换进包，按轮播序号与 640 图对应、
逐张做画面校验（dHash），来历写进「高清原图溯源」与 `trace/hires.json`。
`required = true`（默认）时取不到就整条报失败、原因原样带出（会话失效 / 超限 / 帖子已删 /
张数不一致），不交 640 图冒充；应急才关 `required`。没有 `HIRES_TOKEN` 时自动当没配，
自检明写「高清原图未取」。服务本身在 Mac mini 上（`~/project/hires-service`）。

**杂志背景库（`[magazine]`）**：刊译台（csw-kb）把裁图与整页上传 OSS 后，每本写
`{dir}/{book_key}/manifest.jsonl`（默认 `dir = /data/kanyitai/output`，同在 centos9，只走本机文件；
契约见 `docs/情报收集员工作台_杂志清单契约.md`）。`kb sync` 与定时同步扫这个目录：一张有译文的
裁图一条 `magazine_item`，按本对账（清单里没有的该书条目删掉），入库后写回 `ingested.json`。
杂志**不进判断**——品牌路、全文路、向量路、补位都只看参考库四类。

```bash
csw-collector kb import-magazine /data/kanyitai/output/GO-OUT-2016.06   # 入库一本（不看清单变没变）
csw-collector kb purge --book GO-OUT-2016.06                           # 清掉一本的条目与向量（共用的图向量保留）
```

融合 / 纯图向量由回填任务另算（`fused_batch` / `image_batch` / `image_side`）。

`CSW_COLLECTOR_ALERT_WEBHOOK` 也只从 env 走：它是一个**谁拿到都能往群里发消息**
的地址。不填＝告警关着，只进日志。这个群是给运维看的，不是编辑部群——
流程上的事由引擎播报，这里只发这个服务自己的毛病（磁盘、outbox 冲突、
定时的活没跑成）。
