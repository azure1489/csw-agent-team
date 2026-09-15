package engine

import (
	"context"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func TestSupplementMarksRunningDownstream(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	// a（素材，0 闸）→ b（组版，已开工）；a → c（手动，未开工）。
	buildFlow(t, st, "auto", []fxStage{
		{code: "a", role: "designer"},
		{code: "b", role: "publisher", deps: []string{"a"}, gates: []fxGate{{reviewer: "editor"}}},
		{code: "c", role: "writer", mode: "manual", deps: []string{"a"}},
	})
	editor, editorRole := who(t, st, "editor")
	designer, _ := who(t, st, "designer")
	publisher, _ := who(t, st, "publisher")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	a := taskByCode(t, st, runID, "a")
	out, err := e.Submit(ctx, designer, a.ID, SubmitInput{DownloadURL: "http://x/a-v1"})
	if err != nil {
		t.Fatalf("submit a: %v", err)
	}
	b := taskByCode(t, st, runID, "b")
	if _, err := e.Ack(ctx, publisher, b.ID); err != nil {
		t.Fatalf("ack b: %v", err)
	}

	_, err = e.Submit(ctx, designer, a.ID, SubmitInput{Kind: domain.KindSupplement, DownloadURL: "http://x/a-s1"})
	wantCode(t, err, "affects_required")
	_, err = e.Submit(ctx, publisher, b.ID, SubmitInput{Kind: domain.KindSupplement, AffectsID: &out.ID, DownloadURL: "http://x/b-s1"})
	wantCode(t, err, "supplement_requires_passed")

	sup, err := e.Submit(ctx, designer, a.ID, SubmitInput{Kind: domain.KindSupplement, AffectsID: &out.ID, DownloadURL: "http://x/a-s1", Summary: "换头图"})
	if err != nil {
		t.Fatalf("supplement: %v", err)
	}
	if sup.Kind != domain.KindSupplement || sup.Status != domain.DelPassed || sup.Version != 1 {
		t.Fatalf("supplement shape: %+v", sup)
	}
	if got := taskByCode(t, st, runID, "a"); got.Status != domain.TaskPassed || got.CurVersion != 1 {
		t.Fatalf("supplement must not touch the approved task: %s v%d", got.Status, got.CurVersion)
	}
	if !taskByCode(t, st, runID, "b").ReworkPending {
		t.Fatalf("started downstream b should be rework_pending")
	}
	if taskByCode(t, st, runID, "c").ReworkPending {
		t.Fatalf("not-started downstream c must not be flagged")
	}
	if !hasEvent(t, st, runID, a.ID, domain.EvtSupplementArrived, `"rework_task_ids":[`) {
		t.Fatalf("missing supplement_arrived event")
	}
	_, err = e.Review(ctx, editor, editorRole, sup.ID, ReviewInput{Verdict: domain.VerdictPass})
	wantCode(t, err, "not_reviewable")

	// 未开工的 c 派工时预填上游带上补件。
	dr, err := e.Dispatch(ctx, editor, editorRole, taskByCode(t, st, runID, "c").ID, "写", nil)
	if err != nil {
		t.Fatalf("dispatch c: %v", err)
	}
	if len(dr.Upstreams) != 2 || dr.Upstreams[1].Label != "a 补件#1" || dr.Upstreams[1].UpstreamURL != "http://x/a-s1" {
		t.Fatalf("prefill should include supplement: %+v", dr.Upstreams)
	}
	// b 交新版本即清待返工。
	if _, err := e.Submit(ctx, publisher, b.ID, SubmitInput{DownloadURL: "http://x/b-v1"}); err != nil {
		t.Fatalf("submit b: %v", err)
	}
	if taskByCode(t, st, runID, "b").ReworkPending {
		t.Fatalf("rework_pending should clear after new version")
	}
}

// 定点编辑夹具：f（writer 产出）闸 = 主编 → Van。
func editFlow(t *testing.T) (*Engine, context.Context, domain.Agent, domain.Role, domain.Agent, int64) {
	t.Helper()
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{
		{code: "f", role: "writer", gates: []fxGate{{reviewer: "editor"}, {reviewer: "van", relayed: true}}},
		{code: "g", role: "publisher", deps: []string{"f"}},
	})
	editor, editorRole := who(t, st, "editor")
	writer, _ := who(t, st, "writer")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	return e, ctx, editor, editorRole, writer, res.Run.ID
}

func TestEditSkipsHubGate(t *testing.T) {
	e, ctx, editor, editorRole, writer, runID := editFlow(t)
	st := e.store
	f := taskByCode(t, st, runID, "f")
	v1, err := e.Submit(ctx, writer, f.ID, SubmitInput{DownloadURL: "http://x/f-v1"})
	if err != nil {
		t.Fatalf("submit v1: %v", err)
	}
	one, two := 1, 2
	_, err = e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &one, DownloadURL: "http://x/f-e"})
	wantCode(t, err, "diff_summary_required")
	_, err = e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, DiffSummary: "改时态", DownloadURL: "http://x/f-e"})
	wantCode(t, err, "edit_of_required")
	_, err = e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &two, DiffSummary: "改时态", DownloadURL: "http://x/f-e"})
	wantCode(t, err, "edit_of_mismatch")
	_, err = e.Submit(ctx, writer, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &one, DiffSummary: "改时态", DownloadURL: "http://x/f-e"})
	wantCode(t, err, "not_hub")

	ed, err := e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &one, DiffSummary: "第 2 则时态改为已发布", DownloadURL: "http://x/f-e"})
	if err != nil {
		t.Fatalf("edit: %v", err)
	}
	if ed.Kind != domain.KindEdit || !ed.Collab || ed.Version != 2 || ed.CurGate != 1 || ed.Status != domain.DelInReview {
		t.Fatalf("edit shape: %+v", ed)
	}
	if old, _ := st.Q().GetDeliverable(ctx, v1.ID); old.Status != domain.DelSuperseded {
		t.Fatalf("first draft want superseded, got %s", old.Status)
	}
	if got := taskByCode(t, st, runID, "f"); got.Status != domain.TaskReview || got.CurVersion != 2 {
		t.Fatalf("task after edit: %s v%d", got.Status, got.CurVersion)
	}
	rs, _ := st.Q().ListReviewsByDeliverable(ctx, ed.ID)
	if len(rs) != 1 || rs[0].DecisionType != "edit_pass" || rs[0].Comment != "第 2 则时态改为已发布" {
		t.Fatalf("edit_pass trail: %+v", rs)
	}
	ups, _ := st.Q().ListUpstreams(ctx, ed.ID)
	if len(ups) == 0 || ups[len(ups)-1].Label != "首稿 v1" {
		t.Fatalf("edit should link back to first draft: %+v", ups)
	}
	// 首稿已被取代，不能再审；下一道是 Van 闸，审的是编辑版本。
	_, err = e.Review(ctx, editor, editorRole, v1.ID, ReviewInput{Verdict: domain.VerdictPass})
	wantCode(t, err, "not_in_review")
	r, err := e.Review(ctx, editor, editorRole, ed.ID, ReviewInput{Verdict: domain.VerdictPass, SourceQuote: "Van：可以", DecisionType: "fulltext_approve"})
	if err != nil || !r.Final {
		t.Fatalf("van pass on edit: %+v err=%v", r, err)
	}
	if got := taskByCode(t, st, runID, "g").Status; got != domain.TaskDispatched {
		t.Fatalf("downstream g want dispatched, got %s", got)
	}
}

func TestEditGuards(t *testing.T) {
	e, ctx, editor, editorRole, writer, runID := editFlow(t)
	st := e.store
	f := taskByCode(t, st, runID, "f")
	one := 1
	// 任务未在审核中。
	_, err := e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &one, DiffSummary: "x", DownloadURL: "http://x/e"})
	wantCode(t, err, "cannot_edit")
	v1, err := e.Submit(ctx, writer, f.ID, SubmitInput{DownloadURL: "http://x/f-v1"})
	if err != nil {
		t.Fatalf("submit: %v", err)
	}
	// 主编闸已过、轮到 Van：不能再定点编辑。
	if _, err := e.Review(ctx, editor, editorRole, v1.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatalf("hub gate: %v", err)
	}
	_, err = e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &one, DiffSummary: "x", DownloadURL: "http://x/e"})
	wantCode(t, err, "edit_gate_not_yours")
}

func TestEditAtLastHubGatePasses(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{
		{code: "f", role: "writer", gates: []fxGate{{reviewer: "editor"}}},
		{code: "g", role: "publisher", deps: []string{"f"}},
	})
	editor, editorRole := who(t, st, "editor")
	writer, _ := who(t, st, "writer")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	f := taskByCode(t, st, res.Run.ID, "f")
	if _, err := e.Submit(ctx, writer, f.ID, SubmitInput{DownloadURL: "http://x/f-v1"}); err != nil {
		t.Fatalf("submit: %v", err)
	}
	one := 1
	ed, err := e.Submit(ctx, editor, f.ID, SubmitInput{Kind: domain.KindEdit, EditOf: &one, DiffSummary: "删重复句", DownloadURL: "http://x/f-e"})
	if err != nil || ed.Status != domain.DelPassed {
		t.Fatalf("edit at last gate: %+v err=%v", ed, err)
	}
	if got := taskByCode(t, st, res.Run.ID, "f"); got.Status != domain.TaskPassed || got.CurVersion != 2 {
		t.Fatalf("f want passed v2, got %s v%d", got.Status, got.CurVersion)
	}
	// 下游预填的是编辑版本。
	g := taskByCode(t, st, res.Run.ID, "g")
	ds, _ := st.Q().ListDeliverablesByTask(ctx, g.ID)
	ups, _ := st.Q().ListUpstreams(ctx, ds[0].ID)
	if len(ups) != 1 || ups[0].UpstreamURL != "http://x/f-e" {
		t.Fatalf("downstream should prefill edited version: %+v", ups)
	}
}

func TestReviewExpectedVersion409(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{{code: "a", role: "collector", gates: []fxGate{{reviewer: "editor"}}}})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, _ := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	d, err := e.Submit(ctx, collector, taskByCode(t, st, res.Run.ID, "a").ID, SubmitInput{DownloadURL: "http://x/a"})
	if err != nil {
		t.Fatal(err)
	}
	two, one := 2, 1
	_, err = e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, ExpectedVersion: &two})
	wantCode(t, err, "expected_version_mismatch")
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, ExpectedVersion: &one}); err != nil {
		t.Fatalf("matching expected_version: %v", err)
	}
}

func TestVanGateRequiresQuote(t *testing.T) {
	e, ctx, editor, editorRole, writer, runID := editFlow(t)
	st := e.store
	d, err := e.Submit(ctx, writer, taskByCode(t, st, runID, "f").ID, SubmitInput{DownloadURL: "http://x/f"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatalf("hub gate needs no quote: %v", err)
	}
	_, err = e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, Comment: "可以"})
	wantCode(t, err, "source_quote_required")
	// 现行代录写法：comment 以「Van：」开头视为原话。
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, Comment: "Van：这期可以"}); err != nil {
		t.Fatalf("comment-prefixed quote: %v", err)
	}
	rs, _ := st.Q().ListReviewsByDeliverable(ctx, d.ID)
	if rs[len(rs)-1].SourceQuote != "Van：这期可以" {
		t.Fatalf("quote not persisted: %+v", rs[len(rs)-1])
	}
}

func TestReviewDecisionPersisted(t *testing.T) {
	e, ctx, editor, editorRole, writer, runID := editFlow(t)
	st := e.store
	d, err := e.Submit(ctx, writer, taskByCode(t, st, runID, "f").ID, SubmitInput{DownloadURL: "http://x/f"})
	if err != nil {
		t.Fatal(err)
	}
	_, err = e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, DecisionType: "edit_pass"})
	wantCode(t, err, "bad_decision_type")
	_, err = e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, ItemsJSON: `{"hxo":1}`})
	wantCode(t, err, "bad_items_json")
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{
		Verdict: domain.VerdictPass, DecisionType: "topic_approve", SourceQuote: "Van：HxO 保留，NANGA 再看看",
		ItemsJSON: `["hxo-1a2b"]`,
	}); err != nil {
		t.Fatalf("van pass: %v", err)
	}
	rs, _ := st.Q().ListReviewsByDeliverable(ctx, d.ID)
	last := rs[len(rs)-1]
	if last.DecisionType != "topic_approve" || last.ItemsJSON != `["hxo-1a2b"]` || last.SourceQuote == "" {
		t.Fatalf("decision not persisted: %+v", last)
	}
	if !hasEvent(t, st, runID, d.TaskID, domain.EvtGatePassed, `"decision_type":"topic_approve"`) {
		t.Fatalf("gate_passed detail should carry decision")
	}
}
