package server

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"testing"
)

// TestLifecycleEndpoints 覆盖接单 / 报告失败 / 重开 / 取消的 HTTP 面与失败队列。
func TestLifecycleEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	editorTok, collectorTok := mint("editor"), mint("collector")
	editor, _ := st.Q().ActiveAgentByRole(ctx, "editor")
	role, _ := st.Q().GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-16", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	// v1 为 manual：先派 01-采集。
	var collectID int64
	for _, tk := range res.Tasks {
		if tk.StageCode == "collect" {
			collectID = tk.ID
		}
	}
	if _, err := eng.Dispatch(ctx, editor, role, collectID, "采", nil); err != nil {
		t.Fatalf("dispatch: %v", err)
	}
	url := func(action string) string { return fmt.Sprintf("%s/api/v1/tasks/%d/%s", ts.URL, collectID, action) }
	post := func(tok, action, body string) (int, map[string]any) {
		var rd *bytes.Buffer
		if body != "" {
			rd = bytes.NewBufferString(body)
		} else {
			rd = &bytes.Buffer{}
		}
		return httpJSON(t, http.MethodPost, url(action), tok, rd, "application/json")
	}

	if code, out := post(collectorTok, "ack", ""); code != 200 || out["task"].(map[string]any)["status"] != "in_progress" {
		t.Fatalf("ack: %d %v", code, out)
	}
	if code, out := post(collectorTok, "fail", `{}`); code != 400 || out["code"] != "reason_required" {
		t.Fatalf("fail without reason: %d %v", code, out)
	}
	if code, out := post(collectorTok, "fail", `{"reason":"来源全部失效"}`); code != 200 || out["task"].(map[string]any)["fail_reason"] != "来源全部失效" {
		t.Fatalf("fail: %d %v", code, out)
	}
	code, out := httpJSON(t, http.MethodGet, ts.URL+"/api/v1/me/inbox", editorTok, nil, "")
	if fq, _ := out["failed_queue"].([]any); code != 200 || len(fq) != 1 {
		t.Fatalf("inbox failed_queue: %d %v", code, out["failed_queue"])
	}
	if code, out := post(collectorTok, "reopen", `{"reason":"再试"}`); code != 403 || out["code"] != "not_hub" {
		t.Fatalf("reopen by non-hub: %d %v", code, out)
	}
	if code, out := post(editorTok, "reopen", `{"reason":"换来源"}`); code != 200 || out["task"].(map[string]any)["status"] != "ready" {
		t.Fatalf("reopen: %d %v", code, out)
	}
	code, out = post(editorTok, "cancel", `{"reason":"本期取消"}`)
	ids, _ := out["cancelled_task_ids"].([]any)
	if code != 200 || len(ids) != 10 {
		t.Fatalf("cancel with cascade: %d %v", code, out)
	}
}
