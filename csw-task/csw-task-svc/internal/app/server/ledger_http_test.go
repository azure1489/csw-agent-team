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
	// 知识库同步要靠 is_reference 分辨「正式已发布」与「范例」——这是两类对照材料。
	// 正文只在 include_body=1 时才给：查重那条路不需要，上千篇正文纯属浪费流量。
	p0 := posts[0].(map[string]any)
	if p0["is_reference"] != false {
		t.Fatalf("普通已发条目的 is_reference 应为 false：%v", p0)
	}
	if _, ok := p0["body_text"]; ok {
		t.Fatalf("没要正文就不该回正文：%v", p0)
	}
	refID, _, _ := q.UpsertLedgerPost(ctx, domain.LedgerPost{Platform: "wechat", Account: "营事编集室", PostID: "ref1",
		Title: "范例一则", BodyText: "范例正文", PublishedAt: "2026-09-09T00:00:00Z", State: "published",
		Source: "import", IsReference: true, PublishEvidence: "https://example.com/ref1"})
	_ = q.ReplacePostItems(ctx, refID, []domain.LedgerPostItem{{Brand: "NANGA", Title: "范例条目"}}, "manual", "")
	out = get(researcherTok, "/api/v1/ledger/posts?brand=nanga&since=30d&include_body=1")
	var ref map[string]any
	for _, x := range out["posts"].([]any) {
		if m := x.(map[string]any); m["post_id"] == "ref1" {
			ref = m
		}
	}
	if ref == nil || ref["is_reference"] != true {
		t.Fatalf("范例应当标 is_reference：%v", out["posts"])
	}
	if ref["body_text"] != "范例正文" || ref["publish_evidence"] != "https://example.com/ref1" {
		t.Fatalf("include_body=1 该回正文与发布凭据：%v", ref)
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

	// 选题记忆：准则卡与案例库。
	// **confirmed=1 是 02 / 03 取判断依据的那条路**——未确认的归纳只是我们的猜测，
	// 混进去等于以她的名义下判断。
	if err := q.UpsertSelectionRule(ctx, domain.SelectionRule{
		RuleKey: "lower-plain", Category: "lower", Text: "普通上新降低优先级",
	}); err != nil {
		t.Fatal(err)
	}
	if err := q.UpsertSelectionCase(ctx, domain.SelectionCase{
		CaseKey: "p5-a-1", Decision: "rejected", Quote: "这个不要", Brand: "Snow Peak",
	}); err != nil {
		t.Fatal(err)
	}
	if rs := get(collectorTok, "/api/v1/memory/rules")["rules"].([]any); len(rs) != 1 ||
		rs[0].(map[string]any)["confirmed_by_van"] != false {
		t.Fatalf("导进来的准则该是草稿：%v", rs)
	}
	if rs := get(collectorTok, "/api/v1/memory/rules?confirmed=1")["rules"].([]any); len(rs) != 0 {
		t.Fatalf("未确认的不该出现在 confirmed=1 里：%v", rs)
	}
	if err := q.SetRuleConfirmed(ctx, "lower-plain", true); err != nil {
		t.Fatal(err)
	}
	if rs := get(collectorTok, "/api/v1/memory/rules?confirmed=1")["rules"].([]any); len(rs) != 1 {
		t.Fatalf("确认后该拿得到：%v", rs)
	}
	cs := get(collectorTok, "/api/v1/memory/cases?decision=rejected")["cases"].([]any)
	if len(cs) != 1 || cs[0].(map[string]any)["quote"] != "这个不要" {
		t.Fatalf("案例要带着原话原样回：%v", cs)
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
