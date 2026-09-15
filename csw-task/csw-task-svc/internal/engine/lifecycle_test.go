package engine

import (
	"context"
	"testing"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func TestAckThenSubmit(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector", gates: []fxGate{{reviewer: "editor"}}}})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	writer, _ := who(t, st, "writer")

	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	a := taskByCode(t, st, res.Run.ID, "a")
	_, err = e.Ack(ctx, writer, a.ID)
	wantCode(t, err, "not_assignee")

	got, err := e.Ack(ctx, collector, a.ID)
	if err != nil || got.Status != domain.TaskInProgress || got.StartedAt == "" || got.LastActivityAt == "" {
		t.Fatalf("ack: %+v err=%v", got, err)
	}
	if !hasEvent(t, st, res.Run.ID, a.ID, domain.EvtTaskAcked, "") {
		t.Fatalf("missing task_acked event")
	}
	// 已接单再调用 = 心跳：状态不变、不重复写事件。
	if got, err = e.Ack(ctx, collector, a.ID); err != nil || got.Status != domain.TaskInProgress {
		t.Fatalf("heartbeat ack: %+v err=%v", got, err)
	}
	if _, err := e.Submit(ctx, collector, a.ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatalf("submit: %v", err)
	}
	_, err = e.Ack(ctx, collector, a.ID)
	wantCode(t, err, "cannot_ack")
}

func TestFailAndReopen(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector"}})
	editor, editorRole := who(t, st, "editor")
	collector, collectorRole := who(t, st, "collector")

	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	a := taskByCode(t, st, res.Run.ID, "a")
	_, err = e.Fail(ctx, collector, a.ID, "  ")
	wantCode(t, err, "reason_required")
	got, err := e.Fail(ctx, collector, a.ID, "来源全部 404")
	if err != nil || got.Status != domain.TaskFailed || got.FailReason != "来源全部 404" {
		t.Fatalf("fail: %+v err=%v", got, err)
	}
	_, err = e.Submit(ctx, collector, a.ID, SubmitInput{DownloadURL: "http://x/a"})
	wantCode(t, err, "cannot_submit")
	if mine, _ := st.Q().ListMyTasks(ctx, collector.ID, true); len(mine) != 0 {
		t.Fatalf("failed task should leave my open tasks, got %d", len(mine))
	}
	if failed, _ := st.Q().ListActiveRunTasksByStatus(ctx, domain.TaskFailed); len(failed) != 1 {
		t.Fatalf("failed queue want 1, got %d", len(failed))
	}

	_, err = e.Reopen(ctx, collector, collectorRole, a.ID, "再试")
	wantCode(t, err, "not_hub")
	got, err = e.Reopen(ctx, editor, editorRole, a.ID, "换一个来源再采")
	if err != nil || got.Status != domain.TaskReady || got.FailReason != "" {
		t.Fatalf("reopen: %+v err=%v", got, err)
	}
	// 重开不自动派工：中枢手动派，派工单版本 +1。
	dr, err := e.Dispatch(ctx, editor, editorRole, a.ID, "改用官网", nil)
	if err != nil || dr.Dispatch.Version != 2 {
		t.Fatalf("redispatch: v=%d err=%v", dr.Dispatch.Version, err)
	}
	if _, err := e.Submit(ctx, collector, a.ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatalf("submit after reopen: %v", err)
	}
	if run, _ := st.Q().GetRun(ctx, res.Run.ID); run.Status != domain.RunDone {
		t.Fatalf("run want done, got %s", run.Status)
	}
}

func TestCancelledRunDone(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	// a → b → c；a → d。取消 b 连带 c；d 通过后 run 完成。
	buildFlow(t, st, "auto", []fxStage{
		{code: "a", role: "collector"},
		{code: "b", role: "writer", mode: "manual", deps: []string{"a"}},
		{code: "c", role: "designer", deps: []string{"b"}},
		{code: "d", role: "researcher", deps: []string{"a"}},
	})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	researcher, researcherRole := who(t, st, "researcher")

	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if _, err := e.Submit(ctx, collector, taskByCode(t, st, runID, "a").ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatalf("submit a: %v", err)
	}
	b := taskByCode(t, st, runID, "b")
	_, err = e.Cancel(ctx, researcher, researcherRole, b.ID, "本期不做")
	wantCode(t, err, "not_hub")
	_, err = e.Cancel(ctx, editor, editorRole, b.ID, "")
	wantCode(t, err, "reason_required")
	cr, err := e.Cancel(ctx, editor, editorRole, b.ID, "本期不纳入小红书")
	if err != nil || len(cr.Cancelled) != 2 {
		t.Fatalf("cancel b: %+v err=%v", cr, err)
	}
	if got := taskByCode(t, st, runID, "c").Status; got != domain.TaskCancelled {
		t.Fatalf("c should cascade-cancel, got %s", got)
	}
	_, err = e.Cancel(ctx, editor, editorRole, taskByCode(t, st, runID, "a").ID, "x")
	wantCode(t, err, "cancel_forbidden_state")

	if run, _ := st.Q().GetRun(ctx, runID); run.Status != domain.RunActive {
		t.Fatalf("run should stay active while d open, got %s", run.Status)
	}
	if _, err := e.Submit(ctx, researcher, taskByCode(t, st, runID, "d").ID, SubmitInput{DownloadURL: "http://x/d"}); err != nil {
		t.Fatalf("submit d: %v", err)
	}
	if run, _ := st.Q().GetRun(ctx, runID); run.Status != domain.RunDone {
		t.Fatalf("run want done (all terminal, some passed), got %s", run.Status)
	}

	// 重开已通过的 d：run 回到进行中，d 回 ready 等手动派工。
	d, err := e.Reopen(ctx, editor, editorRole, taskByCode(t, st, runID, "d").ID, "补一条")
	if err != nil || d.Status != domain.TaskReady {
		t.Fatalf("reopen d: %+v err=%v", d, err)
	}
	if run, _ := st.Q().GetRun(ctx, runID); run.Status != domain.RunActive {
		t.Fatalf("run want active after reopen, got %s", run.Status)
	}
	// 重开已取消的 c：依赖 b 未通过 → blocked。
	if c, err := e.Reopen(ctx, editor, editorRole, taskByCode(t, st, runID, "c").ID, "恢复"); err != nil || c.Status != domain.TaskBlocked {
		t.Fatalf("reopen c: %+v err=%v", c, err)
	}
}

func TestCancelledTaskNotReviewable(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector", gates: []fxGate{{reviewer: "editor"}}}})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	a := taskByCode(t, st, res.Run.ID, "a")
	d, err := e.Submit(ctx, collector, a.ID, SubmitInput{DownloadURL: "http://x/a"})
	if err != nil {
		t.Fatalf("submit: %v", err)
	}
	if _, err := e.Cancel(ctx, editor, editorRole, a.ID, "撤稿"); err != nil {
		t.Fatalf("cancel: %v", err)
	}
	_, err = e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass})
	wantCode(t, err, "not_in_review")
	if pending, _ := st.Q().ListPendingReviews(ctx); len(pending) != 0 {
		t.Fatalf("cancelled task must leave review queue, got %d", len(pending))
	}
}

func TestDueAtFromSLA(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector", sla: 30, gates: []fxGate{{reviewer: "editor"}}}})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	a := taskByCode(t, st, res.Run.ID, "a")
	due, err := time.Parse(time.RFC3339, a.DueAt)
	if err != nil {
		t.Fatalf("due_at %q: %v", a.DueAt, err)
	}
	if d := time.Until(due); d < 28*time.Minute || d > 31*time.Minute {
		t.Fatalf("due_at should be ~30m ahead, got %v", d)
	}
	if od, _ := st.Q().ListOverdueTasks(ctx); len(od) != 0 {
		t.Fatalf("not overdue yet, got %d", len(od))
	}
	if _, err := st.DB().Exec(`UPDATE tasks SET due_at='2000-01-01T00:00:00Z' WHERE id=?`, a.ID); err != nil {
		t.Fatal(err)
	}
	od, _ := st.Q().ListOverdueTasks(ctx)
	if len(od) != 1 || od[0].ID != a.ID {
		t.Fatalf("want a overdue, got %+v", od)
	}
	if err := st.Q().MarkOverdueNotified(ctx, a.ID); err != nil {
		t.Fatal(err)
	}
	if od, _ := st.Q().ListOverdueTasks(ctx); len(od) != 0 {
		t.Fatalf("overdue should be notified once, got %d", len(od))
	}
	// 退回 = 重新交办：按时限重新计时。
	d, err := e.Submit(ctx, collector, a.ID, SubmitInput{DownloadURL: "http://x/a"})
	if err != nil {
		t.Fatalf("submit: %v", err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictReject, ReturnDirection: "补来源", ReturnLocation: "第 2 条"}); err != nil {
		t.Fatalf("reject: %v", err)
	}
	if got := taskByCode(t, st, res.Run.ID, "a"); got.DueAt <= "2000-01-02" {
		t.Fatalf("due_at should restart after return, got %s", got.DueAt)
	}
}
