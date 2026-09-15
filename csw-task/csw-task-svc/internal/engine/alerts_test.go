package engine

import (
	"context"
	"fmt"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
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
