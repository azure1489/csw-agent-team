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
