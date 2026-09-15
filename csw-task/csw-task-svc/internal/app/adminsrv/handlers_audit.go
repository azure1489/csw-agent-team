package adminsrv

import (
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"
)

// GET /admin/audit?user_id=&action=&date=&target_prefix=
func (s *Server) handleListAudit(c *gin.Context) {
	if tp := c.Query("target_prefix"); tp != "" {
		entries, err := s.store.Q().ListAuditByTargetPrefix(c.Request.Context(), tp, 200)
		if err != nil {
			s.renderErr(c, err)
			return
		}
		out := make([]gin.H, 0, len(entries))
		for _, a := range entries {
			out = append(out, auditDTO(a))
		}
		c.JSON(http.StatusOK, gin.H{"audit": out})
		return
	}
	var userID *int64
	if v := c.Query("user_id"); v != "" {
		if n, err := strconv.ParseInt(v, 10, 64); err == nil {
			userID = &n
		}
	}
	entries, err := s.store.Q().ListAudit(c.Request.Context(), userID, c.Query("action"), c.Query("date"), 200)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(entries))
	for _, a := range entries {
		out = append(out, auditDTO(a))
	}
	c.JSON(http.StatusOK, gin.H{"audit": out})
}
