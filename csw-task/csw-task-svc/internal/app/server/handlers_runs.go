package server

import (
	"net/http"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// utcNow 与库内时间戳同格式（UTC 秒级 ISO8601），可直接字符串比较。
func utcNow() string { return time.Now().UTC().Format("2006-01-02T15:04:05Z") }

// GET /runs/:id —— 整条流程进度（run + 任务图）
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
	tasks, err := s.store.Q().ListTasksByRun(ctx, id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	taskDTOs := make([]taskDTO, 0, len(tasks))
	for _, t := range tasks {
		taskDTOs = append(taskDTOs, toTaskDTO(t))
	}
	auths, err := s.store.Q().ListActiveAuthorizations(ctx, id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	scopes := make([]string, 0, len(auths))
	for _, a := range auths {
		scopes = append(scopes, string(a.Scope))
	}
	c.JSON(http.StatusOK, gin.H{
		"run": gin.H{
			"id": run.ID, "workflow_id": run.WorkflowID, "workflow_ver": run.WorkflowVer,
			"subject": run.Subject, "title": run.Title, "status": string(run.Status),
			"authorizations": scopes,
		},
		"tasks": taskDTOs,
	})
}

// GET /runs/:id/authorizations —— 授权记录（含已撤销、已过期）
func (s *Server) handleListAuthorizations(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	if _, err := s.store.Q().GetRun(ctx, id); err != nil {
		s.renderErr(c, domain.NotFound("run_not_found", "无此实例"))
		return
	}
	auths, err := s.store.Q().ListAuthorizations(ctx, id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	now := utcNow()
	out := make([]authorizationDTO, 0, len(auths))
	for _, a := range auths {
		out = append(out, toAuthorizationDTO(a, now))
	}
	c.JSON(http.StatusOK, gin.H{"authorizations": out})
}

type grantReq struct {
	Scope       string `json:"scope" binding:"required"`
	SourceQuote string `json:"source_quote"`
	ExpiresAt   string `json:"expires_at"` // RFC3339，可省略（不过期）
}

// POST /runs/:id/authorizations —— 中枢录入授权（须带 Van 原话）；随后续派等授权的自动派工任务
func (s *Server) handleGrantAuthorization(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req grantReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	var expires *string
	if strings.TrimSpace(req.ExpiresAt) != "" {
		t, err := time.Parse(time.RFC3339, strings.TrimSpace(req.ExpiresAt))
		if err != nil {
			s.renderErr(c, domain.BadRequest("bad_expires_at", "expires_at 须为 RFC3339 时间，如 2026-09-15T12:00:00+08:00"))
			return
		}
		if !t.After(time.Now()) {
			s.renderErr(c, domain.BadRequest("bad_expires_at", "expires_at 已是过去时间"))
			return
		}
		v := t.UTC().Format("2006-01-02T15:04:05Z")
		expires = &v
	}
	agent, role := mwAgent(c)
	res, err := s.eng.Grant(c.Request.Context(), agent, role, id, domain.AuthScope(req.Scope), req.SourceQuote, expires)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	dispatched := res.Dispatched
	if dispatched == nil {
		dispatched = []int64{}
	}
	code := http.StatusCreated
	if res.Existing {
		code = http.StatusOK
	}
	c.JSON(code, gin.H{
		"authorization":       toAuthorizationDTO(res.Authorization, utcNow()),
		"existing":            res.Existing,
		"dispatched_task_ids": dispatched,
	})
}

// DELETE /runs/:id/authorizations/:scope —— 中枢撤销授权
func (s *Server) handleRevokeAuthorization(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	agent, role := mwAgent(c)
	n, err := s.eng.Revoke(c.Request.Context(), agent, role, id, domain.AuthScope(c.Param("scope")))
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"ok": true, "revoked": n})
}

// GET /runs/:id/timeline —— 事件流水
func (s *Server) handleTimeline(c *gin.Context) {
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
