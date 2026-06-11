package server

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

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
	c.JSON(http.StatusOK, gin.H{
		"run": gin.H{
			"id": run.ID, "workflow_id": run.WorkflowID, "workflow_ver": run.WorkflowVer,
			"subject": run.Subject, "title": run.Title, "status": string(run.Status),
		},
		"tasks": taskDTOs,
	})
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
