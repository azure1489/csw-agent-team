package engine

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func blockerOf(p Progress, code string) (ProgressTask, bool) {
	for _, t := range p.Tasks {
		if t.StageCode == code {
			return t, true
		}
	}
	return ProgressTask{}, false
}

func TestProgressBlockers(t *testing.T) {
	e, st := setupVersion(t, 2) // 通用阻塞判定：固定在 v2 形态（10 手动待派工紧接 07）
	ctx := context.Background()
	editor, editorRole := who(t, st, "editor")
	collector, _ := who(t, st, "collector")
	res, err := e.Trigger(ctx, editor, editorRole, "daily_news", "2026-09-16", "", "")
	if err != nil {
		t.Fatal(err)
	}
	runID := res.Run.ID
	p, err := e.Progress(ctx, runID)
	if err != nil {
		t.Fatal(err)
	}
	if p.Total != 11 || p.Done != 0 {
		t.Fatalf("counts: %d/%d", p.Done, p.Total)
	}
	if it, _ := blockerOf(p, "intake"); it.Blocker == nil || it.Blocker.Kind != "ack" || !strings.Contains(it.Blocker.Text, "情报收集员") {
		t.Fatalf("intake blocker: %+v", it.Blocker)
	}
	if sl, _ := blockerOf(p, "shortlist"); sl.Blocker == nil || sl.Blocker.Text != "待上游：01-情报逐条" {
		t.Fatalf("shortlist blocker: %+v", sl.Blocker)
	}
	d1 := p.Digest

	// 接单：状态变化 → 摘要变化；同一状态重复计算摘要不变。
	intake := taskByCode(t, st, runID, "intake")
	if _, err := e.Ack(ctx, collector, intake.ID); err != nil {
		t.Fatal(err)
	}
	p2, _ := e.Progress(ctx, runID)
	p3, _ := e.Progress(ctx, runID)
	if p2.Digest == d1 || p2.Digest != p3.Digest {
		t.Fatalf("digest should track state only: %s %s %s", d1, p2.Digest, p3.Digest)
	}
	// 30 分钟无活动 → idle。
	if _, err := st.DB().Exec(`UPDATE tasks SET last_activity_at='2000-01-01T00:00:00Z' WHERE id=?`, intake.ID); err != nil {
		t.Fatal(err)
	}
	p4, _ := BuildProgress(ctx, st.Q(), runID, time.Now())
	if it, _ := blockerOf(p4, "intake"); !it.Idle || it.Blocker.Kind != "idle" {
		t.Fatalf("idle: %+v", it)
	}
	// 逾期标记。
	if _, err := st.DB().Exec(`UPDATE tasks SET due_at='2000-01-01T00:00:00Z' WHERE id=?`, intake.ID); err != nil {
		t.Fatal(err)
	}
	p5, _ := e.Progress(ctx, runID)
	if it, _ := blockerOf(p5, "intake"); !it.Overdue {
		t.Fatalf("overdue flag missing")
	}
	if len(p5.CurrentNodes) != 1 || p5.CurrentNodes[0] != "01-情报逐条" {
		t.Fatalf("current nodes: %v", p5.CurrentNodes)
	}

	// 手动派工阶段 / 平台写待授权。
	if _, err := st.DB().Exec(`UPDATE tasks SET status='ready' WHERE run_id=? AND stage_code IN ('xhs_text','wx_save')`, runID); err != nil {
		t.Fatal(err)
	}
	p6, _ := e.Progress(ctx, runID)
	if w, _ := blockerOf(p6, "xhs_text"); w.Blocker == nil || w.Blocker.Kind != "dispatch" || !strings.Contains(w.Blocker.Text, "手动") {
		t.Fatalf("xhs_text blocker: %+v", w.Blocker)
	}
	if ft, _ := blockerOf(p6, "fulltext"); ft.Blocker == nil || ft.Blocker.Text != "待选题批准条目" {
		t.Fatalf("fulltext should wait for approved items: %+v", ft.Blocker)
	}
	if s, _ := blockerOf(p6, "wx_save"); s.Blocker == nil || s.Blocker.Text != "待授权：公众号草稿" {
		t.Fatalf("wx_save blocker: %+v", s.Blocker)
	}
	_, err = e.Progress(ctx, 9999)
	wantCode(t, err, "run_not_found")
	_ = domain.TaskReady
}
