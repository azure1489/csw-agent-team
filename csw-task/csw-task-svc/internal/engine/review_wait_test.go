package engine

import (
	"context"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// TestReviewWaitReminders 待审提醒：交付物停在闸上 → 提醒审核方，每档只发一次；升级 @Van 抄送中枢、只升级一次；
// 过闸后是新的一次等待；Van 闸提醒中枢去跟；交新版本从头计。
// 09-28 r56 #592：v5 交上去时主编正忙，@ 被并进上一轮漏掉，之后八小时三类告警一条都没有。
func TestReviewWaitReminders(t *testing.T) {
	e, st := setup(t)
	ctx := context.Background()
	buildFlow(t, st, "auto", []fxStage{
		{code: "a", role: "writer", gates: []fxGate{{reviewer: "editor"}, {reviewer: "van", relayed: true}}},
	})
	editor, editorRole := who(t, st, "editor")
	writer, _ := who(t, st, "writer")
	res, err := e.Trigger(ctx, editor, editorRole, "t_flow", "s1", "", "")
	if err != nil {
		t.Fatal(err)
	}
	a := taskByCode(t, st, res.Run.ID, "a")
	d, err := e.Submit(ctx, writer, a.ID, SubmitInput{DownloadURL: "http://x/a"})
	if err != nil {
		t.Fatal(err)
	}
	waits := func() []sqlite.ReviewWait {
		t.Helper()
		ws, err := st.Q().ListReviewWaits(ctx)
		if err != nil {
			t.Fatal(err)
		}
		return ws
	}
	ws := waits()
	if len(ws) != 1 || ws[0].DeliverableID != d.ID || ws[0].GateOrder != 1 || ws[0].ReviewerRole != "editor" || ws[0].Reminded != 0 || ws[0].Since == "" {
		t.Fatalf("待审列表：%+v", ws)
	}

	w := ws[0]
	if ok, err := e.NotifyReviewWaiting(ctx, w, 20); err != nil || !ok {
		t.Fatalf("第一次提醒：ok=%v err=%v", ok, err)
	}
	// 同一档（仍按「提醒过 0 次」来）不重复发
	if ok, _ := e.NotifyReviewWaiting(ctx, w, 20); ok {
		t.Fatal("同一档不该发两次")
	}
	if r := lastOf(t, st, domain.EvtReviewWaiting); r.target != "editor" || r.cc != "" || r.payload["reminded"] != float64(1) {
		t.Fatalf("待审提醒：%+v", r)
	}
	w = waits()[0]
	if w.Reminded != 1 || w.LastRemindedAt == "" {
		t.Fatalf("应记下提醒过 1 次：%+v", w)
	}
	if ok, err := e.NotifyReviewWaiting(ctx, w, 35); err != nil || !ok {
		t.Fatalf("第二档：ok=%v err=%v", ok, err)
	}

	if ok, err := e.NotifyReviewEscalated(ctx, waits()[0], 200); err != nil || !ok {
		t.Fatalf("升级：ok=%v err=%v", ok, err)
	}
	if ok, _ := e.NotifyReviewEscalated(ctx, waits()[0], 260); ok {
		t.Fatal("同一次等待只升级一次")
	}
	if r := lastOf(t, st, domain.EvtReviewEscalated); r.target != "van" || r.cc != "editor" {
		t.Fatalf("升级去向：%+v", r)
	}

	// 主编闸过了 → 到 Van 闸：新的一次等待，从 0 计；Van 闸提醒中枢去跟
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	ws = waits()
	if len(ws) != 1 || ws[0].GateOrder != 2 || ws[0].Reminded != 0 || ws[0].EscalatedAt != "" || !ws[0].RelayedByHub {
		t.Fatalf("过闸后应是新的一次等待：%+v", ws)
	}
	if ok, err := e.NotifyReviewWaiting(ctx, ws[0], 16); err != nil || !ok {
		t.Fatalf("Van 闸提醒：ok=%v err=%v", ok, err)
	}
	if r := lastOf(t, st, domain.EvtReviewWaiting); r.target != "editor" || r.payload["relayed"] != true {
		t.Fatalf("Van 闸提醒应给中枢：%+v", r)
	}

	// 退回后重交：新版本从头计
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictReject, ReturnDirection: "补图", ReturnLocation: "第 2 段", SourceQuote: "Van：图不对"}); err != nil {
		t.Fatal(err)
	}
	if len(waits()) != 0 {
		t.Fatal("退回后不在待审")
	}
	d2, err := e.Submit(ctx, writer, a.ID, SubmitInput{DownloadURL: "http://x/a2"})
	if err != nil {
		t.Fatal(err)
	}
	ws = waits()
	if len(ws) != 1 || ws[0].DeliverableID != d2.ID || ws[0].GateOrder != 1 || ws[0].Reminded != 0 {
		t.Fatalf("新版本应从头计：%+v", ws)
	}

	// 等待开始于 48 小时以前的（历史遗留）不捞
	if _, err := st.DB().Exec(`UPDATE deliverables SET created_at=strftime('%Y-%m-%dT%H:%M:%SZ','now','-3 days') WHERE id=?`, d2.ID); err != nil {
		t.Fatal(err)
	}
	if len(waits()) != 0 {
		t.Fatal("三天前开始的等待不该补提醒")
	}
}
