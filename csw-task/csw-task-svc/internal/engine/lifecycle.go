package engine

import (
	"context"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// checkAssignee 仅任务负责人可操作（合流阶段=中枢）；assignee 未解析时回退到角色匹配。
func checkAssignee(task domain.Task, agent domain.Agent, action string) error {
	switch {
	case task.AssigneeID != nil && *task.AssigneeID == agent.ID:
	case task.AssigneeID == nil && task.RoleCode == agent.RoleCode:
	default:
		return domain.Forbidden("not_assignee", "仅该任务负责人可"+action)
	}
	return nil
}

func loadTask(ctx context.Context, q *sqlite.Queries, id int64) (domain.Task, error) {
	t, err := q.GetTask(ctx, id)
	if isNoRows(err) {
		return t, domain.NotFound("task_not_found", "无此任务")
	}
	return t, err
}

func requireReason(reason, what string) (string, error) {
	reason = strings.TrimSpace(reason)
	if reason == "" {
		return "", domain.BadRequest("reason_required", what+"须写明原因")
	}
	return reason, nil
}

// Ack 执行者接单：dispatched → in_progress（记 started_at）。已接单时再调用只刷新活动时间（心跳）。
func (e *Engine) Ack(ctx context.Context, agent domain.Agent, taskID int64) (domain.Task, error) {
	var out domain.Task
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := loadTask(ctx, q, taskID)
		if err != nil {
			return err
		}
		if err := checkAssignee(task, agent, "接单"); err != nil {
			return err
		}
		if !domain.CanAck(task.Status) {
			return domain.Conflict("cannot_ack", "仅已派工或进行中的任务可接单，当前："+string(task.Status))
		}
		if task.Status == domain.TaskDispatched {
			if err := q.SetTaskInProgress(ctx, taskID); err != nil {
				return err
			}
			if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, ActorID: &agent.ID, Type: domain.EvtTaskAcked}); err != nil {
				return err
			}
		} else if err := q.TouchTaskActivity(ctx, taskID); err != nil {
			return err
		}
		out, err = q.GetTask(ctx, taskID)
		return err
	})
	return out, err
}

// Fail 执行者报告无法完成：须附原因；任务转 failed，等中枢重开或取消。
func (e *Engine) Fail(ctx context.Context, agent domain.Agent, taskID int64, reason string) (domain.Task, error) {
	var out domain.Task
	reason, err := requireReason(reason, "报告失败")
	if err != nil {
		return out, err
	}
	err = e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := loadTask(ctx, q, taskID)
		if err != nil {
			return err
		}
		if err := checkAssignee(task, agent, "报告失败"); err != nil {
			return err
		}
		if !domain.CanFail(task.Status) {
			return domain.Conflict("cannot_fail", "仅执行中的任务可报告失败，当前："+string(task.Status))
		}
		if err := q.SetTaskFailed(ctx, taskID, reason); err != nil {
			return err
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, ActorID: &agent.ID, Type: domain.EvtTaskFailed,
			DetailJSON: evtDetail(map[string]any{"reason": reason})}); err != nil {
			return err
		}
		out, err = q.GetTask(ctx, taskID)
		return err
	})
	return out, err
}

// CancelResult 取消结果。
type CancelResult struct {
	Task      domain.Task
	Cancelled []int64 // 本次取消的全部任务（含级联）
}

// Cancel 中枢取消任务：须附原因。尚未开始（blocked / ready）的下游一并取消——它们等的依赖永远不会通过。
// 取消后若全部任务终态且至少一个通过，run 完成。
func (e *Engine) Cancel(ctx context.Context, hub domain.Agent, role domain.Role, taskID int64, reason string) (CancelResult, error) {
	var res CancelResult
	reason, err := requireReason(reason, "取消任务")
	if err != nil {
		return res, err
	}
	err = e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := loadTask(ctx, q, taskID)
		if err != nil {
			return err
		}
		if _, _, err := hubRun(ctx, q, role, task.RunID, "取消任务"); err != nil {
			return err
		}
		if !domain.CanCancel(task.Status) {
			return domain.Conflict("cancel_forbidden_state", "已通过或已取消的任务不能取消，当前："+string(task.Status))
		}
		tasks, err := q.ListTasksByRun(ctx, task.RunID)
		if err != nil {
			return err
		}
		deps, err := q.ListTaskDepsByRun(ctx, task.RunID)
		if err != nil {
			return err
		}
		status := make(map[int64]domain.TaskStatus, len(tasks))
		for _, t := range tasks {
			status[t.ID] = t.Status
		}
		dependents := map[int64][]int64{}
		for _, d := range deps {
			dependents[d.DependsOnID] = append(dependents[d.DependsOnID], d.TaskID)
		}

		cancel := func(id int64, from *int64) error {
			if err := q.SetTaskCancelled(ctx, id); err != nil {
				return err
			}
			detail := map[string]any{"reason": reason}
			if from != nil {
				detail["cascade_from"] = *from
			}
			res.Cancelled = append(res.Cancelled, id)
			return q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &id, ActorID: &hub.ID, Type: domain.EvtTaskCancelled,
				DetailJSON: evtDetail(detail)})
		}
		if err := cancel(taskID, nil); err != nil {
			return err
		}
		seen := map[int64]bool{taskID: true}
		queue := []int64{taskID}
		for len(queue) > 0 {
			n := queue[0]
			queue = queue[1:]
			for _, m := range dependents[n] {
				if seen[m] {
					continue
				}
				seen[m] = true
				if s := status[m]; s != domain.TaskBlocked && s != domain.TaskReady {
					continue
				}
				if err := cancel(m, &taskID); err != nil {
					return err
				}
				queue = append(queue, m)
			}
		}
		if err := e.checkRunDone(ctx, q, task.RunID); err != nil {
			return err
		}
		res.Task, err = q.GetTask(ctx, taskID)
		return err
	})
	return res, err
}

// Reopen 中枢重开任务（failed / cancelled / passed）：依赖已全部通过则 ready，否则 blocked；不自动派工。
// 已完成的 run 重新置为进行中。
func (e *Engine) Reopen(ctx context.Context, hub domain.Agent, role domain.Role, taskID int64, reason string) (domain.Task, error) {
	var out domain.Task
	reason, err := requireReason(reason, "重开任务")
	if err != nil {
		return out, err
	}
	err = e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := loadTask(ctx, q, taskID)
		if err != nil {
			return err
		}
		run, _, err := hubRun(ctx, q, role, task.RunID, "重开任务")
		if err != nil {
			return err
		}
		if !domain.CanReopen(task.Status) {
			return domain.Conflict("cannot_reopen", "仅失败、已取消或已通过的任务可重开，当前："+string(task.Status))
		}
		depIDs, err := q.TaskDepIDs(ctx, taskID)
		if err != nil {
			return err
		}
		to := domain.TaskReady
		for _, id := range depIDs {
			dt, err := q.GetTask(ctx, id)
			if err != nil {
				return err
			}
			if dt.Status != domain.TaskPassed {
				to = domain.TaskBlocked
				break
			}
		}
		if err := q.ReopenTask(ctx, taskID, to); err != nil {
			return err
		}
		if run.Status == domain.RunDone {
			if err := q.SetRunStatus(ctx, run.ID, domain.RunActive); err != nil {
				return err
			}
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, ActorID: &hub.ID, Type: domain.EvtTaskReopened,
			DetailJSON: evtDetail(map[string]any{"reason": reason, "from": string(task.Status), "to": string(to)})}); err != nil {
			return err
		}
		if to == domain.TaskReady {
			if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, Type: domain.EvtTaskReady}); err != nil {
				return err
			}
		}
		out, err = q.GetTask(ctx, taskID)
		return err
	})
	return out, err
}
