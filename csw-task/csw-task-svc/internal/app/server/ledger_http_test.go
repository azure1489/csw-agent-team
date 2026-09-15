package server

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func TestLedgerAndFeedbackEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	researcherTok, collectorTok, editorTok := mint("researcher"), mint("collector"), mint("editor")
	get := func(tok, path string) map[string]any {
		t.Helper()
		code, out := httpJSON(t, http.MethodGet, ts.URL+path, tok, nil, "")
		if code != 200 {
			t.Fatalf("GET %s: %d %v", path, code, out)
		}
		return out
	}
	out := get(researcherTok, "/api/v1/ledger/posts?brand=nanga")
	if out["verdict"] != "not_found_in_synced_records" || !strings.Contains(out["note"].(string), "还没有任何成功的同步") {
		t.Fatalf("empty ledger: %v", out)
	}
	q := st.Q()
	runID, _ := q.StartSyncRun(ctx, "wechat", "backfill", "2026-08-16T00:00:00Z", "2026-09-15T00:00:00Z")
	_ = q.FinishSyncRun(ctx, domain.LedgerSyncRun{ID: runID, OK: true, Fetched: 1, Inserted: 1})
	id, _, _ := q.UpsertLedgerPost(ctx, domain.LedgerPost{Platform: "wechat", Account: "营事编集室", PostID: "p1", Title: "营事编集室 vol.10",
		PublishedAt: "2026-09-08T00:00:00Z", State: "published"})
	_ = q.ReplacePostItems(ctx, id, []domain.LedgerPostItem{{Brand: "NANGA", Title: "羽绒进城"}}, "auto", "")
	out = get(researcherTok, "/api/v1/ledger/posts?brand=nanga&since=30d")
	posts := out["posts"].([]any)
	if out["verdict"] != "found" || len(posts) != 1 || len(posts[0].(map[string]any)["items"].([]any)) != 1 {
		t.Fatalf("found: %v", out)
	}
	out = get(researcherTok, "/api/v1/ledger/posts?brand=coleman")
	if out["verdict"] != "not_found_in_synced_records" || !strings.Contains(out["note"].(string), "不代表从未发布") || len(out["coverage"].([]any)) != 1 {
		t.Fatalf("not found with coverage: %v", out)
	}

	body := `{"quote":"采集阶段别再抓通稿","kind":"long_term","tags":["stage:collect"]}`
	if code, _ := httpJSON(t, http.MethodPost, ts.URL+"/api/v1/memory/feedback", collectorTok, bytes.NewBufferString(body), "application/json"); code != 403 {
		t.Fatalf("non-management must not write feedback: %d", code)
	}
	if code, out := httpJSON(t, http.MethodPost, ts.URL+"/api/v1/memory/feedback", editorTok, bytes.NewBufferString(`{"quote":"x","kind":"temporary"}`), "application/json"); code != 400 || out["code"] != "expires_required" {
		t.Fatalf("temporary without expiry: %d %v", code, out)
	}
	if code, out := httpJSON(t, http.MethodPost, ts.URL+"/api/v1/memory/feedback", editorTok, bytes.NewBufferString(body), "application/json"); code != 201 {
		t.Fatalf("create feedback: %d %v", code, out)
	}
	if fb := get(collectorTok, "/api/v1/memory/feedback?tags=stage:collect")["feedback"].([]any); len(fb) != 1 {
		t.Fatalf("list feedback: %v", fb)
	}
	// 任务详情随附相关反馈（v1 的 01-采集 阶段 code 为 collect）。
	editor, _ := q.ActiveAgentByRole(ctx, "editor")
	role, _ := q.GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-16", "", "")
	if err != nil {
		t.Fatal(err)
	}
	var collectID int64
	for _, tk := range res.Tasks {
		if tk.StageCode == "collect" {
			collectID = tk.ID
		}
	}
	detail := get(collectorTok, fmt.Sprintf("/api/v1/tasks/%d", collectID))
	if fb, _ := detail["feedback"].([]any); len(fb) != 1 || fb[0].(map[string]any)["quote"] != "采集阶段别再抓通稿" {
		t.Fatalf("task detail feedback: %v", detail["feedback"])
	}
}
