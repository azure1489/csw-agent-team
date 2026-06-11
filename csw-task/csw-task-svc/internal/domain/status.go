// Package domain 定义实体、状态枚举与纯状态机规则（无框架/DB 依赖）。
package domain

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
)

// ── 纯状态机规则 ──

// CanSubmit 判断 task 是否允许提交新版本产出。
// 仅 dispatched / returned / in_progress 可提交（review/passed/blocked/ready 不可）。
func CanSubmit(s TaskStatus) bool {
	return s == TaskDispatched || s == TaskReturned || s == TaskInProgress
}

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
