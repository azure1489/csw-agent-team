package engine

import (
	"context"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// TestFullFlowV11 daily_news v11：去掉 02 / 03，Van 在 01 的末闸选题（主编代录）→ 04 / 05 逐条、06 就绪，07 等条目。
// 另固定两件事：01 的历史选题反馈（stage:topic）在 01 读得到；01 通过后交补件会把已开工的下游全部标待返工。
func TestFullFlowV11(t *testing.T) {
	e, st := setupVersion(t, 11)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	collector, collectorRole := who(t, st, "collector")

	// v10 时期 Van 在 03 说过的选题原话。
	if _, _, err := st.Q().InsertFeedback(ctx, domain.FeedbackRecord{Quote: "Van：只换配色的不要", Stance: "explicit",
		Tags: []string{"stage:topic", "decision:topic_approve"}}); err != nil {
		t.Fatal(err)
	}

	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-10-17", "v11", `{"授权":["local_drill"]}`)
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	// 13 段去掉 02 / 03 剩 11 段，04 / 05 逐条不建模板任务。
	if res.Run.WorkflowVer != 11 || len(res.Tasks) != 9 {
		t.Fatalf("want v11 with 9 template tasks, got v%d %d", res.Run.WorkflowVer, len(res.Tasks))
	}
	tasks, _ := st.Q().ListTasksByRun(ctx, runID)
	for _, tk := range tasks {
		if tk.StageCode == "shortlist" || tk.StageCode == "topic" {
			t.Fatalf("v11 不应再有 %s", tk.StageCode)
		}
	}
	expectT := func(tk domain.Task, want domain.TaskStatus) {
		t.Helper()
		got, _ := st.Q().GetTask(ctx, tk.ID)
		if got.Status != want {
			t.Fatalf("%s/%s want %s, got %s", tk.StageCode, tk.ItemKey, want, got.Status)
		}
	}
	expect := func(code string, want domain.TaskStatus) { t.Helper(); expectT(taskByCode(t, st, runID, code), want) }

	intake := taskByCode(t, st, runID, "intake")
	tgs, _ := st.Q().ListTaskGates(ctx, intake.ID)
	if len(tgs) != 2 || tgs[0].ReviewerRole != "editor" || tgs[1].ReviewerRole != "van" || !tgs[1].RelayedByHub {
		t.Fatalf("01 应是 主编格式核对 → Van选题（代录）：%+v", tgs)
	}
	fb, err := TaskFeedback(ctx, st.Q(), intake, 5)
	if err != nil || len(fb) != 1 || fb[0].Quote != "Van：只换配色的不要" {
		t.Fatalf("01 应读到 03 时期的选题反馈：%+v err=%v", fb, err)
	}

	// 01：条目状态由 01 落定，一次交齐。
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "hxo-a1b2c3", Brand: "HxO", Title: "折叠木椅｜结构改了", Status: "shortlisted", Rank: "primary"},
		{Key: "nanga-d4e5f6", Brand: "NANGA", Title: "羽绒睡袋｜拆洗更方便", Status: "shortlisted", Rank: "alt"},
		{Key: "snowpeak-0a0b0c", Brand: "Snow Peak", Title: "补货通知", Status: "dropped", ReasonCode: "no_value", Reason: "只是补货"},
	}); err != nil {
		t.Fatalf("upsert items: %v", err)
	}
	// 返工重登：备选升主选要跟着改，成熟改判待核要清掉 rank（引擎只许 shortlisted 带 rank）。
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "nanga-d4e5f6", Status: "shortlisted", Rank: "primary"},
		{Key: "snowpeak-0a0b0c", Status: "shortlisted", Rank: "alt"},
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "snowpeak-0a0b0c", Status: "pending_check", Reason: "待核：缺发售日期"},
	}); err != nil {
		t.Fatal(err)
	}
	if it, _ := st.Q().GetItem(ctx, runID, "nanga-d4e5f6"); it.Rank != "primary" {
		t.Fatalf("备选升主选后 rank 应是 primary：%q", it.Rank)
	}
	if it, _ := st.Q().GetItem(ctx, runID, "snowpeak-0a0b0c"); it.Status != "pending_check" || it.Rank != "" {
		t.Fatalf("改判待核应清掉 rank：%s %q", it.Status, it.Rank)
	}
	out, err := e.Submit(ctx, collector, intake.ID, SubmitInput{DownloadURL: "http://x/intake", SelfCheck: "ok"})
	if err != nil {
		t.Fatalf("submit 01: %v", err)
	}
	// 主编格式核对：内容意见写在通过意见里。
	if _, err := e.Review(ctx, editor, editorRole, out.ID, ReviewInput{Verdict: domain.VerdictPass, Comment: "格式齐。我会选 HxO 和 NANGA"}); err != nil {
		t.Fatalf("editor gate: %v", err)
	}
	expectT(intake, domain.TaskReview)
	expect("design_prep", domain.TaskBlocked) // Van 选题前下游都不动
	tasks, _ = st.Q().ListTasksByRun(ctx, runID)
	for _, tk := range tasks {
		if tk.StageCode == "write" || tk.StageCode == "material" {
			t.Fatalf("Van 批准前不该有逐条任务：%s/%s", tk.StageCode, tk.ItemKey)
		}
	}

	// Van选题（主编代录，末闸带条目）→ 写作与配图逐条自动派工，版式同时派出。
	if _, err := e.Review(ctx, editor, editorRole, out.ID, ReviewInput{Verdict: domain.VerdictPass, DecisionType: "topic_approve",
		SourceQuote: "Van：HxO 和 NANGA 可以写", ItemsJSON: `["hxo-a1b2c3","nanga-d4e5f6"]`}); err != nil {
		t.Fatalf("van gate: %v", err)
	}
	expectT(intake, domain.TaskPassed)
	wH, wN := taskByItem(t, st, runID, "write", "hxo-a1b2c3"), taskByItem(t, st, runID, "write", "nanga-d4e5f6")
	mH, mN := taskByItem(t, st, runID, "material", "hxo-a1b2c3"), taskByItem(t, st, runID, "material", "nanga-d4e5f6")
	for _, tk := range []domain.Task{wH, wN, mH, mN} {
		expectT(tk, domain.TaskDispatched)
	}
	expect("design_prep", domain.TaskDispatched)
	expect("fulltext", domain.TaskBlocked)
	var note string
	if err := st.DB().QueryRow(`SELECT COALESCE(editor_note,'') FROM deliverables WHERE task_id=? AND kind='dispatch'`, wH.ID).Scan(&note); err != nil ||
		!strings.Contains(note, "折叠木椅") || !strings.Contains(note, "Van：HxO 和 NANGA 可以写") {
		t.Fatalf("write dispatch note: %q err=%v", note, err)
	}
	// 06 是非逐条下游：01 主编的通过意见随派工单带下来。
	if err := st.DB().QueryRow(`SELECT COALESCE(editor_note,'') FROM deliverables WHERE task_id=? AND kind='dispatch'`,
		taskByCode(t, st, runID, "design_prep").ID).Scan(&note); err != nil || !strings.Contains(note, "我会选 HxO 和 NANGA") {
		t.Fatalf("design_prep dispatch note 应带 01 通过意见：%q err=%v", note, err)
	}

	// 已知行为（v11 交付规范据此要求 01 一次交齐）：01 通过后交补件，已开工的 04 / 05 / 06 全部标待返工，不按条目过滤。
	if _, err := e.Submit(ctx, collector, intake.ID, SubmitInput{Kind: domain.KindSupplement, AffectsID: &out.ID,
		DownloadURL: "http://x/intake-s1", Summary: "改 HxO 原始披露时间"}); err != nil {
		t.Fatalf("01 supplement: %v", err)
	}
	for _, tk := range []domain.Task{wH, wN, mH, mN, taskByCode(t, st, runID, "design_prep")} {
		got, _ := st.Q().GetTask(ctx, tk.ID)
		if !got.ReworkPending {
			t.Fatalf("%s/%s 应被标待返工", tk.StageCode, tk.ItemKey)
		}
	}
}

// TestMergeCancelledKeepsRunActive 合流阶段被取消是中枢止损暂停，不是做完了：整期保持进行中（停滞提醒照发），
// 中枢重开或显式 close 才结束（10-18 r80：07–09 取消等 Van 定配图例外，其余素材交齐后被记成完成）。
func TestMergeCancelledKeepsRunActive(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{
		{code: "a", role: "collector"},
		{code: "m", role: "editor", merge: true, deps: []string{"a"}, gates: []fxGate{{reviewer: "editor"}}},
	})
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatal(err)
	}
	runID := res.Run.ID
	if _, err := e.Submit(ctx, collector, taskByCode(t, st, runID, "a").ID, SubmitInput{DownloadURL: "http://x/a"}); err != nil {
		t.Fatal(err)
	}
	if _, err := e.Cancel(ctx, editor, editorRole, taskByCode(t, st, runID, "m").ID, "等 Van 定配图例外"); err != nil {
		t.Fatal(err)
	}
	run, _ := st.Q().GetRun(ctx, runID)
	if run.Status != domain.RunActive {
		t.Fatalf("合流阶段被取消，整期应保持进行中，got %s", run.Status)
	}
	if _, err := e.CloseRun(ctx, editor, editorRole, runID, "接受本期不出稿"); err != nil {
		t.Fatalf("中枢显式 close 应可收尾：%v", err)
	}
	if run, _ = st.Q().GetRun(ctx, runID); run.Status != domain.RunDone {
		t.Fatalf("close 后应完成，got %s", run.Status)
	}
}

// TestHubNotice 中枢按角色发群消息：只有中枢能发，角色要存在，写 outbox 主送第一个、其余抄送。
func TestHubNotice(t *testing.T) {
	e, st := setupVersion(t, 11)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	collector, collectorRole := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-10-18", "", "")
	if err != nil {
		t.Fatal(err)
	}
	runID := res.Run.ID
	wantCode(t, e.HubNotice(ctx, collector, collectorRole, runID, []string{"van"}, "x"), "not_hub")
	wantCode(t, e.HubNotice(ctx, editor, editorRole, runID, []string{"nobody"}, "x"), "unknown_role")
	wantCode(t, e.HubNotice(ctx, editor, editorRole, runID, nil, "x"), "to_required")
	wantCode(t, e.HubNotice(ctx, editor, editorRole, runID, []string{"van"}, "  "), "text_required")
	if err := e.HubNotice(ctx, editor, editorRole, runID, []string{"van", "collector", "van"}, "请确认两图例外"); err != nil {
		t.Fatal(err)
	}
	var target, cc, payload string
	if err := st.DB().QueryRow(`SELECT target_role, cc_roles, payload_json FROM outbox WHERE event_type=? AND run_id=?`,
		domain.EvtHubNotice, runID).Scan(&target, &cc, &payload); err != nil {
		t.Fatal(err)
	}
	if target != "van" || cc != "collector" || !strings.Contains(payload, "请确认两图例外") {
		t.Fatalf("outbox: target=%s cc=%s payload=%s", target, cc, payload)
	}
}
