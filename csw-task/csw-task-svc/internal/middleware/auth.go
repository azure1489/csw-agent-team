package middleware

import (
	"strings"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

const (
	ctxAgent = "csw_agent"
	ctxRole  = "csw_role"
)

// AgentAuth 校验 Bearer token → agent + role，注入上下文；失败 401/403。
func AgentAuth(store *sqlite.Store) gin.HandlerFunc {
	return func(c *gin.Context) {
		h := c.GetHeader("Authorization")
		token := strings.TrimSpace(strings.TrimPrefix(h, "Bearer "))
		if token == "" || token == h {
			c.AbortWithStatusJSON(401, gin.H{"code": "unauthorized", "message": "缺少 Bearer token"})
			return
		}
		agent, role, tokenID, err := store.Q().AuthByToken(c.Request.Context(), auth.HashToken(token))
		if err != nil {
			c.AbortWithStatusJSON(401, gin.H{"code": "unauthorized", "message": "token 无效或已过期"})
			return
		}
		if !agent.Active {
			c.AbortWithStatusJSON(403, gin.H{"code": "agent_disabled", "message": "agent 已禁用"})
			return
		}
		_ = store.Q().TouchToken(c.Request.Context(), tokenID) // best-effort
		c.Set(ctxAgent, agent)
		c.Set(ctxRole, role)
		c.Next()
	}
}

// AgentFrom 从上下文取已认证的 agent + role。
func AgentFrom(c *gin.Context) (domain.Agent, domain.Role) {
	a, _ := c.Get(ctxAgent)
	r, _ := c.Get(ctxRole)
	agent, _ := a.(domain.Agent)
	role, _ := r.(domain.Role)
	return agent, role
}
