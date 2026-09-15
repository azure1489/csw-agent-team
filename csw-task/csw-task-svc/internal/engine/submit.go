package engine

import (
	"context"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// SubmitInput agent 提交产出入参。
type SubmitInput struct {
	DocType     string
	DownloadURL string
	Filename    string
	Title       string
	Summary     string
	MetaJSON    string
	SelfCheck   string
	FileID      *int64
	Upstreams   []UpstreamInput // 合流阶段可显式传上游
}

// Submit agent 提交产出：守卫状态 → version+1 → 0 闸直 passed，否则入 review。
func (e *Engine) Submit(ctx context.Context, agent domain.Agent, taskID int64, in SubmitInput) (domain.Deliverable, error) {
	var out domain.Deliverable
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := q.GetTask(ctx, taskID)
		if isNoRows(err) {
			return domain.NotFound("task_not_found", "无此任务")
		}
		if err != nil {
			return err
		}

		// 权限：仅该任务 assignee（合流阶段=中枢）；assignee 未解析时回退到角色匹配。
		if err := checkAssignee(task, agent, "提交产出"); err != nil {
			return err
		}
		if !domain.CanSubmit(task.Status) {
			return domain.Conflict("cannot_submit", "任务当前状态不可提交："+string(task.Status))
		}
		if err := RequireAuthorization(ctx, q, task); err != nil {
			return err
		}

		// doc_type 缺省取阶段产出类型（快照列）；两者皆空才报错。
		if in.DocType == "" {
			in.DocType = task.OutputType
		}
		if in.DocType == "" {
			return domain.BadRequest("doc_type_required", "doc_type 缺失且该阶段未定义产出类型")
		}

		ver, err := q.MaxDeliverableVersion(ctx, taskID, false)
		if err != nil {
			return err
		}
		ver++

		producer := agent.ID
		id, err := q.InsertDeliverable(ctx, domain.Deliverable{
			TaskID:      taskID,
			IsDispatch:  false,
			Version:     ver,
			DocType:     in.DocType,
			ProducerID:  &producer,
			FileID:      in.FileID,
			DownloadURL: in.DownloadURL,
			Filename:    in.Filename,
			Title:       in.Title,
			Summary:     in.Summary,
			MetaJSON:    in.MetaJSON,
			SelfCheck:   in.SelfCheck,
			CurGate:     0,
			Status:      domain.DelSubmitted,
		})
		if err != nil {
			return err
		}
		for _, u := range in.Upstreams {
			if err := q.InsertUpstream(ctx, id, u.Label, u.URL, u.UpstreamID); err != nil {
				return err
			}
		}

		if in.FileID != nil {
			if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, DeliverableID: &id, ActorID: &producer, Type: domain.EvtFileUploaded}); err != nil {
				return err
			}
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, DeliverableID: &id, ActorID: &producer, Type: domain.EvtSubmitted}); err != nil {
			return err
		}
		// 有产出即有活动；清逾期提醒标记（退回后再逾期可再提醒一次）。
		if err := q.TouchTaskActivity(ctx, taskID); err != nil {
			return err
		}
		if err := q.ClearOverdueNotified(ctx, taskID); err != nil {
			return err
		}

		gates, err := q.ListTaskGates(ctx, taskID)
		if err != nil {
			return err
		}
		if len(gates) == 0 {
			// 0 闸阶段：提交即 passed，触发下游就绪重算。
			if err := q.SetDeliverableGate(ctx, id, 0, domain.DelPassed, nil); err != nil {
				return err
			}
			if err := q.SetTaskPassed(ctx, taskID, ver); err != nil {
				return err
			}
			if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, Type: domain.EvtStagePassed}); err != nil {
				return err
			}
			run, err := q.GetRun(ctx, task.RunID)
			if err != nil {
				return err
			}
			wf, err := q.GetWorkflow(ctx, run.WorkflowID)
			if err != nil {
				return err
			}
			if err := e.recomputeReady(ctx, q, task.RunID, wf); err != nil {
				return err
			}
		} else if err := q.SetTaskReview(ctx, taskID, ver); err != nil {
			return err
		}

		out, err = q.GetDeliverable(ctx, id)
		return err
	})
	return out, err
}
