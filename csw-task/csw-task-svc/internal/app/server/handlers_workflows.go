package server

import (
	"encoding/json"
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// GET /workflows —— 列已激活的工作流类型（供选择触发）
func (s *Server) handleListWorkflows(c *gin.Context) {
	wfs, err := s.store.Q().ListActiveWorkflows(c.Request.Context())
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(wfs))
	for _, w := range wfs {
		out = append(out, gin.H{
			"wf_key": w.WfKey, "name": w.Name, "version": w.Version,
			"hub_role": w.HubRoleCode, "dispatch_mode": string(w.DispatchMode),
		})
	}
	c.JSON(http.StatusOK, gin.H{"workflows": out})
}

type triggerReq struct {
	Subject string         `json:"subject" binding:"required"`
	Title   string         `json:"title"`
	Inputs  map[string]any `json:"inputs"`
}

// POST /workflows/:key/runs —— 触发实例
func (s *Server) handleTrigger(c *gin.Context) {
	key := c.Param("key")
	var req triggerReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	agent, role := mwAgent(c)

	var inputsJSON string
	if len(req.Inputs) > 0 {
		if b, err := json.Marshal(req.Inputs); err == nil {
			inputsJSON = string(b)
		}
	}

	res, err := s.eng.Trigger(c.Request.Context(), agent, role, key, req.Subject, req.Title, inputsJSON)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	tasks := make([]taskDTO, 0, len(res.Tasks))
	for _, t := range res.Tasks {
		tasks = append(tasks, toTaskDTO(t))
	}
	c.JSON(http.StatusCreated, gin.H{
		"run_id": res.Run.ID, "subject": res.Run.Subject, "status": string(res.Run.Status), "tasks": tasks,
	})
}
