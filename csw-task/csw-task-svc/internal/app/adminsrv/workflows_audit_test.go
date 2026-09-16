package adminsrv

import (
	"context"
	"fmt"
	"net/http"
	"strings"
	"testing"
)

// TestWorkflowAuditCarriesReason 定义变更审计带修改原因与变更摘要（target=<key>@<ver>）；只有草稿能激活；
// 前端无请求体的激活 / 复制照常工作。
func TestWorkflowAuditCarriesReason(t *testing.T) {
	ts, st, token := setupAdmin(t)
	wf, err := st.Q().ActiveWorkflowByKey(context.Background(), "daily_news")
	if err != nil {
		t.Fatal(err)
	}
	path := fmt.Sprintf("/admin/workflows/%d", wf.ID)
	var cur rtWorkflow
	adminDo(t, ts, token, http.MethodGet, path, nil, &cur)
	body := cur.toPut()
	body.Reason, body.ChangeSummary = "Van：选题时限放宽", "topic 时限 15→20"
	if code := adminDo(t, ts, token, http.MethodPut, path, body, nil); code != 200 {
		t.Fatalf("put: %d", code)
	}

	var e struct {
		Code string `json:"code"`
	}
	if code := adminDo(t, ts, token, http.MethodPost, path+"/activate", map[string]string{"reason": "x"}, &e); code != 409 || e.Code != "not_draft" {
		t.Fatalf("activate active version: %d %s", code, e.Code)
	}
	var cl struct {
		ID      int64 `json:"id"`
		Version int   `json:"version"`
	}
	if code := adminDo(t, ts, token, http.MethodPost, path+"/clone", map[string]string{"reason": "复制做实验"}, &cl); code != 201 || cl.Version != 5 {
		t.Fatalf("clone: %d v%d", code, cl.Version)
	}
	var au struct {
		Audit []struct {
			Action string `json:"action"`
			Target string `json:"target"`
			Detail string `json:"detail"`
		} `json:"audit"`
	}
	if code := adminDo(t, ts, token, http.MethodGet, "/admin/audit?target_prefix=daily_news@", nil, &au); code != 200 || len(au.Audit) < 2 {
		t.Fatalf("audit by prefix: %d %+v", code, au)
	}
	first, second := au.Audit[0], au.Audit[1]
	if first.Action != "workflow_clone" || first.Target != "daily_news@5" || !strings.Contains(first.Detail, `"from_version":4`) || !strings.Contains(first.Detail, "复制做实验") {
		t.Fatalf("clone audit: %+v", first)
	}
	if second.Action != "workflow_update" || second.Target != "daily_news@4" || !strings.Contains(second.Detail, "Van：选题时限放宽") || !strings.Contains(second.Detail, "topic 时限 15→20") {
		t.Fatalf("update audit: %+v", second)
	}
	// 前端调用不带请求体。
	if code := adminDo(t, ts, token, http.MethodPost, fmt.Sprintf("/admin/workflows/%d/activate", cl.ID), nil, nil); code != 200 {
		t.Fatalf("activate draft without body: %d", code)
	}
}
