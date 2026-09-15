package server

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/files"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// setupHTTP 起一个真实装配的运行面（迁移 + seed + LocalStore），返回 server、store、引擎与发 token 函数。
func setupHTTP(t *testing.T) (*httptest.Server, *sqlite.Store, *engine.Engine, func(role string) string) {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	st := sqlite.New(db)
	// 本文件用例按 daily_news v1（十阶段双闸）断言。
	if err := st.Q().ActivateWorkflowVersion(context.Background(), "daily_news", 1); err != nil {
		t.Fatalf("activate daily_news v1: %v", err)
	}
	eng := engine.New(st)
	blobs, err := files.NewLocalStore(filepath.Join(t.TempDir(), "blobs"))
	if err != nil {
		t.Fatalf("blobs: %v", err)
	}
	cfg := config.Config{BaseURL: "http://test", MaxUploadBytes: 8 << 20}
	srv := New(st, eng, blobs, cfg, slog.New(slog.NewTextHandler(io.Discard, nil)))
	ts := httptest.NewServer(srv.Router())
	t.Cleanup(ts.Close)

	mint := func(role string) string {
		a, err := st.Q().ActiveAgentByRole(context.Background(), role)
		if err != nil {
			t.Fatalf("agent %s: %v", role, err)
		}
		plain, _ := auth.NewToken()
		if _, err := st.Q().InsertToken(context.Background(), a.ID, auth.HashToken(plain), "test", nil); err != nil {
			t.Fatalf("token %s: %v", role, err)
		}
		return plain
	}
	return ts, st, eng, mint
}

func httpJSON(t *testing.T, method, url, token string, body io.Reader, contentType string) (int, map[string]any) {
	t.Helper()
	req, _ := http.NewRequest(method, url, body)
	req.Header.Set("Authorization", "Bearer "+token)
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, url, err)
	}
	defer resp.Body.Close()
	var out map[string]any
	_ = json.NewDecoder(resp.Body).Decode(&out)
	return resp.StatusCode, out
}

// TestSubmitOneShot 覆盖：multipart 一步提交（服务端派生 filename/object_key、doc_type 缺省取
// output_type）→ task 详情暴露 output_type → 退回后 latest_review 自助可读（A1/A2/A3 的 HTTP 面）。
func TestSubmitOneShot(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()

	editor, editorRole := func() (domain.Agent, domain.Role) {
		a, _ := st.Q().ActiveAgentByRole(ctx, "editor")
		r, _ := st.Q().GetRole(ctx, "editor")
		return a, r
	}()

	// 触发 + 派工 01-采集（engine 直调，HTTP 面专测新增路径）。
	res, err := eng.Trigger(ctx, editor, editorRole, "daily_news", "2026-06-09", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	runID := res.Run.ID
	var collect domain.Task
	for _, tk := range res.Tasks {
		if tk.StageCode == "collect" {
			collect = tk
		}
	}
	if collect.OutputType != "资讯包" {
		t.Fatalf("output_type 快照缺失: %q", collect.OutputType)
	}
	if _, err := eng.Dispatch(ctx, editor, editorRole, collect.ID, "采集要求", nil); err != nil {
		t.Fatalf("dispatch: %v", err)
	}

	collectorToken := mint("collector")

	// multipart 一步提交：不传 doc_type / filename / object_key。
	var buf bytes.Buffer
	mw := multipart.NewWriter(&buf)
	fw, _ := mw.CreateFormFile("file", "本地随便起的名.zip")
	_, _ = fw.Write([]byte("zip-bytes"))
	_ = mw.WriteField("self_check", "来源✓ 链接✓ 去重✓")
	_ = mw.WriteField("upstreams", `[{"label":"派工单","url":"http://up/1"}]`)
	_ = mw.Close()

	code, body := httpJSON(t, "POST", fmt.Sprintf("%s/api/v1/tasks/%d/deliverables", ts.URL, collect.ID),
		collectorToken, &buf, mw.FormDataContentType())
	if code != http.StatusCreated {
		t.Fatalf("submit status %d: %v", code, body)
	}
	deliv := body["deliverable"].(map[string]any)
	wantFn := fmt.Sprintf("资讯日更_情报收集员_20260609_r%d_v1.zip", runID)
	if deliv["filename"] != wantFn {
		t.Fatalf("filename want %s, got %v", wantFn, deliv["filename"])
	}
	if deliv["doc_type"] != "资讯包" {
		t.Fatalf("doc_type 未取 output_type 缺省: %v", deliv["doc_type"])
	}
	wantKey := fmt.Sprintf("资讯日更/2026-06-09/r%d/01-采集/%s", runID, wantFn)
	if got := body["file"].(map[string]any)["object_key"]; got != wantKey {
		t.Fatalf("object_key want %s, got %v", wantKey, got)
	}
	if !strings.Contains(deliv["download_url"].(string), "/api/v1/files/") {
		t.Fatalf("download_url: %v", deliv["download_url"])
	}

	// 下载回读，确认 blob 按语义路径落盘可取（download_url 域名是 cfg.BaseURL，测试中重写到 httptest 地址）。
	durl := strings.Replace(deliv["download_url"].(string), "http://test", ts.URL, 1)
	req, _ := http.NewRequest("GET", durl, nil)
	req.Header.Set("Authorization", "Bearer "+collectorToken)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("download: %v", err)
	}
	got, _ := io.ReadAll(resp.Body)
	resp.Body.Close()
	if string(got) != "zip-bytes" {
		t.Fatalf("download body: %q", got)
	}

	// 主编闸 reject → task 详情 latest_review 自助读到方向/位置。
	delivID := int64(deliv["id"].(float64))
	if _, err := eng.Review(ctx, editor, editorRole, delivID, engine.ReviewInput{
		Verdict: domain.VerdictReject, ReturnDirection: "补图", ReturnLocation: "第2条",
	}); err != nil {
		t.Fatalf("reject: %v", err)
	}
	code, body = httpJSON(t, "GET", fmt.Sprintf("%s/api/v1/tasks/%d", ts.URL, collect.ID), collectorToken, nil, "")
	if code != http.StatusOK {
		t.Fatalf("task detail %d: %v", code, body)
	}
	if body["task"].(map[string]any)["output_type"] != "资讯包" {
		t.Fatalf("task.output_type: %v", body["task"])
	}
	lr := body["deliverables"].([]any)[0].(map[string]any)["latest_review"].(map[string]any)
	if lr["verdict"] != "reject" || lr["return_direction"] != "补图" || lr["return_location"] != "第2条" || lr["gate_order"] != float64(1) {
		t.Fatalf("latest_review: %v", lr)
	}

	// 退回后 multipart 重交 → 版本与命名自动 +1。
	var buf2 bytes.Buffer
	mw2 := multipart.NewWriter(&buf2)
	fw2, _ := mw2.CreateFormFile("file", "v2.zip")
	_, _ = fw2.Write([]byte("zip-bytes-v2"))
	_ = mw2.Close()
	code, body = httpJSON(t, "POST", fmt.Sprintf("%s/api/v1/tasks/%d/deliverables", ts.URL, collect.ID),
		collectorToken, &buf2, mw2.FormDataContentType())
	if code != http.StatusCreated {
		t.Fatalf("resubmit status %d: %v", code, body)
	}
	wantFn2 := fmt.Sprintf("资讯日更_情报收集员_20260609_r%d_v2.zip", runID)
	if got := body["deliverable"].(map[string]any)["filename"]; got != wantFn2 {
		t.Fatalf("v2 filename want %s, got %v", wantFn2, got)
	}
}
