package engine

import (
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func TestReviewQuoteBecomesFeedback(t *testing.T) {
	e, ctx, editor, editorRole, writer, runID := editFlow(t)
	st := e.store
	f := taskByCode(t, st, runID, "f")
	d, err := e.Submit(ctx, writer, f.ID, SubmitInput{DownloadURL: "http://x/f"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictPass}); err != nil {
		t.Fatal(err)
	}
	if fs, _ := st.Q().ListActiveFeedback(ctx, nil, 10); len(fs) != 0 {
		t.Fatalf("review without a quote must not create feedback: %+v", fs)
	}
	if _, err := e.Review(ctx, editor, editorRole, d.ID, ReviewInput{Verdict: domain.VerdictReject, ReturnDirection: "换角度", ReturnLocation: "第三段",
		SourceQuote: "Van：第三段只是复述参数，没有判断"}); err != nil {
		t.Fatal(err)
	}
	fs, _ := st.Q().ListActiveFeedback(ctx, []string{"stage:f"}, 10)
	if len(fs) != 1 || fs[0].Stance != "explicit" || !strings.Contains(fs[0].Quote, "没有判断") || !strings.Contains(fs[0].Quote, "位置=第三段") {
		t.Fatalf("feedback: %+v", fs)
	}
	tags := strings.Join(fs[0].Tags, ",")
	if !strings.Contains(tags, "stage:f") || !strings.Contains(tags, "kind:return") {
		t.Fatalf("tags: %s", tags)
	}
	// 同一审核记录重放不重复沉淀。
	id, created, err := st.Q().InsertFeedback(ctx, domain.FeedbackRecord{Quote: "x", ReviewID: fs[0].ReviewID})
	if err != nil || created || id != fs[0].ID {
		t.Fatalf("review_id must be idempotent: id=%d created=%v err=%v", id, created, err)
	}
	got, _ := TaskFeedback(ctx, st.Q(), f, 5)
	if len(got) != 1 {
		t.Fatalf("task feedback: %+v", got)
	}
}
