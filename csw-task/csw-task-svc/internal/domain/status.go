// Package domain 定义实体、状态枚举与纯状态机规则（无框架/DB 依赖）。
package domain

import "strings"

// TaskStatus 阶段任务状态。
type TaskStatus string

const (
	TaskBlocked    TaskStatus = "blocked"     // 依赖未就绪
	TaskReady      TaskStatus = "ready"       // 依赖全通过，待派工
	TaskDispatched TaskStatus = "dispatched"  // 已派工，待 agent 提交
	TaskInProgress TaskStatus = "in_progress" // agent 进行中（可选中间态）
	TaskReview     TaskStatus = "review"      // 已提交某版本，闸审核中
	TaskPassed     TaskStatus = "passed"      // 末闸通过
	TaskReturned   TaskStatus = "returned"    // 被退回，待升版本重交
	TaskFailed     TaskStatus = "failed"      // 执行者报告无法完成（附原因），待中枢重开或取消
	TaskCancelled  TaskStatus = "cancelled"   // 中枢取消（尚未开始的下游一并取消）
)

// DeliverableStatus 交付物状态（派工单 issued；产出 submitted→...）。
type DeliverableStatus string

const (
	DelIssued     DeliverableStatus = "issued"    // 派工单
	DelSubmitted  DeliverableStatus = "submitted" // 产出刚提交
	DelInReview   DeliverableStatus = "in_review" // 审核中
	DelPassed     DeliverableStatus = "passed"
	DelReturned   DeliverableStatus = "returned"
	DelSuperseded DeliverableStatus = "superseded" // 被主编定点编辑版本取代的首稿（原样保留）
)

// DeliverableKind 交付物类别。
type DeliverableKind string

const (
	KindDispatch   DeliverableKind = "dispatch"   // 派工单
	KindOutput     DeliverableKind = "output"     // 执行者产出
	KindSupplement DeliverableKind = "supplement" // 补件：挂在已通过任务上，不重开审核
	KindEdit       DeliverableKind = "edit"       // 主编定点编辑版本（与产出共用版本流）
)

// CanAccept 某类交付物在任务当前状态下能否提交：产出同 CanSubmit；补件要求已通过；定点编辑要求审核中。
func CanAccept(kind DeliverableKind, s TaskStatus) bool {
	switch kind {
	case "", KindOutput:
		return CanSubmit(s)
	case KindSupplement:
		return s == TaskPassed
	case KindEdit:
		return s == TaskReview
	}
	return false
}

// DecisionType 审核决定类别。edit_pass 由引擎在主编定点编辑时自动写入，调用方不能提交。
type DecisionType string

const (
	DecResearchOK      DecisionType = "research_ok"
	DecTopicApprove    DecisionType = "topic_approve"
	DecFulltextApprove DecisionType = "fulltext_approve"
	DecTemplateApprove DecisionType = "template_approve"
	DecLocalVerify     DecisionType = "local_verify"
	DecDraftSaveOK     DecisionType = "draft_save_ok"
	DecPublishOK       DecisionType = "publish_ok"
	DecEditPass        DecisionType = "edit_pass"
)

// ValidDecisionType 调用方可提交的决定类别（不含 edit_pass）。
func ValidDecisionType(s string) bool {
	switch DecisionType(s) {
	case DecResearchOK, DecTopicApprove, DecFulltextApprove, DecTemplateApprove, DecLocalVerify, DecDraftSaveOK, DecPublishOK:
		return true
	}
	return false
}

// RunStatus 实例状态。
type RunStatus string

const (
	RunActive  RunStatus = "active"
	RunDone    RunStatus = "done"
	RunPaused  RunStatus = "paused"
	RunAborted RunStatus = "aborted"
)

// Verdict 审核结论。
type Verdict string

const (
	VerdictPass   Verdict = "pass"
	VerdictReject Verdict = "reject"
)

// WorkflowStatus 定义状态。
type WorkflowStatus string

const (
	WfDraft    WorkflowStatus = "draft"
	WfActive   WorkflowStatus = "active"
	WfArchived WorkflowStatus = "archived"
)

// DispatchMode 派工模式。
type DispatchMode string

const (
	DispatchManual DispatchMode = "manual"
	DispatchAuto   DispatchMode = "auto"
)

// 事件类型（进度时间线）。
const (
	EvtRunCreated   = "run_created"
	EvtTaskReady    = "task_ready"
	EvtDispatched   = "dispatched"
	EvtFileUploaded = "file_uploaded"
	EvtSubmitted    = "submitted"
	EvtGatePassed   = "gate_passed"
	EvtGateReturned = "gate_returned"
	EvtStagePassed  = "stage_passed"
	EvtRunDone      = "run_done"

	EvtAuthorizationRequired = "authorization_required" // 平台写操作就绪但缺授权，停在 ready
	EvtAuthorizationGranted  = "authorization_granted"
	EvtAuthorizationRevoked  = "authorization_revoked"

	EvtTaskAcked     = "task_acked"
	EvtTaskFailed    = "task_failed"
	EvtTaskCancelled = "task_cancelled"
	EvtTaskReopened  = "task_reopened"
	EvtOverdue       = "overdue"

	EvtSupplementArrived = "supplement_arrived"
)

// 接续告警：事件类型与默认阈值（阶段未设 ack_minutes / idle_minutes 时）。
const (
	EvtAckOverdue   = "ack_overdue"   // 派工后超过阈值仍未接单：提醒执行者、抄送中枢
	EvtAckEscalated = "ack_escalated" // 未接单超过阈值 × AckEscalateFactor：升级中枢处置
	EvtTaskIdle     = "task_idle"     // 接单后超过阈值没有心跳或产物：提醒执行者、抄送中枢

	DefaultAckMinutes  = 5
	DefaultIdleMinutes = 10
	AckEscalateFactor  = 3
)

// StallKind 接续告警类别。
type StallKind string

const (
	StallAck      StallKind = "ack"      // 派工后未接单
	StallEscalate StallKind = "escalate" // 未接单升级中枢（须已提醒过执行者）
	StallIdle     StallKind = "idle"     // 接单后无活动
)

// AckMinutesOf 任务的未接单提醒阈值（快照为 0 时取默认）。
func AckMinutesOf(t Task) int {
	if t.AckMinutes > 0 {
		return t.AckMinutes
	}
	return DefaultAckMinutes
}

// IdleMinutesOf 任务的无活动提醒阈值（快照为 0 时取默认）。
func IdleMinutesOf(t Task) int {
	if t.IdleMinutes > 0 {
		return t.IdleMinutes
	}
	return DefaultIdleMinutes
}

// ActionRead 普通阶段的动作类别。平台写操作写作 "platform_write:<scope>"。
const (
	ActionRead          = "read"
	actionPlatformWrite = "platform_write:"
)

// AuthScope run 级授权范围。
type AuthScope string

const (
	ScopeLocalDrill AuthScope = "local_drill" // 本地演练：只交本地成品，不进平台后台
	ScopeWxDraft    AuthScope = "wx_draft"
	ScopeWxPublish  AuthScope = "wx_publish"
	ScopeXhsDraft   AuthScope = "xhs_draft"
	ScopeXhsPublish AuthScope = "xhs_publish"
)

// ValidAuthScope 判断授权范围取值是否合法。
func ValidAuthScope(s string) bool {
	switch AuthScope(s) {
	case ScopeLocalDrill, ScopeWxDraft, ScopeWxPublish, ScopeXhsDraft, ScopeXhsPublish:
		return true
	}
	return false
}

// ParseActionClass 解析动作类别：read → (false, "", true)；platform_write:<平台 scope> → (true, scope, true)。
// local_drill 不是平台写操作，不能出现在动作类别里。
func ParseActionClass(s string) (isWrite bool, scope AuthScope, ok bool) {
	if s == ActionRead {
		return false, "", true
	}
	if !strings.HasPrefix(s, actionPlatformWrite) {
		return false, "", false
	}
	sc := AuthScope(strings.TrimPrefix(s, actionPlatformWrite))
	switch sc {
	case ScopeWxDraft, ScopeWxPublish, ScopeXhsDraft, ScopeXhsPublish:
		return true, sc, true
	}
	return false, "", false
}

// ValidActionClass 判断动作类别取值是否合法。
func ValidActionClass(s string) bool {
	_, _, ok := ParseActionClass(s)
	return ok
}

// SatisfyingScopes 列出能满足某平台写动作的授权：发布授权涵盖同平台草稿。
func SatisfyingScopes(scope AuthScope) []AuthScope {
	switch scope {
	case ScopeWxDraft:
		return []AuthScope{ScopeWxDraft, ScopeWxPublish}
	case ScopeXhsDraft:
		return []AuthScope{ScopeXhsDraft, ScopeXhsPublish}
	}
	return []AuthScope{scope}
}

// ValidStageDispatchMode 阶段级派工模式覆盖：空（继承工作流）/ manual / auto。
func ValidStageDispatchMode(s string) bool {
	return s == "" || s == string(DispatchManual) || s == string(DispatchAuto)
}

// ── 纯状态机规则 ──

// CanSubmit 判断 task 是否允许提交新版本产出。
// 仅 dispatched / returned / in_progress 可提交（review/passed/blocked/ready 不可）。
func CanSubmit(s TaskStatus) bool {
	return s == TaskDispatched || s == TaskReturned || s == TaskInProgress
}

// CanAck 已派工的任务可接单；已接单（in_progress）时再调用视为心跳。
func CanAck(s TaskStatus) bool { return s == TaskDispatched || s == TaskInProgress }

// CanFail 执行中的任务（已派工 / 已接单 / 被退回待重交）可报告无法完成。
func CanFail(s TaskStatus) bool {
	return s == TaskDispatched || s == TaskInProgress || s == TaskReturned
}

// CanCancel 未通过、未取消的任务可由中枢取消。
func CanCancel(s TaskStatus) bool { return s != TaskPassed && s != TaskCancelled }

// CanReopen 失败、已取消、已通过的任务可由中枢重开。
func CanReopen(s TaskStatus) bool {
	return s == TaskFailed || s == TaskCancelled || s == TaskPassed
}

// IsTerminal 终态：已通过或已取消（run 完成判定用；failed 仍待中枢处理，不是终态）。
func IsTerminal(s TaskStatus) bool { return s == TaskPassed || s == TaskCancelled }

// AllPassed 判断一组依赖任务是否全部通过（就绪判定）。
// 空依赖（入口任务）视为就绪。
func AllPassed(depStatuses []TaskStatus) bool {
	for _, s := range depStatuses {
		if s != TaskPassed {
			return false
		}
	}
	return true
}

// ItemStatus 条目状态。
type ItemStatus string

const (
	ItemCandidate        ItemStatus = "candidate"         // 已登记的候选（线索）
	ItemPendingCheck     ItemStatus = "pending_check"     // 待核：关键事实未核实，不计入成熟数量
	ItemShortlisted      ItemStatus = "shortlisted"       // 研究员建议采用 / 备选
	ItemDropped          ItemStatus = "dropped"           // 判断后淘汰：与「还没判断」的 candidate 区分开
	ItemApprovedWrite    ItemStatus = "approved_write"    // Van 批准可写：生成逐条任务
	ItemApprovedResearch ItemStatus = "approved_research" // 只开研究，不派写作
	ItemDeferred         ItemStatus = "deferred"
	ItemRejected         ItemStatus = "rejected"
	ItemWritten          ItemStatus = "written" // 该条的逐条任务全部通过
	ItemReviewed         ItemStatus = "reviewed"
	ItemPublished        ItemStatus = "published"
)

// 条目分级：成熟条目分主选与备选（只对 shortlisted 有意义）。
const (
	ItemRankPrimary = "primary"
	ItemRankAlt     = "alt"
)

// 条目决定类别（中枢按 Van 原话录入）。
const (
	ItemApproveWrite    = "approve_write"
	ItemApproveResearch = "approve_research"
	ItemDefer           = "defer"
	ItemReject          = "reject"
)

// ItemDecisionStatus 决定类别 → 条目状态；非法类别返回 false。
func ItemDecisionStatus(decision string) (ItemStatus, bool) {
	switch decision {
	case ItemApproveWrite:
		return ItemApprovedWrite, true
	case ItemApproveResearch:
		return ItemApprovedResearch, true
	case ItemDefer:
		return ItemDeferred, true
	case ItemReject:
		return ItemRejected, true
	}
	return "", false
}

// 条目判断的理由码（闭合词表）。淘汰必须给其中之一，机器才能校验「淘汰是否有交代」；
// 与《生产约定》缺量说明的「为什么缺」五类归因对齐，03 的缺量说明可直接由轨迹汇总得出。
const (
	ReasonNoValue           = "no_value"            // 价值不足
	ReasonNotNew            = "not_new"             // 非新披露，仅为转发
	ReasonDupPublished      = "dup_published"       // 近期已发同角度
	ReasonDupRecentRejected = "dup_recent_rejected" // 近期已被否决或暂缓
	ReasonOutOfWindow       = "out_of_window"       // 不在当期窗口
	ReasonEvidenceMissing   = "evidence_missing"    // 证据或图片不足
	ReasonAestheticMismatch = "aesthetic_mismatch"  // 审美或价值不符
	ReasonSuperseded        = "superseded"          // 被同期更好的条目取代
	ReasonOther             = "other"               // 其他，须写明理由
	ReasonAutoWritten       = "auto_written"        // 引擎自动：逐条任务全部通过
)

// ValidItemReasonCode 校验登记方可写的理由码；内部码（auto_written、决定类）不在其中。
func ValidItemReasonCode(code string) bool {
	switch code {
	case ReasonNoValue, ReasonNotNew, ReasonDupPublished, ReasonDupRecentRejected, ReasonOutOfWindow,
		ReasonEvidenceMissing, ReasonAestheticMismatch, ReasonSuperseded, ReasonOther:
		return true
	}
	return false
}

// 采集轮取值（来源台账与采集轮共用平台词表）。
func ValidSourcePlatform(p string) bool {
	switch p {
	case "instagram", "xhs", "web", "other":
		return true
	}
	return false
}

// ValidSweepTool 采集所用工具。
func ValidSweepTool(t string) bool {
	switch t {
	case "csw_mcp", "opencli", "webfetch", "other":
		return true
	}
	return false
}

// ValidSweepResult 一轮采集的结果。
func ValidSweepResult(r string) bool {
	switch r {
	case "ok", "failed", "partial":
		return true
	}
	return false
}

// ItemCounted 计入整期已写成条数的状态。
func ItemCounted(s ItemStatus) bool {
	return s == ItemWritten || s == ItemReviewed || s == ItemPublished
}

const (
	EvtItemDecided = "item_decided"
	EvtRunClosed   = "run_closed"
	EvtRunAborted  = "run_aborted" // 一无所获的实例被中枢显式作废（区别于按完成结束的 run_done）
)
