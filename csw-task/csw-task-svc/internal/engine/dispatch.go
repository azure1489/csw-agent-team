package engine

import (
	"context"
	"fmt"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// DispatchResult 派工结果。
type DispatchResult struct {
	Dispatch  domain.Deliverable
	Upstreams []domain.Upstream
}

// Dispatch 中枢派工（手动）：仅 ready 任务；建派工单 + 预填上游（传了则覆盖，否则自动）。
func (e *Engine) Dispatch(ctx context.Context, hub domain.Agent, role domain.Role, taskID int64, editorNote string, upstreams []UpstreamInput) (DispatchResult, error) {
	var res DispatchResult
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := q.GetTask(ctx, taskID)
		if isNoRows(err) {
			return domain.NotFound("task_not_found", "无此任务")
		}
		if err != nil {
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
		if role.Code != wf.HubRoleCode {
			return domain.Forbidden("not_hub", "仅中枢角色可派工")
		}
		if task.Status != domain.TaskReady {
			return domain.Conflict("task_not_ready", "仅 ready 任务可派工，当前："+string(task.Status))
		}
		if err := RequireAuthorization(ctx, q, task); err != nil {
			return err
		}

		var explicit []UpstreamInput
		if len(upstreams) > 0 {
			explicit = upstreams
		}
		producer := hub.ID
		res.Dispatch, err = e.dispatchInternal(ctx, q, task, wf, editorNote, explicit, &producer)
		if err != nil {
			return err
		}
		res.Upstreams, err = q.ListUpstreams(ctx, res.Dispatch.ID)
		return err
	})
	return res, err
}

// dispatchInternal 建派工单交付物 + 预填上游 + task→dispatched + 事件。供手动派工与自动派工复用。
func (e *Engine) dispatchInternal(ctx context.Context, q *sqlite.Queries, task domain.Task, wf domain.Workflow, editorNote string, explicit []UpstreamInput, producerID *int64) (domain.Deliverable, error) {
	ver, err := q.MaxDeliverableVersion(ctx, task.ID, domain.KindDispatch)
	if err != nil {
		return domain.Deliverable{}, err
	}
	ver++

	id, err := q.InsertDeliverable(ctx, domain.Deliverable{
		TaskID:     task.ID,
		Kind:       domain.KindDispatch,
		IsDispatch: true,
		Version:    ver,
		DocType:    dispatchDocType,
		ProducerID: producerID,
		EditorNote: editorNote,
		Status:     domain.DelIssued,
		CurGate:    0,
	})
	if err != nil {
		return domain.Deliverable{}, err
	}

	ups := explicit
	if ups == nil {
		if ups, err = e.autoPrefill(ctx, q, task); err != nil {
			return domain.Deliverable{}, err
		}
	}
	for _, u := range ups {
		if err := q.InsertUpstream(ctx, id, u.Label, u.URL, u.UpstreamID); err != nil {
			return domain.Deliverable{}, err
		}
	}

	if err := q.SetTaskDispatched(ctx, task.ID); err != nil {
		return domain.Deliverable{}, err
	}
	if task.SLAMinutes > 0 {
		if err := q.SetTaskDue(ctx, task.ID, task.SLAMinutes); err != nil {
			return domain.Deliverable{}, err
		}
	}
	if err := emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, DeliverableID: &id, ActorID: producerID, Type: domain.EvtDispatched},
		&notice{target: task.RoleCode, hub: wf.HubRoleCode, payload: taskPayload(task, map[string]any{"note": editorNote, "sla_minutes": task.SLAMinutes})}); err != nil {
		return domain.Deliverable{}, err
	}
	return q.GetDeliverable(ctx, id)
}

// autoPrefill 自动预填上游：各依赖任务已通过的最新产出，以及该任务的全部补件（「补件#n」）。
func (e *Engine) autoPrefill(ctx context.Context, q *sqlite.Queries, task domain.Task) ([]UpstreamInput, error) {
	depIDs, err := q.TaskDepIDs(ctx, task.ID)
	if err != nil {
		return nil, err
	}
	var ups []UpstreamInput
	for _, depID := range depIDs {
		dt, err := q.GetTask(ctx, depID)
		if err != nil {
			return nil, err
		}
		dd, err := q.LatestPassedDeliverable(ctx, depID)
		switch {
		case isNoRows(err):
		case err != nil:
			return nil, err
		case dd.DownloadURL != "":
			upID := dd.ID
			ups = append(ups, UpstreamInput{Label: dt.StageName, URL: dd.DownloadURL, UpstreamID: &upID})
		}
		sups, err := q.ListSupplementsByTask(ctx, depID)
		if err != nil {
			return nil, err
		}
		for _, s := range sups {
			if s.DownloadURL == "" {
				continue
			}
			sid := s.ID
			ups = append(ups, UpstreamInput{Label: fmt.Sprintf("%s 补件#%d", dt.StageName, s.Version), URL: s.DownloadURL, UpstreamID: &sid})
		}
	}
	return ups, nil
}
