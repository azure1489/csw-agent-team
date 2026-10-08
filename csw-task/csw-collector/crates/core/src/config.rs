//! 配置装配：**env > 配置文件 > 内置默认**（与 `csw-task-svc` 同口径）。
//!
//! **密钥只从 env 取，不写进配置文件、不进日志、不进任何错误信息。**
//! 配置文件可以进仓库当范本；`collector.env`（0600）只在服务器上，由 systemd 注入。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 这几个 env 名是密钥，`Debug` 与日志里一律只出现长度。
pub const SECRET_ENV_KEYS: [&str; 5] = [
    "CSW_API_KEY",
    "SUB2API_API_KEY",
    "TYPESAFE_API_KEY",
    "CSW_ENGINE_TOKEN",
    "HIRES_TOKEN",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub data_dir: PathBuf,
    pub listen: String,
    pub engine: Engine,
    pub csw: Csw,
    pub hires: Hires,
    pub vector: Vector,
    pub model: Model,
    pub jev: Jev,
    pub codex: Codex,
    pub schedule: Schedule,
    pub limits: Limits,
    pub features: Features,
    pub web: Web,
    pub alert: Alert,
    pub magazine: Magazine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Engine {
    /// 运行面（接单、登记、提交）
    pub base_url: String,
    /// 管理后台（只用来转发登录，不做流程动作）
    pub admin_url: String,
    /// 轮询派单的间隔
    pub poll_secs: u64,
    /// 接单后多久 ack 一次。引擎没有独立心跳，重复 ack 就是心跳。
    pub ack_secs: u64,
    /// 上传上限。引擎是 64 MiB，留出余量。
    pub max_upload_mib: u64,
    /// 不接、不做的任务号。运营方手工交付期间用（10-06 r63 #699：手工交的版本被退回后，
    /// 工作台 30 秒内就把它当返工接走、跑全池流程报失败，主编只好取消任务）。用完删掉。
    pub hands_off_tasks: Vec<i64>,
}

/// 高清原图服务（`hires-service`）：输入 shortcode，用已登录的 Instagram 会话取每张图的
/// 最大一档并转存 OSS。来源库（Bright Data）2026-06 起只给 640px，05 / 11 的图从这里补。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hires {
    /// 总开关。开着但没有 `HIRES_TOKEN` 时自动关并告警。
    pub enabled: bool,
    pub base_url: String,
    /// 服务是串行的，排队时一条要等前面的做完（每条 ≥ 8 秒）
    pub timeout_secs: u64,
    /// 取不到高清图时整条 05 / 11 报失败（主编要的是「取不到就报真实原因」，不是交 640 冒充）。
    /// 关掉只在应急时用：交 640 包，自检明写「高清未取到」。
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Csw {
    /// 注意是 agent-api 不是 agent（后者是前端域名），路径前缀 /api/v1
    pub base_url: String,
    /// 窗口单页条数。100 条撞上冷启动会碰到 90 秒超时，50 稳。
    pub window_page: u32,
    /// 冷启动第一个请求可达 40 秒
    pub timeout_secs: u64,
    /// 图片缩略宽度。识别与向量化都用它，05 / 11 才取原图。
    pub thumb_width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vector {
    pub base_url: String,
    /// 产出这些向量的模型**标签**。不是发给服务的参数，只用来防一件事：
    /// 换了模型却没重建向量库。不同模型的向量不可比，混在一起检索会悄悄失准
    /// 且不报错。换模型时改这个值，向量库会拒绝打开并提示重建。
    pub embed_model: String,
    /// 22 GB 卡上的安全批量（实测 1500 字正文批 16 就 OOM，且 OOM 会赖着不走）
    pub text_batch: usize,
    pub fused_batch: usize,
    pub image_batch: usize,
    pub rerank_batch: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Model {
    /// 只用 csw-subapi 这一套。域名与密钥必须配对，混搭直接 401。
    pub base_url: String,
    pub model: String,
    /// 同网关内的降级目标。**默认空 = 不降级**：一轮里混进另一个模型的判断，口径就不一致了
    /// （09-27 第 25 轮：三次超时悄悄换成了 gpt-5.6-sol，推荐从 9 条涨到 20 条）。
    /// 超时由同一个模型重试（`max_attempts`），宁可慢一点。
    pub fallback_model: String,
    /// 实测网关在 8 并发上下限流，再高只换来 429
    pub concurrency: usize,
    /// 判断一次发几条候选。批量省三分之二输入 token，吞吐不变。
    pub judge_batch: usize,
    /// 单批延迟实测到过 405 秒，超时不能低于这个数
    pub timeout_secs: u64,
    /// 每日 token 预算，超了熔断
    pub daily_input_budget: u64,
    pub daily_output_budget: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Jev {
    /// 密钥不在就自动关；关掉时初评跳过、窄判断退回生成模型，并在台账标注
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub concurrency: usize,
    /// 同一事件的合并阈值。实测真负例贴在 0.03，0.5 很安全。
    pub same_event_threshold: f64,
    /// 核对依据的阈值。低于它标出来给人看，**不自动淘汰**。
    pub basis_check_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Codex {
    pub bin: PathBuf,
    pub home: PathBuf,
    /// 固定安装的 csw MCP，不走 npx 拉 GitHub
    pub mcp_dir: PathBuf,
    pub model: String,
    /// 一条线程一个回合，所以并行度就是线程数
    pub parallel: usize,
    /// 首批深核几条（推荐的）
    pub first_batch: usize,
    /// 待核的至多深核几条。深核过的不再留在待核（09-25 用户），所以待核的都核：
    /// r53 深核前 25 条待核，15 的上限留下 10 条没核；周一 72 小时窗口（09-28）62 条，40 也不够
    pub pending_cap: usize,
    pub budget_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Schedule {
    /// 预取轮的时刻（UTC）。实测一轮从零跑约 95–100 分钟，
    /// 05:30 接单再从头跑必然超时，所以预取是必需的不是优化。
    pub prefetch_at_utc: String,
    /// 知识库同步（UTC）
    pub kb_sync_at_utc: Vec<String>,
    /// 这段时间内暂停知识库回填，把 GPU 让给正式轮
    pub gpu_quiet_from_utc: String,
    pub gpu_quiet_to_utc: String,
    /// 图片保留天数
    pub media_retention_days: u32,
    /// 向量库备份的时刻（UTC）
    pub backup_at_utc: String,
    /// 隔几天备一次。到点了但离上一份没这么久，就跳过
    pub backup_every_days: u32,
}

/// 杂志背景库：刊译台写的清单（`{dir}/{book_key}/manifest.jsonl`，契约见
/// `docs/情报收集员工作台_杂志清单契约.md`）。两服务同在 centos9，只走本机文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Magazine {
    /// 刊译台产出目录。空 = 不同步杂志。
    pub dir: PathBuf,
    /// 融合向量一批几条（回填任务用）
    pub fused_batch: usize,
    /// 纯图向量一批几张（回填任务用）
    pub image_batch: usize,
    /// 送向量服务前图缩到多宽（px）
    pub image_side: u32,
}

impl Default for Magazine {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("/data/kanyitai/output"),
            fused_batch: 4,
            image_batch: 2,
            image_side: 768,
        }
    }
}

/// 开发群告警。**默认关着**：地址填进来才发。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Alert {
    /// 飞书自定义机器人的 webhook。空＝关。
    /// **这是给运维看的群，不是编辑部群**——流程上的事由引擎播报。
    pub webhook_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub download_concurrency: usize,
    /// 识别一次发几张图
    pub recognize_batch: usize,
    /// rerank 每条候选排几段（送进重排的召回上限）
    pub rerank_per_candidate: usize,
    /// rerank 每段截多少字。重排按字数计费似的线性变慢：P40 上中文 300 字每段
    /// 约 257 ms、200 字约 190 ms（09-23 实测）。**调它先跑 `m2 --rerank` 看命中率**。
    pub rerank_snippet_chars: usize,
    /// 定点补读：每轮最多补读几条候选的外链（0 = 关）。
    ///
    /// 09-23 用户同意补读外网，前提是封内网（`harvest::netguard`）。一条至多 3 个地址、
    /// 每页 15 秒，15 条最坏约 11 分钟；超出整轮预算就先调低它。
    pub refetch_per_round: usize,
    /// 磁盘占用告警与拒开新轮的水位
    pub disk_warn_pct: u8,
    pub disk_block_pct: u8,
}

/// 高风险项：**实现了但默认关闭**，开之前要有人按下去。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Features {
    /// 小红书采集器。与 Hermes 共用登录态 Chrome，要主机级文件锁与验证码熔断，
    /// 且不进 05:30 的关键路径。
    pub xhs_collector: bool,
    /// 网页 / RSS 采集器。解析后钉 IP，封内网与云元数据地址。
    pub web_collector: bool,
    /// 自定义采集器三层（外部命令 / HTTP / MCP）。仅 superadmin 可登记。
    pub custom_collectors: bool,
    /// 把本地 MCP 挂给 Hermes
    pub mcp_for_hermes: bool,
    /// 深核白名单里加 fetch_page / xhs_search
    pub deepcheck_web_tools: bool,
    /// **逐张**算图片向量。默认关。
    ///
    /// 一期 1871 张图对 358 条候选，逐图算占掉向量化 GPU 时间的八成五，
    /// 而它现在没有任何读取路径：合并用融合向量（跨账号转载时平台会重新编码，
    /// 图哈希对不上，所以合并本来就只能走融合向量 + Jev），知识库检索用的也是
    /// 融合向量，`kb::vectors` 那张 `images` 表全仓没人写也没人查。
    ///
    /// 做以图搜图那天打开它，那一段代码不用再改。
    pub image_vectors: bool,
    /// 把 **Van 的原话**送进模型（判断时的决定类材料、深核时的 `memory_lookup`）。
    /// **默认关。**
    ///
    /// 总方案第 536 / 596 行写的是「Van 群聊原话单独拍板」，而这件事一直没拍过板
    /// ——定下来的只有「不送 TypeSafe」与「P5 抽取送 csw-subapi 要单独同意」。
    /// 模型在 csw-subapi，是第三方；在拍板之前，代码不该默认把原话发出去。
    ///
    /// 关着的时候决定类材料只保留「结论」与「理由码」两行（机器词表），
    /// 模型仍然知道「这件事被否过 / 被采用过」，只是看不到她是怎么说的。
    pub send_van_quotes_to_model: bool,
    /// 把 Van **还没确认**的准则卡也送进判断，标「草稿」。**默认开**（09-27 用户拍板：
    /// 补了相似案例后 M3 仍偏宽，先让草稿卡进判断）。卡片是从原话归纳的准则，不是原话本身。
    pub send_draft_rules_to_model: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Web {
    /// 生产必须 true。本机 http 调试才关。
    pub secure_cookie: bool,
    /// 这些 viewer 用户名在工作台里当 van。
    ///
    /// **不配就没有 Van 模式**：引擎里没有 van 这个角色，她登录进来是个普通 viewer，
    /// 看到的是总览页而不是她那一页。
    pub van_usernames: Vec<String>,
}

/// 密钥。只从 env 读，`Debug` 里只露长度。
#[derive(Clone, Default)]
pub struct Secrets {
    pub csw_api_key: String,
    pub sub2api_key: String,
    pub typesafe_key: String,
    pub engine_token: String,
    /// hires-service 的 Bearer token（Mac mini 上 `~/.config/hires/token`）
    pub hires_token: String,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = |s: &String| {
            if s.is_empty() {
                "缺".to_string()
            } else {
                format!("{} 字符", s.len())
            }
        };
        f.debug_struct("Secrets")
            .field("csw_api_key", &n(&self.csw_api_key))
            .field("sub2api_key", &n(&self.sub2api_key))
            .field("typesafe_key", &n(&self.typesafe_key))
            .field("engine_token", &n(&self.engine_token))
            .field("hires_token", &n(&self.hires_token))
            .finish()
    }
}

impl Secrets {
    pub fn from_env() -> Self {
        let g = |k: &str| std::env::var(k).unwrap_or_default();
        Self {
            csw_api_key: g("CSW_API_KEY"),
            sub2api_key: g("SUB2API_API_KEY"),
            typesafe_key: g("TYPESAFE_API_KEY"),
            engine_token: g("CSW_ENGINE_TOKEN"),
            hires_token: g("HIRES_TOKEN"),
        }
    }

    /// 少哪些密钥。启动时报出来，别等第一次调用才炸。
    pub fn missing(&self, jev_enabled: bool) -> Vec<&'static str> {
        let mut m = Vec::new();
        if self.csw_api_key.is_empty() {
            m.push("CSW_API_KEY");
        }
        if self.sub2api_key.is_empty() {
            m.push("SUB2API_API_KEY");
        }
        if self.engine_token.is_empty() {
            m.push("CSW_ENGINE_TOKEN");
        }
        if jev_enabled && self.typesafe_key.is_empty() {
            m.push("TYPESAFE_API_KEY");
        }
        m
    }
}

/// 子结构的默认值都从 [`Config::default`] 里取，免得同一个默认写两遍。
/// serde 的容器级 `default` 需要每层都有 `Default`，所以这里逐个代理过去。
macro_rules! default_from_config {
    ($($ty:ident => $field:ident),+ $(,)?) => {
        $(impl Default for $ty {
            fn default() -> Self { Config::default().$field }
        })+
    };
}
default_from_config!(
    Engine => engine, Csw => csw, Hires => hires, Vector => vector, Model => model, Jev => jev,
    Codex => codex, Schedule => schedule, Limits => limits, Features => features, Web => web,
);

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("/opt/csw-collector/data"),
            listen: "127.0.0.1:8090".into(),
            engine: Engine {
                base_url: "https://tasks.aworld.ltd/api/v1".into(),
                admin_url: "https://tasks.aworld.ltd".into(),
                poll_secs: 30,
                ack_secs: 120,
                max_upload_mib: 48,
                hands_off_tasks: Vec::new(),
            },
            csw: Csw {
                base_url: "https://agent-api.campsomewhere.com".into(),
                window_page: 50,
                timeout_secs: 120,
                thumb_width: 768,
            },
            hires: Hires {
                enabled: true,
                base_url: "https://hires.aworld.ltd:9877".into(),
                timeout_secs: 180,
                required: true,
            },
            vector: Vector {
                base_url: "http://127.0.0.1:8022".into(),
                embed_model: "qwen3-vl@8022".into(),
                text_batch: 8,
                fused_batch: 4,
                image_batch: 2,
                rerank_batch: 24,
                timeout_secs: 120,
            },
            model: Model {
                base_url: "https://csw-subapi.833233.xyz/v1".into(),
                model: "gpt-6-astra".into(),
                fallback_model: String::new(),
                // 0.4 实测单独用时 8 最合适；但网关账号的并发上限与 Hermes 的十几个 agent 共用，
                // 09-23 切换后 8 条就开始撞 429。降到 5，给 Hermes 留余量
                concurrency: 5,
                judge_batch: 6,
                timeout_secs: 420,
                daily_input_budget: 4_000_000,
                daily_output_budget: 1_000_000,
            },
            jev: Jev {
                enabled: true,
                base_url: "https://api.typesafe.ai".into(),
                model: "jev-latest".into(),
                concurrency: 8,
                same_event_threshold: 0.5,
                basis_check_threshold: 0.3,
            },
            codex: Codex {
                bin: PathBuf::from("/opt/csw-collector/codex/bin/codex"),
                home: PathBuf::from("/opt/csw-collector/codex/home"),
                mcp_dir: PathBuf::from("/opt/csw-collector/csw-mcp"),
                model: "gpt-6-astra".into(),
                parallel: 3,
                first_batch: 6,
                pending_cap: 80,
                budget_secs: 600,
            },
            schedule: Schedule {
                prefetch_at_utc: "17:40".into(), // 北京时间次日 01:40
                // 北京 21:00（接住当天发的）与 11:00（接住早上刚发的）。
                // **不能排在 05:30 那一轮前后**：实测紧跟 kb sync 的那次 csw 取数
                // 用了 1044 秒，前三次都是 7 秒。原先第二次排在 21:00 UTC
                // （北京 05:00），离正式轮只有半小时——那正是踩过的坑。
                kb_sync_at_utc: vec!["13:00".into(), "03:00".into()],
                gpu_quiet_from_utc: "21:25".into(), // 北京 05:25
                gpu_quiet_to_utc: "22:30".into(),   // 北京 06:30
                media_retention_days: 120,
                backup_at_utc: "03:30".into(),
                backup_every_days: 7,
            },
            limits: Limits {
                download_concurrency: 8,
                recognize_batch: 6,
                rerank_per_candidate: 24,
                rerank_snippet_chars: 300,
                refetch_per_round: 15,
                disk_warn_pct: 88,
                disk_block_pct: 92,
            },
            features: Features {
                xhs_collector: false,
                web_collector: false,
                image_vectors: false,
                custom_collectors: false,
                mcp_for_hermes: false,
                deepcheck_web_tools: false,
                send_van_quotes_to_model: false,
                send_draft_rules_to_model: true,
            },
            alert: Alert::default(),
            magazine: Magazine::default(),
            web: Web {
                secure_cookie: true,
                van_usernames: vec![],
            },
        }
    }
}

impl Config {
    /// env > 文件 > 默认。文件可以不存在。
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let mut cfg = match path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p)
                    .with_context(|| format!("读配置 {}", p.display()))?;
                toml::from_str(&text).with_context(|| format!("解析配置 {}", p.display()))?
            }
            Some(p) => anyhow::bail!("配置文件不存在：{}", p.display()),
            None => Self::default(),
        };
        cfg.apply_env();
        Ok(cfg)
    }

    /// 只覆盖会随环境变的那几项。**密钥不在这里**——它们在 [`Secrets`]。
    fn apply_env(&mut self) {
        let s = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = s("CSW_COLLECTOR_DATA_DIR") {
            self.data_dir = PathBuf::from(v);
        }
        if let Some(v) = s("CSW_COLLECTOR_LISTEN") {
            self.listen = v;
        }
        if let Some(v) = s("CSW_TASK_BASE_URL") {
            self.engine.base_url = v;
        }
        if let Some(v) = s("CSW_ADMIN_URL") {
            self.engine.admin_url = v;
        }
        if let Some(v) = s("CSW_API_URL") {
            self.csw.base_url = v;
        }
        if let Some(v) = s("SUB2API_BASE_URL") {
            self.model.base_url = v;
        }
        if let Some(v) = s("CSW_VECTOR_URL") {
            self.vector.base_url = v;
        }
        // 告警地址像密钥一样只从 env 走：它是一个谁拿到都能往群里发消息的地址
        if let Some(v) = s("CSW_COLLECTOR_ALERT_WEBHOOK") {
            self.alert.webhook_url = v;
        }
        if let Some(v) = s("JEV_ENABLED") {
            self.jev.enabled = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        // Jev 密钥不在就自动关，不必改配置
        if self.jev.enabled
            && std::env::var("TYPESAFE_API_KEY")
                .unwrap_or_default()
                .is_empty()
        {
            self.jev.enabled = false;
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("collector.db")
    }
    pub fn lance_path(&self) -> PathBuf {
        self.data_dir.join("lance")
    }
    pub fn media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }
    pub fn blob_dir(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }
    pub fn fixtures_dir(&self) -> PathBuf {
        self.data_dir.join("fixtures")
    }
    /// 向量库的备份放哪。**与库本身同一块盘**——它防的是「库被写坏了」，
    /// 不是「盘坏了」；后者要靠别的机器，那不是这个服务该管的事。
    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backup")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 默认配置能往返序列化() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.model.model, "gpt-6-astra");
        assert_eq!(back.model.concurrency, 5);
    }

    #[test]
    fn 高风险项默认全关() {
        let f = Config::default().features;
        assert!(!f.xhs_collector);
        assert!(!f.web_collector);
        assert!(!f.custom_collectors);
        assert!(!f.mcp_for_hermes);
        assert!(!f.deepcheck_web_tools);
    }

    #[test]
    fn 配置文件里认不得的键要报错而不是默默忽略() {
        let bad = "data_dir = \"/tmp\"\nlisten = \"0.0.0.0:1\"\nmodle = \"typo\"\n";
        assert!(toml::from_str::<Config>(bad).is_err());
    }

    #[test]
    fn 密钥的调试输出只露长度() {
        let key = "sk-csw-abcdefghijklmnop";
        let s = Secrets {
            csw_api_key: key.into(),
            ..Default::default()
        };
        let text = format!("{s:?}");
        assert!(!text.contains("abcdefghijklmnop"), "密钥漏出来了：{text}");
        assert!(text.contains(&format!("{} 字符", key.len())));
        assert!(text.contains("缺"), "空密钥该显示为缺：{text}");
    }

    #[test]
    fn 缺密钥要在启动时就报出来() {
        let s = Secrets::default();
        assert!(s.missing(true).contains(&"TYPESAFE_API_KEY"));
        assert!(!s.missing(false).contains(&"TYPESAFE_API_KEY"));
        assert!(s.missing(false).contains(&"CSW_ENGINE_TOKEN"));
    }
}
