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
	DelIssued    DeliverableStatus = "issued"    // 派工单
	DelSubmitted DeliverableStatus = "submitted" // 产出刚提交
	DelInReview  DeliverableStatus = "in_review" // 审核中
	DelPassed    DeliverableStatus = "passed"
	DelReturned  DeliverableStatus = "returned"
)

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
)

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
