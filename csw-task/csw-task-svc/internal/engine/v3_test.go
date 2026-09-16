package engine

import (
	"context"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// TestFullFlowV3 驱动 daily_news v4（v3 的形态加 01 主编首批校准闸）：01 首批交齐由主编校准；写作随条目决定自动派工（派工单带条目与 Van 原话）；07 内容整合稿主编单闸；
// 08 完整审核稿主编核版 → Van 全文终审；09 缺授权停在待授权、授权后续派；10 / 11 手动不派，
// 主编取消后 12 / 13 级联取消；run done。
func TestFullFlowV3(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")

	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v3", `{"授权":["local_drill"]}`)
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if res.Run.WorkflowVer != 4 || len(res.Tasks) != 11 {
		t.Fatalf("want v4 with 11 template tasks, got v%d %d", res.Run.WorkflowVer, len(res.Tasks))
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
	passT := func(tk domain.Task, d domain.Deliverable, van ReviewInput) {
		t.Helper()
		tgs, _ := st.Q().ListTaskGates(ctx, tk.ID)
		for i, g := range tgs {
			in := ReviewInput{Verdict: domain.VerdictPass, Comment: "ok"}
			if g.RelayedByHub {
				in = van
				in.Verdict = domain.VerdictPass
			}
			if _, err := e.Review(ctx, editor, editorRole, d.ID, in); err != nil {
				t.Fatalf("review %s gate %d: %v", tk.StageCode, i+1, err)
			}
		}
		expectT(tk, domain.TaskPassed)
	}
	pass := func(code, role string, van ReviewInput) {
		t.Helper()
		tk := taskByCode(t, st, runID, code)
		passT(tk, submitT(tk, role), van)
	}
	gatesOf := func(code string) []string {
		t.Helper()
		tgs, _ := st.Q().ListTaskGates(ctx, taskByCode(t, st, runID, code).ID)
		var out []string
		for _, g := range tgs {
			out = append(out, g.ReviewerRole)
		}
		return out
	}
	if got := strings.Join(gatesOf("intake"), ","); got != "editor" {
		t.Fatalf("01 gates: %s", got)
	}
	if got := strings.Join(gatesOf("fulltext"), ","); got != "editor" {
		t.Fatalf("07 gates: %s", got)
	}
	if got := strings.Join(gatesOf("wx_layout"), ","); got != "editor,van" {
		t.Fatalf("08 gates: %s", got)
	}

	pass("intake", "collector", ReviewInput{})
	collector, collectorRole := who(t, st, "collector")
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "hxo-a1b2c3", Brand: "HxO", Title: "折叠木椅"}, {Key: "nanga-d4e5f6", Brand: "NANGA", Title: "羽绒睡袋"},
	}); err != nil {
		t.Fatalf("upsert items: %v", err)
	}
	pass("shortlist", "researcher", ReviewInput{})

	// 03 选题：Van 逐条批准 → 写作与配图都自动派工。
	pass("topic", "editor", ReviewInput{DecisionType: "topic_approve", SourceQuote: "Van：HxO 和 NANGA 可以写", ItemsJSON: `["hxo-a1b2c3","nanga-d4e5f6"]`})
	wH, wN := taskByItem(t, st, runID, "write", "hxo-a1b2c3"), taskByItem(t, st, runID, "write", "nanga-d4e5f6")
	mH, mN := taskByItem(t, st, runID, "material", "hxo-a1b2c3"), taskByItem(t, st, runID, "material", "nanga-d4e5f6")
	for _, tk := range []domain.Task{wH, wN, mH, mN} {
		expectT(tk, domain.TaskDispatched)
	}
	var note string
	if err := st.DB().QueryRow(`SELECT COALESCE(editor_note,'') FROM deliverables WHERE task_id=? AND kind='dispatch'`, wH.ID).Scan(&note); err != nil ||
		!strings.Contains(note, "HxO｜折叠木椅") || !strings.Contains(note, "Van：HxO 和 NANGA 可以写") {
		t.Fatalf("write dispatch note: %q err=%v", note, err)
	}
	expect("design_prep", domain.TaskDispatched)
	expect("fulltext", domain.TaskBlocked)
	for _, p := range []struct {
		tk   domain.Task
		role string
	}{{wH, "writer"}, {wN, "writer"}, {mH, "collector"}, {mN, "collector"}} {
		passT(p.tk, submitT(p.tk, p.role), ReviewInput{})
	}

	// 07 内容整合稿：合流自动派主编，只有主编一道闸。
	expect("fulltext", domain.TaskDispatched)
	pass("fulltext", "editor", ReviewInput{})
	expect("wx_layout", domain.TaskBlocked) // 还等 06
	expect("xhs_text", domain.TaskBlocked)  // 小红书改为依赖 08
	expect("xhs_pick", domain.TaskBlocked)
	pass("design_prep", "designer", ReviewInput{})

	// 08 完整审核稿：主编核版 → Van 全文终审。
	expect("wx_layout", domain.TaskDispatched)
	pass("wx_layout", "publisher", ReviewInput{DecisionType: "fulltext_approve", SourceQuote: "Van：这期可以"})

	// 09 平台写：缺授权停在待授权，授权后自动派。
	wxSave := taskByCode(t, st, runID, "wx_save")
	expect("wx_save", domain.TaskReady)
	if !hasEvent(t, st, runID, wxSave.ID, domain.EvtAuthorizationRequired, `"scope":"wx_draft"`) {
		t.Fatal("wx_save should record authorization_required")
	}
	if g, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：全文获批后存草稿", nil); err != nil || len(g.Dispatched) != 1 {
		t.Fatalf("grant should resume wx_save: %+v err=%v", g, err)
	}
	pass("wx_save", "publisher", ReviewInput{})

	// 小红书首期不派：10 / 11 停在待派工；主编取消两者，12 / 13 级联取消。
	expect("xhs_text", domain.TaskReady)
	expect("xhs_pick", domain.TaskReady)
	for _, code := range []string{"xhs_text", "xhs_pick"} {
		if _, err := e.Cancel(ctx, editor, editorRole, taskByCode(t, st, runID, code).ID, "首期不纳入小红书"); err != nil {
			t.Fatalf("cancel %s: %v", code, err)
		}
	}
	expect("xhs_package", domain.TaskCancelled)
	expect("xhs_save", domain.TaskCancelled)

	run, err := st.Q().GetRun(ctx, runID)
	if err != nil || run.Status != domain.RunDone {
		t.Fatalf("run want done, got %s err=%v", run.Status, err)
	}
}

// TestXhsBranchV3 小红书接入后：10 与 11 并行，12 等两者，主编审图文 → Van 小红书终审。
func TestXhsBranchV3(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-22", "v3", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	// 直接把 08 以前的任务置为通过，只看小红书支线的推进。
	if _, err := st.DB().Exec(`UPDATE tasks SET status='passed' WHERE run_id=? AND seq <= 8`, runID); err != nil {
		t.Fatal(err)
	}
	lay := taskByCode(t, st, runID, "wx_layout") // 事务外先取：库只有一个连接，事务里不能再走 st.Q()
	if err := st.Tx(ctx, func(q *sqlite.Queries) error {
		wf, err := taskWorkflow(ctx, q, lay)
		if err != nil {
			return err
		}
		return e.recomputeReady(ctx, q, runID, wf)
	}); err != nil {
		t.Fatalf("recompute: %v", err)
	}
	for _, code := range []string{"xhs_text", "xhs_pick"} {
		tk := taskByCode(t, st, runID, code)
		if tk.Status != domain.TaskReady {
			t.Fatalf("%s want ready (manual), got %s", code, tk.Status)
		}
		if _, err := e.Dispatch(ctx, editor, editorRole, tk.ID, "本期纳入小红书", nil); err != nil {
			t.Fatalf("dispatch %s: %v", code, err)
		}
	}
	xhswriter, _ := who(t, st, "xhswriter")
	collector, _ := who(t, st, "collector")
	d, err := e.Submit(ctx, xhswriter, taskByCode(t, st, runID, "xhs_text").ID, SubmitInput{DownloadURL: "http://x/xhs_text"})
	if err != nil {
		t.Fatalf("submit xhs_text: %v", err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatalf("review xhs_text: %v", err)
	}
	if got := taskByCode(t, st, runID, "xhs_package").Status; got != domain.TaskBlocked {
		t.Fatalf("12 waits for 11, got %s", got)
	}
	if _, err := e.Submit(ctx, collector, taskByCode(t, st, runID, "xhs_pick").ID, SubmitInput{DownloadURL: "http://x/xhs_pick"}); err != nil {
		t.Fatalf("submit xhs_pick: %v", err)
	}
	pkg := taskByCode(t, st, runID, "xhs_package")
	if pkg.Status != domain.TaskDispatched {
		t.Fatalf("12 want dispatched after 10 and 11, got %s", pkg.Status)
	}
	tgs, _ := st.Q().ListTaskGates(ctx, pkg.ID)
	if len(tgs) != 2 || tgs[0].ReviewerRole != "editor" || tgs[1].ReviewerRole != "van" || !tgs[1].RelayedByHub {
		t.Fatalf("12 gates: %+v", tgs)
	}
}
