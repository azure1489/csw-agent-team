package middleware

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"mime"
	"mime/multipart"
	"net/http"
	"sort"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// idemEnvelope 幂等记录（存 idempotency_keys.response_json）。
//   - State=pending：已占位，业务进行中或结果不确定；Fingerprint 绑定首次请求。
//   - State=complete：已缓存首次 2xx 响应，同指纹重试直接回放。
//   - 旧版记录只有 Status+Body（无 Fingerprint/State）：只缓存过 2xx，按原语义回放。
type idemEnvelope struct {
	Body        json.RawMessage `json:"body,omitempty"`
	Fingerprint string          `json:"fingerprint,omitempty"`
	State       string          `json:"state,omitempty"`
	Status      int             `json:"status,omitempty"`
}

const (
	idemPending  = "pending"
	idemComplete = "complete"
)

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

// Idempotency 对带 Idempotency-Key 的 POST 请求做 fail-closed 去重（按键 at-most-once，非 exactly-once）：
//
//  1. 业务执行前先向 idempotency_keys 持久占位（主键冲突判定唯一执行权，跨进程有效）；
//  2. 占位携带请求指纹 = agent + 方法 + 路由 + RequestURI + 媒体类型 + 内容摘要（multipart 按各 part 内容归一，换 boundary 重试仍匹配）；
//  3. 业务 2xx → CAS 写入首次响应，之后同指纹重试回放、异指纹 409；
//  4. 业务非 2xx → 引擎已在事务内回滚、无副作用 → 释放占位，修正后可用同一键重试；
//  5. 业务中途崩溃/panic → 占位保持 pending，同键重试一律 409，需人工核对任务状态后处置；
//  6. 存储不可用 → 503，不放行业务。
//
// maxBody 为带键请求体缓冲上限（含 multipart 上传），超出 413。
func Idempotency(store *sqlite.Store, maxBody int64) gin.HandlerFunc {
	return func(c *gin.Context) {
		key := c.GetHeader("Idempotency-Key")
		if c.Request.Method != http.MethodPost || key == "" {
			c.Next()
			return
		}
		fail := func(status int, code, msg string) {
			c.AbortWithStatusJSON(status, gin.H{"code": code, "message": msg})
		}

		body, err := io.ReadAll(http.MaxBytesReader(c.Writer, c.Request.Body, maxBody))
		if err != nil {
			fail(http.StatusRequestEntityTooLarge, "idempotency_body_too_large", "请求体不可读或超过上限，未执行")
			return
		}
		c.Request.Body = io.NopCloser(bytes.NewReader(body))

		agent, _ := AgentFrom(c)
		fp := fingerprint(agent.ID, c, body)
		claimJSON, _ := json.Marshal(idemEnvelope{Fingerprint: fp, State: idemPending})
		claim := string(claimJSON)

		ctx := c.Request.Context()
		owned, err := store.Q().ClaimIdempotency(ctx, key, agent.ID, c.FullPath(), claim)
		if err != nil {
			fail(http.StatusServiceUnavailable, "idempotency_store_unavailable", "幂等存储不可用，未执行")
			return
		}
		if !owned {
			raw, found, err := store.Q().Idempotency(ctx, key)
			if err != nil || !found {
				fail(http.StatusServiceUnavailable, "idempotency_store_unavailable", "幂等存储不可用，未执行")
				return
			}
			var env idemEnvelope
			if json.Unmarshal([]byte(raw), &env) != nil {
				fail(http.StatusConflict, "idempotency_in_progress_or_uncertain", "同一幂等键的首次请求结果不确定，请先查询任务状态")
				return
			}
			switch {
			case env.Fingerprint == "" && env.State == "":
				// 旧版记录：只缓存过 2xx，无指纹可比，按原语义回放。
				if env.Status != 0 && json.Valid(env.Body) {
					replay(c, env)
					return
				}
				fail(http.StatusConflict, "idempotency_in_progress_or_uncertain", "同一幂等键的历史记录不完整，请先查询任务状态")
				return
			case env.Fingerprint != fp:
				fail(http.StatusConflict, "idempotency_request_mismatch", "同一幂等键对应的请求内容、身份或目标与首次不一致")
				return
			case env.State != idemComplete || env.Status == 0 || !json.Valid(env.Body):
				fail(http.StatusConflict, "idempotency_in_progress_or_uncertain", "同一幂等键的首次请求仍在处理或结果不确定，请先查询任务状态")
				return
			}
			replay(c, env)
			return
		}

		cw := &captureWriter{ResponseWriter: c.Writer, buf: &bytes.Buffer{}}
		c.Writer = cw
		c.Next()
		// 走到这里说明业务已返回（panic 会跳过下面全部，占位保持 pending = 结果不确定）。

		// 客户端断连不能取消持久化：用独立上下文。
		saveCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()

		status := cw.Status()
		if status >= 200 && status < 300 {
			if !json.Valid(cw.buf.Bytes()) {
				// 成功但响应不可回放：保持 pending（fail-closed），不释放。
				_ = c.Error(errNonJSONResponse)
				return
			}
			result, _ := json.Marshal(idemEnvelope{Fingerprint: fp, State: idemComplete, Status: status, Body: cw.buf.Bytes()})
			if err := store.Q().CompleteIdempotency(saveCtx, key, claim, string(result)); err != nil {
				_ = c.Error(err)
			}
			return
		}
		// 非 2xx：引擎在事务内失败并回滚，无副作用 → 释放占位，允许修正后重试。
		if err := store.Q().ReleaseIdempotency(saveCtx, key, claim); err != nil {
			_ = c.Error(err)
		}
	}
}

func replay(c *gin.Context, env idemEnvelope) {
	c.Data(env.Status, "application/json; charset=utf-8", env.Body)
	c.Abort()
}

type idemError string

func (e idemError) Error() string { return string(e) }

const errNonJSONResponse = idemError("idempotency: 2xx response is not JSON, claim kept pending")

// fingerprint 请求指纹：agent + 方法 + 路由模板 + RequestURI + 媒体类型 + 内容摘要。
func fingerprint(agentID int64, c *gin.Context, body []byte) string {
	mediaType, params, err := mime.ParseMediaType(c.GetHeader("Content-Type"))
	if err != nil {
		mediaType = strings.ToLower(strings.TrimSpace(strings.SplitN(c.GetHeader("Content-Type"), ";", 2)[0]))
	}
	digest := sha256Hex(body)
	if mediaType == "multipart/form-data" && params["boundary"] != "" {
		if d, ok := multipartDigest(body, params["boundary"]); ok {
			digest = d
		}
	}
	enc, _ := json.Marshal([]any{agentID, c.Request.Method, c.FullPath(), c.Request.URL.RequestURI(), mediaType, digest})
	return sha256Hex(enc)
}

// multipartDigest 按各 part 的字段名 / 文件名 / 内容 SHA 归一（排序后再摘要），与 boundary 无关。
func multipartDigest(body []byte, boundary string) (string, bool) {
	mr := multipart.NewReader(bytes.NewReader(body), boundary)
	var parts []string
	for {
		p, err := mr.NextPart()
		if err == io.EOF {
			break
		}
		if err != nil {
			return "", false
		}
		h := sha256.New()
		if _, err := io.Copy(h, p); err != nil {
			return "", false
		}
		parts = append(parts, p.FormName()+"\x00"+p.FileName()+"\x00"+hex.EncodeToString(h.Sum(nil)))
	}
	sort.Strings(parts)
	return sha256Hex([]byte(strings.Join(parts, "\n"))), true
}

func sha256Hex(b []byte) string {
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}
