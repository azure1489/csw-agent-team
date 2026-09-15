package engine

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

type obRow struct {
	typ, target, cc string
	payload         map[string]any
}

func outboxRows(t *testing.T, st *sqlite.Store) []obRow {
	t.Helper()
	rows, err := st.DB().Query(`SELECT event_type, COALESCE(target_role,''), COALESCE(cc_roles,''), payload_json FROM outbox ORDER BY id`)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var out []obRow
	for rows.Next() {
		var r obRow
		var p string
		if err := rows.Scan(&r.typ, &r.target, &r.cc, &p); err != nil {
			t.Fatal(err)
		}
		_ = json.Unmarshal([]byte(p), &r.payload)
		out = append(out, r)
	}
	return out
}

func lastOf(t *testing.T, st *sqlite.Store, typ string) obRow {
	t.Helper()
	rows := outboxRows(t, st)
	for i := len(rows) - 1; i >= 0; i-- {
		if rows[i].typ == typ {
			return rows[i]
		}
	}
	t.Fatalf("no outbox row of %s", typ)
	return obRow{}
}

// TestOutboxTargets 引擎按闸与角色推导通知去向：派工→执行者；待审→下一闸审核人；
// Van 闸→Van 抄送中枢；退回→执行者抄送中枢；阶段通过 / 手动就绪 / 待授权 / 失败→中枢。
func TestOutboxTargets(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{
		{code: "a", role: "writer", gates: []fxGate{{reviewer: "editor"}, {reviewer: "van", relayed: true}}},
		{code: "b", role: "designer", mode: "manual", deps: []string{"a"}},
		{code: "c", role: "publisher", action: "platform_write:wx_draft", deps: []string{"a"}},
	})
	editor, editorRole := who(t, st, "editor")
	writer, _ := who(t, st, "writer")
	designer, _ := who(t, st, "designer")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatal(err)
	}
	runID := res.Run.ID

	if r := lastOf(t, st, domain.EvtDispatched); r.target != "writer" || r.payload["hub_role"] != "editor" {
		t.Fatalf("dispatched: %+v", r)
	}
	a := taskByCode(t, st, runID, "a")
	d, err := e.Submit(ctx, writer, a.ID, SubmitInput{DownloadURL: "http://x/a"})
	if err != nil {
		t.Fatal(err)
	}
	if r := lastOf(t, st, domain.EvtSubmitted); r.target != "editor" || r.cc != "" || r.payload["next_gate_name"] != "editor审" {
		t.Fatalf("submitted: %+v", r)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	if r := lastOf(t, st, domain.EvtGatePassed); r.target != "van" || r.cc != "editor" || r.payload["next_reviewer"] != "van" {
		t.Fatalf("gate_passed → van: %+v", r)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictReject, ReturnDirection: "补图", ReturnLocation: "第 2 段", SourceQuote: "Van：图不对"}); err != nil {
		t.Fatal(err)
	}
	if r := lastOf(t, st, domain.EvtGateReturned); r.target != "writer" || r.cc != "editor" || r.payload["return_location"] != "第 2 段" {
		t.Fatalf("gate_returned: %+v", r)
	}
	d2, err := e.Submit(ctx, writer, a.ID, SubmitInput{DownloadURL: "http://x/a2"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d2.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d2.ID, ReviewInput{Verdict: domain.VerdictPass, SourceQuote: "Van：可以"}); err != nil {
		t.Fatal(err)
	}
	if r := lastOf(t, st, domain.EvtStagePassed); r.target != "editor" || len(r.payload["next_stages"].([]any)) != 2 {
		t.Fatalf("stage_passed: %+v", r)
	}
	if r := lastOf(t, st, domain.EvtTaskReady); r.target != "editor" || r.payload["stage_code"] != "b" {
		t.Fatalf("manual task_ready: %+v", r)
	}
	if r := lastOf(t, st, domain.EvtAuthorizationRequired); r.target != "editor" || r.payload["scope"] != "wx_draft" {
		t.Fatalf("authorization_required: %+v", r)
	}
	if _, err := e.Dispatch(ctx, editor, editorRole, taskByCode(t, st, runID, "b").ID, "做图", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Fail(ctx, designer, taskByCode(t, st, runID, "b").ID, "素材缺失"); err != nil {
		t.Fatal(err)
	}
	if r := lastOf(t, st, domain.EvtTaskFailed); r.target != "editor" || r.payload["reason"] != "素材缺失" {
		t.Fatalf("task_failed: %+v", r)
	}
	// Van 从不作为派工 / 退回 / 失败的收件人。
	for _, r := range outboxRows(t, st) {
		if r.target == "van" && r.typ != domain.EvtGatePassed && r.typ != domain.EvtSubmitted {
			t.Fatalf("van targeted by %s", r.typ)
		}
	}
}
