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

// TestItemsOnlyAtVanGate 条目决定只随 Van 闸或末闸录入：主编自审闸带条目直接拒绝，不会在 Van 批准前生成写作任务。
func TestItemsOnlyAtVanGate(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	editor, editorRole := who(t, st, "editor")
	d, err := e.Submit(ctx, editor, taskByCode(t, st, runID, "b").ID, SubmitInput{DownloadURL: "http://x/b"})
	if err != nil {
		t.Fatal(err)
	}
	_, err = e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, DecisionType: "topic_approve", ItemsJSON: `["hxo-1"]`})
	wantCode(t, err, "items_require_van_gate")
	if items, _ := st.Q().ListItems(ctx, runID); len(items) != 0 {
		t.Fatalf("no item may be decided before the Van gate: %+v", items)
	}
	// 同一闸不带条目照常通过，Van 闸带条目生效。
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatalf("hub gate without items: %v", err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, DecisionType: "topic_approve",
		ItemsJSON: `["hxo-1"]`, SourceQuote: "Van：HxO 可以写"}); err != nil {
		t.Fatalf("van gate with items: %v", err)
	}
	if it, _ := st.Q().GetItem(ctx, runID, "hxo-1"); it.Status != domain.ItemApprovedWrite {
		t.Fatalf("item after van gate: %+v", it)
	}
}

// TestItemStageAndRank 条目五栏：登记可写线索 / 待核 / 成熟，成熟可分主选与备选；非法状态与分级被拒。
func TestItemStageAndRank(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v6", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	collector, collectorRole := who(t, st, "collector")
	items, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "hxo-a1b2c3", Brand: "HxO", Title: "折叠木椅", Status: "pending_check"},
		{Key: "nanga-d4e5f6", Brand: "NANGA", Title: "羽绒睡袋", Status: "shortlisted", Rank: "primary"},
		{Key: "keen-778899", Brand: "KEEN", Title: "凉鞋", Status: "shortlisted", Rank: "alt"},
	})
	if err != nil {
		t.Fatalf("upsert: %v", err)
	}
	got := map[string]string{}
	for _, it := range items {
		got[it.ItemKey] = string(it.Status) + "/" + it.Rank
	}
	if got["hxo-a1b2c3"] != "pending_check/" || got["nanga-d4e5f6"] != "shortlisted/primary" || got["keen-778899"] != "shortlisted/alt" {
		t.Fatalf("items = %v", got)
	}
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID,
		[]ItemInput{{Key: "bad-000001", Title: "x", Status: "approved_write"}}); err == nil {
		t.Fatal("批准状态不该由登记方写入")
	}
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID,
		[]ItemInput{{Key: "bad-000002", Title: "x", Status: "pending_check", Rank: "primary"}}); err == nil {
		t.Fatal("只有成熟条目才分主选与备选")
	}
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID,
		[]ItemInput{{Key: "bad-000003", Title: "x", Status: "shortlisted", Rank: "first"}}); err == nil {
		t.Fatal("非法 rank 应被拒")
	}
}

// TestItemDropAndTrace 淘汰必须给理由并落轨迹；反复上报同样内容不刷屏。
func TestItemDropAndTrace(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v6", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	researcher, researcherRole := who(t, st, "researcher")

	// 淘汰不给理由：拒。
	if _, err := e.UpsertItems(ctx, researcher, researcherRole, runID,
		[]ItemInput{{Key: "aaa-000001", Title: "x", Status: "dropped"}}); err == nil {
		t.Fatal("淘汰缺理由应被拒")
	} else {
		wantCode(t, err, "drop_reason_required")
	}
	// 非法理由码：拒。
	if _, err := e.UpsertItems(ctx, researcher, researcherRole, runID,
		[]ItemInput{{Key: "aaa-000001", Title: "x", Status: "dropped", ReasonCode: "随便写", Reason: "不行"}}); err == nil {
		t.Fatal("非法 reason_code 应被拒")
	} else {
		wantCode(t, err, "bad_reason_code")
	}

	// 合法淘汰：状态落 dropped，并留一条轨迹。
	items, err := e.UpsertItems(ctx, researcher, researcherRole, runID, []ItemInput{
		{Key: "aaa-000001", Title: "只换配色", Status: "dropped", ReasonCode: "no_value", Reason: "只是配色更新，读者拿不到新东西"},
	})
	if err != nil {
		t.Fatalf("upsert: %v", err)
	}
	if len(items) != 1 || items[0].Status != domain.ItemDropped {
		t.Fatalf("want dropped, got %+v", items)
	}
	traces, err := st.Q().ListTracesByRun(ctx, runID)
	if err != nil {
		t.Fatalf("traces: %v", err)
	}
	if len(traces) != 1 || traces[0].ReasonCode != "no_value" || traces[0].ToStatus != "dropped" {
		t.Fatalf("want 1 drop trace, got %+v", traces)
	}

	// 同样内容重报：不再新增轨迹（agent 会反复 PUT 全量条目）。
	if _, err := e.UpsertItems(ctx, researcher, researcherRole, runID, []ItemInput{
		{Key: "aaa-000001", Title: "只换配色", Status: "dropped", ReasonCode: "no_value", Reason: "只是配色更新，读者拿不到新东西"},
	}); err != nil {
		t.Fatalf("re-upsert: %v", err)
	}
	traces, _ = st.Q().ListTracesByRun(ctx, runID)
	if len(traces) != 1 {
		t.Fatalf("重报同样内容不该新增轨迹，got %d", len(traces))
	}

	// 理由变了：补一条。
	if _, err := e.UpsertItems(ctx, researcher, researcherRole, runID, []ItemInput{
		{Key: "aaa-000001", Title: "只换配色", Status: "dropped", ReasonCode: "dup_published", Reason: "9 月初已发过同角度"},
	}); err != nil {
		t.Fatalf("re-upsert2: %v", err)
	}
	traces, _ = st.Q().ListTracesByRun(ctx, runID)
	if len(traces) != 2 {
		t.Fatalf("理由变化应补一条轨迹，got %d", len(traces))
	}

	// 淘汰后仍可被捡回（主编校准后改判），且再落一条轨迹。
	if _, err := e.UpsertItems(ctx, researcher, researcherRole, runID, []ItemInput{
		{Key: "aaa-000001", Title: "只换配色", Status: "shortlisted", Rank: "alt", ReasonCode: "other", Reason: "主编校准后改判"},
	}); err != nil {
		t.Fatalf("revive: %v", err)
	}
	it, _ := st.Q().GetItem(ctx, runID, "aaa-000001")
	if it.Status != domain.ItemShortlisted {
		t.Fatalf("淘汰的条目应能被改回成熟，got %s", it.Status)
	}
	traces, _ = st.Q().ListTracesByRun(ctx, runID)
	if len(traces) != 3 {
		t.Fatalf("改判应再落一条轨迹，got %d", len(traces))
	}
}

// r63：下游（或中枢）定过的状态，上游重登记不推翻；上游自己的、下游改上游的照常。
func TestUpstreamReRegisterKeepsDownstreamStatus(t *testing.T) {
	e, st, ctx, runID := itemFlow(t, "")
	collector, colRole := who(t, st, "collector")
	designer, desRole := who(t, st, "designer")
	editor, edRole := who(t, st, "editor")
	put := func(a domain.Agent, r domain.Role, key, status string) {
		t.Helper()
		in := ItemInput{Key: key, Title: key, Status: status}
		if status == string(domain.ItemDropped) {
			in.ReasonCode, in.Reason = "no_value", "价值不足"
		}
		if _, err := e.UpsertItems(ctx, a, r, runID, []ItemInput{in}); err != nil {
			t.Fatalf("%s %s→%s: %v", r.Code, key, status, err)
		}
	}
	status := func(key string) domain.ItemStatus {
		t.Helper()
		it, err := st.Q().GetItem(ctx, runID, key)
		if err != nil {
			t.Fatal(err)
		}
		return it.Status
	}
	for _, k := range []string{"k1", "k2", "k3"} {
		put(collector, colRole, k, "shortlisted")
	}
	put(designer, desRole, "k1", "dropped")   // 下游淘汰
	put(editor, edRole, "k2", "pending_check") // 中枢改待核
	// 上游重登记同一批判断
	for _, k := range []string{"k1", "k2"} {
		put(collector, colRole, k, "shortlisted")
	}
	if s := status("k1"); s != domain.ItemDropped {
		t.Fatalf("k1 下游已淘汰，上游重登记不该改回：%s", s)
	}
	if s := status("k2"); s != domain.ItemPendingCheck {
		t.Fatalf("k2 中枢已改待核，上游重登记不该改回：%s", s)
	}
	put(collector, colRole, "k3", "dropped") // 自己的条目照常改
	if s := status("k3"); s != domain.ItemDropped {
		t.Fatalf("k3 上游改自己登记的状态应生效：%s", s)
	}
	put(designer, desRole, "k3", "shortlisted") // 下游改上游的照常
	if s := status("k3"); s != domain.ItemShortlisted {
		t.Fatalf("k3 下游改上游应生效：%s", s)
	}
}
