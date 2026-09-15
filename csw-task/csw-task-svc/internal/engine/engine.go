// Package engine 实现通用工作流引擎：触发→快照、就绪重算、派工、提交、按闸审核。
// 每个写动作放一个事务（由 store.Tx 保证原子）。引擎不依赖 gin。
package engine

import (
	"context"
	"database/sql"
	"errors"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// Engine 工作流引擎。
type Engine struct {
	store *sqlite.Store
}

// New 构造引擎。
func New(store *sqlite.Store) *Engine { return &Engine{store: store} }

// 派工单文档类型。
const dispatchDocType = "派工单"

// UpstreamInput 上游链接入参（中枢派工/合流提交时可传）。
type UpstreamInput struct {
	Label      string
	URL        string
	UpstreamID *int64
}

func isNoRows(err error) bool { return errors.Is(err, sql.ErrNoRows) }

// combine 拼接通用段 + 阶段段（任一为空则取另一个）。
func combine(common, specific string) string {
	common, specific = strings.TrimSpace(common), strings.TrimSpace(specific)
	switch {
	case common == "":
		return specific
	case specific == "":
		return common
	default:
		return common + "\n\n" + specific
	}
}

// canTrigger 判断角色是否可触发该工作流：管理类，或在 trigger_roles CSV 内。
func canTrigger(role domain.Role, wf domain.Workflow) bool {
	if role.IsManagement {
		return true
	}
	for _, r := range strings.Split(wf.TriggerRoles, ",") {
		if strings.TrimSpace(r) == role.Code {
			return true
		}
	}
	return false
}

// resolveGates 解析某阶段适用的闸：有覆盖闸用覆盖，否则用默认闸（stage_id 为空）。
func resolveGates(all []domain.Gate, stageID int64) []domain.Gate {
	var override, def []domain.Gate
	for _, g := range all {
		if g.StageID != nil && *g.StageID == stageID {
			override = append(override, g)
		} else if g.StageID == nil {
			def = append(def, g)
		}
	}
	if len(override) > 0 {
		return override
	}
	return def
}

// stageDispatchMode 阶段覆盖非空取覆盖，否则取工作流级；触发时解析后快照进任务。
func stageDispatchMode(st domain.Stage, wf domain.Workflow) domain.DispatchMode {
	if st.DispatchMode != "" {
		return domain.DispatchMode(st.DispatchMode)
	}
	return wf.DispatchMode
}

// resolveAssignee 按角色取唯一活跃 agent；无则 nil。
func resolveAssignee(ctx context.Context, q *sqlite.Queries, roleCode string) *int64 {
	a, err := q.ActiveAgentByRole(ctx, roleCode)
	if err != nil {
		return nil
	}
	id := a.ID
	return &id
}

// TriggerResult 触发结果。
type TriggerResult struct {
	Run   domain.Run
	Tasks []domain.Task
}

// Trigger 触发一个 active 工作流的实例：据定义快照出任务图，入口任务就绪。
func (e *Engine) Trigger(ctx context.Context, actor domain.Agent, role domain.Role, wfKey, subject, title, inputsJSON string) (TriggerResult, error) {
	var res TriggerResult
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		wf, err := q.ActiveWorkflowByKey(ctx, wfKey)
		if isNoRows(err) {
			return domain.NotFound("workflow_not_found", "无此 active 工作流："+wfKey)
		}
		if err != nil {
			return err
		}
		if !canTrigger(role, wf) {
			return domain.Forbidden("trigger_forbidden", "当前角色无权触发该工作流")
		}
		// subject 唯一约束已放宽（见 0004 迁移）：同一工作流同 subject 可触发多个实例，
		// 靠 run_id 区分；交付物路径含 run_id 作「任务ID」。防误触发改用 Idempotency-Key。
		runID, err := q.InsertRun(ctx, wf.ID, wf.Version, subject, title, &actor.ID)
		if err != nil {
			return err
		}

		stages, err := q.ListStages(ctx, wf.ID)
		if err != nil {
			return err
		}
		deps, err := q.ListStageDeps(ctx, wf.ID)
		if err != nil {
			return err
		}
		gates, err := q.ListGates(ctx, wf.ID)
		if err != nil {
			return err
		}

		// 建任务（快照阶段内容；acceptance = 通用合规项 + 本阶段；instructions = 通用约定 + 本阶段）。
		stageToTask := make(map[int64]int64, len(stages))
		hasDep := make(map[int64]bool, len(stages))
		for _, d := range deps {
			hasDep[d.StageID] = true
		}
		for _, st := range stages {
			t := domain.Task{
				RunID:             runID,
				StageCode:         st.Code,
				StageName:         st.Name,
				Seq:               st.Seq,
				RoleCode:          st.RoleCode,
				IsMerge:           st.IsMerge,
				OutputType:        st.OutputType,
				Instructions:      combine(wf.CommonInstructions, st.Instructions),
				SelfCheckCriteria: st.SelfCheckCriteria,
				Acceptance:        combine(wf.CommonAcceptance, st.Acceptance),
				DispatchMode:      stageDispatchMode(st, wf),
				ActionClass:       st.ActionClass,
				SLAMinutes:        st.SLAMinutes,
				AssigneeID:        resolveAssignee(ctx, q, st.RoleCode),
				Status:            domain.TaskBlocked,
			}
			id, err := q.InsertTask(ctx, t)
			if err != nil {
				return err
			}
			stageToTask[st.ID] = id
		}
		// 依赖边。
		for _, d := range deps {
			if err := q.InsertTaskDep(ctx, stageToTask[d.StageID], stageToTask[d.DependsOnID]); err != nil {
				return err
			}
		}
		// 审核闸（解析默认+覆盖，落到每个任务）。
		for _, st := range stages {
			for _, g := range resolveGates(gates, st.ID) {
				if err := q.InsertTaskGate(ctx, stageToTask[st.ID], g.GateOrder, g.ReviewerRole, g.RelayedByHub, g.Name); err != nil {
					return err
				}
			}
		}

		if err := q.InsertEvent(ctx, domain.Event{RunID: &runID, ActorID: &actor.ID, Type: domain.EvtRunCreated, DetailJSON: inputsJSON}); err != nil {
			return err
		}

		// 入口任务（无依赖）就绪；按 dispatch_mode / 合流决定是否自动派工。
		for _, st := range stages {
			if hasDep[st.ID] {
				continue
			}
			t, err := q.GetTask(ctx, stageToTask[st.ID])
			if err != nil {
				return err
			}
			if err := e.onTaskReady(ctx, q, t, wf); err != nil {
				return err
			}
		}

		if res.Run, err = q.GetRun(ctx, runID); err != nil {
			return err
		}
		if res.Tasks, err = q.ListTasksByRun(ctx, runID); err != nil {
			return err
		}
		return nil
	})
	return res, err
}
