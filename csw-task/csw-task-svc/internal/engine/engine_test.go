package engine

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func setup(t *testing.T) (*Engine, *sqlite.Store) {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	st := sqlite.New(db)
	return New(st), st
}

// setupV1 在 setup 基础上把 daily_news 切回 v1（十阶段双闸）：
// 旧用例按 v1 形态断言，v2 定义由 TestFullFlowV2 与 fixture 用例覆盖。
func setupV1(t *testing.T) (*Engine, *sqlite.Store) {
	t.Helper()
	return setupVersion(t, 1)
}

// setupVersion 在 setup 基础上把 daily_news 切到指定版本（归档版本的用例按各自形态断言）。
func setupVersion(t *testing.T, ver int) (*Engine, *sqlite.Store) {
	t.Helper()
	e, st := setup(t)
	if err := st.Q().ActivateWorkflowVersion(context.Background(), "daily_news", ver); err != nil {
		t.Fatalf("activate daily_news v%d: %v", ver, err)
	}
	return e, st
}

func who(t *testing.T, st *sqlite.Store, role string) (domain.Agent, domain.Role) {
	t.Helper()
	ctx := context.Background()
	a, err := st.Q().ActiveAgentByRole(ctx, role)
	if err != nil {
		t.Fatalf("agent for %s: %v", role, err)
	}
	r, err := st.Q().GetRole(ctx, role)
	if err != nil {
		t.Fatalf("role %s: %v", role, err)
	}
	return a, r
}

func taskByCode(t *testing.T, st *sqlite.Store, runID int64, code string) domain.Task {
	t.Helper()
	tasks, err := st.Q().ListTasksByRun(context.Background(), runID)
	if err != nil {
		t.Fatalf("list tasks: %v", err)
	}
	for _, tk := range tasks {
		if tk.StageCode == code {
			return tk
		}
	}
	t.Fatalf("no task %s", code)
	return domain.Task{}
}

// curDeliverable 取 task 当前版本的产出交付物 id。
func curDeliverable(t *testing.T, st *sqlite.Store, taskID int64, version int) int64 {
	t.Helper()
	ds, err := st.Q().ListDeliverablesByTask(context.Background(), taskID)
	if err != nil {
		t.Fatalf("list deliverables: %v", err)
	}
	for _, d := range ds {
		if !d.IsDispatch && d.Version == version {
			return d.ID
		}
	}
	t.Fatalf("no deliverable v%d for task %d", version, taskID)
	return 0
}

// TestFullFlow 驱动 daily_news 整条流程：触发→派工→提交→双闸→退回重提→合流自动派→run done。
func TestFullFlow(t *testing.T) {
	e, st := setupV1(t)
	ctx := context.Background()

	editor, editorRole := who(t, st, "editor")

	// 步 0 · 触发
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-06-09", "测试", `{"采集要求":"露营装备上新"}`)
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	if len(res.Tasks) != 10 {
		t.Fatalf("want 10 tasks, got %d", len(res.Tasks))
	}
	if got := taskByCode(t, st, runID, "collect").Status; got != domain.TaskReady {
		t.Fatalf("01 want ready, got %s", got)
	}
	if got := taskByCode(t, st, runID, "topic").Status; got != domain.TaskBlocked {
		t.Fatalf("02 want blocked, got %s", got)
	}

	// passGate 对当前版本走一道闸（actor 角色由闸决定；人工闸用 editor 代录）。
	passStage := func(stageCode, producerRole, docType string) {
		t.Helper()
		task := taskByCode(t, st, runID, stageCode)
		// 确保已派工：ready 则中枢派工；合流阶段已自动派工。
		if task.Status == domain.TaskReady {
			if _, err := e.Dispatch(ctx, editor, editorRole, task.ID, "做"+stageCode, nil); err != nil {
				t.Fatalf("dispatch %s: %v", stageCode, err)
			}
		}
		pa, _ := who(t, st, producerRole)
		d, err := e.Submit(ctx, pa, task.ID, SubmitInput{DocType: docType, DownloadURL: fmt.Sprintf("http://x/files/%s", stageCode), SelfCheck: "ok"})
		if err != nil {
			t.Fatalf("submit %s: %v", stageCode, err)
		}
		// 两道闸：① editor ② van（中枢 editor 代录）。
		if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, Comment: "ok"}); err != nil {
			t.Fatalf("review gate1 %s: %v", stageCode, err)
		}
		r2, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass, Comment: "Van：ok"})
		if err != nil {
			t.Fatalf("review gate2 %s: %v", stageCode, err)
		}
		if !r2.Final || r2.TaskStatus != domain.TaskPassed {
			t.Fatalf("%s not passed after 2 gates: final=%v status=%s", stageCode, r2.Final, r2.TaskStatus)
		}
	}

	// 步 1-4 · 01-采集
	passStage("collect", "collector", "资讯包")
	if got := taskByCode(t, st, runID, "topic").Status; got != domain.TaskReady {
		t.Fatalf("02 want ready after 01 passed, got %s", got)
	}

	// 步 5 · 派工 02 校验自动预填上游 = 01 链接
	t02 := taskByCode(t, st, runID, "topic")
	dr, err := e.Dispatch(ctx, editor, editorRole, t02.ID, "选5则", nil)
	if err != nil {
		t.Fatalf("dispatch 02: %v", err)
	}
	if len(dr.Upstreams) != 1 || dr.Upstreams[0].UpstreamURL != "http://x/files/collect" {
		t.Fatalf("02 upstream prefill wrong: %+v", dr.Upstreams)
	}

	// 步 6-9 · 02 提交 → 退回一次 → 升版本重交 → 双闸过
	researcher, _ := who(t, st, "researcher")
	d1, err := e.Submit(ctx, researcher, t02.ID, SubmitInput{DocType: "选题成品", DownloadURL: "http://x/files/topic", SelfCheck: "5则"})
	if err != nil {
		t.Fatalf("submit 02 v1: %v", err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d1.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatalf("02 v1 gate1: %v", err)
	}
	// gate2 reject（必填方向/位置）
	if _, err := e.Review(ctx, editor, editorRole, d1.ID, ReviewInput{Verdict: domain.VerdictReject}); err == nil {
		t.Fatalf("reject without direction/location should 400")
	}
	rr, err := e.Review(ctx, editor, editorRole, d1.ID, ReviewInput{Verdict: domain.VerdictReject, ReturnDirection: "补足15张", ReturnLocation: "第3则", Comment: "Van：补足15张"})
	if err != nil {
		t.Fatalf("02 v1 gate2 reject: %v", err)
	}
	if rr.TaskStatus != domain.TaskReturned {
		t.Fatalf("02 want returned, got %s", rr.TaskStatus)
	}
	if rr.Deliverable.ReturnedAtGate == nil || *rr.Deliverable.ReturnedAtGate != 2 {
		t.Fatalf("02 returned_at_gate want 2, got %v", rr.Deliverable.ReturnedAtGate)
	}
	// review/passed 态守卫：returned 后旧版本不能再审，但 task 可重提
	// 升版本重交
	d2, err := e.Submit(ctx, researcher, t02.ID, SubmitInput{DocType: "选题成品", DownloadURL: "http://x/files/topic", SelfCheck: "15图"})
	if err != nil {
		t.Fatalf("submit 02 v2: %v", err)
	}
	if d2.Version != 2 || d2.CurGate != 0 {
		t.Fatalf("02 v2 want version=2 cur_gate=0, got v%d gate%d", d2.Version, d2.CurGate)
	}
	if _, err := e.Review(ctx, editor, editorRole, d2.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatalf("02 v2 gate1: %v", err)
	}
	r2, err := e.Review(ctx, editor, editorRole, d2.ID, ReviewInput{Verdict: domain.VerdictPass, SourceQuote: "Van：通过"})
	if err != nil {
		t.Fatalf("02 v2 gate2: %v", err)
	}
	if !r2.Final {
		t.Fatalf("02 v2 should pass final")
	}

	// 提交守卫：02 已 passed，再提交应 409
	if _, err := e.Submit(ctx, researcher, t02.ID, SubmitInput{DocType: "x", DownloadURL: "y"}); err == nil {
		t.Fatalf("submit on passed task should 409")
	}

	// 步 11-13 · 03 → 04 → 05（合流自动派）
	passStage("wx_content", "writer", "文章")
	passStage("wx_visual", "designer", "封面")
	t05 := taskByCode(t, st, runID, "wx_final")
	if t05.Status != domain.TaskDispatched {
		t.Fatalf("05 merge should auto-dispatch, got %s", t05.Status)
	}
	if t05.AssigneeID == nil || *t05.AssigneeID != editor.ID {
		t.Fatalf("05 assignee should be hub(editor)")
	}

	// 05 由中枢自己提交 + 双闸
	passStage("wx_final", "editor", "成品")
	passStage("wx_publish", "publisher", "发布物")
	passStage("xhs_text", "writer", "小红书文本")
	passStage("xhs_visual", "designer", "小红书卡片")
	t09 := taskByCode(t, st, runID, "xhs_final")
	if t09.Status != domain.TaskDispatched {
		t.Fatalf("09 merge should auto-dispatch, got %s", t09.Status)
	}
	passStage("xhs_final", "editor", "成品")
	passStage("xhs_publish", "publisher", "发布物")

	// run done
	run, err := st.Q().GetRun(ctx, runID)
	if err != nil {
		t.Fatalf("get run: %v", err)
	}
	if run.Status != domain.RunDone {
		t.Fatalf("run want done, got %s", run.Status)
	}

	// 时间线非空
	evs, err := st.Q().ListEventsByRun(ctx, runID)
	if err != nil {
		t.Fatalf("events: %v", err)
	}
	if len(evs) < 20 {
		t.Fatalf("timeline too short: %d events", len(evs))
	}
}

// TestPermissions 校验关键鉴权护栏。
func TestPermissions(t *testing.T) {
	e, st := setupV1(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	collector, collectorRole := who(t, st, "collector")

	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-06-10", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID

	// 非中枢派工 → 403
	t01 := taskByCode(t, st, runID, "collect")
	if _, err := e.Dispatch(ctx, collector, collectorRole, t01.ID, "x", nil); err == nil {
		t.Fatalf("non-hub dispatch should 403")
	}

	// 重复触发同 subject → 已放宽唯一约束（0004），允许并建独立的第二个 run（靠 run_id 区分）
	if res2, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-06-10", "", ""); err != nil {
		t.Fatalf("duplicate subject should now succeed: %v", err)
	} else if res2.Run.ID == runID {
		t.Fatalf("second run should have a distinct id, got %d", res2.Run.ID)
	}

	// 派工后由非 assignee 提交 → 403
	if _, err := e.Dispatch(ctx, editor, editorRole, t01.ID, "采集", nil); err != nil {
		t.Fatalf("dispatch: %v", err)
	}
	writer, _ := who(t, st, "writer")
	if _, err := e.Submit(ctx, writer, t01.ID, SubmitInput{DocType: "资讯包", DownloadURL: "u"}); err == nil {
		t.Fatalf("non-assignee submit should 403")
	}

	// collector 提交 → 进 review；此时再提交 → 409
	if _, err := e.Submit(ctx, collector, t01.ID, SubmitInput{DocType: "资讯包", DownloadURL: "u"}); err != nil {
		t.Fatalf("collector submit: %v", err)
	}
	if _, err := e.Submit(ctx, collector, t01.ID, SubmitInput{DocType: "资讯包", DownloadURL: "u"}); err == nil {
		t.Fatalf("submit during review should 409")
	}
}

// TestInboxQueries 防 inbox 相关含 JOIN 查询的列歧义回归。
func TestInboxQueries(t *testing.T) {
	e, st := setupV1(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	if _, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-06-13", "", ""); err != nil {
		t.Fatalf("trigger: %v", err)
	}
	ready, err := st.Q().ListReadyTasks(ctx)
	if err != nil {
		t.Fatalf("ListReadyTasks: %v", err)
	}
	if len(ready) != 1 || ready[0].StageCode != "collect" {
		t.Fatalf("want 1 ready=collect, got %+v", ready)
	}
	if _, err := st.Q().ListPendingReviews(ctx); err != nil {
		t.Fatalf("ListPendingReviews: %v", err)
	}
}

// TestReviewEventDetailAndDocTypeDefault 退回原因写进事件 detail（timeline 自助可读）；
// submit 不传 doc_type 时缺省取阶段 output_type 快照。
func TestReviewEventDetailAndDocTypeDefault(t *testing.T) {
	e, st := setupV1(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")

	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-06-14", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	task := taskByCode(t, st, runID, "collect")
	if task.OutputType != "资讯包" {
		t.Fatalf("output_type snapshot: %q", task.OutputType)
	}
	if _, err := e.Dispatch(ctx, editor, editorRole, task.ID, "去采", nil); err != nil {
		t.Fatalf("dispatch: %v", err)
	}

	collector, _ := who(t, st, "collector")
	// 不传 DocType → 取 output_type。
	d, err := e.Submit(ctx, collector, task.ID, SubmitInput{DownloadURL: "http://x/1", SelfCheck: "ok"})
	if err != nil {
		t.Fatalf("submit: %v", err)
	}
	if d.DocType != "资讯包" {
		t.Fatalf("doc_type default want 资讯包, got %q", d.DocType)
	}

	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{
		Verdict: domain.VerdictReject, ReturnDirection: "补足来源", ReturnLocation: "第3条", Comment: "缺原文链接",
	}); err != nil {
		t.Fatalf("reject: %v", err)
	}

	events, err := st.Q().ListEventsByRun(ctx, runID)
	if err != nil {
		t.Fatalf("events: %v", err)
	}
	var detail string
	for _, ev := range events {
		if ev.Type == domain.EvtGateReturned {
			detail = ev.DetailJSON
		}
	}
	for _, want := range []string{`"return_direction":"补足来源"`, `"return_location":"第3条"`, `"comment":"缺原文链接"`, `"gate_name":"主编审"`} {
		if !strings.Contains(detail, want) {
			t.Fatalf("gate_returned detail 缺 %s：%s", want, detail)
		}
	}
}
