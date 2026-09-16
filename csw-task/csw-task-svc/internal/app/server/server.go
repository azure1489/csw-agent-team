// Package server 装配运行面 gin 路由与 handlers（§6 运行面 API）。
package server

import (
	"errors"
	"log/slog"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/files"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/middleware"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// Server 运行面服务依赖。
type Server struct {
	store  *sqlite.Store
	eng    *engine.Engine
	blobs  files.BlobStore
	log    *slog.Logger
	health func() map[string]any // /healthz 附加字段（notifier 状态、outbox 积压）
	cfg    config.Config
}

// SetHealth 注入 /healthz 附加字段。
func (s *Server) SetHealth(fn func() map[string]any) { s.health = fn }

// New 构造 Server。
func New(store *sqlite.Store, eng *engine.Engine, blobs files.BlobStore, cfg config.Config, log *slog.Logger) *Server {
	return &Server{store: store, eng: eng, blobs: blobs, cfg: cfg, log: log}
}

// Router 装配 gin 引擎。
func (s *Server) Router() *gin.Engine {
	gin.SetMode(gin.ReleaseMode)
	r := gin.New()
	r.Use(middleware.RequestID(), middleware.Logger(s.log), middleware.Recovery(s.log))

	r.GET("/healthz", func(c *gin.Context) {
		h := gin.H{"status": "ok"}
		if s.health != nil {
			for k, v := range s.health() {
				h[k] = v
			}
		}
		c.JSON(200, h)
	})

	v1 := r.Group("/api/v1")
	v1.Use(middleware.AgentAuth(s.store), middleware.Idempotency(s.store, s.cfg.MaxUploadBytes+(1<<20)))
	{
		// Agent 自助
		v1.GET("/me/tasks", s.handleMyTasks)
		v1.GET("/tasks/:id", s.handleTaskDetail)
		v1.GET("/runs/:id", s.handleRunDetail)
		v1.GET("/runs/:id/timeline", s.handleTimeline)
		v1.GET("/runs/:id/authorizations", s.handleListAuthorizations)
		v1.GET("/runs/:id/progress", s.handleProgress)
		v1.GET("/runs/:id/items", s.handleListItems)
		v1.PUT("/runs/:id/items", s.handleUpsertItems)
		v1.PUT("/runs/:id/sweeps", s.handleReportSweeps)
		v1.GET("/runs/:id/intake-trace", s.handleIntakeTrace)
		v1.POST("/files", s.handleUpload)
		v1.GET("/files/:id", s.handleDownload)
		v1.POST("/tasks/:id/deliverables", s.handleSubmit)
		v1.POST("/tasks/:id/ack", s.handleAck)
		v1.POST("/tasks/:id/fail", s.handleFail)

		// 数据子系统（只读查重 / 反馈记忆；写入反馈仅管理类角色）
		v1.GET("/ledger/posts", s.handleLedgerPosts)
		v1.GET("/memory/feedback", s.handleListFeedback)
		v1.POST("/memory/feedback", s.handleCreateFeedback)

		// 花名册（任意角色 token 可读，群播报查 open_id 用）
		v1.GET("/roster", s.handleRoster)

		// 触发
		v1.GET("/workflows", s.handleListWorkflows)
		v1.POST("/workflows/:key/runs", s.handleTrigger)

		// 中枢
		v1.GET("/me/inbox", s.handleInbox)
		v1.POST("/tasks/:id/dispatch", s.handleDispatch)
		v1.POST("/deliverables/:id/reviews", s.handleReview)
		v1.POST("/tasks/:id/cancel", s.handleCancel)
		v1.POST("/runs/:id/items/:key/decision", s.handleItemDecision)
		v1.POST("/runs/:id/close", s.handleCloseRun)
		v1.POST("/tasks/:id/reopen", s.handleReopen)
		v1.POST("/runs/:id/authorizations", s.handleGrantAuthorization)
		v1.DELETE("/runs/:id/authorizations/:scope", s.handleRevokeAuthorization)
	}
	return r
}

// renderErr 把业务错误翻译为统一 {code,message} + HTTP 码。
func (s *Server) renderErr(c *gin.Context, err error) {
	var de *domain.Error
	if errors.As(err, &de) {
		c.JSON(de.HTTP, gin.H{"code": de.Code, "message": de.Message})
		return
	}
	s.log.Error("handler error", "err", err, "path", c.Request.URL.Path)
	c.JSON(500, gin.H{"code": "internal", "message": "服务内部错误"})
}
