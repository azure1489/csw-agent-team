package server

import (
	"strconv"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/middleware"
)

// mwAgent 取上下文里已认证的 agent + role。
func mwAgent(c *gin.Context) (domain.Agent, domain.Role) {
	return middleware.AgentFrom(c)
}

// optInt64 解析可选整数表单字段：空串 → nil。
func optInt64(s string) (*int64, error) {
	if s == "" {
		return nil, nil
	}
	v, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		return nil, err
	}
	return &v, nil
}

// pathID 解析路径上的 int64 参数。
func pathID(c *gin.Context, name string) (int64, error) {
	id, err := strconv.ParseInt(c.Param(name), 10, 64)
	if err != nil {
		return 0, domain.BadRequest("bad_id", "非法 id 参数："+c.Param(name))
	}
	return id, nil
}
