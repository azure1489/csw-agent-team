//! 引擎运行面的出入参。字段名与引擎的 JSON 严格对齐，**不做善意改名**——
//! 改名会让「接口变了」这件事在编译期无声通过、在运行期才炸。

use serde::{Deserialize, Serialize};

/// 任务状态。**真实枚举**，不是设计文档里那套（不是 in_review / needs_rework）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Ready,
    Dispatched,
    InProgress,
    /// 等某道闸
    Review,
    /// 需返工
    Returned,
    Passed,
    Failed,
    Cancelled,
    #[serde(other)]
    Unknown,
}

impl TaskStatus {
    /// 还需要我干活吗
    pub fn is_actionable(self) -> bool {
        matches!(self, Self::Dispatched | Self::InProgress | Self::Returned)
    }
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Passed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: i64,
    pub run_id: i64,
    pub stage_code: String,
    #[serde(default)]
    pub stage_name: String,
    #[serde(default)]
    pub role_code: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub item_key: String,
    #[serde(default)]
    pub action_class: String,
    #[serde(default)]
    pub due_at: String,
    #[serde(default)]
    pub dispatched_at: String,
    #[serde(default)]
    pub cur_version: i32,
    #[serde(default)]
    pub sla_minutes: i32,
    #[serde(default)]
    pub rework_pending: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MyTask {
    pub task: Task,
    #[serde(default)]
    pub run_subject: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MyTasks {
    #[serde(default)]
    pub tasks: Vec<MyTask>,
    #[serde(default)]
    pub skill_min_version: String,
}

/// 任务详情里最要紧的是三段作业标准——它们会被**原样注入模型**，
/// 所以要连同哈希一起存进本轮，标准变了判断就不能复用。
#[derive(Debug, Clone, Deserialize)]
pub struct TaskDetail {
    pub task: Task,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub self_check_criteria: String,
    #[serde(default)]
    pub acceptance: String,
    #[serde(default)]
    pub editor_note: String,
    #[serde(default)]
    pub upstreams: Vec<serde_json::Value>,
    /// 被退回时的方向与位置从这里自助读，不依赖群消息
    #[serde(default)]
    pub latest_review: Option<serde_json::Value>,
    #[serde(default)]
    pub item: Option<serde_json::Value>,
}

impl TaskDetail {
    /// 三段作业标准的指纹。进 `rounds.instructions_hash`。
    pub fn instructions_hash(&self) -> String {
        let joined = format!(
            "{}\n\u{1}\n{}\n\u{1}\n{}",
            self.instructions, self.self_check_criteria, self.acceptance
        );
        blake3::hash(joined.as_bytes()).to_hex()[..32].to_string()
    }
}

/// 条目登记。`origin` 与 `first_seen_at` 是 0037 加的溯源列。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ItemInput {
    pub item_key: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub brand: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub product: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub published_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason_code: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub discovered_via: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fetched_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub evidence_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub dedup_note: String,
    /// dispatch | van_link | supplement
    #[serde(skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub first_seen_at: String,
}

/// 一轮采集的上报。
///
/// 计数的含义不能凑：`reviewed` 是真读过正文或看过图并形成判断的条数，
/// `unreviewed` 是只加载未展开的。工作台每条都判，所以 `unreviewed` 必须是 0；
/// 不是 0 就说明这一步没跑完，**如实写**——`intake-check` 就是靠它判「漏没漏」。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SweepInput {
    pub sweep_key: String,
    pub platform: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_key: String,
    /// 工作台直接调接口，不经 MCP，一律填 `csw_api`
    pub tool: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ended_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub window_from: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub window_to: String,
    pub found: i64,
    pub fetched_unique: i64,
    pub reviewed: i64,
    pub unreviewed: i64,
    pub corroborated: i64,
    pub in_window: i64,
    pub registered: i64,
    pub result: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub paged_to_end: bool,
}

/// 一条候选的判断。引擎会校验六维齐全、每维有依据、没读到图只能落待核。
#[derive(Debug, Clone, Serialize)]
pub struct JudgementInput {
    pub candidate_key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub item_key: String,
    pub platform: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub post_ref: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    pub tier: String,
    pub dims: serde_json::Value,
    pub three_sentences: serde_json::Value,
    pub comparison: serde_json::Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub heat_note: String,
    pub gaps: serde_json::Value,
    pub hits: serde_json::Value,
    pub jev: serde_json::Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub rubric_version: String,
    pub image_seen: bool,
    pub carried: bool,
}

/// 一条自查判据。
#[derive(Debug, Clone, Deserialize)]
pub struct IntakeCheck {
    pub name: String,
    #[serde(default)]
    pub detail: String,
    pub ok: bool,
    #[serde(default)]
    pub warn: bool,
    #[serde(default)]
    pub skipped: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntakeCheckResult {
    pub checks: Vec<IntakeCheck>,
    pub failed: i64,
    pub warned: i64,
    pub ok: bool,
}

/// 历史决定，五类对照材料的第四类。`quote` 可能为空——取不到就是取不到，别当它是没决定。
#[derive(Debug, Clone, Deserialize)]
pub struct Decision {
    pub run_id: i64,
    #[serde(default)]
    pub subject: String,
    pub item_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub published_at: String,
    pub status: String,
    #[serde(default)]
    pub decision_source: String,
    #[serde(default)]
    pub decided_at: String,
    #[serde(default)]
    pub reason_code: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub quote_ref: String,
    #[serde(default)]
    pub actor_role: String,
}

/// 台账里的一篇发布记录。第一类（正式已发布）与第二类（范例）**同表不同标记**，
/// 靠 `is_reference` 分——判断时不能混成一类。
#[derive(Debug, Clone, Deserialize)]
pub struct LedgerPost {
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub post_id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub is_reference: bool,
    /// 只有 `include_body=1` 才有
    #[serde(default)]
    pub body_text: String,
    #[serde(default)]
    pub publish_evidence: String,
    /// 合集拆条
    #[serde(default)]
    pub items: Vec<LedgerPostItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LedgerPostItem {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub angle: String,
    #[serde(default)]
    pub item_key: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub split_by: String,
}

/// `/ledger/posts` 的整份回答。`verdict` 为 `not_found_in_synced_records` 时
/// **不等于「从未发布」**，只等于「在已同步的记录里没找到」，覆盖范围见 `coverage`。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LedgerPosts {
    #[serde(default)]
    pub posts: Vec<LedgerPost>,
    #[serde(default)]
    pub coverage: Vec<serde_json::Value>,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub note: String,
}

/// `GET /runs/:id/items` 里的一条。
///
/// **`status` 是引擎的条目状态，不是我们的四档**：`approved_write` 才是
/// 「Van 批准可写」，11 选图包只给这些条目配图。把 `shortlisted`（研究员建议）
/// 当成批准会多做一批没人要的图。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunItem {
    #[serde(default)]
    pub item_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub status: String,
    /// primary 主选 / alt 备选（只对 shortlisted 有意义）
    #[serde(default)]
    pub rank: String,
}

impl RunItem {
    /// Van 批准可写的那几条。
    pub fn approved(&self) -> bool {
        self.status == "approved_write"
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunItems {
    #[serde(default)]
    pub items: Vec<RunItem>,
    #[serde(default)]
    pub target_count: i64,
    #[serde(default)]
    pub written: i64,
    #[serde(default)]
    pub gap: i64,
}

/// 交付物提交的入参。zip **只构建一次**，字节先落盘，重试只发已落盘的那一份。
#[derive(Debug, Clone)]
pub struct SubmitInput {
    pub task_id: i64,
    /// 产出 | 补件 | 定点编辑
    pub kind: String,
    pub zip_path: std::path::PathBuf,
    pub file_name: String,
    /// 幂等键。**一旦写下就不许换**——引擎的 Idempotency-Key 只对 POST 生效，
    /// 请求中途崩溃则同键永久 409，换键会造成重复提交。
    pub idem_key: String,
    pub note: String,
    /// 补件必须带
    pub affects_deliverable_id: Option<i64>,
    pub item_key: String,
}
