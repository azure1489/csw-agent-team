package middleware

import (
	"strings"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
)

const ctxAdminUser = "csw_admin_user"

// AdminClaims 注入上下文的后台用户身份。
type AdminClaims struct {
	Role   string
	UserID int64
}

// JWTAuth 校验 access JWT（验签 + 过期），注入 AdminClaims；失败 401。
func JWTAuth(secret []byte) gin.HandlerFunc {
	return func(c *gin.Context) {
		h := c.GetHeader("Authorization")
		token := strings.TrimSpace(strings.TrimPrefix(h, "Bearer "))
		if token == "" || token == h {
			c.AbortWithStatusJSON(401, gin.H{"code": "unauthorized", "message": "缺少 access token"})
			return
		}
		claims, err := auth.ParseAccess(secret, token)
		if err != nil {
			c.AbortWithStatusJSON(401, gin.H{"code": "unauthorized", "message": "token 无效或已过期"})
			return
		}
		uid, err := claims.UserID()
		if err != nil {
			c.AbortWithStatusJSON(401, gin.H{"code": "unauthorized", "message": "token sub 非法"})
			return
		}
		c.Set(ctxAdminUser, AdminClaims{UserID: uid, Role: claims.Role})
		c.Next()
	}
}

// AdminFrom 取上下文的后台用户身份。
func AdminFrom(c *gin.Context) (AdminClaims, bool) {
	v, ok := c.Get(ctxAdminUser)
	if !ok {
		return AdminClaims{}, false
	}
	ac, ok := v.(AdminClaims)
	return ac, ok
}

// RequireRole 要求当前后台用户角色在允许集合内，否则 403。
func RequireRole(roles ...string) gin.HandlerFunc {
	allowed := make(map[string]bool, len(roles))
	for _, r := range roles {
		allowed[r] = true
	}
	return func(c *gin.Context) {
		ac, ok := AdminFrom(c)
		if !ok {
			c.AbortWithStatusJSON(401, gin.H{"code": "unauthorized", "message": "未登录"})
			return
		}
		if !allowed[ac.Role] {
			c.AbortWithStatusJSON(403, gin.H{"code": "forbidden", "message": "无权限执行该操作"})
			return
		}
		c.Next()
	}
}
