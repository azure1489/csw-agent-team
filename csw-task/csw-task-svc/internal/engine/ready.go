package engine

import (
	"context"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// effectiveDispatchMode 任务快照的派工模式优先；快照为空（v2 之前触发的旧任务）回退工作流级。
func effectiveDispatchMode(task domain.Task, wf domain.Workflow) domain.DispatchMode {
	if task.DispatchMode != "" {
		return task.DispatchMode
	}
	return wf.DispatchMode
}

// onTaskReady 置任务就绪、写事件；据派工模式/合流决定是否引擎自动派工。
// - auto 模式：就绪即自动派。
// - 合流阶段（is_merge，assignee=中枢）：即便 manual 也自动出派工单给中枢自己。
// - 平台写操作缺 run 授权：保持 ready 不派，写 authorization_required 事件，授权录入后由 Grant 续派。
func (e *Engine) onTaskReady(ctx context.Context, q *sqlite.Queries, task domain.Task, wf domain.Workflow) error {
	if err := q.SetTaskReady(ctx, task.ID); err != nil {
		return err
	}
	auto := effectiveDispatchMode(task, wf) == domain.DispatchAuto || task.IsMerge
	var rn *notice // 手动派工阶段：通知中枢「待派工」；自动派工由随后的 dispatched 通知执行者
	if !auto {
		rn = &notice{target: wf.HubRoleCode, hub: wf.HubRoleCode, payload: taskPayload(task, nil)}
	}
	if err := emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, Type: domain.EvtTaskReady}, rn); err != nil {
		return err
	}
	if auto {
		ok, scope, err := authorized(ctx, q, task)
		if err != nil {
			return err
		}
		if !ok {
			return emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, Type: domain.EvtAuthorizationRequired,
				DetailJSON: evtDetail(map[string]any{"scope": string(scope)})},
				&notice{target: wf.HubRoleCode, hub: wf.HubRoleCode, payload: taskPayload(task, map[string]any{"scope": string(scope)})})
		}
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

	return e.checkRunDone(ctx, q, runID)
}

// checkRunDone run 完成判定：全部任务处于终态（passed / cancelled）且至少一个 passed。
// 依赖就绪判定仍只认 passed——取消的上游不会放行下游。
func (e *Engine) checkRunDone(ctx context.Context, q *sqlite.Queries, runID int64) error {
	run, err := q.GetRun(ctx, runID)
	if err != nil {
		return err
	}
	if run.Status != domain.RunActive {
		return nil
	}
	tasks, err := q.ListTasksByRun(ctx, runID)
	if err != nil {
		return err
	}
	anyPassed := false
	for _, t := range tasks {
		if !domain.IsTerminal(t.Status) {
			return nil
		}
		if t.Status == domain.TaskPassed {
			anyPassed = true
		}
	}
	if !anyPassed {
		return nil
	}
	if err := q.SetRunStatus(ctx, runID, domain.RunDone); err != nil {
		return err
	}
	return q.InsertEvent(ctx, domain.Event{RunID: &runID, Type: domain.EvtRunDone})
}
