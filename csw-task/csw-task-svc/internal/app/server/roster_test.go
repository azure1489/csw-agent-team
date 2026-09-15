package server

import (
	"net/http"
	"testing"
)

// TestRosterEndpoint 验证 GET /api/v1/roster 返回结构与 skill/roster.json 等价。
func TestRosterEndpoint(t *testing.T) {
	ts, _, _, mint := setupHTTP(t)
	token := mint("editor")

	code, body := httpJSON(t, "GET", ts.URL+"/api/v1/roster", token, nil, "")
	if code != http.StatusOK {
		t.Fatalf("status %d: %v", code, body)
	}

	if body["chat_id"] != "oc_c7484358fbf366ba1c806e9d65db3103" {
		t.Fatalf("chat_id=%v", body["chat_id"])
	}

	roster, _ := body["roster"].(map[string]any)
	// 0009 起三个原 reserved bot 转为 mapped：7 + 3 = 10。
	if len(roster) != 10 {
		t.Fatalf("roster 应 10 个角色，got %d", len(roster))
	}
	if x, _ := roster["xhswriter"].(map[string]any); x["bot"] != "小红书图文作者" || x["open_id"] != "ou_003deac0bece81514fca9cd9ba081139" {
		t.Fatalf("xhswriter=%v", x)
	}
	// writer ← 深度内容创作者
	w, _ := roster["writer"].(map[string]any)
	if w["bot"] != "深度内容创作者" || w["name"] != "文案" {
		t.Fatalf("writer=%v", w)
	}
	// 非 van 不带 human/user_id 键（与现有 json 一致）
	if _, ok := w["human"]; ok {
		t.Fatalf("writer 不应有 human 键")
	}
	// van 带 human + user_id
	van, _ := roster["van"].(map[string]any)
	if van["human"] != true || van["user_id"] != "ag83acf1" {
		t.Fatalf("van=%v", van)
	}

	if reserved, _ := body["reserved_bots"].(map[string]any); len(reserved) != 0 {
		t.Fatalf("reserved_bots 应为空，got %v", reserved)
	}
}
