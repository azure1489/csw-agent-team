package engine

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// notice 一条通知的去向与内容：主送 target、抄送 cc（均为 role_code），hub 为该 run 的中枢角色。
// 去向由引擎按闸与角色推导，请求方不能指定；notifier 只负责渲染与发送。
type notice struct {
	payload map[string]any
	target  string
	hub     string
	cc      []string
}

// emit 写事件；n 非空时在同一事务里写 outbox——事件落库即必有通知，同一事件只发一次（event_id 唯一）。
func emit(ctx context.Context, q *sqlite.Queries, ev domain.Event, n *notice) error {
	id, err := q.InsertEventID(ctx, ev)
	if err != nil || n == nil {
		return err
	}
	payload := n.payload
	if payload == nil {
		payload = map[string]any{}
	}
	payload["hub_role"] = n.hub
	var cc []string
	seen := map[string]bool{n.target: true}
	for _, r := range n.cc {
		if r != "" && !seen[r] {
			seen[r] = true
			cc = append(cc, r)
		}
	}
	b, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	_, err = q.InsertOutbox(ctx, domain.Outbox{
		EventID: id, EventType: ev.Type, RunID: ev.RunID, TaskID: ev.TaskID, DeliverableID: ev.DeliverableID,
		TargetRole: n.target, CCRoles: strings.Join(cc, ","), PayloadJSON: string(b),
	})
	return err
}

// taskPayload 通知公共字段。
func taskPayload(task domain.Task, extra map[string]any) map[string]any {
	m := map[string]any{
		"run_id": task.RunID, "task_id": task.ID, "stage_name": task.StageName,
		"stage_code": task.StageCode, "role_code": task.RoleCode,
	}
	if task.ItemKey != "" {
		m["item_key"] = task.ItemKey
	}
	for k, v := range extra {
		m[k] = v
	}
	return m
}

// reviewNotice 待审通知：主送下一道闸的审核人；中枢代录的人工闸（Van）主送 Van、抄送中枢。
func reviewNotice(task domain.Task, hub string, tg domain.TaskGate, version int, delivID int64, url string, extra map[string]any) *notice {
	p := taskPayload(task, map[string]any{
		"version": version, "deliverable_id": delivID, "download_url": url,
		"next_gate": tg.GateOrder, "next_gate_name": tg.Name, "next_reviewer": tg.ReviewerRole, "relayed": tg.RelayedByHub,
	})
	for k, v := range extra {
		p[k] = v
	}
	n := &notice{target: tg.ReviewerRole, hub: hub, payload: p}
	if tg.RelayedByHub {
		n.cc = []string{hub}
	}
	return n
}

// stagePassed 任务末闸通过后的公共收尾：写 stage_passed（通知中枢，附下一棒），再重算下游就绪。
func (e *Engine) stagePassed(ctx context.Context, q *sqlite.Queries, task domain.Task, wf domain.Workflow, version int) error {
	downs, err := q.ListDirectDownstream(ctx, task.ID)
	if err != nil {
		return err
	}
	next := make([]string, 0, len(downs))
	for _, d := range downs {
		next = append(next, d.StageName)
	}
	if err := emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, Type: domain.EvtStagePassed},
		&notice{target: wf.HubRoleCode, hub: wf.HubRoleCode, payload: taskPayload(task, map[string]any{"version": version, "next_stages": next})}); err != nil {
		return err
	}
	if err := markItemWritten(ctx, q, task.RunID, task.ItemKey); err != nil {
		return err
	}
	return e.recomputeReady(ctx, q, task.RunID, wf)
}

// NotifyOverdue 逾期提醒：条件标记成功（仍逾期、未提醒）才写 overdue 事件与通知，同一次逾期只提醒一次。
func (e *Engine) NotifyOverdue(ctx context.Context, taskID int64) (bool, error) {
	var sent bool
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		ok, err := q.MarkOverdueNotified(ctx, taskID)
		if err != nil || !ok {
			return err
		}
		task, err := q.GetTask(ctx, taskID)
		if err != nil {
			return err
		}
		wf, err := taskWorkflow(ctx, q, task)
		if err != nil {
			return err
		}
		sent = true
		return emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, Type: domain.EvtOverdue,
			DetailJSON: evtDetail(map[string]any{"due_at": task.DueAt, "sla_minutes": task.SLAMinutes})},
			&notice{target: task.RoleCode, hub: wf.HubRoleCode, cc: []string{wf.HubRoleCode},
				payload: taskPayload(task, map[string]any{"due_at": task.DueAt, "sla_minutes": task.SLAMinutes})})
	})
	return sent, err
}

// NotifyRunStalled 整期停滞告警：run 还活着却没有任何人在动，提醒中枢接出下一步。
// 这是任务级三类告警的补位——中枢报完失败后任务是终态、下游是阻塞，那三类都不会触发。
func (e *Engine) NotifyRunStalled(ctx context.Context, runID int64) (bool, error) {
	var sent bool
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		ok, err := q.MarkRunStallNotified(ctx, runID)
		if err != nil || !ok {
			return err
		}
		run, err := q.GetRun(ctx, runID)
		if err != nil {
			return err
		}
		wf, err := q.GetWorkflow(ctx, run.WorkflowID)
		if err != nil {
			return err
		}
		pending, err := q.RunStallSummary(ctx, runID)
		if err != nil {
			return err
		}
		detail := map[string]any{"minutes": domain.RunStallMinutes, "subject": run.Subject, "pending": pending}
		n := &notice{target: wf.HubRoleCode, hub: wf.HubRoleCode,
			payload: map[string]any{"run_id": run.ID, "subject": run.Subject,
				"minutes": domain.RunStallMinutes, "pending": pending}}
		sent = true
		return emit(ctx, q, domain.Event{RunID: &run.ID, Type: domain.EvtRunStalled, DetailJSON: evtDetail(detail)}, n)
	})
	return sent, err
}

// NotifyStall 接续告警（未接单 / 未接单升级 / 接单后无活动），每类每次停滞只提醒一次；返回是否本次发出。
// 未接单与无活动主送执行者、抄送中枢；升级只主送中枢，由中枢改派、重开或取消。
func (e *Engine) NotifyStall(ctx context.Context, taskID int64, k domain.StallKind) (bool, error) {
	var sent bool
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		ok, err := q.MarkStallNotified(ctx, taskID, k)
		if err != nil || !ok {
			return err
		}
		task, err := q.GetTask(ctx, taskID)
		if err != nil {
			return err
		}
		wf, err := taskWorkflow(ctx, q, task)
		if err != nil {
			return err
		}
		n := &notice{target: task.RoleCode, hub: wf.HubRoleCode, cc: []string{wf.HubRoleCode}}
		var typ string
		var detail map[string]any
		switch k {
		case domain.StallAck:
			typ = domain.EvtAckOverdue
			detail = map[string]any{"minutes": domain.AckMinutesOf(task), "dispatched_at": task.DispatchedAt}
		case domain.StallEscalate:
			typ = domain.EvtAckEscalated
			n.target, n.cc = wf.HubRoleCode, nil
			detail = map[string]any{"minutes": domain.AckMinutesOf(task) * domain.AckEscalateFactor,
				"dispatched_at": task.DispatchedAt, "assignee_role": task.RoleCode}
		default:
			typ = domain.EvtTaskIdle
			detail = map[string]any{"minutes": domain.IdleMinutesOf(task), "last_activity_at": task.LastActivityAt}
		}
		n.payload = taskPayload(task, detail)
		sent = true
		return emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, Type: typ, DetailJSON: evtDetail(detail)}, n)
	})
	return sent, err
}

// NotifyReviewWaiting 待审提醒：交付物在这道闸上等了 waited 分钟还没人审，按档位再提醒一次审核方。
// 条件记次成功（提醒次数仍是 w.Reminded）才写事件，并发扫描同一档只发一次。
// Van 闸由中枢代录，提醒中枢去跟 Van；Van 本人只在升级时才被 @（见 NotifyReviewEscalated）。
func (e *Engine) NotifyReviewWaiting(ctx context.Context, w sqlite.ReviewWait, waited int) (bool, error) {
	var sent bool
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		ok, err := q.MarkReviewReminded(ctx, w.DeliverableID, w.GateOrder, w.Reminded)
		if err != nil || !ok {
			return err
		}
		task, err := q.GetTask(ctx, w.TaskID)
		if err != nil {
			return err
		}
		wf, err := taskWorkflow(ctx, q, task)
		if err != nil {
			return err
		}
		detail := reviewWaitDetail(w, waited, w.Reminded+1)
		n := &notice{target: w.ReviewerRole, hub: wf.HubRoleCode, payload: taskPayload(task, detail)}
		if w.RelayedByHub || w.ReviewerRole == "" {
			n.target = wf.HubRoleCode
		}
		sent = true
		return emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, DeliverableID: &w.DeliverableID,
			Type: domain.EvtReviewWaiting, DetailJSON: evtDetail(detail)}, n)
	})
	return sent, err
}

// NotifyReviewEscalated 待审升级：提醒够次数仍无动作，@ Van、抄送中枢。同一次等待只升级一次。
func (e *Engine) NotifyReviewEscalated(ctx context.Context, w sqlite.ReviewWait, waited int) (bool, error) {
	var sent bool
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		ok, err := q.MarkReviewEscalated(ctx, w.DeliverableID, w.GateOrder)
		if err != nil || !ok {
			return err
		}
		task, err := q.GetTask(ctx, w.TaskID)
		if err != nil {
			return err
		}
		wf, err := taskWorkflow(ctx, q, task)
		if err != nil {
			return err
		}
		detail := reviewWaitDetail(w, waited, w.Reminded)
		sent = true
		return emit(ctx, q, domain.Event{RunID: &task.RunID, TaskID: &task.ID, DeliverableID: &w.DeliverableID,
			Type: domain.EvtReviewEscalated, DetailJSON: evtDetail(detail)},
			&notice{target: "van", hub: wf.HubRoleCode, cc: []string{wf.HubRoleCode}, payload: taskPayload(task, detail)})
	})
	return sent, err
}

func reviewWaitDetail(w sqlite.ReviewWait, waited, reminded int) map[string]any {
	return map[string]any{
		"version": w.Version, "deliverable_id": w.DeliverableID, "gate_name": w.GateName,
		"reviewer_role": w.ReviewerRole, "relayed": w.RelayedByHub,
		"minutes": waited, "reminded": reminded, "since": w.Since,
	}
}
