package server

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"testing"
)

// TestAuthorizationEndpoints 覆盖授权录入 / 重复录入 / 列表 / 撤销与 run 详情回显的 HTTP 面。
func TestAuthorizationEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	editorTok, collectorTok := mint("editor"), mint("collector")

	editor, err := st.Q().ActiveAgentByRole(ctx, "editor")
	if err != nil {
		t.Fatal(err)
	}
	role, _ := st.Q().GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-16", "", "")
	if err != nil {
		t.Fatalf("trigger: %v", err)
	}
	base := fmt.Sprintf("%s/api/v1/runs/%d", ts.URL, res.Run.ID)
	post := func(tok, body string) (int, map[string]any) {
		return httpJSON(t, http.MethodPost, base+"/authorizations", tok, bytes.NewBufferString(body), "application/json")
	}

	if code, out := post(editorTok, `{"scope":"wx_draft"}`); code != 400 || out["code"] != "source_quote_required" {
		t.Fatalf("missing quote: %d %v", code, out)
	}
	if code, out := post(collectorTok, `{"scope":"wx_draft","source_quote":"Van：可以"}`); code != 403 || out["code"] != "not_hub" {
		t.Fatalf("non-hub: %d %v", code, out)
	}
	if code, out := post(editorTok, `{"scope":"wx_draft","source_quote":"Van：可以","expires_at":"yesterday"}`); code != 400 || out["code"] != "bad_expires_at" {
		t.Fatalf("bad expires_at: %d %v", code, out)
	}
	if code, out := post(editorTok, `{"scope":"wx_draft","source_quote":"Van：今天可以存草稿"}`); code != 201 {
		t.Fatalf("grant: %d %v", code, out)
	}
	if code, out := post(editorTok, `{"scope":"wx_draft","source_quote":"Van：重复"}`); code != 200 || out["existing"] != true {
		t.Fatalf("repeat grant: %d %v", code, out)
	}

	code, out := httpJSON(t, http.MethodGet, base, collectorTok, nil, "")
	if code != 200 {
		t.Fatalf("run detail: %d", code)
	}
	scopes, _ := out["run"].(map[string]any)["authorizations"].([]any)
	if len(scopes) != 1 || scopes[0] != "wx_draft" {
		t.Fatalf("run detail authorizations: %v", scopes)
	}

	if code, out := httpJSON(t, http.MethodDelete, base+"/authorizations/wx_draft", editorTok, nil, ""); code != 200 {
		t.Fatalf("revoke: %d %v", code, out)
	}
	code, out = httpJSON(t, http.MethodGet, base+"/authorizations", collectorTok, nil, "")
	list, _ := out["authorizations"].([]any)
	if code != 200 || len(list) != 1 || list[0].(map[string]any)["status"] != "revoked" {
		t.Fatalf("list after revoke: %d %v", code, out)
	}
}
