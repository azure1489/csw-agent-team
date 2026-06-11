package adminsrv

import (
	"net/http"
	"time"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/middleware"
)

type loginReq struct {
	Username string `json:"username" binding:"required"`
	Password string `json:"password" binding:"required"`
}

// POST /admin/login
func (s *Server) handleLogin(c *gin.Context) {
	var req loginReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	ctx := c.Request.Context()
	user, hash, err := s.store.Q().GetUserByUsername(ctx, req.Username)
	if err != nil {
		s.renderErr(c, domain.NewError(401, "invalid_credentials", "用户名或密码不正确"))
		return
	}
	if user.Status != "active" {
		s.renderErr(c, domain.Forbidden("account_disabled", "账号已禁用，请联系管理员"))
		return
	}
	ok, _ := auth.VerifyPassword(hash, req.Password)
	if !ok {
		s.renderErr(c, domain.NewError(401, "invalid_credentials", "用户名或密码不正确"))
		return
	}

	access, err := s.issueSession(c, user.ID, user.Role)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	_ = s.store.Q().UpdateLastLogin(ctx, user.ID)
	uid := user.ID
	_ = s.store.Q().InsertAudit(ctx, &uid, "login", "", "")

	c.JSON(http.StatusOK, gin.H{
		"access_token": access,
		"expires_in":   int(s.cfg.AccessTTL.Seconds()),
		"user":         adminUserDTO(user),
	})
}

// POST /admin/refresh —— 读 refresh cookie → 轮换 → 新 access
func (s *Server) handleRefresh(c *gin.Context) {
	raw, err := c.Cookie(refreshCookieName)
	if err != nil || raw == "" {
		s.renderErr(c, domain.NewError(401, "no_refresh", "缺少 refresh cookie"))
		return
	}
	ctx := c.Request.Context()
	id, userID, err := s.store.Q().RefreshByHash(ctx, auth.HashToken(raw))
	if err != nil {
		s.clearRefreshCookie(c)
		s.renderErr(c, domain.NewError(401, "bad_refresh", "refresh 无效或已过期"))
		return
	}
	_ = s.store.Q().RevokeRefresh(ctx, id) // 轮换：吊销旧

	user, err := s.store.Q().GetUser(ctx, userID)
	if err != nil || user.Status != "active" {
		s.clearRefreshCookie(c)
		s.renderErr(c, domain.NewError(401, "bad_refresh", "用户不可用"))
		return
	}
	access, err := s.issueSession(c, user.ID, user.Role)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"access_token": access,
		"expires_in":   int(s.cfg.AccessTTL.Seconds()),
		"user":         adminUserDTO(user),
	})
}

// POST /admin/logout —— 吊销当前 refresh + 清 cookie
func (s *Server) handleLogout(c *gin.Context) {
	if raw, err := c.Cookie(refreshCookieName); err == nil && raw != "" {
		if id, _, e := s.store.Q().RefreshByHash(c.Request.Context(), auth.HashToken(raw)); e == nil {
			_ = s.store.Q().RevokeRefresh(c.Request.Context(), id)
		}
	}
	s.clearRefreshCookie(c)
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// GET /admin/me
func (s *Server) handleMe(c *gin.Context) {
	ac, _ := middleware.AdminFrom(c)
	user, err := s.store.Q().GetUser(c.Request.Context(), ac.UserID)
	if err != nil {
		s.renderErr(c, domain.NewError(401, "unauthorized", "用户不存在"))
		return
	}
	c.JSON(http.StatusOK, gin.H{"user": adminUserDTO(user)})
}

// issueSession 签发 access + 写 refresh（DB + cookie），返回 access。
func (s *Server) issueSession(c *gin.Context, userID int64, role string) (string, error) {
	access, err := auth.SignAccess(s.cfg.JWTSecret, userID, role, s.cfg.AccessTTL)
	if err != nil {
		return "", err
	}
	refresh, err := auth.NewRefresh()
	if err != nil {
		return "", err
	}
	exp := time.Now().UTC().Add(s.cfg.RefreshTTL).Format(time.RFC3339)
	if _, err := s.store.Q().InsertRefresh(c.Request.Context(), userID, auth.HashToken(refresh), exp,
		c.Request.UserAgent(), c.ClientIP()); err != nil {
		return "", err
	}
	s.setRefreshCookie(c, refresh, int(s.cfg.RefreshTTL.Seconds()))
	return access, nil
}

func (s *Server) setRefreshCookie(c *gin.Context, value string, maxAge int) {
	c.SetSameSite(http.SameSiteLaxMode)
	c.SetCookie(refreshCookieName, value, maxAge, cookiePath, "", s.cfg.CookieSecure, true)
}

func (s *Server) clearRefreshCookie(c *gin.Context) {
	c.SetSameSite(http.SameSiteLaxMode)
	c.SetCookie(refreshCookieName, "", -1, cookiePath, "", s.cfg.CookieSecure, true)
}
