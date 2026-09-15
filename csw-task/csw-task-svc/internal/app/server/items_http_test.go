package server

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"testing"
)

// TestItemEndpoints 条目登记 / 查询 / 决定 / 结束 run 的 HTTP 面（daily_news v2）。
func TestItemEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	if err := st.Q().ActivateWorkflowVersion(ctx, "daily_news", 2); err != nil {
		t.Fatal(err)
	}
	editorTok, collectorTok := mint("editor"), mint("collector")
	editor, _ := st.Q().ActiveAgentByRole(ctx, "editor")
	role, _ := st.Q().GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-16", "", `{"目标":{"主选":6,"备选":2}}`)
	if err != nil {
		t.Fatal(err)
	}
	base := fmt.Sprintf("%s/api/v1/runs/%d", ts.URL, res.Run.ID)
	body := `{"items":[{"item_key":"hxo-3fa91c","title":"HxO 折叠木椅","brand":"HxO","source_url":"https://example.com/hxo"}]}`
	if code, out := httpJSON(t, http.MethodPut, base+"/items", collectorTok, bytes.NewBufferString(body), "application/json"); code != 200 {
		t.Fatalf("upsert: %d %v", code, out)
	}
	code, out := httpJSON(t, http.MethodGet, base+"/items", collectorTok, nil, "")
	if code != 200 || out["target_count"].(float64) != 6 || out["gap"].(float64) != 6 || len(out["items"].([]any)) != 1 {
		t.Fatalf("list: %d %v", code, out)
	}
	if code, out := httpJSON(t, http.MethodPost, base+"/items/hxo-3fa91c/decision", collectorTok,
		bytes.NewBufferString(`{"decision":"approve_write","source_quote":"Van：写"}`), "application/json"); code != 403 {
		t.Fatalf("non-hub decision: %d %v", code, out)
	}
	code, out = httpJSON(t, http.MethodPost, base+"/items/hxo-3fa91c/decision", editorTok,
		bytes.NewBufferString(`{"decision":"approve_write","source_quote":"Van：HxO 可以写"}`), "application/json")
	if code != 200 || len(out["spawned_task_ids"].([]any)) != 2 || out["item"].(map[string]any)["status"] != "approved_write" {
		t.Fatalf("decision: %d %v", code, out)
	}
	if code, out := httpJSON(t, http.MethodPost, base+"/close", editorTok, bytes.NewBufferString(`{"reason":"收工"}`), "application/json"); code != 409 || out["code"] != "run_has_open_tasks" {
		t.Fatalf("close with open tasks: %d %v", code, out)
	}
}
