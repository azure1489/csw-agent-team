package server

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
)

// GET /me/tasks?status=open|all
func (s *Server) handleMyTasks(c *gin.Context) {
	agent, _ := mwAgent(c)
	openOnly := c.DefaultQuery("status", "open") != "all"
	tasks, err := s.store.Q().ListMyTasks(c.Request.Context(), agent.ID, openOnly)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	subjects := map[int64]string{}
	out := make([]gin.H, 0, len(tasks))
	for _, t := range tasks {
		subj, ok := subjects[t.RunID]
		if !ok {
			if r, err := s.store.Q().GetRun(c.Request.Context(), t.RunID); err == nil {
				subj = r.Subject
				subjects[t.RunID] = subj
			}
		}
		out = append(out, gin.H{"task": toTaskDTO(t), "run_subject": subj})
	}
	c.JSON(http.StatusOK, gin.H{"tasks": out})
}

// GET /tasks/:id —— 作业手册 + 自检标准 + 验收标准 + 派工单 + 各版本产出 + 闸进度
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
	gateDTOs := make([]gateDTO, 0, len(gates))
	for _, g := range gates {
		gateDTOs = append(gateDTOs, gateDTO{GateOrder: g.GateOrder, ReviewerRole: g.ReviewerRole, RelayedByHub: g.RelayedByHub, Name: g.Name})
	}

	delivs, _ := q.ListDeliverablesByTask(ctx, id)
	var latestDispatch *deliverableDTO
	produced := make([]deliverableDTO, 0)
	for _, d := range delivs {
		ups, _ := q.ListUpstreams(ctx, d.ID)
		dto := toDeliverableDTO(d, ups)
		if d.IsDispatch {
			cp := dto
			latestDispatch = &cp // 按 id 递增，最后一条即最新派工单
		} else {
			produced = append(produced, dto)
		}
	}

	c.JSON(http.StatusOK, gin.H{
		"task":                toTaskDTO(t),
		"instructions":        t.Instructions,
		"self_check_criteria": t.SelfCheckCriteria,
		"acceptance":          t.Acceptance,
		"gates":               gateDTOs,
		"dispatch":            latestDispatch,
		"deliverables":        produced,
	})
}

type submitReq struct {
	DocType     string `json:"doc_type" binding:"required"`
	DownloadURL string `json:"download_url"`
	Filename    string `json:"filename"`
	Title       string `json:"title"`
	Summary     string `json:"summary"`
	MetaJSON    string `json:"meta_json"`
	SelfCheck   string `json:"self_check"`
	FileID      *int64 `json:"file_id"`
	Upstreams   []struct {
		Label      string `json:"label"`
		URL        string `json:"url"`
		UpstreamID *int64 `json:"upstream_id"`
	} `json:"upstreams"`
}

// POST /tasks/:id/deliverables —— agent 提交产出
func (s *Server) handleSubmit(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req submitReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	agent, _ := mwAgent(c)

	// 附了 file 但没给 download_url 时，用本服务下载链接。
	if req.FileID != nil && req.DownloadURL == "" {
		req.DownloadURL = s.fileURL(*req.FileID)
	}

	in := engine.SubmitInput{
		DocType: req.DocType, DownloadURL: req.DownloadURL, Filename: req.Filename,
		Title: req.Title, Summary: req.Summary, MetaJSON: req.MetaJSON, SelfCheck: req.SelfCheck, FileID: req.FileID,
	}
	for _, u := range req.Upstreams {
		in.Upstreams = append(in.Upstreams, engine.UpstreamInput{Label: u.Label, URL: u.URL, UpstreamID: u.UpstreamID})
	}

	d, err := s.eng.Submit(c.Request.Context(), agent, id, in)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ups, _ := s.store.Q().ListUpstreams(c.Request.Context(), d.ID)
	c.JSON(http.StatusCreated, gin.H{"deliverable": toDeliverableDTO(d, ups), "task_id": id})
}
