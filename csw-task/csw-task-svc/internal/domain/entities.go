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

// Deliverable 交付物（派工单 IsDispatch=1 / agent 产出 =0），版本化。
type Deliverable struct {
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
	ProducerID     *int64
	FileID         *int64
	ID             int64
	TaskID         int64
	Version        int
	CurGate        int
	IsDispatch     bool
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
	Verdict         Verdict
	Comment         string
	ReturnDirection string
	ReturnLocation  string
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
