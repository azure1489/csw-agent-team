package adminsrv

import (
	"net/http"
	"time"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// GET /admin/overview
func (s *Server) handleOverview(c *gin.Context) {
	ctx := c.Request.Context()
	q := s.store.Q()
	today := time.Now().UTC().Format("2006-01-02")

	activeWf, _ := q.CountWorkflowsByStatus(ctx, "active")
	activeRuns, _ := q.CountRunsByStatus(ctx, "active")
	doneToday, _ := q.CountRunsDoneOn(ctx, today)

	recentRuns, _ := q.ListRuns(ctx, "", "", "", 5)
	runs := make([]gin.H, 0, len(recentRuns))
	for _, r := range recentRuns {
		runs = append(runs, runDTO(r))
	}
	audit, _ := q.ListAudit(ctx, nil, "", "", 8)
	au := make([]gin.H, 0, len(audit))
	for _, a := range audit {
		au = append(au, auditDTO(a))
	}

	c.JSON(http.StatusOK, gin.H{
		"counts": gin.H{
			"active_workflows": activeWf, "active_runs": activeRuns,
			"done_today": doneToday, "pending_alerts": 0,
		},
		"recent_runs":  runs,
		"recent_audit": au,
	})
}

// GET /admin/runs?workflow=&status=&date=
func (s *Server) handleListRuns(c *gin.Context) {
	runs, err := s.store.Q().ListRuns(c.Request.Context(), c.Query("workflow"), c.Query("status"), c.Query("date"), 0)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(runs))
	for _, r := range runs {
		out = append(out, runDTO(r))
	}
	c.JSON(http.StatusOK, gin.H{"runs": out})
}

// GET /admin/runs/:id
func (s *Server) handleRunDetail(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	run, err := s.store.Q().GetRun(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("run_not_found", "无此实例"))
		return
	}
	tasks, _ := s.store.Q().ListTasksByRun(ctx, id)
	ts := make([]gin.H, 0, len(tasks))
	for _, t := range tasks {
		ts = append(ts, taskBriefDTO(t))
	}
	auths, _ := s.store.Q().ListAuthorizations(ctx, id)
	now := time.Now().UTC().Format("2006-01-02T15:04:05Z")
	as := make([]gin.H, 0, len(auths))
	for _, a := range auths {
		st := "active"
		switch {
		case a.RevokedAt != "":
			st = "revoked"
		case a.ExpiresAt != "" && a.ExpiresAt <= now:
			st = "expired"
		}
		as = append(as, gin.H{
			"id": a.ID, "scope": string(a.Scope), "source_quote": a.SourceQuote, "granted_at": a.GrantedAt,
			"expires_at": a.ExpiresAt, "revoked_at": a.RevokedAt, "status": st,
		})
	}
	c.JSON(http.StatusOK, gin.H{
		"run": gin.H{
			"id": run.ID, "workflow_id": run.WorkflowID, "workflow_ver": run.WorkflowVer,
			"subject": run.Subject, "title": run.Title, "status": string(run.Status),
		},
		"tasks":          ts,
		"authorizations": as,
	})
}

// GET /admin/runs/:id/timeline
func (s *Server) handleRunTimeline(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	events, err := s.store.Q().ListEventsByRun(c.Request.Context(), id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(events))
	for _, e := range events {
		out = append(out, gin.H{
			"id": e.ID, "type": e.Type, "task_id": e.TaskID, "deliverable_id": e.DeliverableID,
			"actor_id": e.ActorID, "detail": e.DetailJSON, "created_at": e.CreatedAt,
		})
	}
	c.JSON(http.StatusOK, gin.H{"events": out})
}

// GET /admin/tasks/:id —— 任务全貌（含各版本产出 + 审核记录）
func (s *Server) handleTaskDetail(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	q := s.store.Q()
	t, err := q.GetTask(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("task_not_found", "无此任务"))
		return
	}
	gates, _ := q.ListTaskGates(ctx, id)
	gs := make([]gin.H, 0, len(gates))
	for _, g := range gates {
		gs = append(gs, gin.H{"gate_order": g.GateOrder, "reviewer_role": g.ReviewerRole, "relayed_by_hub": g.RelayedByHub, "name": g.Name})
	}
	delivs, _ := q.ListDeliverablesByTask(ctx, id)
	ds := make([]gin.H, 0, len(delivs))
	for _, d := range delivs {
		ups, _ := q.ListUpstreams(ctx, d.ID)
		reviews, _ := q.ListReviewsByDeliverable(ctx, d.ID)
		ds = append(ds, deliverableDetailDTO(d, ups, reviews))
	}
	c.JSON(http.StatusOK, gin.H{
		"task":                taskBriefDTO(t),
		"instructions":        t.Instructions,
		"self_check_criteria": t.SelfCheckCriteria,
		"acceptance":          t.Acceptance,
		"gates":               gs,
		"deliverables":        ds,
	})
}

// GET /admin/deliverables/:id
func (s *Server) handleDeliverableDetail(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	q := s.store.Q()
	d, err := q.GetDeliverable(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("deliverable_not_found", "无此交付物"))
		return
	}
	ups, _ := q.ListUpstreams(ctx, id)
	reviews, _ := q.ListReviewsByDeliverable(ctx, id)
	c.JSON(http.StatusOK, gin.H{"deliverable": deliverableDetailDTO(d, ups, reviews)})
}
