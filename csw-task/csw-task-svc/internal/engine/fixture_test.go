package engine

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// fxGate 夹具闸。
type fxGate struct {
	reviewer string
	relayed  bool
}

// fxStage 夹具阶段：只填需要的字段，其余取默认（三段文本固定非空）。
type fxStage struct {
	code, role, mode, action string
	deps                     []string
	gates                    []fxGate
	sla                      int
	merge, perItem           bool
}

// buildFlow 建一个 active 的 t_flow 小流程（中枢=editor），不依赖 seed 的 daily_news。
func buildFlow(t *testing.T, st *sqlite.Store, wfMode string, stages []fxStage) {
	t.Helper()
	ctx := context.Background()
	err := st.Tx(ctx, func(q *sqlite.Queries) error {
		id, err := q.CreateWorkflowDraft(ctx, "t_flow", "测试流程", "editor", wfMode, "editor")
		if err != nil {
			return err
		}
		ids := map[string]int64{}
		for i, s := range stages {
			sid, err := q.InsertStage(ctx, domain.Stage{
				WorkflowID: id, Seq: i + 1, Code: s.code, Name: s.code, RoleCode: s.role, OutputType: "产出",
				Instructions: "做", SelfCheckCriteria: "查", Acceptance: "验", IsMerge: s.merge,
				DispatchMode: s.mode, ActionClass: s.action, SLAMinutes: s.sla, PerItem: s.perItem,
			})
			if err != nil {
				return err
			}
			ids[s.code] = sid
		}
		for _, s := range stages {
			for _, d := range s.deps {
				if err := q.InsertStageDep(ctx, id, ids[s.code], ids[d]); err != nil {
					return err
				}
			}
			for i, g := range s.gates {
				sid := ids[s.code]
				if err := q.InsertGate(ctx, domain.Gate{WorkflowID: id, StageID: &sid, GateOrder: i + 1,
					ReviewerRole: g.reviewer, RelayedByHub: g.relayed, Name: g.reviewer + "审"}); err != nil {
					return err
				}
			}
		}
		return q.SetWorkflowStatus(ctx, id, domain.WfActive)
	})
	if err != nil {
		t.Fatalf("build t_flow: %v", err)
	}
}

func wantCode(t *testing.T, err error, code string) {
	t.Helper()
	var de *domain.Error
	if !errors.As(err, &de) || de.Code != code {
		t.Fatalf("want error %s, got %v", code, err)
	}
}

func hasEvent(t *testing.T, st *sqlite.Store, runID, taskID int64, typ, detailSub string) bool {
	t.Helper()
	evs, err := st.Q().ListEventsByRun(context.Background(), runID)
	if err != nil {
		t.Fatalf("events: %v", err)
	}
	for _, ev := range evs {
		if ev.Type == typ && ev.TaskID != nil && *ev.TaskID == taskID && strings.Contains(ev.DetailJSON, detailSub) {
			return true
		}
	}
	return false
}

// 两阶段：a（collector 普通，0 闸）→ b（publisher 平台写 wx_draft，0 闸）。
func writeFlow() []fxStage {
	return []fxStage{
		{code: "a", role: "collector"},
		{code: "b", role: "publisher", action: "platform_write:wx_draft", deps: []string{"a"}},
	}
}

func TestTaskSnapshotColumns(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "manual", []fxStage{
		{code: "a", role: "collector"},
		{code: "b", role: "writer", mode: "auto", sla: 30, deps: []string{"a"}},
		{code: "c", role: "publisher", action: "platform_write:xhs_draft", deps: []string{"b"}},
	})
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	a, b, c := taskByCode(t, st, res.Run.ID, "a"), taskByCode(t, st, res.Run.ID, "b"), taskByCode(t, st, res.Run.ID, "c")
	if a.DispatchMode != domain.DispatchManual || b.DispatchMode != domain.DispatchAuto {
		t.Fatalf("dispatch_mode snapshot: a=%s b=%s", a.DispatchMode, b.DispatchMode)
	}
	if b.SLAMinutes != 30 || a.SLAMinutes != 0 {
		t.Fatalf("sla snapshot: a=%d b=%d", a.SLAMinutes, b.SLAMinutes)
	}
	if a.ActionClass != domain.ActionRead || c.ActionClass != "platform_write:xhs_draft" {
		t.Fatalf("action_class snapshot: a=%s c=%s", a.ActionClass, c.ActionClass)
	}
	// manual 入口不自动派。
	if a.Status != domain.TaskReady {
		t.Fatalf("a want ready (manual), got %s", a.Status)
	}
	// 改定义不影响已触发实例的快照。
	if _, err := st.DB().Exec(`UPDATE workflow_stages SET action_class='read', dispatch_mode='manual' WHERE code IN ('b','c')`); err != nil {
		t.Fatal(err)
	}
	if got := taskByCode(t, st, res.Run.ID, "c").ActionClass; got != "platform_write:xhs_draft" {
		t.Fatalf("snapshot changed after definition edit: %s", got)
	}
}

func TestPlatformWriteBlockedWithoutAuth(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", writeFlow())
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")

	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	a := taskByCode(t, st, runID, "a")
	if a.Status != domain.TaskDispatched {
		t.Fatalf("a want auto-dispatched, got %s", a.Status)
	}
	if _, err := e.Submit(ctx, collector, a.ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatalf("submit a: %v", err)
	}

	// b 就绪但缺授权：停在 ready，事件带 scope。
	b := taskByCode(t, st, runID, "b")
	if b.Status != domain.TaskReady {
		t.Fatalf("b want ready without auth, got %s", b.Status)
	}
	if !hasEvent(t, st, runID, b.ID, domain.EvtAuthorizationRequired, `"scope":"wx_draft"`) {
		t.Fatalf("missing authorization_required event for b")
	}
	// 手动派工同样被拦。
	_, err = e.Dispatch(ctx, editor, editorRole, b.ID, "存草稿", nil)
	wantCode(t, err, "authorization_required")

	// 本地演练授权不覆盖平台写。
	g, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeLocalDrill, "Van：今天只做本地演练", nil)
	if err != nil {
		t.Fatalf("grant local_drill: %v", err)
	}
	if len(g.Dispatched) != 0 || taskByCode(t, st, runID, "b").Status != domain.TaskReady {
		t.Fatalf("local_drill must not unlock platform_write")
	}
	// 发布授权涵盖同平台草稿：录入后自动续派。
	g, err = e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxPublish, "Van：可以发", nil)
	if err != nil {
		t.Fatalf("grant wx_publish: %v", err)
	}
	if len(g.Dispatched) != 1 || g.Dispatched[0] != b.ID {
		t.Fatalf("grant should resume b, got %v", g.Dispatched)
	}
	if got := taskByCode(t, st, runID, "b").Status; got != domain.TaskDispatched {
		t.Fatalf("b want dispatched after grant, got %s", got)
	}
}

func TestSubmitRequiresAuth(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", writeFlow())
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	publisher, _ := who(t, st, "publisher")

	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if _, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：存草稿", nil); err != nil {
		t.Fatalf("grant: %v", err)
	}
	if _, err := e.Submit(ctx, collector, taskByCode(t, st, runID, "a").ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatalf("submit a: %v", err)
	}
	b := taskByCode(t, st, runID, "b")
	if b.Status != domain.TaskDispatched {
		t.Fatalf("b with auth should auto-dispatch, got %s", b.Status)
	}

	// 撤销后提交被拒；已派工不收回。
	if n, err := e.Revoke(ctx, editor, editorRole, runID, domain.ScopeWxDraft); err != nil || n != 1 {
		t.Fatalf("revoke: n=%d err=%v", n, err)
	}
	_, err = e.Submit(ctx, publisher, b.ID, SubmitInput{DownloadURL: "http://x/b"})
	wantCode(t, err, "authorization_required")
	_, err = e.Revoke(ctx, editor, editorRole, runID, domain.ScopeWxDraft)
	wantCode(t, err, "authorization_not_found")

	// 已过期的授权不算有效。
	past := "2000-01-01T00:00:00Z"
	if _, err := st.Q().InsertAuthorization(ctx, domain.RunAuthorization{RunID: runID, Scope: domain.ScopeWxDraft, SourceQuote: "旧授权", ExpiresAt: past}); err != nil {
		t.Fatal(err)
	}
	if ok, err := st.Q().HasActiveAuthorization(ctx, runID, []domain.AuthScope{domain.ScopeWxDraft}); err != nil || ok {
		t.Fatalf("expired authorization must not count: ok=%v err=%v", ok, err)
	}

	// 重新授权后可提交。
	if _, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：重新允许存草稿", nil); err != nil {
		t.Fatalf("regrant: %v", err)
	}
	if _, err := e.Submit(ctx, publisher, b.ID, SubmitInput{DownloadURL: "http://x/b"}); err != nil {
		t.Fatalf("submit b after regrant: %v", err)
	}
	if run, _ := st.Q().GetRun(ctx, runID); run.Status != domain.RunDone {
		t.Fatalf("run want done, got %s", run.Status)
	}
}

func TestGrantRequiresHubAndQuote(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", writeFlow())
	editor, editorRole := who(t, st, "editor")
	collector, collectorRole := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID

	_, err = e.Grant(ctx, collector, collectorRole, runID, domain.ScopeWxDraft, "Van：可以", nil)
	wantCode(t, err, "not_hub")
	_, err = e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "   ", nil)
	wantCode(t, err, "source_quote_required")
	_, err = e.Grant(ctx, editor, editorRole, runID, "wechat", "Van：可以", nil)
	wantCode(t, err, "bad_scope")
	_, err = e.Grant(ctx, editor, editorRole, 9999, domain.ScopeWxDraft, "Van：可以", nil)
	wantCode(t, err, "run_not_found")

	g1, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：可以存草稿", nil)
	if err != nil || g1.Existing {
		t.Fatalf("first grant: existing=%v err=%v", g1.Existing, err)
	}
	g2, err := e.Grant(ctx, editor, editorRole, runID, domain.ScopeWxDraft, "Van：再说一次", nil)
	if err != nil || !g2.Existing || g2.Authorization.ID != g1.Authorization.ID {
		t.Fatalf("repeat grant should return existing: %+v err=%v", g2, err)
	}
	all, _ := st.Q().ListAuthorizations(ctx, runID)
	if len(all) != 1 || all[0].SourceQuote != "Van：可以存草稿" {
		t.Fatalf("want one authorization row, got %+v", all)
	}
}
