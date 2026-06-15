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
	ID                int64
	WorkflowID        int64
	Seq               int
	IsMerge           bool
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
	Status            TaskStatus
	AssigneeID        *int64
	ID                int64
	RunID             int64
	Seq               int
	CurVersion        int
	IsMerge           bool
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
