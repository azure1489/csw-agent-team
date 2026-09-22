//! 采集服务进程：HTTP 工作台 + 编排 + CLI。
//!
//! 子命令与实施计划一一对应：
//!   serve                 常驻：任务驱动轮询 + 工作台 HTTP + 进程内定时
//!   run --manual          手动开启一轮（不关联任务）
//!   replay                回放一轮（录制回放层，不出网、不花模型钱）
//!   kb sync | kb import   知识库同步 / 导入
//!   mcp                   以 stdio 方式跑本地 MCP 服务
//!   p5                    历史反馈回收（离线，只读打开会话库，每次运行单独取得同意）
//!   xcheck                依赖自检（阶段 0.2：在目标主机上验证重依赖真能跑）
//!   auth-probe            登录转发自检（阶段 0.6：对一台真引擎跑通 login/refresh/me/CSRF）
//!   schema                导出契约的 JSON Schema，给前端生成 TS 类型
//!   m1                    M1 验收：对真数据跑一遍采集媒体信息
//!   m2                    M2 验收：知识库检索命中率

mod authprobe;
mod bff;
mod codexprobe;
mod kb;
mod m1;
mod m2;
mod mcp;
mod p5;
mod p5_distill;
mod schema;
mod serve;
mod xcheck;

use clap::{Parser, Subcommand};

/// 向量维度。与 Qwen3-VL 对齐；**改它要重建向量库**（见 `kb::vectors`）。
pub const EMBED_DIM: i32 = 2048;

#[derive(Parser)]
#[command(name = "csw-collector", version, about = "情报收集员工作台 · 采集服务")]
struct Cli {
    /// 配置文件路径（env > 文件 > 默认；密钥只从 env 取）
    #[arg(long, env = "CSW_COLLECTOR_CONFIG", global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 常驻服务：任务驱动 + 工作台 + 定时
    Serve,
    /// 手动跑一轮，跑完就退出（只写本地，不写引擎）
    Run {
        /// 必须显式给。CLI 只开手动轮——派单轮由 `serve` 接单开，
        /// 那条路上还有 ack 心跳、登记与提交，不是命令行能代劳的。
        #[arg(long)]
        manual: bool,
        /// 往回取几天
        #[arg(long, default_value_t = 1)]
        days: i64,
    },
    /// 回放一轮（不出网）
    Replay {
        /// 夹具目录
        #[arg(long)]
        fixtures: String,
    },
    /// 知识库
    #[command(subcommand)]
    Kb(KbCommand),
    /// 以 stdio 方式跑本地 MCP 服务
    Mcp,
    /// 历史反馈回收（离线只读；**每次运行须单独取得同意**）
    ///
    /// 不给 `--who` 就是探查模式：只列出各库里有哪些人、各说了多少条，
    /// 一条消息内容都不读。认出是哪几个 id 之后再用 `--who` 抽。
    P5 {
        /// 扫哪个目录下的 profile
        #[arg(long, default_value = "/root/.hermes/profiles")]
        profiles_dir: String,
        /// 只抽这几个人的话（open_id）。**按应用隔离，一个人在每个 profile 里都不同**，
        /// 所以通常要给好几个。不给就是探查模式。
        #[arg(long)]
        who: Vec<String>,
        /// 从哪天起（`YYYY-MM-DD` 或 RFC3339）
        #[arg(long, default_value = "2026-06-01")]
        since: String,
        /// 一条原话前后各带几条上下文
        #[arg(long, default_value_t = 3)]
        context: usize,
        /// 产物写到哪
        #[arg(long, default_value = "/tmp/p5")]
        out: String,
    },
    /// P5 第二步：把第一步捞出来的原话分类、归纳成准则卡草稿。
    ///
    /// **这一步要把她的原话送给模型（第三方）。** 不给
    /// `--confirm-send-to-model` 就只干跑：算清要送什么、打印出来，
    /// 一个字节都不发。
    P5Distill {
        /// 第一步产出的 quotes.jsonl
        #[arg(long, default_value = "/tmp/p5/quotes.jsonl")]
        quotes: String,
        /// 产物写到哪
        #[arg(long, default_value = "/tmp/p5")]
        out: String,
        /// 确认把原话送模型。**不给就只干跑**
        #[arg(long)]
        confirm_send_to_model: bool,
        /// 只处理前几条（试水）。0 = 全部
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// M1 验收：对真数据跑一遍「采集媒体信息」
    M1 {
        /// 窗口起点（含），RFC3339 UTC
        #[arg(long)]
        from: String,
        /// 窗口终点（不含）
        #[arg(long)]
        to: String,
        /// 对前 N 条跑下载识别向量化。0 = 只取数。
        #[arg(long, default_value_t = 8)]
        prepare: usize,
        /// 全量跑完四段。要花模型与 GPU，一期约 42 分钟。
        #[arg(long)]
        full: bool,
        /// Van 本期补的短码，逗号分隔
        #[arg(long, default_value = "")]
        van: String,
    },
    /// M2 验收：知识库检索命中率（历史决定召回 + 已发条目自检）
    M2 {
        /// 往回看几天
        #[arg(long, default_value_t = 30)]
        days: i64,
        /// 每组最多抽几条。0 = 全部
        #[arg(long, default_value_t = 0)]
        sample: usize,
        /// 跑重排、报终选命中。占 GPU，每条约 3.5 秒
        #[arg(long)]
        rerank: bool,
        /// 本地候选里没有原帖时不向 csw 取
        #[arg(long)]
        no_fetch: bool,
        /// 明细报告写到哪
        #[arg(long, default_value = "/tmp/m2.md")]
        out: String,
    },
    /// 导出契约 JSON Schema（阶段 1 的契约冻结产物）
    Schema {
        /// 写到哪；不给就打到标准输出
        #[arg(short, long)]
        out: Option<String>,
    },
    /// 登录转发自检：起一个 BFF，对真引擎跑通 login / refresh / me / logout / CSRF
    AuthProbe {
        /// 引擎管理后台地址，如 http://127.0.0.1:8081
        #[arg(long)]
        engine: String,
        #[arg(long)]
        username: String,
        /// 口令。只在自检里用，不进日志；生产由浏览器直接提交。
        #[arg(long, env = "CSW_PROBE_PASSWORD")]
        password: String,
        /// 把这个用户名当 van（验证角色映射）
        #[arg(long)]
        van: Option<String>,
    },
    /// 深核客户端自检：对真 codex app-server 跑通握手、建线程、一个回合
    CodexProbe {
        #[arg(long, default_value = "/opt/csw-collector/bin/codex")]
        bin: String,
        #[arg(long, default_value = "/opt/csw-collector/codex-home")]
        home: String,
        /// codex 要一个合法的 cwd。深核不写文件。
        #[arg(long, default_value = "/tmp")]
        cwd: String,
        #[arg(long, default_value = "gpt-6-astra")]
        model: String,
        #[arg(long, default_value_t = 120)]
        budget_secs: u64,
        /// 附一张本地图片，验证 localImage 那条路
        #[arg(long)]
        image: Option<String>,
        /// 放行给 codex 的环境变量名，逗号分隔。默认只放模型网关的密钥。
        #[arg(long, default_value = "SUB2API_API_KEY")]
        env: String,
    },
    /// 依赖自检：SQLite、LanceDB、TLS、axum 各跑一遍
    Xcheck {
        /// TLS 握手与根证书链的验证目标（任何 HTTP 状态都算通过）
        #[arg(long, default_value = "https://agent-api.campsomewhere.com/")]
        http_url: String,
        /// 向量维度，与 Qwen3-VL 对齐
        #[arg(long, default_value_t = 2048)]
        dim: i32,
        /// 写入行数
        #[arg(long, default_value_t = 200)]
        rows: usize,
    },
}

#[derive(Subcommand)]
enum KbCommand {
    /// 从引擎与 csw 增量同步
    Sync {
        /// 全量重拉，不用重叠窗口。第一次建库不必加——没有游标时本来就是全量。
        #[arg(long)]
        full: bool,
        /// 这一轮最多算多少条向量。0 = 不限。白天补跑时限一下，免得和正式轮抢 GPU。
        #[arg(long, default_value_t = 0)]
        embed_limit: usize,
        /// 重建别名表。默认只在表空时建——它包含人工加的别名，重建会冲掉。
        #[arg(long)]
        refresh_brands: bool,
    },
    /// 从本地文件导入范例（JSONL，格式同引擎侧 `syncer kb-import`）
    Import {
        #[arg(long)]
        path: String,
    },
    /// 待补录清单：进过选题却不在 csw 在册名单里的品牌（只读）
    Backfill {
        /// 只列前几行。0 = 全部
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            // lance / datafusion 的 INFO 每条记录都刷屏，默认压到 warn；
            // 要看细节用 RUST_LOG 覆盖。
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "info,lance=warn,lance_index=warn,lance_core=warn,lance_io=warn,\
                 lance_encoding=warn,lance_table=warn,datafusion=warn"
                    .into()
            }),
        )
        .init();

    // rustls 走 ring（aws-lc-rs 要 cmake + nasm，交叉编译过不去）。
    // 各个客户端构造函数里也会兜底调一次，这里显式装是为了让启动顺序一眼可见。
    csw_collector_core::ensure_crypto_provider();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve => {
            let cfg =
                csw_collector_core::Config::load(cli.config.as_deref().map(std::path::Path::new))?;
            let secrets = csw_collector_core::Secrets::from_env();
            let missing = secrets.missing(cfg.jev.enabled);
            anyhow::ensure!(missing.is_empty(), "缺环境变量：{}", missing.join("、"));
            serve::run(&cfg, &secrets).await
        }
        Command::Run { manual, days } => {
            anyhow::ensure!(
                manual,
                "CLI 只能开手动轮，请加 --manual。\n\
                 派单轮由 `serve` 接单开——那条路上还有 ack 心跳、登记与提交，\n\
                 命令行代劳的话引擎那边会看到一个没人接的单。"
            );
            let cfg =
                csw_collector_core::Config::load(cli.config.as_deref().map(std::path::Path::new))?;
            let secrets = csw_collector_core::Secrets::from_env();
            let missing = secrets.missing(cfg.jev.enabled);
            anyhow::ensure!(missing.is_empty(), "缺环境变量：{}", missing.join("、"));
            serve::run_once(&cfg, &secrets, days).await
        }
        Command::Replay { fixtures } => {
            // 回放层（`core::record`）挂在模型、向量、Jev 三个客户端上，**csw 不在其中**。
            // 所以现在没法做到这个子命令承诺的「不出网」：取候选那一步照样要打 csw。
            // 与其给一个名不副实的命令，不如说清差什么。
            anyhow::bail!(
                "回放还差一截，先别用这个子命令。\n\
                 \n\
                 录制回放层挂在模型、向量、Jev 上（`CSW_COLLECTOR_RECORD_MODE=record|replay`\n\
                 + `CSW_COLLECTOR_FIXTURES=<目录>`，对 `run --manual` 与 `serve` 都生效），\n\
                 但 csw 客户端没挂——取候选那一步照样出网，「不出网」做不到。\n\
                 \n\
                 要真回放，还缺两样之一：给 csw 客户端也挂上回放层，\n\
                 或者让回放从本地已有轮次的候选出发、跳过取数那一步。\n\
                 夹具目录 {fixtures} 里的东西是好的，缺的是这一段。"
            )
        }
        Command::Kb(sub) => {
            let cfg =
                csw_collector_core::Config::load(cli.config.as_deref().map(std::path::Path::new))?;
            match sub {
                KbCommand::Sync {
                    full,
                    embed_limit,
                    refresh_brands,
                } => {
                    let secrets = csw_collector_core::Secrets::from_env();
                    let missing = secrets.missing(false);
                    anyhow::ensure!(missing.is_empty(), "缺环境变量：{}", missing.join("、"));
                    kb::sync(
                        &cfg,
                        &secrets,
                        kb::SyncOpts {
                            full,
                            embed_limit,
                            refresh_brands,
                        },
                    )
                    .await
                }
                KbCommand::Import { path } => kb::import(&cfg, &path),
                KbCommand::Backfill { limit } => kb::backfill(&cfg, limit),
            }
        }
        Command::Mcp => {
            let cfg =
                csw_collector_core::Config::load(cli.config.as_deref().map(std::path::Path::new))?;
            mcp::run(&cfg).await
        }
        Command::P5 {
            profiles_dir,
            who,
            since,
            context,
            out,
        } => p5::run(&p5::Opts {
            profiles_dir: std::path::PathBuf::from(profiles_dir),
            who,
            since,
            context,
            out: std::path::PathBuf::from(out),
        }),
        Command::P5Distill {
            quotes,
            out,
            confirm_send_to_model,
            limit,
        } => {
            // 干跑不需要客户端，也就**不需要密钥**——这条路径上连建都不建
            let model = if confirm_send_to_model {
                let cfg = csw_collector_core::Config::load(
                    cli.config.as_deref().map(std::path::Path::new),
                )?;
                let secrets = csw_collector_core::Secrets::from_env();
                Some(csw_collector_core::model::ModelClient::new(
                    csw_collector_core::model::ModelConfig {
                        base_url: cfg.model.base_url.clone(),
                        api_key: secrets.sub2api_key.clone(),
                        model: cfg.model.model.clone(),
                        fallback_model: cfg.model.fallback_model.clone(),
                        concurrency: 1,
                        timeout: std::time::Duration::from_secs(cfg.model.timeout_secs),
                        max_attempts: 3,
                    },
                )?)
            } else {
                None
            };
            p5_distill::run(
                &p5_distill::Opts {
                    quotes: std::path::PathBuf::from(quotes),
                    out: std::path::PathBuf::from(out),
                    confirm_send_to_model,
                    limit,
                },
                model.as_ref(),
            )
            .await
        }
        Command::M1 {
            from,
            to,
            prepare,
            full,
            van,
        } => {
            let cfg =
                csw_collector_core::Config::load(cli.config.as_deref().map(std::path::Path::new))?;
            let secrets = csw_collector_core::Secrets::from_env();
            let missing = secrets.missing(false);
            anyhow::ensure!(missing.is_empty(), "缺环境变量：{}", missing.join("、"));
            m1::run(
                &cfg,
                &secrets,
                m1::Opts {
                    from,
                    to,
                    prepare,
                    full,
                    van_links: van
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                },
            )
            .await
        }
        Command::M2 {
            days,
            sample,
            rerank,
            no_fetch,
            out,
        } => {
            let cfg =
                csw_collector_core::Config::load(cli.config.as_deref().map(std::path::Path::new))?;
            let secrets = csw_collector_core::Secrets::from_env();
            m2::run(
                &cfg,
                &secrets,
                m2::Opts {
                    days,
                    sample,
                    rerank,
                    fetch: !no_fetch,
                    out: out.into(),
                },
            )
            .await
        }
        Command::Schema { out } => schema::run(out.as_deref()),
        Command::AuthProbe {
            engine,
            username,
            password,
            van,
        } => authprobe::run(&engine, &username, &password, van.as_deref()).await,
        Command::CodexProbe {
            bin,
            home,
            cwd,
            model,
            budget_secs,
            image,
            env,
        } => {
            codexprobe::run(codexprobe::Opts {
                bin: bin.into(),
                home: home.into(),
                cwd: cwd.into(),
                model,
                budget_secs,
                image: image.map(Into::into),
                env: env
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
            .await
        }
        Command::Xcheck {
            http_url,
            dim,
            rows,
        } => {
            xcheck::run(xcheck::Opts {
                http_url,
                dim,
                rows,
            })
            .await
        }
    }
}
