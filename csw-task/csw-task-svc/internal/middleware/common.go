// Package middleware 提供运行面 gin 中间件：requestid / logger / recovery / agentauth / idempotency。
package middleware

import (
	"log/slog"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

const headerRequestID = "X-Request-Id"

// RequestID 为每个请求注入/透传 request id。
func RequestID() gin.HandlerFunc {
	return func(c *gin.Context) {
		rid := c.GetHeader(headerRequestID)
		if rid == "" {
			rid = uuid.NewString()
		}
		c.Set("request_id", rid)
		c.Writer.Header().Set(headerRequestID, rid)
		c.Next()
	}
}

// Logger 用 slog 记录每个请求。
func Logger(log *slog.Logger) gin.HandlerFunc {
	return func(c *gin.Context) {
		start := time.Now()
		c.Next()
		rid, _ := c.Get("request_id")
		log.Info("http",
			"method", c.Request.Method,
			"path", c.Request.URL.Path,
			"status", c.Writer.Status(),
			"latency_ms", time.Since(start).Milliseconds(),
			"request_id", rid,
		)
	}
}

// Recovery 捕获 panic，返回统一 500 错误体。
func Recovery(log *slog.Logger) gin.HandlerFunc {
	return func(c *gin.Context) {
		defer func() {
			if r := recover(); r != nil {
				log.Error("panic", "recover", r, "path", c.Request.URL.Path)
				c.AbortWithStatusJSON(500, gin.H{"code": "internal", "message": "服务内部错误"})
			}
		}()
		c.Next()
	}
}
