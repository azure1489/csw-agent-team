package engine

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// idleAfter 已接单但超过该时长没有心跳或产物，进度上标「无活动」。
const idleAfter = 30 * time.Minute

var scopeLabel = map[domain.AuthScope]string{
	domain.ScopeLocalDrill: "本地演练", domain.ScopeWxDraft: "公众号草稿", domain.ScopeWxPublish: "公众号发布",
	domain.ScopeXhsDraft: "小红书草稿", domain.ScopeXhsPublish: "小红书发布",
}

// Blocker 真正卡住任务的原因。
type Blocker struct {
	Kind string `json:"kind"` // upstream | authorization | dispatch | ack | idle | gate | rework | failed | supplement
	Text string `json:"text"`
}

// ProgressDeliverable 任务最近一份产出。
type ProgressDeliverable struct {
	Kind        string `json:"kind"`
	Status      string `json:"status"`
	DownloadURL string `json:"download_url,omitempty"`
	ID          int64  `json:"id"`
	Version     int    `json:"version"`
}

// ProgressTask 单个任务的进度。
type ProgressTask struct {
	StageCode     string               `json:"stage_code"`
	StageName     string               `json:"stage_name"`
	RoleCode      string               `json:"role_code"`
	Status        string               `json:"status"`
	ItemKey       string               `json:"item_key,omitempty"`
	Next          string               `json:"next,omitempty"`
	DueAt         string               `json:"due_at,omitempty"`
	Blocker       *Blocker             `json:"blocker,omitempty"`
	Latest        *ProgressDeliverable `json:"latest,omitempty"`
	AssigneeID    *int64               `json:"assignee_id"`
	ID            int64                `json:"task_id"`
	Seq           int                  `json:"seq"`
	CurVersion    int                  `json:"cur_version"`
	Overdue       bool                 `json:"overdue"`
	Idle          bool                 `json:"idle"`
	ReworkPending bool                 `json:"rework_pending"`
}

// Progress run 进度投影。Digest 只含状态类字段，用于进度卡去重。
type Progress struct {
	Status         string         `json:"status"`
	Subject        string         `json:"subject"`
	Digest         string         `json:"digest"`
	Authorizations []string       `json:"authorizations"`
	CurrentNodes   []string       `json:"current_nodes"`
	Tasks          []ProgressTask `json:"tasks"`
	RunID          int64          `json:"run_id"`
	Done           int            `json:"done"`
	Total          int            `json:"total"`
	TargetCount    int            `json:"target_count"`
	Gap            int            `json:"gap"`
}

// Progress 进度投影（只读）。
func (e *Engine) Progress(ctx context.Context, runID int64) (Progress, error) {
	return BuildProgress(ctx, e.store.Q(), runID, time.Now())
}

// BuildProgress 计算 run 的进度投影：每个任务的真实阻塞项、下一步、最近产出与逾期；run 级授权与当前节点。
func BuildProgress(ctx context.Context, q *sqlite.Queries, runID int64, now time.Time) (Progress, error) {
	var p Progress
	run, err := q.GetRun(ctx, runID)
	if isNoRows(err) {
		return p, domain.NotFound("run_not_found", "无此实例")
	}
	if err != nil {
		return p, err
	}
	wf, err := q.GetWorkflow(ctx, run.WorkflowID)
	if err != nil {
		return p, err
	}
	tasks, err := q.ListTasksByRun(ctx, runID)
	if err != nil {
		return p, err
	}
	deps, err := q.ListTaskDepsByRun(ctx, runID)
	if err != nil {
		return p, err
	}
	auths, err := q.ListActiveAuthorizations(ctx, runID)
	if err != nil {
		return p, err
	}
	p.RunID, p.Status, p.Subject, p.Total = run.ID, string(run.Status), run.Subject, len(tasks)
	p.Authorizations, p.CurrentNodes = []string{}, []string{}
	for _, a := range auths {
		p.Authorizations = append(p.Authorizations, string(a.Scope))
	}
	sort.Strings(p.Authorizations)

	byID := map[int64]domain.Task{}
	for _, t := range tasks {
		byID[t.ID] = t
	}
	upstream := map[int64][]int64{}
	for _, d := range deps {
		upstream[d.TaskID] = append(upstream[d.TaskID], d.DependsOnID)
	}
	roleNames := map[string]string{}
	roleName := func(code string) string {
		if n, ok := roleNames[code]; ok {
			return n
		}
		n := code
		if r, err := q.GetRole(ctx, code); err == nil && r.Name != "" {
			n = r.Name
		}
		roleNames[code] = n
		return n
	}
	nowS := now.UTC().Format("2006-01-02T15:04:05Z")

	for _, t := range tasks {
		pt := ProgressTask{
			ID: t.ID, Seq: t.Seq, StageCode: t.StageCode, StageName: t.StageName, RoleCode: t.RoleCode,
			Status: string(t.Status), ItemKey: t.ItemKey, AssigneeID: t.AssigneeID, CurVersion: t.CurVersion,
			DueAt: t.DueAt, ReworkPending: t.ReworkPending,
		}
		active := t.Status == domain.TaskDispatched || t.Status == domain.TaskInProgress || t.Status == domain.TaskReturned
		pt.Overdue = active && t.DueAt != "" && t.DueAt <= nowS

		ds, err := q.ListDeliverablesByTask(ctx, t.ID)
		if err != nil {
			return p, err
		}
		var latest *domain.Deliverable
		for i := range ds {
			if ds[i].Kind == domain.KindOutput || ds[i].Kind == domain.KindEdit {
				latest = &ds[i]
			}
		}
		if latest != nil {
			pt.Latest = &ProgressDeliverable{ID: latest.ID, Version: latest.Version, Kind: string(latest.Kind),
				Status: string(latest.Status), DownloadURL: latest.DownloadURL}
		}

		who := roleName(t.RoleCode)
		switch t.Status {
		case domain.TaskBlocked:
			var waiting []string
			for _, id := range upstream[t.ID] {
				if u := byID[id]; u.Status != domain.TaskPassed {
					waiting = append(waiting, u.StageName)
				}
			}
			pt.Blocker = &Blocker{Kind: "upstream", Text: "待上游：" + strings.Join(waiting, "、")}
			pt.Next = "等上游通过"
		case domain.TaskReady:
			ok, scope, err := authorized(ctx, q, t)
			if err != nil {
				return p, err
			}
			switch {
			case !ok:
				label := scopeLabel[scope]
				if label == "" {
					label = string(scope)
				}
				pt.Blocker = &Blocker{Kind: "authorization", Text: "待授权：" + label}
				pt.Next = "主编录入 Van 授权原话"
			case effectiveDispatchMode(t, wf) == domain.DispatchManual && !t.IsMerge:
				pt.Blocker = &Blocker{Kind: "dispatch", Text: "待派工（手动派工阶段）"}
				pt.Next = "主编派工"
			default:
				pt.Blocker = &Blocker{Kind: "dispatch", Text: "待派工"}
				pt.Next = "主编派工"
			}
		case domain.TaskDispatched:
			pt.Blocker = &Blocker{Kind: "ack", Text: "待" + who + "接单"}
			pt.Next = who + "接单"
		case domain.TaskInProgress:
			if last, err := time.Parse(time.RFC3339, t.LastActivityAt); err == nil && now.Sub(last) > idleAfter {
				pt.Idle = true
				pt.Blocker = &Blocker{Kind: "idle", Text: fmt.Sprintf("已接单，%d 分钟无活动", int(now.Sub(last).Minutes()))}
			}
			pt.Next = who + "提交"
		case domain.TaskReview:
			if latest != nil {
				if g, err := q.TaskGateByOrder(ctx, t.ID, latest.CurGate+1); err == nil {
					reviewer := roleName(g.ReviewerRole)
					pt.Blocker = &Blocker{Kind: "gate", Text: "待闸：" + g.Name + "（" + reviewer + "）"}
					pt.Next = reviewer + "审核"
				}
			}
		case domain.TaskReturned:
			text := fmt.Sprintf("需返工 v%d", t.CurVersion)
			if latest != nil {
				if rs, err := q.ListReviewsByDeliverable(ctx, latest.ID); err == nil && len(rs) > 0 {
					if r := rs[len(rs)-1]; r.Verdict == domain.VerdictReject {
						text += "：" + r.ReturnDirection + "@" + r.ReturnLocation
					}
				}
			}
			pt.Blocker = &Blocker{Kind: "rework", Text: text}
			pt.Next = who + "按意见重交"
		case domain.TaskFailed:
			pt.Blocker = &Blocker{Kind: "failed", Text: "失败：" + t.FailReason}
			pt.Next = "主编重开或取消"
		}
		if pt.Blocker == nil && t.ReworkPending && !domain.IsTerminal(t.Status) {
			pt.Blocker = &Blocker{Kind: "supplement", Text: "上游补件已到，待返工"}
		}
		if t.Status == domain.TaskPassed {
			p.Done++
		}
		if t.Status != domain.TaskBlocked && !domain.IsTerminal(t.Status) {
			p.CurrentNodes = append(p.CurrentNodes, t.StageName)
		}
		p.Tasks = append(p.Tasks, pt)
	}
	p.Digest = progressDigest(p)
	return p, nil
}

// progressDigest 只取状态类字段（不含时间），同样的进度只发一次进度卡。
func progressDigest(p Progress) string {
	type row struct {
		Blocker string
		Status  string
		ID      int64
		Ver     int
		Rework  bool
	}
	rows := make([]row, 0, len(p.Tasks))
	for _, t := range p.Tasks {
		r := row{ID: t.ID, Status: t.Status, Ver: t.CurVersion, Rework: t.ReworkPending}
		if t.Blocker != nil {
			r.Blocker = t.Blocker.Kind
		}
		rows = append(rows, r)
	}
	b, _ := json.Marshal(struct {
		Rows   []row
		Status string
		Auths  []string
	}{rows, p.Status, p.Authorizations})
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:8])
}
