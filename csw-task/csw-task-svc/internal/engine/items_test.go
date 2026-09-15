package engine

import (
	"context"
	"fmt"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// itemFlow：a（采集 0 闸）→ b（主编合流，主编 → Van）→ c（写作·逐条·手动·主编闸）、d（配图·逐条·自动·0 闸）、f（设计·0 闸）；
// e（主编合流·主编闸）依赖 c、d。
func itemFlow(t *testing.T, inputs string) (*Engine, *sqlite.Store, context.Context, int64) {
	t.Helper()
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{
		{code: "a", role: "collector"},
		{code: "b", role: "editor", merge: true, deps: []string{"a"}, gates: []fxGate{{reviewer: "editor"}, {reviewer: "van", relayed: true}}},
		{code: "c", role: "writer", mode: "manual", perItem: true, deps: []string{"b"}, gates: []fxGate{{reviewer: "editor"}}},
		{code: "d", role: "collector", perItem: true, deps: []string{"b"}},
		{code: "f", role: "designer", deps: []string{"b"}},
		{code: "e", role: "editor", merge: true, deps: []string{"c", "d"}, gates: []fxGate{{reviewer: "editor"}}},
	})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", inputs)
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if len(res.Tasks) != 4 {
		t.Fatalf("per_item stages must not get template tasks: %d tasks", len(res.Tasks))
	}
	if _, err := e.Submit(ctx, collector, taskByCode(t, st, runID, "a").ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatal(err)
	}
	return e, st, ctx, runID
}

// passB 让合流 b 过两道闸（Van 闸可带条目决定）。
func passB(t *testing.T, e *Engine, st *sqlite.Store, ctx context.Context, runID int64, van ReviewInput) {
	t.Helper()
	editor, editorRole := who(t, st, "editor")
	d, err := e.Submit(ctx, editor, taskByCode(t, st, runID, "b").ID, SubmitInput{DownloadURL: "http://x/b"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	van.Verdict = domain.VerdictPass
	if van.SourceQuote == "" {
		van.SourceQuote = "Van：可以"
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, van); err != nil {
		t.Fatalf("van gate: %v", err)
	}
}

func finishItem(t *testing.T, e *Engine, st *sqlite.Store, ctx context.Context, runID int64, key string) {
	t.Helper()
	editor, editorRole := who(t, st, "editor")
	writer, _ := who(t, st, "writer")
	collector, _ := who(t, st, "collector")
	c := taskByItem(t, st, runID, "c", key)
	if c.Status == domain.TaskReady {
		if _, err := e.Dispatch(ctx, editor, editorRole, c.ID, "写 "+key, nil); err != nil {
			t.Fatal(err)
		}
	}
	d, err := e.Submit(ctx, writer, c.ID, SubmitInput{DownloadURL: "http://x/c/" + key})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Submit(ctx, collector, taskByItem(t, st, runID, "d", key).ID, SubmitInput{DownloadURL: "http://x/d/" + key}); err != nil {
		t.Fatal(err)
	}
}

func TestApproveWriteSpawnsItemTask(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "topic_approve", ItemsJSON: `["hxo-1"]`})
	c, d := taskByItem(t, st, runID, "c", "hxo-1"), taskByItem(t, st, runID, "d", "hxo-1")
	if c.Status != domain.TaskReady || d.Status != domain.TaskDispatched {
		t.Fatalf("spawned tasks: c=%s d=%s", c.Status, d.Status)
	}
	if c.DispatchMode != domain.DispatchManual || taskByCode(t, st, runID, "f").Status != domain.TaskDispatched {
		t.Fatalf("snapshot / parallel branch wrong")
	}
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskBlocked {
		t.Fatalf("merge waits for item tasks, got %s", got)
	}
	it, _ := st.Q().GetItem(ctx, runID, "hxo-1")
	if it.Status != domain.ItemApprovedWrite || it.DecisionSource != "Van：可以" {
		t.Fatalf("item decision: %+v", it)
	}
}

func TestFulltextWaitsForAnApprovedItem(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	passB(t, e, st, ctx, runID, ReviewInput{}) // 没批条目
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskBlocked {
		t.Fatalf("merge with zero approved items must stay blocked, got %s", got)
	}
}

func TestFulltextReadyByApprovedItems(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	editor, editorRole := who(t, st, "editor")
	if _, err := e.UpsertItems(ctx, editor, editorRole, runID, []ItemInput{{Key: "hxo-1", Title: "HxO 折叠木椅"}, {Key: "nanga-2", Title: "NANGA 羽绒"}}); err != nil {
		t.Fatal(err)
	}
	passB(t, e, st, ctx, runID, ReviewInput{})
	for _, k := range []string{"hxo-1", "nanga-2"} {
		if r, err := e.DecideItem(ctx, editor, editorRole, runID, k, "approve_write", "Van：这两条都写"); err != nil || len(r.Spawned) != 2 {
			t.Fatalf("decide %s: %+v err=%v", k, r, err)
		}
	}
	finishItem(t, e, st, ctx, runID, "hxo-1")
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskBlocked {
		t.Fatalf("e must wait for nanga, got %s", got)
	}
	finishItem(t, e, st, ctx, runID, "nanga-2")
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskDispatched {
		t.Fatalf("e should be dispatched after all approved items, got %s", got)
	}
	for _, k := range []string{"hxo-1", "nanga-2"} {
		if it, _ := st.Q().GetItem(ctx, runID, k); it.Status != domain.ItemWritten {
			t.Fatalf("%s want written, got %s", k, it.Status)
		}
	}
}

func TestGapHoldsDoneUntilClose(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, `{"目标":{"主选":2}}`)
	editor, editorRole := who(t, st, "editor")
	designer, _ := who(t, st, "designer")
	researcher, researcherRole := who(t, st, "researcher")
	if run, _ := st.Q().GetRun(ctx, runID); run.TargetCount != 2 {
		t.Fatalf("target from inputs: %d", run.TargetCount)
	}
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "topic_approve", ItemsJSON: `["hxo-1"]`})
	finishItem(t, e, st, ctx, runID, "hxo-1")
	d, err := e.Submit(ctx, editor, taskByCode(t, st, runID, "e").ID, SubmitInput{DownloadURL: "http://x/e"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Submit(ctx, designer, taskByCode(t, st, runID, "f").ID, SubmitInput{DownloadURL: "http://x/f"}); err != nil {
		t.Fatal(err)
	}
	run, _ := st.Q().GetRun(ctx, runID)
	if run.Status != domain.RunActive {
		t.Fatalf("gap 1 must hold run active, got %s", run.Status)
	}
	p, _ := e.Progress(ctx, runID)
	if p.TargetCount != 2 || p.Gap != 1 || p.Items["written"] != 1 {
		t.Fatalf("progress gap: target=%d gap=%d items=%v", p.TargetCount, p.Gap, p.Items)
	}
	_, err = e.CloseRun(ctx, researcher, researcherRole, runID, "接受缺口")
	wantCode(t, err, "not_hub")
	_, err = e.CloseRun(ctx, editor, editorRole, runID, "")
	wantCode(t, err, "reason_required")
	if run, err = e.CloseRun(ctx, editor, editorRole, runID, "Van：今天就这一条，缺口接受"); err != nil || run.Status != domain.RunDone {
		t.Fatalf("close: %s err=%v", run.Status, err)
	}
}

func TestResearchOkNoSpawn(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "research_ok", ItemsJSON: `["static-9"]`})
	it, _ := st.Q().GetItem(ctx, runID, "static-9")
	if it.Status != domain.ItemApprovedResearch {
		t.Fatalf("research_ok item status: %s", it.Status)
	}
	ts, _ := st.Q().ListTasksByRun(ctx, runID)
	for _, tk := range ts {
		if tk.ItemKey != "" {
			t.Fatalf("research-only item must not spawn tasks: %+v", tk)
		}
	}
}

func TestItemWithdrawDoesNotCancelMerge(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	editor, editorRole := who(t, st, "editor")
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "topic_approve", ItemsJSON: `["hxo-1","coleman-3"]`})
	finishItem(t, e, st, ctx, runID, "hxo-1")
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskBlocked {
		t.Fatalf("e waits for coleman, got %s", got)
	}
	if _, err := e.DecideItem(ctx, editor, editorRole, runID, "coleman-3", "defer", "Van：Coleman 先不要"); err != nil {
		t.Fatalf("defer: %v", err)
	}
	if got := taskByItem(t, st, runID, "c", "coleman-3").Status; got != domain.TaskCancelled {
		t.Fatalf("withdrawn item tasks cancelled, got %s", got)
	}
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskDispatched {
		t.Fatalf("merge must not be cancelled and should proceed, got %s", got)
	}
	_, err := e.DecideItem(ctx, editor, editorRole, runID, "hxo-1", "reject", "Van：撤")
	wantCode(t, err, "item_already_written")
}

func TestLateApprovalMarksMergeRework(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	editor, editorRole := who(t, st, "editor")
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "topic_approve", ItemsJSON: `["hxo-1"]`})
	finishItem(t, e, st, ctx, runID, "hxo-1")
	if got := taskByCode(t, st, runID, "e").Status; got != domain.TaskDispatched {
		t.Fatalf("e dispatched, got %s", got)
	}
	if _, err := e.UpsertItems(ctx, editor, editorRole, runID, []ItemInput{{Key: "nanga-2", Title: "NANGA"}}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.DecideItem(ctx, editor, editorRole, runID, "nanga-2", "approve_write", "Van：NANGA 也写"); err != nil {
		t.Fatal(err)
	}
	if !taskByCode(t, st, runID, "e").ReworkPending {
		t.Fatalf("late approval should flag the started merge task")
	}
}

func TestUpsertItemsGuards(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	collector, collectorRole := who(t, st, "collector")
	researcher, researcherRole := who(t, st, "researcher")
	editor, editorRole := who(t, st, "editor")
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{{Key: "hxo-1", Title: "HxO"}}); err != nil {
		t.Fatalf("collector upsert: %v", err)
	}
	_, err := e.UpsertItems(ctx, researcher, researcherRole, runID, []ItemInput{{Key: "hxo-1", Status: "shortlisted"}})
	wantCode(t, err, "not_participant") // 本 fixture 里没有研究员的任务
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{{Key: "hxo-1", Status: "shortlisted"}}); err != nil {
		t.Fatal(err)
	}
	_, err = e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{{Key: "HxO 1"}})
	wantCode(t, err, "bad_item_key")
	_, err = e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{{Key: "hxo-1", Status: "approved_write"}})
	wantCode(t, err, "bad_item_status")
	if _, err := e.DecideItem(ctx, editor, editorRole, runID, "hxo-1", "approve_write", "Van：写"); err != nil {
		t.Fatal(err)
	}
	// 再次登记不覆盖中枢的决定。
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{{Key: "hxo-1", Title: "HxO 折叠木椅", Status: "candidate"}}); err != nil {
		t.Fatal(err)
	}
	it, _ := st.Q().GetItem(ctx, runID, "hxo-1")
	if it.Status != domain.ItemApprovedWrite || it.Title != "HxO 折叠木椅" {
		t.Fatalf("upsert must not downgrade decision: %+v", it)
	}
	_, err = e.DecideItem(ctx, editor, editorRole, runID, "nope-0", "approve_write", "Van：写")
	wantCode(t, err, "item_not_found")
	_, err = e.DecideItem(ctx, editor, editorRole, runID, "hxo-1", "approve_write", " ")
	wantCode(t, err, "source_quote_required")
}

func TestCancelItemTaskKeepsMerge(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	editor, editorRole := who(t, st, "editor")
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "topic_approve", ItemsJSON: `["hxo-1","nanga-2"]`})
	finishItem(t, e, st, ctx, runID, "hxo-1")
	cr, err := e.Cancel(ctx, editor, editorRole, taskByItem(t, st, runID, "d", "nanga-2").ID, "图片无授权，本条不配了")
	if err != nil || len(cr.Cancelled) != 1 {
		t.Fatalf("cancel item task must not cascade: %+v err=%v", cr, err)
	}
	if got := taskByCode(t, st, runID, "e").Status; got == domain.TaskCancelled {
		t.Fatalf("merge task must not be cancelled")
	}
}

// TestAutoDispatchItemNote 逐条任务自动派工时，派工意见由引擎写明条目、来源与 Van 决定原话；
// 手动派工留空意见时同样补上，写了意见则原样保留。
func TestAutoDispatchItemNote(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	collector, collectorRole := who(t, st, "collector")
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "hxo-1", Brand: "HxO", Title: "折叠木椅", SourceURL: "https://example.com/hxo"},
		{Key: "nanga-2", Brand: "NANGA", Title: "羽绒睡袋"},
	}); err != nil {
		t.Fatalf("upsert items: %v", err)
	}
	passB(t, e, st, ctx, runID, ReviewInput{DecisionType: "topic_approve", ItemsJSON: `["hxo-1","nanga-2"]`, SourceQuote: "Van：HxO 和 NANGA 可以写"})

	note := func(taskID int64) string {
		t.Helper()
		var n string
		if err := st.DB().QueryRow(`SELECT COALESCE(editor_note,'') FROM deliverables WHERE task_id=? AND kind='dispatch' ORDER BY id DESC LIMIT 1`, taskID).Scan(&n); err != nil {
			t.Fatalf("dispatch note: %v", err)
		}
		return n
	}
	d := taskByItem(t, st, runID, "d", "hxo-1")
	if d.Status != domain.TaskDispatched {
		t.Fatalf("auto per-item task should be dispatched, got %s", d.Status)
	}
	want := "条目：HxO｜折叠木椅（hxo-1）；来源：https://example.com/hxo；Van 决定：Van：HxO 和 NANGA 可以写"
	if got := note(d.ID); got != want {
		t.Fatalf("auto note:\n got  %s\n want %s", got, want)
	}
	var noticeNote string
	for _, r := range outboxRows(t, st) {
		if r.typ == domain.EvtDispatched && fmt.Sprint(r.payload["task_id"]) == fmt.Sprint(d.ID) {
			noticeNote = fmt.Sprint(r.payload["note"])
		}
	}
	if noticeNote != want {
		t.Fatalf("dispatched notice should carry the item note: %q", noticeNote)
	}

	editor, editorRole := who(t, st, "editor")
	cH, cN := taskByItem(t, st, runID, "c", "hxo-1"), taskByItem(t, st, runID, "c", "nanga-2")
	if _, err := e.Dispatch(ctx, editor, editorRole, cH.ID, "", nil); err != nil {
		t.Fatalf("dispatch empty note: %v", err)
	}
	if got := note(cH.ID); !strings.HasPrefix(got, "条目：HxO｜折叠木椅（hxo-1）") {
		t.Fatalf("manual dispatch with empty note should get item note: %s", got)
	}
	if _, err := e.Dispatch(ctx, editor, editorRole, cN.ID, "只写睡袋的温标变化", nil); err != nil {
		t.Fatalf("dispatch with note: %v", err)
	}
	if got := note(cN.ID); got != "只写睡袋的温标变化" {
		t.Fatalf("explicit note must be kept: %s", got)
	}
}
