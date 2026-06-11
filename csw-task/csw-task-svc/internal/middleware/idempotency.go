package middleware

import (
	"bytes"
	"encoding/json"
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

type idemEnvelope struct {
	Body   json.RawMessage `json:"body"`
	Status int             `json:"status"`
}

type captureWriter struct {
	gin.ResponseWriter
	buf *bytes.Buffer
}

func (w *captureWriter) Write(b []byte) (int, error) {
	w.buf.Write(b)
	return w.ResponseWriter.Write(b)
}

func (w *captureWriter) WriteString(s string) (int, error) {
	w.buf.WriteString(s)
	return w.ResponseWriter.WriteString(s)
}

// Idempotency 对带 Idempotency-Key 的 POST 请求：命中则回放首次响应，否则捕获 2xx 响应存库。
func Idempotency(store *sqlite.Store) gin.HandlerFunc {
	return func(c *gin.Context) {
		key := c.GetHeader("Idempotency-Key")
		if c.Request.Method != http.MethodPost || key == "" {
			c.Next()
			return
		}

		ctx := c.Request.Context()
		if raw, found, err := store.Q().Idempotency(ctx, key); err == nil && found {
			var env idemEnvelope
			if json.Unmarshal([]byte(raw), &env) == nil && env.Status != 0 {
				c.Data(env.Status, "application/json; charset=utf-8", env.Body)
				c.Abort()
				return
			}
		}

		cw := &captureWriter{ResponseWriter: c.Writer, buf: &bytes.Buffer{}}
		c.Writer = cw
		c.Next()

		status := cw.Status()
		if status >= 200 && status < 300 {
			env := idemEnvelope{Status: status, Body: cw.buf.Bytes()}
			if b, err := json.Marshal(env); err == nil {
				var agentID *int64
				if agent, _ := AgentFrom(c); agent.ID != 0 {
					agentID = &agent.ID
				}
				_ = store.Q().PutIdempotency(ctx, key, agentID, c.FullPath(), string(b))
			}
		}
	}
}
