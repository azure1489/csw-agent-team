package engine

import (
	"context"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func taskByItem(t *testing.T, st *sqlite.Store, runID int64, code, key string) domain.Task {
	t.Helper()
	ts, err := st.Q().ListTasksByRun(context.Background(), runID)
	if err != nil {
		t.Fatal(err)
	}
	for _, tk := range ts {
		if tk.StageCode == code && tk.ItemKey == key {
			return tk
		}
	}
	t.Fatalf("no task %s/%s", code, key)
	return domain.Task{}
}

// TestFullFlowV2 驱动 daily_news v2：0 闸直通、合流自动派主编、选题逐条批准生成条目任务、
// 手动派工、并行分支、完整审核稿等全部条目、平台写停在待授权、授权后续派、run done。
func TestFullFlowV2(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")

	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-16", "v2", `{"授权":["local_drill"]}`)
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if len(res.Tasks) != 11 { // 13 阶段，逐条阶段（write / material）不建模板任务
		t.Fatalf("want 11 tasks, got %d", len(res.Tasks))
	}
	if ft := taskByCode(t, st, runID, "fulltext"); ft.WaitItemStages != "write,material" {
		t.Fatalf("fulltext wait snapshot: %q", ft.WaitItemStages)
	}
	expectT := func(tk domain.Task, want domain.TaskStatus) {
		t.Helper()
		got, _ := st.Q().GetTask(ctx, tk.ID)
		if got.Status != want {
			t.Fatalf("%s/%s want %s, got %s", tk.StageCode, tk.ItemKey, want, got.Status)
		}
	}
	expect := func(code string, want domain.TaskStatus) { t.Helper(); expectT(taskByCode(t, st, runID, code), want) }
	submitT := func(tk domain.Task, role string) domain.Deliverable {
		t.Helper()
		a, _ := who(t, st, role)
		d, err := e.Submit(ctx, a, tk.ID, SubmitInput{DownloadURL: "http://x/" + tk.StageCode + "/" + tk.ItemKey, SelfCheck: "ok"})
		if err != nil {
			t.Fatalf("submit %s/%s: %v", tk.StageCode, tk.ItemKey, err)
		}
		return d
	}
	passT := func(tk domain.Task, d domain.Deliverable, extra ReviewInput) {
		t.Helper()
		tgs, _ := st.Q().ListTaskGates(ctx, tk.ID)
		if len(tgs) == 0 && d.Status != domain.DelPassed {
			t.Fatalf("%s 0-gate submit want passed, got %s", tk.StageCode, d.Status)
		}
		for i, g := range tgs {
			in := ReviewInput{Verdict: domain.VerdictPass, Comment: "ok"}
			if g.RelayedByHub {
				in = extra
				if in.SourceQuote == "" {
					in.SourceQuote = "Van：可以"
				}
				in.Verdict = domain.VerdictPass
			}
			r, err := e.Review(ctx, editor, editorRole, d.ID, in)
			if err != nil {
				t.Fatalf("review %s gate %d: %v", tk.StageCode, i+1, err)
			}
			if (i == len(tgs)-1) != r.Final {
				t.Fatalf("%s gate %d final=%v", tk.StageCode, i+1, r.Final)
			}
		}
		expectT(tk, domain.TaskPassed)
	}
	pass := func(code, role string, extra ReviewInput) {
		t.Helper()
		tk := taskByCode(t, st, runID, code)
		passT(tk, submitT(tk, role), extra)
	}
	dispatch := func(tk domain.Task, note string) {
		t.Helper()
		if _, err := e.Dispatch(ctx, editor, editorRole, tk.ID, note, nil); err != nil {
			t.Fatalf("dispatch %s/%s: %v", tk.StageCode, tk.ItemKey, err)
		}
	}

	expect("intake", domain.TaskDispatched)
	pass("intake", "collector", ReviewInput{})
	pass("shortlist", "researcher", ReviewInput{})

	// 03 选题：Van 闸逐条批准两条 → 每条生成写作与配图任务。
	expect("topic", domain.TaskDispatched)
	pass("topic", "editor", ReviewInput{DecisionType: "topic_approve", SourceQuote: "Van：HxO 和 NANGA 可以写",
		ItemsJSON: `["hxo-a1b2c3","nanga-d4e5f6"]`})
	items, _ := st.Q().ListItems(ctx, runID)
	if len(items) != 2 || items[0].Status != domain.ItemApprovedWrite {
		t.Fatalf("items after topic approval: %+v", items)
	}
	wH, wN := taskByItem(t, st, runID, "write", "hxo-a1b2c3"), taskByItem(t, st, runID, "write", "nanga-d4e5f6")
	mH, mN := taskByItem(t, st, runID, "material", "hxo-a1b2c3"), taskByItem(t, st, runID, "material", "nanga-d4e5f6")
	expectT(wH, domain.TaskReady) // 手动派工
	expectT(wN, domain.TaskReady)
	expectT(mH, domain.TaskDispatched) // 自动派工
	expectT(mN, domain.TaskDispatched)
	expect("design_prep", domain.TaskDispatched)
	expect("fulltext", domain.TaskBlocked)

	dispatch(wH, "写 HxO（Van：可以写）")
	dispatch(wN, "写 NANGA（Van：可以写）")
	passT(wH, submitT(wH, "writer"), ReviewInput{})
	passT(wN, submitT(wN, "writer"), ReviewInput{})
	passT(mH, submitT(mH, "collector"), ReviewInput{})
	expect("fulltext", domain.TaskBlocked) // 还等 NANGA 的配图
	passT(mN, submitT(mN, "collector"), ReviewInput{})
	for _, it := range func() []domain.RunItem { x, _ := st.Q().ListItems(ctx, runID); return x }() {
		if it.Status != domain.ItemWritten {
			t.Fatalf("item %s want written, got %s", it.ItemKey, it.Status)
		}
	}

	// 07 全文：合流自动派主编，主编定点编辑 → Van 全文。
	expect("fulltext", domain.TaskDispatched)
	pass("fulltext", "editor", ReviewInput{DecisionType: "fulltext_approve"})
	expect("wx_layout", domain.TaskBlocked) // 还等 design_prep
	expect("xhs_text", domain.TaskReady)    // 手动，首期不派
	pass("design_prep", "designer", ReviewInput{})
	expect("wx_layout", domain.TaskDispatched)
	pass("wx_layout", "publisher", ReviewInput{})

	// 09 草稿保存：平台写，缺授权停在 ready。
	wxSave := taskByCode(t, st, runID, "wx_save")
	expect("wx_save", domain.TaskReady)
	if !hasEvent(t, st, runID, wxSave.ID, domain.EvtAuthorizationRequired, `"scope":"wx_draft"`) {
		t.Fatalf("wx_save should record authorization_required")
	}
	_, err = e.Dispatch(ctx, editor, editorRole, wxSave.ID, "存草稿", nil)
	wantCode(t, err, "authorization_required")
	g, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：今天可以存草稿", nil)
	if err != nil || len(g.Dispatched) != 1 || g.Dispatched[0] != wxSave.ID {
		t.Fatalf("grant should resume wx_save: %+v err=%v", g, err)
	}
	pass("wx_save", "publisher", ReviewInput{})

	// 小红书支线：10 手动派 → 主编 + Van 两道 → 11/12 自动 → 13 平台写待授权。
	dispatch(taskByCode(t, st, runID, "xhs_text"), "本期纳入小红书")
	pass("xhs_text", "xhswriter", ReviewInput{})
	expect("xhs_visual", domain.TaskDispatched)
	expect("xhs_package", domain.TaskBlocked)
	pass("xhs_visual", "designer", ReviewInput{})
	pass("xhs_package", "publisher", ReviewInput{})
	expect("xhs_save", domain.TaskReady)
	if g, err = e.Grant(ctx, editor, editorRole, runID, domain.ScopeXhsPublish, "Van：小红书直接发", nil); err != nil || len(g.Dispatched) != 1 {
		t.Fatalf("grant xhs_publish should resume xhs_save: %+v err=%v", g, err)
	}
	pass("xhs_save", "publisher", ReviewInput{})

	run, err := st.Q().GetRun(ctx, runID)
	if err != nil || run.Status != domain.RunDone {
		t.Fatalf("run want done, got %s err=%v", run.Status, err)
	}
}
