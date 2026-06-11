package adminsrv

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
)

// settingsView 后台设置只读视图（配置 env 驱动）。
func (s *Server) settingsView() gin.H {
	types := make([]string, 0, len(s.cfg.AllowedContentTypes))
	for t := range s.cfg.AllowedContentTypes {
		types = append(types, t)
	}
	return gin.H{
		"version": config.Version,
		"jwt": gin.H{
			"access_ttl_seconds":  int(s.cfg.AccessTTL.Seconds()),
			"refresh_ttl_seconds": int(s.cfg.RefreshTTL.Seconds()),
			"secret_generated":    s.cfg.JWTSecretGenerated,
		},
		"files": gin.H{
			"max_upload_bytes":      s.cfg.MaxUploadBytes,
			"allowed_content_types": types,
			"storage":               s.cfg.BlobBackend,
		},
		"base_url":     s.cfg.BaseURL,
		"db_path":      s.cfg.DBPath,
		"cors_origins": s.cfg.CORSOrigins,
		"readonly":     true,
	}
}

// GET /admin/settings
func (s *Server) handleGetSettings(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{"settings": s.settingsView()})
}

// PUT /admin/settings —— 配置 env 驱动，本轮 no-op，回显当前设置。
func (s *Server) handlePutSettings(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{
		"settings": s.settingsView(),
		"note":     "配置由环境变量驱动，本轮设置为只读；修改请调整 env 后重启",
	})
}
