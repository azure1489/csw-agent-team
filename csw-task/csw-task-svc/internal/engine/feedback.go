package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// recordReviewFeedback 审核带原话时沉淀一条 explicit 编辑反馈（review_id 唯一，重放不重复）。
// 标签：stage:<阶段>、decision:<决定类别>、kind:return（退回时）、brand:<条目品牌>、item:<条目键>。
func recordReviewFeedback(ctx context.Context, q *sqlite.Queries, run domain.Run, task domain.Task, reviewID int64, in ReviewInput, quote string) error {
	if strings.TrimSpace(quote) == "" {
		return nil
	}
	tags := []string{"stage:" + task.StageCode}
	if in.DecisionType != "" {
		tags = append(tags, "decision:"+in.DecisionType)
	}
	if in.Verdict == domain.VerdictReject {
		tags = append(tags, "kind:return")
	}
	keys := []string{}
	if task.ItemKey != "" {
		keys = append(keys, task.ItemKey)
	}
	if in.ItemsJSON != "" {
		var more []string
		_ = json.Unmarshal([]byte(in.ItemsJSON), &more)
		keys = append(keys, more...)
	}
	for _, k := range keys {
		tags = append(tags, "item:"+k)
		if it, err := q.GetItem(ctx, run.ID, k); err == nil && it.Brand != "" {
			tags = append(tags, "brand:"+strings.ToLower(it.Brand))
		}
	}
	body := quote
	if in.Verdict == domain.VerdictReject && (in.ReturnDirection != "" || in.ReturnLocation != "") {
		body = fmt.Sprintf("%s（退回：方向=%s；位置=%s）", quote, in.ReturnDirection, in.ReturnLocation)
	}
	_, _, err := q.InsertFeedback(ctx, domain.FeedbackRecord{
		Quote: body, SaidAt: time.Now().UTC().Format("2006-01-02T15:04:05Z"),
		SourceRef: fmt.Sprintf("r%d 任务#%d 审核#%d", run.ID, task.ID, reviewID), ObjectRef: fmt.Sprintf("task:%d", task.ID),
		ObjectVersion: fmt.Sprintf("v%d", task.CurVersion), Kind: "recent", Stance: "explicit", ReviewID: &reviewID, Tags: tags,
	})
	return err
}

// TaskFeedback 与任务相关的有效编辑反馈（按阶段、条目品牌匹配，至多 limit 条），任务详情随附。
func TaskFeedback(ctx context.Context, q *sqlite.Queries, task domain.Task, limit int) ([]domain.FeedbackRecord, error) {
	tags := []string{"stage:" + task.StageCode}
	if task.ItemKey != "" {
		tags = append(tags, "item:"+task.ItemKey)
		if it, err := q.GetItem(ctx, task.RunID, task.ItemKey); err == nil && it.Brand != "" {
			tags = append(tags, "brand:"+strings.ToLower(it.Brand))
		}
	}
	return q.ListActiveFeedback(ctx, tags, limit)
}
