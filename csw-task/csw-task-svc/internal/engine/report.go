package engine

import (
	"context"
	"sort"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// RunReport 一期用时报告：把耗时拆成实际作业、等中枢（派工与审核）、等 Van（代录闸）、返工轮次，
// 用于「用结果验证改进」，不把等待合并后归咎于某个角色。
type RunReport struct {
	Subject    string        `json:"subject"`
	Status     string        `json:"status"`
	RunID      int64         `json:"run_id"`
	Stages     []StageTiming `json:"stages"`
	WorkMin    int           `json:"work_minutes"`     // 派工 / 退回到提交之间（执行方作业）
	WaitHubMin int           `json:"wait_hub_minutes"` // 提交到主编闸出结论，以及通过到下一次派工
	WaitVanMin int           `json:"wait_van_minutes"` // 主编闸通过到 Van 闸出结论（代录）
	Reworks    int           `json:"rework_rounds"`    // 首版之外的版本数
	Failures   int           `json:"failures"`         // 报告失败次数
}

// StageTiming 单个阶段的用时。
type StageTiming struct {
	Stage      string `json:"stage"`
	Role       string `json:"role"`
	Status     string `json:"status"`
	Versions   int    `json:"versions"`
	WorkMin    int    `json:"work_minutes"`
	WaitHubMin int    `json:"wait_hub_minutes"`
	WaitVanMin int    `json:"wait_van_minutes"`
}

func minutesBetween(a, b string) int {
	ta, err1 := time.Parse(time.RFC3339, a)
	tb, err2 := time.Parse(time.RFC3339, b)
	if err1 != nil || err2 != nil || tb.Before(ta) {
		return 0
	}
	return int(tb.Sub(ta).Minutes())
}

// BuildRunReport 从任务、交付物与审核记录算出一期的用时拆分。
func BuildRunReport(ctx context.Context, q *sqlite.Queries, runID int64) (RunReport, error) {
	run, err := q.GetRun(ctx, runID)
	if err != nil {
		return RunReport{}, err
	}
	rep := RunReport{RunID: runID, Subject: run.Subject, Status: string(run.Status)}
	tasks, err := q.ListTasksByRun(ctx, runID)
	if err != nil {
		return rep, err
	}
	for _, t := range tasks {
		if t.Status == domain.TaskCancelled || t.Status == domain.TaskBlocked {
			continue
		}
		st := StageTiming{Stage: t.StageCode, Role: t.RoleCode, Status: string(t.Status)}
		if t.FailReason != "" {
			rep.Failures++
		}
		delivs, err := q.ListDeliverablesByTask(ctx, t.ID)
		if err != nil {
			return rep, err
		}
		gates, err := q.ListTaskGates(ctx, t.ID)
		if err != nil {
			return rep, err
		}
		relayed := make(map[int64]bool, len(gates))
		for _, g := range gates {
			relayed[g.ID] = g.RelayedByHub
		}
		sort.Slice(delivs, func(i, j int) bool { return delivs[i].ID < delivs[j].ID })
		var lastDispatch, lastOutput string
		for _, d := range delivs {
			switch d.Kind {
			case domain.KindDispatch:
				lastDispatch = d.CreatedAt
				if lastOutput != "" { // 上一版通过到本次再派工：中枢决定下一轮的时间
					st.WaitHubMin += minutesBetween(lastOutput, d.CreatedAt)
					lastOutput = ""
				}
			case domain.KindSupplement:
			default:
				st.Versions++
				if lastDispatch != "" {
					st.WorkMin += minutesBetween(lastDispatch, d.CreatedAt)
				}
				lastOutput = d.CreatedAt
				reviews, err := q.ListReviewsByDeliverable(ctx, d.ID)
				if err != nil {
					return rep, err
				}
				prev := d.CreatedAt
				for _, r := range reviews {
					wait := minutesBetween(prev, r.CreatedAt)
					if relayed[r.TaskGateID] {
						st.WaitVanMin += wait
					} else {
						st.WaitHubMin += wait
					}
					prev = r.CreatedAt
				}
			}
		}
		if st.Versions > 1 {
			rep.Reworks += st.Versions - 1
		}
		rep.WorkMin += st.WorkMin
		rep.WaitHubMin += st.WaitHubMin
		rep.WaitVanMin += st.WaitVanMin
		rep.Stages = append(rep.Stages, st)
	}
	return rep, nil
}
