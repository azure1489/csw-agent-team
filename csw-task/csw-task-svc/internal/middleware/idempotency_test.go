package middleware

import (
	"bytes"
	"database/sql"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// 一套最小夹具：真实 sqlite 文件 + 两个独立连接（模拟两个服务进程共享同一 DB）。
type idemFixture struct {
	t          *testing.T
	path       string
	db         *sql.DB
	executions atomic.Int64
	failNext   atomic.Bool
}

func newIdemFixture(t *testing.T) *idemFixture {
	t.Helper()
	gin.SetMode(gin.TestMode)
	path := filepath.Join(t.TempDir(), "idem.db")
	db, err := sqlite.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	if _, err := db.Exec(`CREATE TABLE idempotency_keys (key TEXT PRIMARY KEY, agent_id INTEGER, endpoint TEXT, response_json TEXT)`); err != nil {
		t.Fatal(err)
	}
	return &idemFixture{t: t, path: path, db: db}
}

func (f *idemFixture) openAnother() *sql.DB {
	db, err := sqlite.Open(f.path)
	if err != nil {
		f.t.Fatal(err)
	}
	f.t.Cleanup(func() { db.Close() })
	return db
}

func (f *idemFixture) router(db *sql.DB) *gin.Engine {
	r := gin.New()
	r.Use(func(c *gin.Context) {
		id := int64(1)
		if c.GetHeader("Actor") == "other" {
			id = 2
		}
		c.Set(ctxAgent, domain.Agent{ID: id})
	}, Idempotency(sqlite.New(db), 8<<20))
	r.POST("/action/:id", func(c *gin.Context) {
		f.executions.Add(1)
		if f.failNext.CompareAndSwap(true, false) {
			c.JSON(409, gin.H{"code": "cannot_submit"})
			return
		}
		c.JSON(201, gin.H{"created": true, "n": f.executions.Load()})
	})
	r.POST("/upload", func(c *gin.Context) {
		f.executions.Add(1)
		fh, err := c.FormFile("file")
		if err != nil {
			c.JSON(400, gin.H{"code": "no_file"})
			return
		}
		c.JSON(201, gin.H{"file": fh.Filename, "title": c.PostForm("title")})
	})
	return r
}

type call struct {
	key, body, path, actor, contentType string
}

func (f *idemFixture) do(r *gin.Engine, k call) *httptest.ResponseRecorder {
	path := k.path
	if path == "" {
		path = "/action/1"
	}
	ct := k.contentType
	if ct == "" {
		ct = "application/json"
	}
	req := httptest.NewRequest(http.MethodPost, path, bytes.NewBufferString(k.body))
	req.Header.Set("Idempotency-Key", k.key)
	req.Header.Set("Content-Type", ct)
	req.Header.Set("Actor", k.actor)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	return w
}

func (f *idemFixture) row(key string) (string, bool) {
	var resp sql.NullString
	err := f.db.QueryRow(`SELECT response_json FROM idempotency_keys WHERE key=?`, key).Scan(&resp)
	if err == sql.ErrNoRows {
		return "", false
	}
	if err != nil {
		f.t.Fatal(err)
	}
	return resp.String, true
}

func TestIdempotency_ConcurrentSameKeyExecutesOnce(t *testing.T) {
	f := newIdemFixture(t)
	a, b := f.router(f.db), f.router(f.openAnother())

	var wg sync.WaitGroup
	codes := make([]int, 12)
	for i := 0; i < 12; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			r := a
			if i%2 == 1 {
				r = b
			}
			codes[i] = f.do(r, call{key: "k1", body: "{}"}).Code
		}(i)
	}
	wg.Wait()
	if got := f.executions.Load(); got != 1 {
		t.Fatalf("executions=%d, want 1", got)
	}
	for _, c := range codes {
		if c != 201 && c != 409 {
			t.Fatalf("unexpected status %d", c)
		}
	}
	// 完成后同指纹重试：回放首次响应，不再执行。
	w := f.do(b, call{key: "k1", body: "{}"})
	if w.Code != 201 || !strings.Contains(w.Body.String(), `"n":1`) {
		t.Fatalf("replay: code=%d body=%s", w.Code, w.Body)
	}
	if f.executions.Load() != 1 {
		t.Fatalf("replay must not execute")
	}
}

func TestIdempotency_FingerprintMismatch(t *testing.T) {
	f := newIdemFixture(t)
	r := f.router(f.db)
	if w := f.do(r, call{key: "k2", body: "{}"}); w.Code != 201 {
		t.Fatal(w.Code)
	}
	for _, x := range []call{
		{key: "k2", body: `{"x":1}`},               // body 不同
		{key: "k2", body: "{}", path: "/action/2"}, // 路径参数不同
		{key: "k2", body: "{}", actor: "other"},    // 身份不同
	} {
		if w := f.do(r, x); w.Code != 409 || !strings.Contains(w.Body.String(), "idempotency_request_mismatch") {
			t.Fatalf("%+v: code=%d body=%s", x, w.Code, w.Body)
		}
	}
	if f.executions.Load() != 1 {
		t.Fatalf("mismatch must not execute")
	}
}

func TestIdempotency_Non2xxReleasesClaim(t *testing.T) {
	f := newIdemFixture(t)
	r := f.router(f.db)
	f.failNext.Store(true)
	if w := f.do(r, call{key: "k3", body: "{}"}); w.Code != 409 || !strings.Contains(w.Body.String(), "cannot_submit") {
		t.Fatalf("first: code=%d body=%s", w.Code, w.Body)
	}
	if _, found := f.row("k3"); found {
		t.Fatal("claim must be released after non-2xx")
	}
	// 修正后同键重试：应真正执行。
	if w := f.do(r, call{key: "k3", body: "{}"}); w.Code != 201 {
		t.Fatalf("retry: code=%d body=%s", w.Code, w.Body)
	}
	if f.executions.Load() != 2 {
		t.Fatalf("executions=%d, want 2", f.executions.Load())
	}
}

func TestIdempotency_LegacyRowReplays(t *testing.T) {
	f := newIdemFixture(t)
	r := f.router(f.db)
	if _, err := f.db.Exec(`INSERT INTO idempotency_keys(key,response_json) VALUES('legacy','{"status":201,"body":{"legacy":true}}')`); err != nil {
		t.Fatal(err)
	}
	w := f.do(r, call{key: "legacy", body: "{}"})
	if w.Code != 201 || !strings.Contains(w.Body.String(), `"legacy":true`) {
		t.Fatalf("legacy replay: code=%d body=%s", w.Code, w.Body)
	}
	if f.executions.Load() != 0 {
		t.Fatal("legacy replay must not execute")
	}
}

func TestIdempotency_PendingClaimFailsClosedAcrossReopen(t *testing.T) {
	f := newIdemFixture(t)
	r := f.router(f.db)
	if w := f.do(r, call{key: "k4", body: "{}"}); w.Code != 201 {
		t.Fatal(w.Code)
	}
	// 模拟「业务已提交、响应缓存未完成」：把记录改回 pending。
	if _, err := f.db.Exec(`UPDATE idempotency_keys SET response_json=json_set(response_json,'$.state','pending') WHERE key='k4'`); err != nil {
		t.Fatal(err)
	}
	r2 := f.router(f.openAnother())
	w := f.do(r2, call{key: "k4", body: "{}"})
	if w.Code != 409 || !strings.Contains(w.Body.String(), "idempotency_in_progress_or_uncertain") {
		t.Fatalf("pending: code=%d body=%s", w.Code, w.Body)
	}
	if f.executions.Load() != 1 {
		t.Fatal("pending must not execute")
	}
}

func TestIdempotency_StoreUnavailableIs503(t *testing.T) {
	f := newIdemFixture(t)
	db := f.openAnother()
	r := f.router(db)
	db.Close()
	w := f.do(r, call{key: "k5", body: "{}"})
	if w.Code != 503 || !strings.Contains(w.Body.String(), "idempotency_store_unavailable") {
		t.Fatalf("code=%d body=%s", w.Code, w.Body)
	}
	if f.executions.Load() != 0 {
		t.Fatal("must not execute without a claim")
	}
}

func TestIdempotency_NoKeyOrNonPostPassesThrough(t *testing.T) {
	f := newIdemFixture(t)
	r := f.router(f.db)
	for i := 0; i < 2; i++ {
		if w := f.do(r, call{key: "", body: "{}"}); w.Code != 201 {
			t.Fatal(w.Code)
		}
	}
	if f.executions.Load() != 2 {
		t.Fatal("no-key requests are never deduplicated")
	}
	if _, found := f.row(""); found {
		t.Fatal("no row for empty key")
	}
}

func multipartBody(t *testing.T, boundary, title, filename, content string) (string, string) {
	t.Helper()
	var buf bytes.Buffer
	mw := multipart.NewWriter(&buf)
	if err := mw.SetBoundary(boundary); err != nil {
		t.Fatal(err)
	}
	if err := mw.WriteField("title", title); err != nil {
		t.Fatal(err)
	}
	fw, err := mw.CreateFormFile("file", filename)
	if err != nil {
		t.Fatal(err)
	}
	fw.Write([]byte(content))
	mw.Close()
	return buf.String(), mw.FormDataContentType()
}

func TestIdempotency_MultipartIgnoresBoundary(t *testing.T) {
	f := newIdemFixture(t)
	r := f.router(f.db)

	b1, ct1 := multipartBody(t, "AAAAaaaa0001", "v1", "x.zip", "zip-bytes")
	b2, ct2 := multipartBody(t, "BBBBbbbb0002", "v1", "x.zip", "zip-bytes") // 同内容、换 boundary
	b3, ct3 := multipartBody(t, "CCCCcccc0003", "v1", "x.zip", "zip-bytes-changed")

	if w := f.do(r, call{key: "up", body: b1, path: "/upload", contentType: ct1}); w.Code != 201 || !strings.Contains(w.Body.String(), `"file":"x.zip"`) {
		t.Fatalf("first upload: code=%d body=%s", w.Code, w.Body)
	}
	if w := f.do(r, call{key: "up", body: b2, path: "/upload", contentType: ct2}); w.Code != 201 {
		t.Fatalf("boundary-changed retry must replay: code=%d body=%s", w.Code, w.Body)
	}
	if f.executions.Load() != 1 {
		t.Fatalf("executions=%d, want 1", f.executions.Load())
	}
	if w := f.do(r, call{key: "up", body: b3, path: "/upload", contentType: ct3}); w.Code != 409 || !strings.Contains(w.Body.String(), "idempotency_request_mismatch") {
		t.Fatalf("changed content must mismatch: code=%d body=%s", w.Code, w.Body)
	}
}
