package adminsrv

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── 角色 ──

// GET /admin/roles
func (s *Server) handleListRoles(c *gin.Context) {
	roles, err := s.store.Q().ListRolesWithCounts(c.Request.Context())
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(roles))
	for _, r := range roles {
		out = append(out, gin.H{
			"code": r.Code, "name": r.Name, "is_management": r.IsManagement,
			"is_human": r.IsHuman, "member_count": r.MemberCount,
		})
	}
	c.JSON(http.StatusOK, gin.H{"roles": out})
}

type roleReq struct {
	Code         string `json:"code"`
	Name         string `json:"name" binding:"required"`
	IsManagement bool   `json:"is_management"`
	IsHuman      bool   `json:"is_human"`
}

// POST /admin/roles
func (s *Server) handleCreateRole(c *gin.Context) {
	var req roleReq
	if err := c.ShouldBindJSON(&req); err != nil || req.Code == "" {
		s.renderErr(c, domain.BadRequest("bad_body", "code/name 必填"))
		return
	}
	if err := s.store.Q().CreateRole(c.Request.Context(), req.Code, req.Name, req.IsManagement, req.IsHuman); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "role_create", req.Code, "")
	c.JSON(http.StatusCreated, gin.H{"ok": true})
}

// PATCH /admin/roles/:code
func (s *Server) handleUpdateRole(c *gin.Context) {
	code := c.Param("code")
	var req roleReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if err := s.store.Q().UpdateRole(c.Request.Context(), code, req.Name, req.IsManagement, req.IsHuman); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "role_update", code, "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// ── 成员 + token ──

// GET /admin/agents —— 成员 + 各自 token
func (s *Server) handleListAgents(c *gin.Context) {
	ctx := c.Request.Context()
	agents, err := s.store.Q().ListAgents(ctx)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(agents))
	for _, a := range agents {
		tokens, _ := s.store.Q().ListTokensByAgent(ctx, a.ID)
		tks := make([]gin.H, 0, len(tokens))
		for _, t := range tokens {
			tks = append(tks, gin.H{
				"id": t.ID, "label": t.Label, "status": t.Status,
				"last_used_at": t.LastUsedAt, "expires_at": t.ExpiresAt, "created_at": t.CreatedAt,
			})
		}
		out = append(out, gin.H{
			"id": a.ID, "name": a.Name, "role_code": a.RoleCode, "active": a.Active, "tokens": tks,
		})
	}
	c.JSON(http.StatusOK, gin.H{"agents": out})
}

type createAgentReq struct {
	RoleCode string `json:"role_code" binding:"required"`
	Name     string `json:"name" binding:"required"`
}

// POST /admin/agents
func (s *Server) handleCreateAgent(c *gin.Context) {
	var req createAgentReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	id, err := s.store.Q().CreateAgent(c.Request.Context(), req.RoleCode, req.Name)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "agent_create", req.Name, "")
	c.JSON(http.StatusCreated, gin.H{"id": id})
}

type updateAgentReq struct {
	Name     string `json:"name" binding:"required"`
	RoleCode string `json:"role_code" binding:"required"`
	Active   bool   `json:"active"`
}

// PATCH /admin/agents/:id
func (s *Server) handleUpdateAgent(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req updateAgentReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if err := s.store.Q().UpdateAgent(c.Request.Context(), id, req.Name, req.RoleCode, req.Active); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "agent_update", c.Param("id"), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

type issueTokenReq struct {
	Label     string  `json:"label"`
	ExpiresAt *string `json:"expires_at"` // 可选 RFC3339；空=永不过期
}

// POST /admin/agents/:id/tokens —— 签发 token（明文仅一次）
func (s *Server) handleIssueToken(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	agent, err := s.store.Q().GetAgent(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("agent_not_found", "无此成员"))
		return
	}
	// 人工角色（Van）不签发 token。
	if role, err := s.store.Q().GetRole(ctx, agent.RoleCode); err == nil && role.IsHuman {
		s.renderErr(c, domain.BadRequest("human_no_token", "人工角色不签发 token（结论由中枢代录）"))
		return
	}

	var req issueTokenReq
	_ = c.ShouldBindJSON(&req)

	plain, err := auth.NewToken()
	if err != nil {
		s.renderErr(c, err)
		return
	}
	tid, err := s.store.Q().InsertToken(ctx, id, auth.HashToken(plain), req.Label, req.ExpiresAt)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "token_issue", "agent:"+agent.Name+" #"+itoa(tid), "")
	c.JSON(http.StatusCreated, gin.H{"token_id": tid, "token": plain, "label": req.Label})
}

// POST /admin/agents/:id/tokens/:tid/revoke
func (s *Server) handleRevokeToken(c *gin.Context) {
	tid, err := pathID(c, "tid")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	if err := s.store.Q().RevokeToken(c.Request.Context(), tid); err != nil {
		s.renderErr(c, domain.NotFound("token_not_found", "无此 token"))
		return
	}
	s.audit(c, "token_revoke", "token #"+c.Param("tid"), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}
