package engine

import (
	"context"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// onTaskReady 置任务就绪、写事件；据 dispatch_mode/合流决定是否引擎自动派工。
// - auto 模式：就绪即自动派。
// - 合流阶段（is_merge，assignee=中枢）：即便 manual 也自动出派工单给中枢自己。
func (e *Engine) onTaskReady(ctx context.Context, q *sqlite.Queries, task domain.Task, wf domain.Workflow) error {
	if err := q.SetTaskReady(ctx, task.ID); err != nil {
		return err
	}
	if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &task.ID, Type: domain.EvtTaskReady}); err != nil {
		return err
	}
	if wf.DispatchMode == domain.DispatchAuto || task.IsMerge {
		producer := resolveAssignee(ctx, q, wf.HubRoleCode)
		if _, err := e.dispatchInternal(ctx, q, task, wf, "", nil, producer); err != nil {
			return err
		}
	}
	return nil
}

// recomputeReady 某任务通过后，重算下游：所有依赖 passed 的 blocked 任务转就绪；全部 passed → run done。
func (e *Engine) recomputeReady(ctx context.Context, q *sqlite.Queries, runID int64, wf domain.Workflow) error {
	tasks, err := q.ListTasksByRun(ctx, runID)
	if err != nil {
		return err
	}
	deps, err := q.ListTaskDepsByRun(ctx, runID)
	if err != nil {
		return err
	}
	status := make(map[int64]domain.TaskStatus, len(tasks))
	for _, t := range tasks {
		status[t.ID] = t.Status
	}
	depMap := make(map[int64][]int64)
	for _, d := range deps {
		depMap[d.TaskID] = append(depMap[d.TaskID], d.DependsOnID)
	}

	for _, t := range tasks {
		if t.Status != domain.TaskBlocked {
			continue
		}
		ready := true
		for _, dep := range depMap[t.ID] {
			if status[dep] != domain.TaskPassed {
				ready = false
				break
			}
		}
		if ready {
			if err := e.onTaskReady(ctx, q, t, wf); err != nil {
				return err
			}
		}
	}

	// 全部 passed → run done（slice 已反映本次刚 passed 的任务；新转就绪的不影响该判定）。
	allPassed := len(tasks) > 0
	for _, t := range tasks {
		if t.Status != domain.TaskPassed {
			allPassed = false
			break
		}
	}
	if allPassed {
		if err := q.SetRunStatus(ctx, runID, domain.RunDone); err != nil {
			return err
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &runID, Type: domain.EvtRunDone}); err != nil {
			return err
		}
	}
	return nil
}
