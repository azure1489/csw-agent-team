package domain

// Role 工作流角色（不写死，纯数据）。
type Role struct {
	Code         string
	Name         string
	IsManagement bool
	IsHuman      bool
}

// Agent 运行时执行者（一角色一 agent 假设）。
type Agent struct {
	RoleCode string
	Name     string
	Active   bool
	ID       int64
}

// ── 定义层 ──

// Workflow 工作流定义（一行 = 一个不可变版本）。
type Workflow struct {
	WfKey              string
	Name               string
	HubRoleCode        string
	DispatchMode       DispatchMode
	TriggerRoles       string // CSV，空=默认(管理类+调度器)
	CommonInstructions string
	CommonAcceptance   string
	Status             WorkflowStatus
	ID                 int64
	Version            int
}

// Stage 工作流阶段定义。
type Stage struct {
	Code              string
	Name              string
	RoleCode          string
	OutputType        string
	Instructions      string
	SelfCheckCriteria string
	Acceptance        string
	DispatchMode      string // 阶段级派工模式覆盖：空=继承工作流 / manual / auto
	ActionClass       string // read | platform_write:<scope>（平台写操作受 run 授权约束）
	ID                int64
	WorkflowID        int64
	Seq               int
	SLAMinutes        int // 派工后时限（分钟），0=不设
	AckMinutes        int // 派工后多久未接单提醒（分钟），0=默认
	IdleMinutes       int // 接单后多久无活动提醒（分钟），0=默认
	IsMerge           bool
	PerItem           bool // 按条目生成任务（条目级推进）
}

// StageDep 定义层依赖边（stage 依赖 dependsOn 全部通过才就绪）。
type StageDep struct {
	StageID     int64
	DependsOnID int64
}

// TaskDep 实例层依赖边。
type TaskDep struct {
	TaskID      int64
	DependsOnID int64
}

// Gate 审核闸定义；StageID 为 nil 表示工作流级默认闸。
type Gate struct {
	StageID      *int64
	Name         string
	ReviewerRole string
	ID           int64
	WorkflowID   int64
	GateOrder    int
	RelayedByHub bool
}

// ── 实例层 ──

// Run 工作流实例（一次批次）。
type Run struct {
	TargetCount int // 整期目标条数（0=不设）
	Subject     string
	Title       string
	Status      RunStatus
	ID          int64
	WorkflowID  int64
	WorkflowVer int
	CreatedBy   *int64
}

// Task 阶段任务（从 Stage 快照）。
type Task struct {
	StageCode         string
	StageName         string
	RoleCode          string
	OutputType        string
	Instructions      string
	SelfCheckCriteria string
	Acceptance        string
	ActionClass       string       // 快照：read | platform_write:<scope>
	DispatchMode      DispatchMode // 快照：触发时解析的派工模式；空=旧任务，回退工作流级
	ItemKey           string       // 条目键；非条目级任务为空串
	WaitItemStages    string       // 快照：要等哪些逐条阶段的条目任务（CSV）；空=不等条目
	FailReason        string       // 执行者报告失败的原因
	DueAt             string       // 派工（或退回）时按 sla 写入
	DispatchedAt      string
	StartedAt         string // 接单或首次提交
	CompletedAt       string
	LastActivityAt    string // 接单心跳 / 提交
	Status            TaskStatus
	AssigneeID        *int64
	ID                int64
	RunID             int64
	Seq               int
	CurVersion        int
	SLAMinutes        int // 快照：派工后时限（分钟），0=不设
	AckMinutes        int // 快照：未接单提醒阈值（分钟），0=默认
	IdleMinutes       int // 快照：无活动提醒阈值（分钟），0=默认
	IsMerge           bool
	ReworkPending     bool // 上游补件到达，需按新资产返工
}

// TaskGate 任务审核闸（从 Gate 解析快照）。
type TaskGate struct {
	ReviewerRole string
	Name         string
	ID           int64
	TaskID       int64
	GateOrder    int
	RelayedByHub bool
}

// File 托管文件（内容寻址）。
type File struct {
	SHA256      string
	Filename    string
	ContentType string
	StoragePath string
	ID          int64
	ByteSize    int64
	UploadedBy  *int64
}

// Deliverable 交付物，版本化。Kind 区分派工单 / 产出 / 补件 / 定点编辑；IsDispatch 与 Kind==dispatch 同义（兼容）。
type Deliverable struct {
	CreatedAt      string
	Kind           DeliverableKind
	DiffSummary    string // 定点编辑的修改摘要
	DocType        string
	DownloadURL    string
	Filename       string
	Title          string
	Summary        string
	MetaJSON       string
	SelfCheck      string
	EditorNote     string
	Status         DeliverableStatus
	ReturnedAtGate *int
	EditOf         *int // 定点编辑基于的版本
	ProducerID     *int64
	FileID         *int64
	AffectsID      *int64 // 补件影响的产出交付物
	ID             int64
	TaskID         int64
	Version        int
	CurGate        int
	IsDispatch     bool
	Collab         bool // 协作版本（主编定点编辑）
}

// Chat 飞书群（花名册容器）。
type Chat struct {
	ChatKey   string
	Name      string
	Note      string
	CreatedAt string
	UpdatedAt string
	ID        int64
}

// ChatMember 群成员：mapped（映射 role_code 的 agent）/ reserved（未接入工作流的 bot）/ 人类 van。
type ChatMember struct {
	Kind        string  // mapped | reserved
	RoleCode    *string // mapped 必填；reserved 为 nil
	OpenID      string
	UserID      *string // 仅 van
	DisplayName string
	BotName     string
	CreatedAt   string
	UpdatedAt   string
	ID          int64
	ChatID      int64
	Sort        int
	IsHuman     bool
}

// Upstream 派工单/成品的上游链接条目。
type Upstream struct {
	Label       string
	UpstreamURL string
	UpstreamID  *int64
}

// Review 一条审核记录，挂到某道 task_gate。
type Review struct {
	CreatedAt       string
	Verdict         Verdict
	Comment         string
	ReturnDirection string
	ReturnLocation  string
	DecisionType    string // 业务决定类别，见 DecisionType
	SourceQuote     string // Van 原话（人工闸必填）
	ItemsJSON       string // 条目范围：JSON 字符串数组
	ExpectedVersion *int
	DeliverableID   int64
	TaskGateID      int64
	ReviewerID      *int64
	ID              int64
}

// RunAuthorization run 级授权（平台写操作护栏）。SourceQuote 为 Van 原话，由中枢录入。
type RunAuthorization struct {
	Scope       AuthScope
	SourceQuote string
	GrantedAt   string
	ExpiresAt   string // 空=不过期
	RevokedAt   string // 空=未撤销
	GrantedBy   *int64
	RevokedBy   *int64
	ID          int64
	RunID       int64
}

// Outbox 待发通知（事件接续）。TargetRole / CCRoles 由引擎按闸与角色推导，请求方不能指定。
type Outbox struct {
	EventType     string
	Channel       string
	TargetRole    string
	CCRoles       string // CSV
	PayloadJSON   string
	Status        string // pending | sent | dead
	LastError     string
	ClaimedAt     string
	CreatedAt     string
	RunID         *int64
	TaskID        *int64
	DeliverableID *int64
	ID            int64
	EventID       int64
	Attempts      int
}

// RunItem 一期里的一条候选资讯（条目级推进）。
type RunItem struct {
	ItemKey        string
	Title          string
	Brand          string
	Product        string
	SourceURL      string
	PublishedAt    string
	Status         ItemStatus
	Rank           string // primary 主选 / alt 备选（仅 shortlisted）
	DecidedAt      string
	DecisionSource string // Van 原话
	DiscoveredVia  string // 这条来自哪一轮采集（intake_sweeps.sweep_key）
	FetchedAt      string // 抓取时间（与 PublishedAt 原始披露时间分开）
	EvidenceURL    string // 证据快照：预览图或正文截图的原始地址
	DedupNote      string // 查重对照结论
	CreatedAt      string
	UpdatedAt      string
	DecidedBy      *int64
	ID             int64
	RunID          int64
}

// IntakeSource 来源台账：本期「应该扫哪些」的基准。Required 为每轮必扫，Enabled 只表示可用。
type IntakeSource struct {
	Platform  string // instagram | xhs | web | other
	SourceKey string // 账号名 / 关键词 / 域名；channel 为通道级占位
	Name      string
	EntryURL  string
	LastOKAt  string
	AddedBy   string
	SourceRef string // 凭什么加进台账（主编或 Van 原话、回填来源）
	Note      string
	CreatedAt string
	UpdatedAt string
	ID        int64
	Enabled   bool
	Required  bool
}

// IntakeSweep 一轮采集：回答「找过哪里」，让「没找到」与「没去找」可区分。
type IntakeSweep struct {
	SweepKey      string // 上报方生成，(run_id, sweep_key) 幂等
	Platform      string
	SourceKey     string
	Tool          string // csw_mcp | opencli | webfetch | other
	Query         string // 找了什么（关键词 / 账号 / 页面）
	StartedAt     string
	EndedAt       string
	WindowFrom    string
	WindowTo      string
	Result        string // ok | failed | partial
	Error         string
	RoleCode      string
	CreatedAt     string
	UpdatedAt     string
	TaskID        *int64
	ActorID       *int64
	ID            int64
	RunID         int64
	Found         int // 接口返回数：这一轮拿到多少条（含重复，不等于看过）
	FetchedUnique int // 去重获取数：去掉跨轮重复后的不同内容数
	Reviewed      int // 已审数：实际读过正文或看过图、形成了判断的条数
	Unreviewed    int // 未审数：只加载未展开的条数（不得事后补成淘汰）
	Corroborated  int // 佐证数：已审里为核实线索而读的页面（官方页、原始出处），不作为候选登记
	InWindow      int // 其中落在当期窗口内的条数
	Registered    int // 其中登记成条目的条数
	PagedToEnd    bool
}

// IntakeJudgement 一条候选的判断结论：四档 + Van 的六个维度各自成立与否 + 每维依据。
//
// 与 ItemTrace 的分工：ItemTrace 记「状态怎么变的」，这里记「为什么这么判」。
// 被判不推荐的候选不进 run_items，所以 ItemKey 可以为空——台账要能看见它，
// 否则「每条都判」就无从验证。
//
// 刻意没有分数字段，也不会有：留了字段就一定会有人去填、去排序，
// 「不打数字分、无权重」的口径就名存实亡了。
type IntakeJudgement struct {
	CandidateKey   string // 品牌小写 + '-' + 链接 sha256 前 6 位；与交付物里的条目键同一个
	ItemKey        string // 进了 run_items 的才有
	ItemKeySet     bool   // 上报时明确带了 item_key：空串表示清掉旧归属；不落库
	Platform       string
	PostRef        string // 平台侧 id（Instagram 短码）
	SourceURL      string
	Tier           string // recommend | alternate | not_recommend | pending_check
	DimsJSON       string // {"change":{"verdict":"yes|no|unclear","basis":"..."}, ...}
	ThreeJSON      string // v10 起 {"what","why_worth","grounds","headline","novelty"}；v9 及以前 {"what_changed","why_it_matters","how_different"}
	ComparisonJSON string // v10 起 {"verdict","against","note","hits":[…],"readiness"}；verdict 多了 unconfirmed
	HeatNote       string // 热度是输入的呈现，不是维度
	GapsJSON       string // v10 起是对象数组 {"level","what","owner","tried","next"}；之前是字符串数组
	HitsJSON       string // 命中的七类优先关注 / 七类降低优先级；是倾向不是黑名单
	JevJSON        string // 初评与核对留痕；初评概率只用于排序，不参与结论
	RubricVersion  string
	RoleCode       string
	CreatedAt      string
	UpdatedAt      string
	ActorID        *int64
	ID             int64
	RunID          int64
	ImageSeen      bool // false 时 Tier 必须是 pending_check，库里有 CHECK 兜着
	Carried        bool // 昨天已判、今天仍在窗口内：台账出现这一行但不重判
}

// ItemTrace 条目的一次判断：回答「为什么留、为什么弃」，淘汰理由由此落库而不是只写在交付物里。
type ItemTrace struct {
	ItemKey    string
	FromStatus string // 首条无前态时为空
	ToStatus   string
	ReasonCode string
	Reason     string
	ActorRole  string
	QuoteRef   string // 引用的主编校准或 Van 原话
	CreatedAt  string
	ActorID    *int64
	ID         int64
	RunID      int64
}

// Event 进度事件。
type Event struct {
	Type          string
	DetailJSON    string
	CreatedAt     string
	ID            int64
	RunID         *int64
	TaskID        *int64
	DeliverableID *int64
	ActorID       *int64
}
