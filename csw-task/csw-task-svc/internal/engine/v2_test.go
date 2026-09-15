package engine

import (
	"context"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// TestFullFlowV2 驱动 daily_news v2 十三阶段：0 闸直通、合流自动派主编、阶段级手动派工、
// 并行分支、平台写停在待授权、授权后续派、run done。
func TestFullFlowV2(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")

	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-16", "v2", `{"授权":["local_drill"]}`)
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if len(res.Tasks) != 13 {
		t.Fatalf("want 13 tasks, got %d", len(res.Tasks))
	}
	status := func(code string) domain.TaskStatus { return taskByCode(t, st, runID, code).Status }
	expect := func(code string, want domain.TaskStatus) {
		t.Helper()
		if got := status(code); got != want {
			t.Fatalf("%s want %s, got %s", code, want, got)
		}
	}
	submit := func(code, role string) domain.Deliverable {
		t.Helper()
		a, _ := who(t, st, role)
		d, err := e.Submit(ctx, a, taskByCode(t, st, runID, code).ID, SubmitInput{DownloadURL: "http://x/" + code, SelfCheck: "ok"})
		if err != nil {
			t.Fatalf("submit %s: %v", code, err)
		}
		return d
	}
	pass := func(code string, d domain.Deliverable, gates int) {
		t.Helper()
		if gates == 0 {
			if d.Status != domain.DelPassed {
				t.Fatalf("%s 0-gate submit want passed, got %s", code, d.Status)
			}
			return
		}
		tgs, _ := st.Q().ListTaskGates(ctx, d.TaskID)
		for i := 1; i <= gates; i++ {
			in := ReviewInput{Verdict: domain.VerdictPass, Comment: "ok"}
			if tgs[i-1].RelayedByHub {
				in.SourceQuote = "Van：可以"
			}
			r, err := e.Review(ctx, editor, editorRole, d.ID, in)
			if err != nil {
				t.Fatalf("review %s gate %d: %v", code, i, err)
			}
			if (i == gates) != r.Final {
				t.Fatalf("%s gate %d final=%v", code, i, r.Final)
			}
		}
		expect(code, domain.TaskPassed)
	}
	manualDispatch := func(code, note string) {
		t.Helper()
		if _, err := e.Dispatch(ctx, editor, editorRole, taskByCode(t, st, runID, code).ID, note, nil); err != nil {
			t.Fatalf("dispatch %s: %v", code, err)
		}
	}

	// 01/02：auto 自动派，0 闸提交即通过。
	expect("intake", domain.TaskDispatched)
	pass("intake", submit("intake", "collector"), 0)
	expect("shortlist", domain.TaskDispatched)
	pass("shortlist", submit("shortlist", "researcher"), 0)

	// 03 选题：合流自动派给主编，主编自审 → Van 选题。
	expect("topic", domain.TaskDispatched)
	if tk := taskByCode(t, st, runID, "topic"); tk.AssigneeID == nil || *tk.AssigneeID != editor.ID {
		t.Fatalf("topic assignee should be hub")
	}
	pass("topic", submit("topic", "editor"), 2)

	// 选题后三路并行：write 手动（要写已批条目），material / design_prep 自动。
	expect("write", domain.TaskReady)
	expect("material", domain.TaskDispatched)
	expect("design_prep", domain.TaskDispatched)
	manualDispatch("write", "已批：HxO")
	pass("write", submit("write", "writer"), 1)
	expect("fulltext", domain.TaskBlocked) // 还等 material
	pass("material", submit("material", "collector"), 0)

	// 07 全文：合流自动派主编，主编定点编辑 → Van 全文。
	expect("fulltext", domain.TaskDispatched)
	pass("fulltext", submit("fulltext", "editor"), 2)
	expect("wx_layout", domain.TaskBlocked) // 还等 design_prep
	expect("xhs_text", domain.TaskReady)    // 手动，首期不派
	pass("design_prep", submit("design_prep", "designer"), 0)

	expect("wx_layout", domain.TaskDispatched)
	pass("wx_layout", submit("wx_layout", "publisher"), 1)

	// 09 草稿保存：平台写，缺授权停在 ready。
	wxSave := taskByCode(t, st, runID, "wx_save")
	expect("wx_save", domain.TaskReady)
	if !hasEvent(t, st, runID, wxSave.ID, domain.EvtAuthorizationRequired, `"scope":"wx_draft"`) {
		t.Fatalf("wx_save should record authorization_required")
	}
	_, err = e.Dispatch(ctx, editor, editorRole, wxSave.ID, "存草稿", nil)
	wantCode(t, err, "authorization_required")
	g, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：今天可以存草稿", nil)
	if err != nil {
		t.Fatalf("grant wx_draft: %v", err)
	}
	if len(g.Dispatched) != 1 || g.Dispatched[0] != wxSave.ID {
		t.Fatalf("grant should resume wx_save, got %v", g.Dispatched)
	}
	pass("wx_save", submit("wx_save", "publisher"), 0)

	// 小红书支线：10 手动派 → 主编 + Van 两道 → 11/12 自动 → 13 平台写待授权。
	manualDispatch("xhs_text", "本期纳入小红书")
	pass("xhs_text", submit("xhs_text", "xhswriter"), 2)
	expect("xhs_visual", domain.TaskDispatched)
	expect("xhs_package", domain.TaskBlocked)
	pass("xhs_visual", submit("xhs_visual", "designer"), 1)
	pass("xhs_package", submit("xhs_package", "publisher"), 1)
	expect("xhs_save", domain.TaskReady)
	if g, err = e.Grant(ctx, editor, editorRole, runID, domain.ScopeXhsPublish, "Van：小红书直接发", nil); err != nil || len(g.Dispatched) != 1 {
		t.Fatalf("grant xhs_publish should resume xhs_save: %+v err=%v", g, err)
	}
	pass("xhs_save", submit("xhs_save", "publisher"), 0)

	run, err := st.Q().GetRun(ctx, runID)
	if err != nil || run.Status != domain.RunDone {
		t.Fatalf("run want done, got %s err=%v", run.Status, err)
	}
}
