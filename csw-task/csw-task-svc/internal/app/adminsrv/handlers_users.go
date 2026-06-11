package adminsrv

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/middleware"
)

// GET /admin/users
func (s *Server) handleListUsers(c *gin.Context) {
	users, err := s.store.Q().ListUsers(c.Request.Context())
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(users))
	for _, u := range users {
		out = append(out, adminUserDTO(u))
	}
	c.JSON(http.StatusOK, gin.H{"users": out})
}

type createUserReq struct {
	Username    string `json:"username" binding:"required"`
	DisplayName string `json:"display_name"`
	Role        string `json:"role" binding:"required"`
	Password    string `json:"password" binding:"required"`
}

func validAdminRole(r string) bool {
	return r == domain.AdminSuperadmin || r == domain.AdminOperator || r == domain.AdminViewer
}

// POST /admin/users
func (s *Server) handleCreateUser(c *gin.Context) {
	var req createUserReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if !validAdminRole(req.Role) {
		s.renderErr(c, domain.BadRequest("bad_role", "role 须为 superadmin/operator/viewer"))
		return
	}
	hash, err := auth.HashPassword(req.Password)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	id, err := s.store.Q().CreateUser(c.Request.Context(), req.Username, hash, req.DisplayName, req.Role)
	if err != nil {
		s.renderErr(c, domain.Conflict("username_taken", "用户名已存在或非法"))
		return
	}
	s.audit(c, "user_create", req.Username, "")
	c.JSON(http.StatusCreated, gin.H{"id": id})
}

type updateUserReq struct {
	DisplayName string `json:"display_name"`
	Role        string `json:"role" binding:"required"`
	Status      string `json:"status" binding:"required"`
}

// PATCH /admin/users/:id —— 防自锁 + 保 ≥1 active superadmin
func (s *Server) handleUpdateUser(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req updateUserReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if !validAdminRole(req.Role) || (req.Status != "active" && req.Status != "disabled") {
		s.renderErr(c, domain.BadRequest("bad_fields", "role/status 非法"))
		return
	}
	ctx := c.Request.Context()
	target, err := s.store.Q().GetUser(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("user_not_found", "无此用户"))
		return
	}
	me, _ := middleware.AdminFrom(c)

	// 防自锁：不能禁用或降级自己。
	if me.UserID == id {
		if req.Status != "active" {
			s.renderErr(c, domain.BadRequest("self_lock", "不能禁用自己"))
			return
		}
		if req.Role != domain.AdminSuperadmin {
			s.renderErr(c, domain.BadRequest("self_lock", "不能降级自己"))
			return
		}
	}

	// 保 ≥1 active superadmin：若目标当前是 active superadmin，且本次改动会取消其超管/启用身份。
	if target.Role == domain.AdminSuperadmin && target.Status == "active" &&
		(req.Role != domain.AdminSuperadmin || req.Status != "active") {
		cnt, _ := s.store.Q().CountActiveSuperadmins(ctx)
		if cnt <= 1 {
			s.renderErr(c, domain.BadRequest("last_superadmin", "必须保留至少一个启用的超级管理员"))
			return
		}
	}

	if err := s.store.Q().UpdateUser(ctx, id, req.DisplayName, req.Role, req.Status); err != nil {
		s.renderErr(c, err)
		return
	}
	// 禁用即失效其 refresh。
	if req.Status == "disabled" {
		_ = s.store.Q().RevokeAllRefreshForUser(ctx, id)
	}
	s.audit(c, "user_update", target.Username, "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// POST /admin/users/:id/reset-password —— 生成临时密码（仅返回一次），并失效其 refresh
func (s *Server) handleResetPassword(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	target, err := s.store.Q().GetUser(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("user_not_found", "无此用户"))
		return
	}
	temp, err := auth.NewToken() // 作临时密码，仅此一次返回
	if err != nil {
		s.renderErr(c, err)
		return
	}
	hash, err := auth.HashPassword(temp)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	if err := s.store.Q().UpdatePassword(ctx, id, hash); err != nil {
		s.renderErr(c, err)
		return
	}
	_ = s.store.Q().RevokeAllRefreshForUser(ctx, id)
	s.audit(c, "user_reset_password", target.Username, "")
	c.JSON(http.StatusOK, gin.H{"temp_password": temp, "note": "请用此临时密码登录后立即修改"})
}
