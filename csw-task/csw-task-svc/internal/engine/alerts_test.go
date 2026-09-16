package engine

import (
	"context"
	"fmt"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// TestStallAlertsOnceEach 接续告警：派工超阈值未接单 → 提醒执行者一次（抄送中枢）；超过 3 倍 → 升级中枢一次；
// 重开再派后接单标记清零；接单后无活动 → 提醒一次，心跳后可再次提醒。
func TestStallAlertsOnceEach(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector", ack: 4, gates: []fxGate{{reviewer: "editor"}}}})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	a := taskByCode(t, st, runID, "a")
	if a.Status != domain.TaskDispatched || a.AckMinutes != 4 || a.IdleMinutes != 0 {
		t.Fatalf("snapshot: status=%s ack=%d idle=%d", a.Status, a.AckMinutes, a.IdleMinutes)
	}
	stalled := func(k domain.StallKind) int {
		t.Helper()
		ts, err := st.Q().ListStalledTasks(ctx, k)
		if err != nil {
			t.Fatalf("list %s: %v", k, err)
		}
		return len(ts)
	}
	shift := func(col string, minutes int) {
		t.Helper()
		if _, err := st.DB().Exec(`UPDATE tasks SET `+col+`=strftime('%Y-%m-%dT%H:%M:%SZ','now', ?) WHERE id=?`,
			fmt.Sprintf("-%d minutes", minutes), a.ID); err != nil {
			t.Fatal(err)
		}
	}
	notify := func(k domain.StallKind, want bool) {
		t.Helper()
		ok, err := e.NotifyStall(ctx, a.ID, k)
		if err != nil || ok != want {
			t.Fatalf("notify %s: ok=%v want %v err=%v", k, ok, want, err)
		}
	}

	if stalled(domain.StallAck) != 0 {
		t.Fatal("just dispatched: not stalled yet")
	}
	shift("dispatched_at", 5)
	if stalled(domain.StallAck) != 1 || stalled(domain.StallEscalate) != 0 {
		t.Fatalf("5m > 4m: ack due, escalate not")
	}
	notify(domain.StallAck, true)
	notify(domain.StallAck, false)
	if r := lastOf(t, st, domain.EvtAckOverdue); r.target != "collector" || r.cc != "editor" {
		t.Fatalf("ack_overdue target=%s cc=%s", r.target, r.cc)
	}
	if stalled(domain.StallEscalate) != 0 {
		t.Fatal("escalate needs 3x threshold")
	}
	shift("dispatched_at", 13)
	notify(domain.StallEscalate, true)
	notify(domain.StallEscalate, false)
	if r := lastOf(t, st, domain.EvtAckEscalated); r.target != "editor" || r.cc != "" {
		t.Fatalf("ack_escalated target=%s cc=%s", r.target, r.cc)
	}

	// 报失败 → 重开 → 再派：按新派工时间计，标记清零。
	if _, err := e.Fail(ctx, collector, a.ID, "源站打不开"); err != nil {
		t.Fatalf("fail: %v", err)
	}
	if _, err := e.Reopen(ctx, editor, editorRole, a.ID, "换来源重做"); err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if _, err := e.Dispatch(ctx, editor, editorRole, a.ID, "换 B 站源", nil); err != nil {
		t.Fatalf("redispatch: %v", err)
	}
	if stalled(domain.StallAck) != 0 || stalled(domain.StallEscalate) != 0 {
		t.Fatal("redispatch resets ack alerts")
	}

	// 接单后无活动（默认 10 分钟）。
	if _, err := e.Ack(ctx, collector, a.ID); err != nil {
		t.Fatalf("ack: %v", err)
	}
	if stalled(domain.StallAck) != 0 || stalled(domain.StallIdle) != 0 {
		t.Fatal("acked just now: nothing stalled")
	}
	shift("last_activity_at", 11)
	notify(domain.StallIdle, true)
	notify(domain.StallIdle, false)
	if r := lastOf(t, st, domain.EvtTaskIdle); r.target != "collector" || r.cc != "editor" {
		t.Fatalf("task_idle target=%s cc=%s", r.target, r.cc)
	}
	// 心跳清标记；再次沉默可再提醒。
	if _, err := e.Ack(ctx, collector, a.ID); err != nil {
		t.Fatalf("heartbeat: %v", err)
	}
	if stalled(domain.StallIdle) != 0 {
		t.Fatal("heartbeat resets idle")
	}
	shift("last_activity_at", 11)
	if stalled(domain.StallIdle) != 1 {
		t.Fatal("silent again: idle due again")
	}
}

// TestStallIgnoresOldHistory 停滞开始于 24 小时之前的任务（历史遗留）不补发接续告警。
func TestStallIgnoresOldHistory(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector", gates: []fxGate{{reviewer: "editor"}}}})
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	a := taskByCode(t, st, res.Run.ID, "a")
	if _, err := st.DB().Exec(`UPDATE tasks SET dispatched_at=strftime('%Y-%m-%dT%H:%M:%SZ','now','-25 hours') WHERE id=?`, a.ID); err != nil {
		t.Fatal(err)
	}
	for _, k := range []domain.StallKind{domain.StallAck, domain.StallEscalate, domain.StallIdle} {
		if ts, err := st.Q().ListStalledTasks(ctx, k); err != nil || len(ts) != 0 {
			t.Fatalf("%s: old stall must be ignored, got %d err=%v", k, len(ts), err)
		}
	}
	if ok, err := e.NotifyStall(ctx, a.ID, domain.StallAck); err != nil || ok {
		t.Fatalf("old stall must not be notified: ok=%v err=%v", ok, err)
	}
}

// TestRunStalledAlert 整期停滞：run 还 active 但没有任何已派工/进行中的任务、仍有没走完的阶段，
// 超过阈值要提醒中枢一次且只一次；有新动作后清掉标记，下次停滞还能再提醒。
// 这是任务级三类告警的补位——中枢报完失败后任务是终态、下游是阻塞，那三类都不触发（r44 实测沉默 47 分钟）。
func TestRunStalledAlert(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "停滞用例", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID

	// 刚触发时 01 已派工，不算停滞。
	if runs, err := st.Q().ListStalledRuns(ctx); err != nil {
		t.Fatal(err)
	} else if len(runs) != 0 {
		t.Fatalf("有已派工任务时不该判停滞，got %d", len(runs))
	}

	// 让 01 报失败：此时没有任何 dispatched/in_progress，下游仍 blocked —— 正是 r44 那个死角。
	intake := taskByCode(t, st, runID, "intake")
	collector, _ := who(t, st, "collector")
	if _, err := e.Ack(ctx, collector, intake.ID); err != nil {
		t.Fatalf("ack: %v", err)
	}
	if _, err := e.Fail(ctx, collector, intake.ID, "来源全部失效"); err != nil {
		t.Fatalf("fail: %v", err)
	}

	// 时间没到，先不提醒。
	if runs, _ := st.Q().ListStalledRuns(ctx); len(runs) != 0 {
		t.Fatalf("未到阈值不该提醒，got %d", len(runs))
	}

	// 把最近一次事件拨回到阈值之前。
	if _, err := st.DB().ExecContext(ctx,
		`UPDATE events SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ','now','-40 minutes') WHERE run_id=?`, runID); err != nil {
		t.Fatal(err)
	}
	runs, err := st.Q().ListStalledRuns(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(runs) != 1 || runs[0].ID != runID {
		t.Fatalf("应捞到停滞的 run，got %+v", runs)
	}

	sent, err := e.NotifyRunStalled(ctx, runID)
	if err != nil || !sent {
		t.Fatalf("首次应提醒：sent=%v err=%v", sent, err)
	}
	// 只提醒一次。
	if sent, _ := e.NotifyRunStalled(ctx, runID); sent {
		t.Fatal("同一次停滞不该重复提醒")
	}
	if runs, _ := st.Q().ListStalledRuns(ctx); len(runs) != 0 {
		t.Fatal("已提醒过的不该再被捞出")
	}

	// 通知内容要带卡点摘要。
	if !hasRunEvent(t, st, runID, domain.EvtRunStalled) {
		t.Fatal("应留下 run_stalled 事件")
	}
	summary, err := st.Q().RunStallSummary(ctx, runID)
	if err != nil || summary == "" {
		t.Fatalf("卡点摘要不该为空：%q err=%v", summary, err)
	}

	// 有新动作后清标记，下次停滞还能再提醒。
	if err := st.Q().ClearRunStallNotified(ctx, runID); err != nil {
		t.Fatal(err)
	}
	if runs, _ := st.Q().ListStalledRuns(ctx); len(runs) != 1 {
		t.Fatal("清掉标记后应能再次捞到")
	}
}

// hasRunEvent run 级事件（TaskID 为 nil），hasEvent 只适用于任务级。
func hasRunEvent(t *testing.T, st *sqlite.Store, runID int64, typ string) bool {
	t.Helper()
	evs, err := st.Q().ListEventsByRun(context.Background(), runID)
	if err != nil {
		t.Fatalf("events: %v", err)
	}
	for _, ev := range evs {
		if ev.Type == typ {
			return true
		}
	}
	return false
}

// TestRunStalledIgnoresOldRuns 长期挂着的旧 run（最近动静在 24 小时以前）不补发停滞告警。
// 上线当晚漏了这条下限，6 月以来 39 个旧 v1 run 一次性全被判停滞并 @ 了中枢。
func TestRunStalledIgnoresOldRuns(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "旧 run", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	intake := taskByCode(t, st, runID, "intake")
	collector, _ := who(t, st, "collector")
	if _, err := e.Ack(ctx, collector, intake.ID); err != nil {
		t.Fatalf("ack: %v", err)
	}
	if _, err := e.Fail(ctx, collector, intake.ID, "来源全部失效"); err != nil {
		t.Fatalf("fail: %v", err)
	}
	// 把全部事件拨到三天前：这是一个早就没人管的历史 run。
	if _, err := st.DB().ExecContext(ctx,
		`UPDATE events SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 days') WHERE run_id=?`, runID); err != nil {
		t.Fatal(err)
	}
	if _, err := st.DB().ExecContext(ctx,
		`UPDATE runs SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 days') WHERE id=?`, runID); err != nil {
		t.Fatal(err)
	}
	runs, err := st.Q().ListStalledRuns(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, r := range runs {
		if r.ID == runID {
			t.Fatal("超过 24 小时没动静的历史 run 不该补发停滞告警")
		}
	}
}
