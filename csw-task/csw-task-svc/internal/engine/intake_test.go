package engine

import (
	"context"
	"testing"
)

// TestReportSweepsIdempotent 同一 sweep_key 重报只更新不新增；非参与角色被拒。
func TestReportSweepsIdempotent(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v6", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	collector, collectorRole := who(t, st, "collector")

	sweeps, err := e.ReportSweeps(ctx, collector, collectorRole, runID, []SweepInput{
		{SweepKey: "ig-1", Platform: "instagram", SourceKey: "channel", Tool: "csw_mcp", Found: 12, InWindow: 5, Registered: 3, Result: "ok"},
		{SweepKey: "xhs-1", Platform: "xhs", SourceKey: "channel", Tool: "opencli", Result: "failed", Error: "登录态失效"},
	})
	if err != nil {
		t.Fatalf("report: %v", err)
	}
	if len(sweeps) != 2 {
		t.Fatalf("want 2 sweeps, got %d", len(sweeps))
	}

	// 同键重报：条数不变，计数被更新。
	sweeps, err = e.ReportSweeps(ctx, collector, collectorRole, runID, []SweepInput{
		{SweepKey: "ig-1", Platform: "instagram", SourceKey: "channel", Tool: "csw_mcp", Found: 20, InWindow: 8, Registered: 6, Result: "ok"},
	})
	if err != nil {
		t.Fatalf("re-report: %v", err)
	}
	if len(sweeps) != 2 {
		t.Fatalf("重报后仍应是 2 轮，got %d", len(sweeps))
	}
	for _, sw := range sweeps {
		if sw.SweepKey == "ig-1" && (sw.Found != 20 || sw.Registered != 6) {
			t.Fatalf("同键重报未更新计数：%+v", sw)
		}
	}

	// 失败轮必须写 error，否则分不清没找到与没去找。
	if _, err := e.ReportSweeps(ctx, collector, collectorRole, runID,
		[]SweepInput{{SweepKey: "web-1", Platform: "web", Result: "failed"}}); err == nil {
		t.Fatal("失败的采集轮缺 error 应被拒")
	} else {
		wantCode(t, err, "sweep_error_required")
	}

	// 非本 run 参与角色被拒。
	analyst, analystRole := who(t, st, "analyst")
	if _, err := e.ReportSweeps(ctx, analyst, analystRole, runID,
		[]SweepInput{{SweepKey: "x", Platform: "web", Result: "ok"}}); err == nil {
		t.Fatal("非参与角色不该能上报采集轮")
	} else {
		wantCode(t, err, "not_participant")
	}
}

// TestIntakeTraceAndCheckOnLegacyRun 旧 run 没有任何采集记录时，合成与判据都不报错，
// 判据给「不判定」而不是失败——明早 05:30 之前建的 run 全是这种。
func TestIntakeTraceAndCheckOnLegacyRun(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v6", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID

	tr, err := BuildIntakeTrace(ctx, st.Q(), runID)
	if err != nil {
		t.Fatalf("trace: %v", err)
	}
	if tr.Sweeps == nil || tr.Items == nil || tr.Traces == nil || tr.Coverage == nil {
		t.Fatal("空数据应返回空切片而不是 nil，否则 JSON 会出 null")
	}
	if len(tr.Sweeps) != 0 {
		t.Fatalf("want 0 sweeps, got %d", len(tr.Sweeps))
	}

	checks, err := BuildIntakeCheck(ctx, st.Q(), runID)
	if err != nil {
		t.Fatalf("check: %v", err)
	}
	if len(checks) != 8 {
		t.Fatalf("want 8 checks, got %d", len(checks))
	}
	for _, c := range checks {
		if !c.OK && !c.Skipped {
			t.Fatalf("旧 run 不该判失败：%s（%s）", c.Name, c.Detail)
		}
	}
}

// TestIntakeCheckCatchesMissingRequiredSource 必扫来源一轮没扫时判失败，并指出是哪个。
func TestIntakeCheckCatchesMissingRequiredSource(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-17", "v6", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	collector, collectorRole := who(t, st, "collector")

	// 只扫 Instagram：台账里 xhs 与 web 也是必扫，应被点名。
	if _, err := e.ReportSweeps(ctx, collector, collectorRole, runID, []SweepInput{
		{SweepKey: "ig-1", Platform: "instagram", SourceKey: "channel", Tool: "csw_mcp", Found: 9, FetchedUnique: 9, Reviewed: 3, Unreviewed: 6, InWindow: 3, Registered: 3, Result: "ok"},
	}); err != nil {
		t.Fatalf("report: %v", err)
	}
	if _, err := e.UpsertItems(ctx, collector, collectorRole, runID, []ItemInput{
		{Key: "hxo-a1b2c3", Brand: "HxO", Title: "折叠木椅", Status: "pending_check", DiscoveredVia: "ig-1", DedupNote: "近 30 天未见同角度"},
		{Key: "nanga-d4e5f6", Brand: "NANGA", Title: "羽绒睡袋", Status: "shortlisted", Rank: "primary", DiscoveredVia: "ig-1", DedupNote: "与 9/2 那篇角度不同"},
		{Key: "keen-778899", Brand: "KEEN", Title: "凉鞋", Status: "dropped", ReasonCode: "no_value", Reason: "只是配色更新", DiscoveredVia: "ig-1"},
	}); err != nil {
		t.Fatalf("upsert: %v", err)
	}

	checks, err := BuildIntakeCheck(ctx, st.Q(), runID)
	if err != nil {
		t.Fatalf("check: %v", err)
	}
	byName := map[string]IntakeCheck{}
	for _, c := range checks {
		byName[c.Name] = c
	}
	cov := byName["来源覆盖：必扫的源是否都扫过"]
	if cov.OK || cov.Skipped {
		t.Fatalf("漏扫必扫来源应判失败，got %+v", cov)
	}
	if !contains(cov.Detail, "xhs") || !contains(cov.Detail, "web") {
		t.Fatalf("结论应点名漏掉的来源，got %q", cov.Detail)
	}
	spread := byName["来源分布：条目是否集中在单一平台"]
	if !spread.Warn {
		t.Fatalf("条目全来自单一平台应给提醒，got %+v", spread)
	}
	drop := byName["淘汰交代：没入选的条目都有理由"]
	if !drop.OK {
		t.Fatalf("淘汰都写了理由码应通过，got %+v", drop)
	}
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (len(sub) == 0 || indexOf(s, sub) >= 0)
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}
