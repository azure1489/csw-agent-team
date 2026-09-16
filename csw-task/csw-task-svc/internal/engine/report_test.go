package engine

import (
	"context"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// TestBuildRunReport 一期用时报告：把耗时拆成作业、等主编、等 Van，并数出返工轮次。
func TestBuildRunReport(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v6", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	collector, _ := who(t, st, "collector")
	intake := taskByCode(t, st, runID, "intake")

	// 首版被退回 → 第二版通过：01 有一道主编闸，两版即一轮返工。
	d1, err := e.Submit(ctx, collector, intake.ID, SubmitInput{DownloadURL: "http://x/1", SelfCheck: "ok"})
	if err != nil {
		t.Fatalf("submit v1: %v", err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d1.ID, ReviewInput{
		Verdict: domain.VerdictReject, Comment: "方向不对", ReturnDirection: "换来源", ReturnLocation: "全篇"}); err != nil {
		t.Fatalf("reject: %v", err)
	}
	d2, err := e.Submit(ctx, collector, intake.ID, SubmitInput{DownloadURL: "http://x/2", SelfCheck: "ok"})
	if err != nil {
		t.Fatalf("submit v2: %v", err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d2.ID, ReviewInput{Verdict: domain.VerdictPass, Comment: "可以"}); err != nil {
		t.Fatalf("pass: %v", err)
	}

	rep, err := BuildRunReport(ctx, st.Q(), runID)
	if err != nil {
		t.Fatalf("report: %v", err)
	}
	if rep.RunID != runID || rep.Subject != "2026-09-17" {
		t.Fatalf("report head: %+v", rep)
	}
	if rep.Reworks != 1 {
		t.Fatalf("want 1 rework round, got %d", rep.Reworks)
	}
	var intakeTiming *StageTiming
	for i := range rep.Stages {
		if rep.Stages[i].Stage == "intake" {
			intakeTiming = &rep.Stages[i]
		}
	}
	if intakeTiming == nil || intakeTiming.Versions != 2 || intakeTiming.Role != "collector" {
		t.Fatalf("intake timing: %+v", intakeTiming)
	}
	if rep.WaitVanMin != 0 {
		t.Fatalf("01 没有 Van 闸，等 Van 应为 0，got %d", rep.WaitVanMin)
	}
	if rep.WorkMin < 0 || rep.WaitHubMin < 0 {
		t.Fatalf("negative durations: %+v", rep)
	}
}
