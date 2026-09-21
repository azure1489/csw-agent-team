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

mod authprobe;
mod bff;
mod schema;
mod xcheck;

use clap::{Parser, Subcommand};

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
    /// 手动开启一轮
    Run {
        /// 不关联引擎任务，只写本地
        #[arg(long)]
        manual: bool,
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
    /// 历史反馈回收（离线；每次运行须单独取得同意）
    P5,
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
    Sync,
    /// 从本地文件导入（范例、决定）
    Import {
        #[arg(long)]
        path: String,
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
        Command::Serve => todo!("阶段 6：任务驱动 + 工作台 HTTP + 进程内定时"),
        Command::Run { .. } => todo!("阶段 6：手动开启一轮"),
        Command::Replay { .. } => todo!("阶段 1.3：录制回放层"),
        Command::Kb(_) => todo!("阶段 3：知识库同步与导入"),
        Command::Mcp => todo!("阶段 5：本地 MCP 服务"),
        Command::P5 => todo!("阶段 8：历史反馈回收"),
        Command::Schema { out } => schema::run(out.as_deref()),
        Command::AuthProbe {
            engine,
            username,
            password,
            van,
        } => authprobe::run(&engine, &username, &password, van.as_deref()).await,
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
