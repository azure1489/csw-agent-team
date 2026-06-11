package server

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
)

// GET /me/inbox —— 中枢聚合：派工队列 + 审核队列 + 我的（合流）任务
func (s *Server) handleInbox(c *gin.Context) {
	agent, role := mwAgent(c)
	ctx := c.Request.Context()
	q := s.store.Q()

	// run → workflow 缓存（判定我是否该 run 的中枢）。
	wfCache := map[int64]domain.Workflow{}
	wfOf := func(runID int64) (domain.Workflow, bool) {
		if w, ok := wfCache[runID]; ok {
			return w, true
		}
		run, err := q.GetRun(ctx, runID)
		if err != nil {
			return domain.Workflow{}, false
		}
		w, err := q.GetWorkflow(ctx, run.WorkflowID)
		if err != nil {
			return domain.Workflow{}, false
		}
		wfCache[runID] = w
		return w, true
	}

	// 派工队列：ready 任务，且我是该 run 的中枢。
	ready, err := q.ListReadyTasks(ctx)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	dispatchQueue := make([]taskDTO, 0)
	for _, t := range ready {
		if w, ok := wfOf(t.RunID); ok && w.HubRoleCode == role.Code {
			dispatchQueue = append(dispatchQueue, toTaskDTO(t))
		}
	}

	// 审核队列：待审交付物的下一道闸由我审（普通闸=我的角色；人工闸=我是中枢）。
	pending, err := q.ListPendingReviews(ctx)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	reviewQueue := make([]gin.H, 0)
	for _, p := range pending {
		mine := false
		if p.RelayedByHub {
			if w, ok := wfOf(p.RunID); ok && w.HubRoleCode == role.Code {
				mine = true
			}
		} else if p.ReviewerRole == role.Code {
			mine = true
		}
		if mine {
			reviewQueue = append(reviewQueue, gin.H{
				"deliverable_id": p.DeliverableID, "task_id": p.TaskID, "run_id": p.RunID,
				"stage_name": p.StageName, "version": p.Version, "next_gate": p.NextGate,
				"reviewer_role": p.ReviewerRole, "relayed_by_hub": p.RelayedByHub, "gate_name": p.GateName,
			})
		}
	}

	// 我的任务（含合流自产）：assignee=我且未完成。
	mine, err := q.ListMyTasks(ctx, agent.ID, true)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	myTasks := make([]taskDTO, 0, len(mine))
	for _, t := range mine {
		myTasks = append(myTasks, toTaskDTO(t))
	}

	c.JSON(http.StatusOK, gin.H{
		"dispatch_queue": dispatchQueue,
		"review_queue":   reviewQueue,
		"my_tasks":       myTasks,
	})
}

type dispatchReq struct {
	EditorNote string `json:"editor_note"`
	Upstreams  []struct {
		Label      string `json:"label"`
		URL        string `json:"url" binding:"required"`
		UpstreamID *int64 `json:"upstream_id"`
	} `json:"upstreams"`
}

// POST /tasks/:id/dispatch —— 中枢派工
func (s *Server) handleDispatch(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req dispatchReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	agent, role := mwAgent(c)

	var ups []engine.UpstreamInput
	for _, u := range req.Upstreams {
		ups = append(ups, engine.UpstreamInput{Label: u.Label, URL: u.URL, UpstreamID: u.UpstreamID})
	}

	res, err := s.eng.Dispatch(c.Request.Context(), agent, role, id, req.EditorNote, ups)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"dispatch": toDeliverableDTO(res.Dispatch, res.Upstreams)})
}

type reviewReq struct {
	Verdict         string `json:"verdict" binding:"required"`
	Comment         string `json:"comment"`
	ReturnDirection string `json:"return_direction"`
	ReturnLocation  string `json:"return_location"`
}

// POST /deliverables/:id/reviews —— 审核（按闸推进；人工闸代录）
func (s *Server) handleReview(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req reviewReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	agent, role := mwAgent(c)

	res, err := s.eng.Review(c.Request.Context(), agent, role, id, engine.ReviewInput{
		Verdict:         domain.Verdict(req.Verdict),
		Comment:         req.Comment,
		ReturnDirection: req.ReturnDirection,
		ReturnLocation:  req.ReturnLocation,
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ups, _ := s.store.Q().ListUpstreams(c.Request.Context(), res.Deliverable.ID)
	c.JSON(http.StatusOK, gin.H{
		"deliverable": toDeliverableDTO(res.Deliverable, ups),
		"task_status": string(res.TaskStatus),
		"gate_passed": res.GatePassed,
		"final":       res.Final,
	})
}
