// Package adminsrv 装配管理后台 gin 路由与 handlers（§6.1 + 《管理后台·功能与线框》§1–§14）。
// 纯 JSON、JWT 登录、与运行面分进程分凭证。
package adminsrv

import (
	"errors"
	"log/slog"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/middleware"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

const (
	refreshCookieName = "csw_refresh"
	cookiePath        = "/admin"
)

// Server 管理后台服务依赖。
type Server struct {
	store *sqlite.Store
	cfg   config.Config
	log   *slog.Logger
}

// New 构造 Server。
func New(store *sqlite.Store, cfg config.Config, log *slog.Logger) *Server {
	return &Server{store: store, cfg: cfg, log: log}
}

// Router 装配 gin 引擎。
func (s *Server) Router() *gin.Engine {
	gin.SetMode(gin.ReleaseMode)
	r := gin.New()
	r.Use(middleware.RequestID(), middleware.Logger(s.log), middleware.Recovery(s.log),
		middleware.CORS(s.cfg.CORSOrigins))

	r.GET("/healthz", func(c *gin.Context) { c.JSON(200, gin.H{"status": "ok"}) })

	admin := r.Group("/admin")

	// 公开（无需 JWT）
	admin.POST("/login", s.handleLogin)
	admin.POST("/refresh", s.handleRefresh)
	admin.POST("/logout", s.handleLogout)

	// 需登录
	p := admin.Group("")
	p.Use(middleware.JWTAuth(s.cfg.JWTSecret))
	{
		p.GET("/me", s.handleMe)
		p.GET("/overview", s.handleOverview)

		// 只读（含 viewer）
		p.GET("/workflows", s.handleListWorkflows)
		p.GET("/workflows/:id", s.handleGetWorkflow)
		p.GET("/roles", s.handleListRoles)
		p.GET("/agents", s.handleListAgents)
		p.GET("/chats", s.handleListChats)
		p.GET("/chats/:id/members", s.handleListChatMembers)
		p.GET("/runs", s.handleListRuns)
		p.GET("/runs/:id", s.handleRunDetail)
		p.GET("/runs/:id/timeline", s.handleRunTimeline)
		p.GET("/tasks/:id", s.handleTaskDetail)
		p.GET("/deliverables/:id", s.handleDeliverableDetail)
		p.GET("/audit", s.handleListAudit)
		p.GET("/settings", s.handleGetSettings)

		// 写（operator+）
		w := p.Group("")
		w.Use(middleware.RequireRole(domain.AdminSuperadmin, domain.AdminOperator))
		{
			w.POST("/workflows", s.handleCreateWorkflow)
			w.PUT("/workflows/:id", s.handlePutWorkflow)
			w.POST("/workflows/:id/validate", s.handleValidateWorkflow)
			w.POST("/workflows/:id/activate", s.handleActivateWorkflow)
			w.POST("/workflows/:id/archive", s.handleArchiveWorkflow)
			w.POST("/workflows/:id/clone", s.handleCloneWorkflow)

			w.POST("/roles", s.handleCreateRole)
			w.PATCH("/roles/:code", s.handleUpdateRole)
			w.POST("/agents", s.handleCreateAgent)
			w.PATCH("/agents/:id", s.handleUpdateAgent)
			w.POST("/agents/:id/tokens", s.handleIssueToken)
			w.POST("/agents/:id/tokens/:tid/revoke", s.handleRevokeToken)

			w.POST("/chats", s.handleCreateChat)
			w.PATCH("/chats/:id", s.handleUpdateChat)
			w.POST("/chats/:id/members", s.handleCreateChatMember)
			w.PATCH("/chats/:id/members/:mid", s.handleUpdateChatMember)
			w.DELETE("/chats/:id/members/:mid", s.handleDeleteChatMember)
		}

		// superadmin only
		su := p.Group("")
		su.Use(middleware.RequireRole(domain.AdminSuperadmin))
		{
			su.GET("/users", s.handleListUsers)
			su.POST("/users", s.handleCreateUser)
			su.PATCH("/users/:id", s.handleUpdateUser)
			su.POST("/users/:id/reset-password", s.handleResetPassword)
			su.PUT("/settings", s.handlePutSettings)

			su.DELETE("/chats/:id", s.handleDeleteChat)
		}
	}
	return r
}

// renderErr 统一翻译业务错误为 {code,message} + HTTP 码。
func (s *Server) renderErr(c *gin.Context, err error) {
	var de *domain.Error
	if errors.As(err, &de) {
		c.JSON(de.HTTP, gin.H{"code": de.Code, "message": de.Message})
		return
	}
	s.log.Error("admin handler error", "err", err, "path", c.Request.URL.Path)
	c.JSON(500, gin.H{"code": "internal", "message": "服务内部错误"})
}

// audit 记一条后台审计（best-effort）。
func (s *Server) audit(c *gin.Context, action, target, detailJSON string) {
	var uid *int64
	if ac, ok := middleware.AdminFrom(c); ok {
		id := ac.UserID
		uid = &id
	}
	_ = s.store.Q().InsertAudit(c.Request.Context(), uid, action, target, detailJSON)
}
