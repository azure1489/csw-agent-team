package engine

import (
	"context"
	"fmt"
	"regexp"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// itemKeyRe 条目键：小写字母数字与短横，2–64 位（登记方生成后不再变，如 hxo-3fa91c）。
var itemKeyRe = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{1,63}$`)

// ItemInput 条目登记入参。
type ItemInput struct {
	Key         string
	Title       string
	Brand       string
	Product     string
	SourceURL   string
	PublishedAt string
	Status      string // 空 / candidate / shortlisted
}

// UpsertItems 登记或更新条目（本 run 的参与角色或中枢）：只能写 candidate / shortlisted，已决定的条目不被覆盖。
func (e *Engine) UpsertItems(ctx context.Context, actor domain.Agent, role domain.Role, runID int64, items []ItemInput) ([]domain.RunItem, error) {
	if len(items) == 0 {
		return nil, domain.BadRequest("items_required", "items 不能为空")
	}
	for _, it := range items {
		if !itemKeyRe.MatchString(it.Key) {
			return nil, domain.BadRequest("bad_item_key", "条目键须为小写字母数字与短横（2–64 位），收到："+it.Key)
		}
		if it.Status != "" && it.Status != string(domain.ItemCandidate) && it.Status != string(domain.ItemShortlisted) {
			return nil, domain.BadRequest("bad_item_status", "登记只能写 candidate / shortlisted；批准、暂缓、否决由中枢按 Van 原话决定")
		}
	}
	var out []domain.RunItem
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, err := q.GetRun(ctx, runID)
		if isNoRows(err) {
			return domain.NotFound("run_not_found", "无此实例")
		}
		if err != nil {
			return err
		}
		if run.Status != domain.RunActive {
			return domain.Conflict("run_not_active", "仅进行中的 run 可登记条目")
		}
		wf, err := q.GetWorkflow(ctx, run.WorkflowID)
		if err != nil {
			return err
		}
		if role.Code != wf.HubRoleCode {
			tasks, err := q.ListTasksByRun(ctx, runID)
			if err != nil {
				return err
			}
			member := false
			for _, t := range tasks {
				if t.RoleCode == role.Code {
					member = true
					break
				}
			}
			if !member {
				return domain.Forbidden("not_participant", "只有本期 run 的参与角色或中枢可以登记条目")
			}
		}
		for _, it := range items {
			if err := q.UpsertItem(ctx, domain.RunItem{
				RunID: runID, ItemKey: it.Key, Title: it.Title, Brand: it.Brand, Product: it.Product,
				SourceURL: it.SourceURL, PublishedAt: it.PublishedAt, Status: domain.ItemStatus(it.Status),
			}); err != nil {
				return err
			}
		}
		out, err = q.ListItems(ctx, runID)
		return err
	})
	return out, err
}

// ItemDecision 条目决定结果。
type ItemDecision struct {
	Item    domain.RunItem
	Spawned []int64 // 本次生成的逐条任务
}

// DecideItem 中枢按 Van 原话决定条目：批准可写 → 生成逐条任务；只开研究 → 不派写作；
// 暂缓 / 否决已批准的条目 → 取消其未完成的逐条任务，并摘掉合流任务对它的等待。
func (e *Engine) DecideItem(ctx context.Context, hub domain.Agent, role domain.Role, runID int64, key, decision, quote string) (ItemDecision, error) {
	var res ItemDecision
	if _, ok := domain.ItemDecisionStatus(decision); !ok {
		return res, domain.BadRequest("bad_decision", "decision 须为 approve_write / approve_research / defer / reject")
	}
	quote = strings.TrimSpace(quote)
	if quote == "" {
		return res, domain.BadRequest("source_quote_required", "条目决定须附 Van 原话（source_quote）")
	}
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, wf, err := hubRun(ctx, q, role, runID, "决定条目")
		if err != nil {
			return err
		}
		if run.Status != domain.RunActive {
			return domain.Conflict("run_not_active", "仅进行中的 run 可决定条目")
		}
		if res.Spawned, err = e.decideInTx(ctx, q, run, wf, hub, key, decision, quote, false); err != nil {
			return err
		}
		if err := e.recomputeReady(ctx, q, runID, wf); err != nil {
			return err
		}
		res.Item, err = q.GetItem(ctx, runID, key)
		return err
	})
	return res, err
}

// decideInTx 事务内应用一条条目决定。autoCreate=true 时未登记的条目按键名自动登记（审核随附 items 的场景）。
func (e *Engine) decideInTx(ctx context.Context, q *sqlite.Queries, run domain.Run, wf domain.Workflow, actor domain.Agent, key, decision, quote string, autoCreate bool) ([]int64, error) {
	status, _ := domain.ItemDecisionStatus(decision)
	if !itemKeyRe.MatchString(key) {
		return nil, domain.BadRequest("bad_item_key", "条目键须为小写字母数字与短横（2–64 位），收到："+key)
	}
	it, err := q.GetItem(ctx, run.ID, key)
	if isNoRows(err) {
		if !autoCreate {
			return nil, domain.NotFound("item_not_found", "本期没有条目："+key)
		}
		if err := q.UpsertItem(ctx, domain.RunItem{RunID: run.ID, ItemKey: key, Title: key}); err != nil {
			return nil, err
		}
		it, err = q.GetItem(ctx, run.ID, key)
	}
	if err != nil {
		return nil, err
	}
	if it.Status == status {
		return nil, nil // 同一决定重复录入：幂等
	}
	if domain.ItemCounted(it.Status) {
		return nil, domain.Conflict("item_already_written", "该条已写成，不能再改决定；需要撤下请重开相关任务")
	}
	if err := q.SetItemDecision(ctx, run.ID, key, status, &actor.ID, quote); err != nil {
		return nil, err
	}
	var spawned []int64
	switch {
	case status == domain.ItemApprovedWrite:
		if spawned, err = e.spawnItemTasks(ctx, q, run, wf, key); err != nil {
			return nil, err
		}
	case it.Status == domain.ItemApprovedWrite:
		if err := withdrawItemTasks(ctx, q, run.ID, key, &actor.ID, "条目改为"+string(status)); err != nil {
			return nil, err
		}
	}
	return spawned, q.InsertEvent(ctx, domain.Event{RunID: &run.ID, ActorID: &actor.ID, Type: domain.EvtItemDecided,
		DetailJSON: evtDetail(map[string]any{"item_key": key, "decision": decision, "source_quote": quote, "spawned": spawned})})
}

func csvHas(csv, v string) bool {
	for _, x := range strings.Split(csv, ",") {
		if x == v {
			return true
		}
	}
	return false
}

// spawnItemTasks 为批准可写的条目生成全部逐条阶段的任务（按 seq，快照同 Trigger），并补依赖：
// 上游是逐条阶段 → 指向同条目任务；否则指向该阶段的整期任务。等这一逐条阶段的合流任务补一条等待边；
// 合流任务已开工（新条目晚到）时标「补件待返工」。已存在的条目任务跳过（幂等）。
func (e *Engine) spawnItemTasks(ctx context.Context, q *sqlite.Queries, run domain.Run, wf domain.Workflow, key string) ([]int64, error) {
	stages, err := q.ListStages(ctx, run.WorkflowID)
	if err != nil {
		return nil, err
	}
	sdeps, err := q.ListStageDeps(ctx, run.WorkflowID)
	if err != nil {
		return nil, err
	}
	gates, err := q.ListGates(ctx, run.WorkflowID)
	if err != nil {
		return nil, err
	}
	tasks, err := q.ListTasksByRun(ctx, run.ID)
	if err != nil {
		return nil, err
	}
	byID := map[int64]domain.Stage{}
	for _, st := range stages {
		byID[st.ID] = st
	}
	find := func(code, item string) (domain.Task, bool) {
		for _, t := range tasks {
			if t.StageCode == code && t.ItemKey == item {
				return t, true
			}
		}
		return domain.Task{}, false
	}
	var spawned []int64
	for _, st := range stages {
		if !st.PerItem {
			continue
		}
		if _, ok := find(st.Code, key); ok {
			continue
		}
		t := domain.Task{
			RunID: run.ID, StageCode: st.Code, ItemKey: key, StageName: st.Name, Seq: st.Seq, RoleCode: st.RoleCode,
			IsMerge: st.IsMerge, OutputType: st.OutputType, Instructions: combine(wf.CommonInstructions, st.Instructions),
			SelfCheckCriteria: st.SelfCheckCriteria, Acceptance: combine(wf.CommonAcceptance, st.Acceptance),
			DispatchMode: stageDispatchMode(st, wf), ActionClass: st.ActionClass, SLAMinutes: st.SLAMinutes,
			AckMinutes: st.AckMinutes, IdleMinutes: st.IdleMinutes,
			AssigneeID: resolveAssignee(ctx, q, st.RoleCode), Status: domain.TaskBlocked,
		}
		id, err := q.InsertTask(ctx, t)
		if err != nil {
			return nil, err
		}
		t.ID = id
		tasks = append(tasks, t)
		for _, d := range sdeps {
			if d.StageID != st.ID {
				continue
			}
			up := byID[d.DependsOnID]
			upItem := ""
			if up.PerItem {
				upItem = key
			}
			if ut, ok := find(up.Code, upItem); ok {
				if err := q.InsertTaskDep(ctx, id, ut.ID); err != nil {
					return nil, err
				}
			}
		}
		for _, g := range resolveGates(gates, st.ID) {
			if err := q.InsertTaskGate(ctx, id, g.GateOrder, g.ReviewerRole, g.RelayedByHub, g.Name); err != nil {
				return nil, err
			}
		}
		for _, w := range tasks {
			if w.ItemKey != "" || !csvHas(w.WaitItemStages, st.Code) {
				continue
			}
			if err := q.InsertTaskDep(ctx, w.ID, id); err != nil {
				return nil, err
			}
			if w.Status != domain.TaskBlocked && !domain.IsTerminal(w.Status) {
				if err := q.SetTaskReworkPending(ctx, w.ID, true); err != nil {
					return nil, err
				}
			}
		}
		spawned = append(spawned, id)
	}
	return spawned, nil
}

// withdrawItemTasks 撤回条目：摘掉合流任务对该条全部逐条任务的等待，取消其中尚未完成的。
func withdrawItemTasks(ctx context.Context, q *sqlite.Queries, runID int64, key string, by *int64, reason string) error {
	tasks, err := q.ListTasksByRun(ctx, runID)
	if err != nil {
		return err
	}
	deps, err := q.ListTaskDepsByRun(ctx, runID)
	if err != nil {
		return err
	}
	itemOf := map[int64]string{}
	for _, t := range tasks {
		itemOf[t.ID] = t.ItemKey
	}
	for _, t := range tasks {
		if t.ItemKey != key {
			continue
		}
		for _, d := range deps {
			if d.DependsOnID == t.ID && itemOf[d.TaskID] == "" {
				if err := q.DeleteTaskDep(ctx, d.TaskID, t.ID); err != nil {
					return err
				}
			}
		}
		if domain.IsTerminal(t.Status) {
			continue
		}
		if err := q.SetTaskCancelled(ctx, t.ID); err != nil {
			return err
		}
		id := t.ID
		if err := q.InsertEvent(ctx, domain.Event{RunID: &runID, TaskID: &id, ActorID: by, Type: domain.EvtTaskCancelled,
			DetailJSON: evtDetail(map[string]any{"reason": reason, "item_key": key})}); err != nil {
			return err
		}
	}
	return nil
}

// markItemWritten 条目的逐条任务全部通过 → 条目置 written（计入整期已写成条数）。
func markItemWritten(ctx context.Context, q *sqlite.Queries, runID int64, key string) error {
	if key == "" {
		return nil
	}
	tasks, err := q.ListTasksByRun(ctx, runID)
	if err != nil {
		return err
	}
	for _, t := range tasks {
		if t.ItemKey == key && t.Status != domain.TaskPassed {
			return nil
		}
	}
	it, err := q.GetItem(ctx, runID, key)
	if isNoRows(err) {
		return nil
	}
	if err != nil {
		return err
	}
	if it.Status == domain.ItemApprovedWrite {
		return q.SetItemStatus(ctx, runID, key, domain.ItemWritten)
	}
	return nil
}

// CloseRun 中枢接受缺口结束 run：须附原因；要求全部任务已终态且至少一个通过。
func (e *Engine) CloseRun(ctx context.Context, hub domain.Agent, role domain.Role, runID int64, reason string) (domain.Run, error) {
	var out domain.Run
	reason, err := requireReason(reason, "结束实例")
	if err != nil {
		return out, err
	}
	err = e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, _, err := hubRun(ctx, q, role, runID, "结束实例")
		if err != nil {
			return err
		}
		if run.Status != domain.RunActive {
			return domain.Conflict("run_not_active", "实例不在进行中："+string(run.Status))
		}
		tasks, err := q.ListTasksByRun(ctx, runID)
		if err != nil {
			return err
		}
		var open []string
		passed := false
		for _, t := range tasks {
			if !domain.IsTerminal(t.Status) {
				open = append(open, fmt.Sprintf("%s(%s)", t.StageName, t.Status))
			}
			if t.Status == domain.TaskPassed {
				passed = true
			}
		}
		if len(open) > 0 {
			return domain.Conflict("run_has_open_tasks", "还有未完成的任务："+strings.Join(open, "、")+"；先完成、取消或重开处理")
		}
		if !passed {
			return domain.Conflict("run_nothing_passed", "没有任何已交付的任务，不能按完成结束")
		}
		items, err := q.ListItems(ctx, runID)
		if err != nil {
			return err
		}
		written := 0
		for _, it := range items {
			if domain.ItemCounted(it.Status) {
				written++
			}
		}
		if err := q.SetRunStatus(ctx, runID, domain.RunDone); err != nil {
			return err
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &runID, ActorID: &hub.ID, Type: domain.EvtRunClosed,
			DetailJSON: evtDetail(map[string]any{"reason": reason, "target_count": run.TargetCount, "written": written})}); err != nil {
			return err
		}
		if err := q.InsertEvent(ctx, domain.Event{RunID: &runID, Type: domain.EvtRunDone}); err != nil {
			return err
		}
		out, err = q.GetRun(ctx, runID)
		return err
	})
	return out, err
}
