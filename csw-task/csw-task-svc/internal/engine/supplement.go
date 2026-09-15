package engine

import (
	"context"
	"fmt"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// CheckAccept 某类交付物在任务当前状态下能否提交；不能时返回对应业务错误（handler 预检与引擎共用）。
func CheckAccept(kind domain.DeliverableKind, s domain.TaskStatus) error {
	if domain.CanAccept(kind, s) {
		return nil
	}
	switch kind {
	case domain.KindSupplement:
		return domain.Conflict("supplement_requires_passed", "补件只能挂在已通过的任务上，当前："+string(s))
	case domain.KindEdit:
		return domain.Conflict("cannot_edit", "仅审核中的任务可定点编辑，当前："+string(s))
	case "", domain.KindOutput:
		return domain.Conflict("cannot_submit", "任务当前状态不可提交："+string(s))
	}
	return domain.BadRequest("bad_kind", "kind 须为 output / supplement / edit")
}

func taskWorkflow(ctx context.Context, q *sqlite.Queries, task domain.Task) (domain.Workflow, error) {
	run, err := q.GetRun(ctx, task.RunID)
	if err != nil {
		return domain.Workflow{}, err
	}
	return q.GetWorkflow(ctx, run.WorkflowID)
}

// submitSupplement 补件：挂在已通过的任务上（换图、换资产、补事实），不重开审核、不触碰已批准版本。
// 直接下游中已经开工的任务标「待返工」，未开工的在派工预填时自动带上补件。
func (e *Engine) submitSupplement(ctx context.Context, agent domain.Agent, taskID int64, in SubmitInput) (domain.Deliverable, error) {
	var out domain.Deliverable
	if in.AffectsID == nil {
		return out, domain.BadRequest("affects_required", "补件须指明受影响的产出交付物（affects_deliverable_id）")
	}
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := loadTask(ctx, q, taskID)
		if err != nil {
			return err
		}
		wf, err := taskWorkflow(ctx, q, task)
		if err != nil {
			return err
		}
		if checkAssignee(task, agent, "提交补件") != nil && agent.RoleCode != wf.HubRoleCode {
			return domain.Forbidden("not_assignee", "仅该任务负责人或中枢可提交补件")
		}
		if err := CheckAccept(domain.KindSupplement, task.Status); err != nil {
			return err
		}
		aff, err := q.GetDeliverable(ctx, *in.AffectsID)
		if isNoRows(err) || (err == nil && (aff.TaskID != taskID || (aff.Kind != domain.KindOutput && aff.Kind != domain.KindEdit))) {
			return domain.BadRequest("bad_affects", "受影响的交付物须是本任务的产出版本")
		}
		if err != nil {
			return err
		}
		if in.DocType == "" {
			in.DocType = task.OutputType
		}
		ver, err := q.MaxDeliverableVersion(ctx, taskID, domain.KindSupplement)
		if err != nil {
			return err
		}
		producer := agent.ID
		id, err := q.InsertDeliverable(ctx, domain.Deliverable{
			TaskID: taskID, Kind: domain.KindSupplement, Version: ver + 1, DocType: in.DocType,
			ProducerID: &producer, FileID: in.FileID, DownloadURL: in.DownloadURL, Filename: in.Filename,
			Title: in.Title, Summary: in.Summary, MetaJSON: in.MetaJSON, SelfCheck: in.SelfCheck,
			AffectsID: in.AffectsID, Status: domain.DelPassed,
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

		downs, err := q.ListDirectDownstream(ctx, taskID)
		if err != nil {
			return err
		}
		rework := []int64{}
		var reworkNames, reworkRoles []string
		for _, t := range downs {
			switch t.Status {
			case domain.TaskDispatched, domain.TaskInProgress, domain.TaskReview, domain.TaskReturned:
				if err := q.SetTaskReworkPending(ctx, t.ID, true); err != nil {
					return err
				}
				rework = append(rework, t.ID)
				reworkNames = append(reworkNames, t.StageName)
				reworkRoles = append(reworkRoles, t.RoleCode)
			}
		}
		if err := emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &taskID, DeliverableID: &id, ActorID: &producer,
			Type: domain.EvtSupplementArrived, DetailJSON: evtDetail(map[string]any{
				"affects_deliverable_id": *in.AffectsID, "version": ver + 1, "rework_task_ids": rework,
			})}, &notice{target: wf.HubRoleCode, hub: wf.HubRoleCode, cc: reworkRoles, payload: taskPayload(task, map[string]any{
			"version": ver + 1, "affects_deliverable_id": *in.AffectsID, "download_url": in.DownloadURL,
			"summary": in.Summary, "rework_stages": reworkNames,
		})}); err != nil {
			return err
		}
		out, err = q.GetDeliverable(ctx, id)
		return err
	})
	return out, err
}

// submitEdit 主编定点编辑：任务在审核中、且当前待审的闸是中枢自己的闸时，中枢提交一个编辑版本。
// 首稿标 superseded 原样保留；编辑版本从该闸之后进入（自动写一条 edit_pass 审核留痕），末闸即直接通过。
func (e *Engine) submitEdit(ctx context.Context, agent domain.Agent, taskID int64, in SubmitInput) (domain.Deliverable, error) {
	var out domain.Deliverable
	diff := strings.TrimSpace(in.DiffSummary)
	if diff == "" {
		return out, domain.BadRequest("diff_summary_required", "定点编辑须写修改摘要（diff_summary）")
	}
	if in.EditOf == nil {
		return out, domain.BadRequest("edit_of_required", "定点编辑须指明基于的版本（edit_of）")
	}
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		task, err := loadTask(ctx, q, taskID)
		if err != nil {
			return err
		}
		wf, err := taskWorkflow(ctx, q, task)
		if err != nil {
			return err
		}
		if agent.RoleCode != wf.HubRoleCode {
			return domain.Forbidden("not_hub", "仅中枢角色可定点编辑")
		}
		if err := CheckAccept(domain.KindEdit, task.Status); err != nil {
			return err
		}
		if *in.EditOf != task.CurVersion {
			return domain.Conflict("edit_of_mismatch", fmt.Sprintf("定点编辑须基于当前版本 v%d，收到 v%d", task.CurVersion, *in.EditOf))
		}
		cur, err := q.OutputByVersion(ctx, taskID, task.CurVersion)
		if err != nil {
			return err
		}
		if cur.Status != domain.DelSubmitted && cur.Status != domain.DelInReview {
			return domain.Conflict("not_in_review", "当前版本不在待审状态："+string(cur.Status))
		}
		nextGate := cur.CurGate + 1
		tg, err := q.TaskGateByOrder(ctx, taskID, nextGate)
		if isNoRows(err) {
			return domain.Conflict("no_gate", "无对应审核闸")
		}
		if err != nil {
			return err
		}
		if tg.RelayedByHub || tg.ReviewerRole != wf.HubRoleCode {
			return domain.Conflict("edit_gate_not_yours", "当前待审的是「"+tg.Name+"」，不是中枢的闸，不能定点编辑")
		}
		gates, err := q.ListTaskGates(ctx, taskID)
		if err != nil {
			return err
		}
		final := nextGate >= len(gates)

		ver, err := q.MaxDeliverableVersion(ctx, taskID, domain.KindOutput)
		if err != nil {
			return err
		}
		ver++
		if in.DocType == "" {
			in.DocType = cur.DocType
		}
		if err := q.SetDeliverableStatus(ctx, cur.ID, domain.DelSuperseded); err != nil {
			return err
		}
		status := domain.DelInReview
		if final {
			status = domain.DelPassed
		}
		producer := agent.ID
		id, err := q.InsertDeliverable(ctx, domain.Deliverable{
			TaskID: taskID, Kind: domain.KindEdit, Version: ver, DocType: in.DocType,
			ProducerID: &producer, FileID: in.FileID, DownloadURL: in.DownloadURL, Filename: in.Filename,
			Title: in.Title, Summary: in.Summary, MetaJSON: in.MetaJSON, SelfCheck: in.SelfCheck,
			EditOf: in.EditOf, DiffSummary: diff, Collab: true, CurGate: nextGate, Status: status,
		})
		if err != nil {
			return err
		}
		// 上游：沿用首稿的上游，并链回首稿本身（首稿 / 协作稿 / 终审稿三份可追溯）。
		ups, err := q.ListUpstreams(ctx, cur.ID)
		if err != nil {
			return err
		}
		for _, u := range ups {
			if err := q.InsertUpstream(ctx, id, u.Label, u.UpstreamURL, u.UpstreamID); err != nil {
				return err
			}
		}
		if cur.DownloadURL != "" {
			curID := cur.ID
			if err := q.InsertUpstream(ctx, id, fmt.Sprintf("首稿 v%d", cur.Version), cur.DownloadURL, &curID); err != nil {
				return err
			}
		}
		for _, u := range in.Upstreams {
			if err := q.InsertUpstream(ctx, id, u.Label, u.URL, u.UpstreamID); err != nil {
				return err
			}
		}
		if _, err := q.InsertReview(ctx, domain.Review{
			DeliverableID: id, TaskGateID: tg.ID, ReviewerID: &producer, Verdict: domain.VerdictPass,
			Comment: diff, DecisionType: string(domain.DecEditPass),
		}); err != nil {
			return err
		}
		if in.FileID != nil {
			if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, DeliverableID: &id, ActorID: &producer, Type: domain.EvtFileUploaded}); err != nil {
				return err
			}
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &task.RunID, TaskID: &taskID, DeliverableID: &id, ActorID: &producer, Type: domain.EvtSubmitted,
			DetailJSON: evtDetail(map[string]any{"kind": string(domain.KindEdit), "edit_of": *in.EditOf, "diff_summary": diff})}); err != nil {
			return err
		}
		var gn *notice // 未到末闸：通知下一道闸（通常是 Van 闸）审编辑版本
		if !final {
			ng, err := q.TaskGateByOrder(ctx, taskID, nextGate+1)
			if err != nil {
				return err
			}
			gn = reviewNotice(task, wf.HubRoleCode, ng, ver, id, in.DownloadURL, map[string]any{"passed_gate_name": tg.Name, "edit": true, "diff_summary": diff})
		}
		if err := emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &taskID, DeliverableID: &id, ActorID: &producer, Type: domain.EvtGatePassed,
			DetailJSON: evtDetail(map[string]any{"gate": nextGate, "gate_name": tg.Name, "edit": true, "final": final})}, gn); err != nil {
			return err
		}
		if err := q.TouchTaskActivity(ctx, taskID); err != nil {
			return err
		}
		if final {
			if err := q.SetTaskPassed(ctx, taskID, ver); err != nil {
				return err
			}
			if err := e.stagePassed(ctx, q, task, wf, ver); err != nil {
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
