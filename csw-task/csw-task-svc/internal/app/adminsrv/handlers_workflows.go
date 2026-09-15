package adminsrv

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/workflow"
)

// GET /admin/workflows?search=&status=
func (s *Server) handleListWorkflows(c *gin.Context) {
	items, err := s.store.Q().ListWorkflowsAll(c.Request.Context(), c.Query("search"), c.Query("status"))
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(items))
	for _, it := range items {
		h := workflowDTO(it.Workflow)
		h["stage_count"] = it.StageCount
		out = append(out, h)
	}
	c.JSON(http.StatusOK, gin.H{"workflows": out})
}

// GET /admin/workflows/:id —— 完整定义（basics + stages[含deps/override闸] + default_gates）
func (s *Server) handleGetWorkflow(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	q := s.store.Q()
	wf, err := q.GetWorkflow(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("workflow_not_found", "无此工作流"))
		return
	}
	stages, _ := q.ListStages(ctx, id)
	deps, _ := q.ListStageDeps(ctx, id)
	gates, _ := q.ListGates(ctx, id)

	codeOf := map[int64]string{}
	for _, st := range stages {
		codeOf[st.ID] = st.Code
	}
	depsByStage := map[int64][]string{}
	for _, d := range deps {
		depsByStage[d.StageID] = append(depsByStage[d.StageID], codeOf[d.DependsOnID])
	}
	overrideByStage := map[int64][]gin.H{}
	var defaultGates []gin.H
	gateToH := func(g domain.Gate) gin.H {
		return gin.H{"gate_order": g.GateOrder, "reviewer_role": g.ReviewerRole, "relayed_by_hub": g.RelayedByHub, "name": g.Name}
	}
	for _, g := range gates {
		if g.StageID == nil {
			defaultGates = append(defaultGates, gateToH(g))
		} else {
			overrideByStage[*g.StageID] = append(overrideByStage[*g.StageID], gateToH(g))
		}
	}

	stageDTOs := make([]gin.H, 0, len(stages))
	for _, st := range stages {
		stageDTOs = append(stageDTOs, stageDTO(st, depsByStage[st.ID], overrideByStage[st.ID]))
	}
	c.JSON(http.StatusOK, gin.H{
		"workflow":      workflowDTO(wf),
		"stages":        stageDTOs,
		"default_gates": defaultGates,
	})
}

type createWorkflowReq struct {
	WfKey        string `json:"wf_key" binding:"required"`
	Name         string `json:"name" binding:"required"`
	HubRole      string `json:"hub_role" binding:"required"`
	DispatchMode string `json:"dispatch_mode"`
	TriggerRoles string `json:"trigger_roles"`
}

// POST /admin/workflows —— 新建草稿
func (s *Server) handleCreateWorkflow(c *gin.Context) {
	var req createWorkflowReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if req.DispatchMode == "" {
		req.DispatchMode = "manual"
	}
	id, err := s.store.Q().CreateWorkflowDraft(c.Request.Context(), req.WfKey, req.Name, req.HubRole, req.DispatchMode, req.TriggerRoles)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "workflow_create", req.WfKey, "")
	c.JSON(http.StatusCreated, gin.H{"id": id})
}

type putStage struct {
	Code              string    `json:"code" binding:"required"`
	Name              string    `json:"name" binding:"required"`
	RoleCode          string    `json:"role_code" binding:"required"`
	OutputType        string    `json:"output_type"`
	Instructions      string    `json:"instructions"`
	SelfCheckCriteria string    `json:"self_check_criteria"`
	Acceptance        string    `json:"acceptance"`
	DispatchMode      string    `json:"dispatch_mode"` // 空=继承工作流 / manual / auto
	ActionClass       string    `json:"action_class"`  // 空=read / platform_write:<scope>
	IsMerge           bool      `json:"is_merge"`
	PerItem           bool      `json:"per_item"`
	SLAMinutes        int       `json:"sla_minutes"`
	Deps              []string  `json:"deps"`
	Gates             []putGate `json:"gates"`
}

type putGate struct {
	ReviewerRole string `json:"reviewer_role" binding:"required"`
	Name         string `json:"name"`
	RelayedByHub bool   `json:"relayed_by_hub"`
}

type putWorkflowReq struct {
	Name               string     `json:"name" binding:"required"`
	HubRole            string     `json:"hub_role" binding:"required"`
	DispatchMode       string     `json:"dispatch_mode"`
	TriggerRoles       string     `json:"trigger_roles"`
	CommonInstructions string     `json:"common_instructions"`
	CommonAcceptance   string     `json:"common_acceptance"`
	Stages             []putStage `json:"stages"`
	DefaultGates       []putGate  `json:"default_gates"`
}

// PUT /admin/workflows/:id —— 整份提交（仅 draft；事务内 delete+reinsert 子表）
func (s *Server) handlePutWorkflow(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req putWorkflowReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if req.DispatchMode == "" {
		req.DispatchMode = "manual"
	}

	// 校验 deps 引用的 code 均存在、不自依赖。
	codeSet := map[string]bool{}
	for _, st := range req.Stages {
		if codeSet[st.Code] {
			s.renderErr(c, domain.BadRequest("dup_code", "阶段 code 重复："+st.Code))
			return
		}
		codeSet[st.Code] = true
	}
	for _, st := range req.Stages {
		if !domain.ValidStageDispatchMode(st.DispatchMode) {
			s.renderErr(c, domain.BadRequest("bad_dispatch_mode", st.Code+" 的派工模式须为空（继承）、manual 或 auto"))
			return
		}
		if st.ActionClass != "" && !domain.ValidActionClass(st.ActionClass) {
			s.renderErr(c, domain.BadRequest("bad_action_class", st.Code+" 的动作类别须为 read 或 platform_write:<wx_draft|wx_publish|xhs_draft|xhs_publish>"))
			return
		}
		if st.SLAMinutes < 0 {
			s.renderErr(c, domain.BadRequest("bad_sla", st.Code+" 的时限不能为负数"))
			return
		}
		for _, d := range st.Deps {
			if d == st.Code {
				s.renderErr(c, domain.BadRequest("self_dep", st.Code+" 不能依赖自己"))
				return
			}
			if !codeSet[d] {
				s.renderErr(c, domain.BadRequest("unknown_dep", "依赖的阶段不存在："+d))
				return
			}
		}
	}

	var valRep *workflow.Report // active 就地编辑时回填校验报告（不阻断保存，仅回显）
	err = s.store.Tx(c.Request.Context(), func(q *sqlite.Queries) error {
		wf, err := q.GetWorkflow(c.Request.Context(), id)
		if err != nil {
			return domain.NotFound("workflow_not_found", "无此工作流")
		}
		// draft、active 均可整份编辑（active 为「就地热改」，仅影响此后新触发的实例——
		// 运行实例在 Trigger 时已快照定义，不受改动影响）。归档版本只读。
		if wf.Status == domain.WfArchived {
			return domain.Conflict("archived_readonly", "归档版本只读；如需复用请「复制为新版本」生成草稿再改")
		}
		if err := q.UpdateWorkflowBasics(c.Request.Context(), id, req.Name, req.HubRole, req.DispatchMode,
			req.TriggerRoles, req.CommonInstructions, req.CommonAcceptance); err != nil {
			return err
		}
		if err := q.DeleteWorkflowChildren(c.Request.Context(), id); err != nil {
			return err
		}
		// 插入阶段，记 code→新id。
		stageID := map[string]int64{}
		for i, st := range req.Stages {
			sid, err := q.InsertStage(c.Request.Context(), domain.Stage{
				WorkflowID: id, Seq: i + 1, Code: st.Code, Name: st.Name, RoleCode: st.RoleCode,
				OutputType: st.OutputType, Instructions: st.Instructions,
				SelfCheckCriteria: st.SelfCheckCriteria, Acceptance: st.Acceptance, IsMerge: st.IsMerge,
				DispatchMode: st.DispatchMode, ActionClass: st.ActionClass, SLAMinutes: st.SLAMinutes, PerItem: st.PerItem,
			})
			if err != nil {
				return err
			}
			stageID[st.Code] = sid
		}
		// 依赖边 + 覆盖闸。
		for _, st := range req.Stages {
			for _, d := range st.Deps {
				if err := q.InsertStageDep(c.Request.Context(), id, stageID[st.Code], stageID[d]); err != nil {
					return err
				}
			}
			for gi, g := range st.Gates {
				sid := stageID[st.Code]
				if err := q.InsertGate(c.Request.Context(), domain.Gate{
					WorkflowID: id, StageID: &sid, GateOrder: gi + 1,
					ReviewerRole: g.ReviewerRole, RelayedByHub: g.RelayedByHub, Name: g.Name,
				}); err != nil {
					return err
				}
			}
		}
		// 默认闸。
		for gi, g := range req.DefaultGates {
			if err := q.InsertGate(c.Request.Context(), domain.Gate{
				WorkflowID: id, StageID: nil, GateOrder: gi + 1,
				ReviewerRole: g.ReviewerRole, RelayedByHub: g.RelayedByHub, Name: g.Name,
			}); err != nil {
				return err
			}
		}
		// active 就地编辑：保存后跑一次校验回显（事务内可见刚写入的子表），
		// 不阻断保存——若未过则随响应提示「触发前请修复」，由人决定。
		if wf.Status == domain.WfActive {
			updated, err := q.GetWorkflow(c.Request.Context(), id)
			if err != nil {
				return err
			}
			rep, err := workflow.Validate(c.Request.Context(), q, updated)
			if err != nil {
				return err
			}
			valRep = &rep
		}
		return nil
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "workflow_update", c.Param("id"), "")
	resp := gin.H{"ok": true}
	if valRep != nil {
		resp["validation"] = gin.H{"all_ok": valRep.AllOK(), "checks": reportToJSON(*valRep)}
	}
	c.JSON(http.StatusOK, resp)
}

func reportToJSON(rep workflow.Report) []gin.H {
	out := make([]gin.H, 0, len(rep.Checks))
	for _, ch := range rep.Checks {
		out = append(out, gin.H{"name": ch.Name, "ok": ch.OK, "detail": ch.Detail})
	}
	return out
}

// POST /admin/workflows/:id/validate
func (s *Server) handleValidateWorkflow(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	wf, err := s.store.Q().GetWorkflow(c.Request.Context(), id)
	if err != nil {
		s.renderErr(c, domain.NotFound("workflow_not_found", "无此工作流"))
		return
	}
	rep, err := workflow.Validate(c.Request.Context(), s.store.Q(), wf)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"all_ok": rep.AllOK(), "checks": reportToJSON(rep)})
}

// POST /admin/workflows/:id/activate —— 校验通过则激活并归档同 key 旧 active
func (s *Server) handleActivateWorkflow(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	wf, err := s.store.Q().GetWorkflow(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("workflow_not_found", "无此工作流"))
		return
	}
	rep, err := workflow.Validate(ctx, s.store.Q(), wf)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	if !rep.AllOK() {
		c.JSON(http.StatusConflict, gin.H{"code": "validation_failed", "message": "校验未通过", "checks": reportToJSON(rep)})
		return
	}
	err = s.store.Tx(ctx, func(q *sqlite.Queries) error {
		if err := q.ArchiveOtherActive(ctx, wf.WfKey, id); err != nil {
			return err
		}
		return q.SetWorkflowStatus(ctx, id, domain.WfActive)
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "workflow_activate", wf.WfKey, "")
	c.JSON(http.StatusOK, gin.H{"ok": true, "checks": reportToJSON(rep)})
}

// POST /admin/workflows/:id/archive
func (s *Server) handleArchiveWorkflow(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	if err := s.store.Q().SetWorkflowStatus(c.Request.Context(), id, domain.WfArchived); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "workflow_archive", c.Param("id"), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// POST /admin/workflows/:id/clone —— 复制为同 key 新 draft 版本
func (s *Server) handleCloneWorkflow(c *gin.Context) {
	srcID, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	var newID int64
	err = s.store.Tx(ctx, func(q *sqlite.Queries) error {
		src, err := q.GetWorkflow(ctx, srcID)
		if err != nil {
			return domain.NotFound("workflow_not_found", "无此工作流")
		}
		newID, err = q.CreateWorkflowDraft(ctx, src.WfKey, src.Name, src.HubRoleCode, string(src.DispatchMode), src.TriggerRoles)
		if err != nil {
			return err
		}
		if err := q.UpdateWorkflowBasics(ctx, newID, src.Name, src.HubRoleCode, string(src.DispatchMode),
			src.TriggerRoles, src.CommonInstructions, src.CommonAcceptance); err != nil {
			return err
		}
		stages, err := q.ListStages(ctx, srcID)
		if err != nil {
			return err
		}
		deps, err := q.ListStageDeps(ctx, srcID)
		if err != nil {
			return err
		}
		gates, err := q.ListGates(ctx, srcID)
		if err != nil {
			return err
		}
		oldToNew := map[int64]int64{}
		for _, st := range stages {
			st.WorkflowID = newID
			sid, err := q.InsertStage(ctx, st)
			if err != nil {
				return err
			}
			oldToNew[st.ID] = sid
		}
		for _, d := range deps {
			if err := q.InsertStageDep(ctx, newID, oldToNew[d.StageID], oldToNew[d.DependsOnID]); err != nil {
				return err
			}
		}
		for _, g := range gates {
			g.WorkflowID = newID
			if g.StageID != nil {
				ns := oldToNew[*g.StageID]
				g.StageID = &ns
			}
			if err := q.InsertGate(ctx, g); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "workflow_clone", c.Param("id"), "")
	c.JSON(http.StatusCreated, gin.H{"id": newID})
}
