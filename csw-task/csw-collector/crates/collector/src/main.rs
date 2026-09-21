//! 采集服务进程：HTTP 工作台 + 编排 + CLI。
//!
//! 子命令与计划一一对应：
//!   serve                 常驻：任务驱动轮询 + 工作台 HTTP + 进程内定时
//!   run --manual          手动开启一轮（不关联任务）
//!   replay                回放一轮（录制回放层，不出网、不花模型钱）
//!   kb sync | kb import   知识库同步 / 导入
//!   mcp                   以 stdio 方式跑本地 MCP 服务
//!   p5                    历史反馈回收（离线，只读打开会话库，每次运行单独取得同意）

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "csw-collector", version, about = "情报收集员工作台 · 采集服务")]
struct Cli {
    /// 配置文件路径（env > 文件 > 默认；密钥只从 env 取）
    #[arg(long, env = "CSW_COLLECTOR_CONFIG")]
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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve => todo!("阶段 6：任务驱动 + 工作台 HTTP + 进程内定时"),
        Command::Run { .. } => todo!("阶段 6：手动开启一轮"),
        Command::Replay { .. } => todo!("阶段 1.3：录制回放层"),
        Command::Kb(_) => todo!("阶段 3：知识库同步与导入"),
        Command::Mcp => todo!("阶段 5：本地 MCP 服务"),
        Command::P5 => todo!("阶段 8：历史反馈回收"),
    }
}
