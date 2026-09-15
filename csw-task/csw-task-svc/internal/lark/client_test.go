package lark

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
)

// fakeLark 模拟飞书两个端点；invalidOnce=true 时第一次发消息返回 token 失效。
func fakeLark(t *testing.T, invalidOnce bool) (*httptest.Server, *atomic.Int64, *atomic.Int64, *atomic.Value) {
	t.Helper()
	var tokenCalls, sendCalls atomic.Int64
	var last atomic.Value
	var failed atomic.Bool
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.URL.Path == "/open-apis/auth/v3/tenant_access_token/internal":
			n := tokenCalls.Add(1)
			var req map[string]string
			_ = json.NewDecoder(r.Body).Decode(&req)
			if req["app_id"] != "cli_x" || req["app_secret"] != "sec" {
				_ = json.NewEncoder(w).Encode(map[string]any{"code": 10014, "msg": "app secret invalid"})
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"code": 0, "tenant_access_token": "t-" + string(rune('0'+n)), "expire": 7200})
		case r.URL.Path == "/open-apis/im/v1/messages":
			sendCalls.Add(1)
			if r.URL.Query().Get("receive_id_type") != "chat_id" {
				t.Errorf("receive_id_type=%s", r.URL.Query().Get("receive_id_type"))
			}
			if invalidOnce && failed.CompareAndSwap(false, true) {
				w.WriteHeader(400)
				_ = json.NewEncoder(w).Encode(map[string]any{"code": 99991663, "msg": "Invalid access token"})
				return
			}
			var body map[string]string
			_ = json.NewDecoder(r.Body).Decode(&body)
			body["auth"] = r.Header.Get("Authorization")
			last.Store(body)
			_ = json.NewEncoder(w).Encode(map[string]any{"code": 0, "data": map[string]string{"message_id": "om_1"}})
		default:
			w.WriteHeader(404)
		}
	}))
	t.Cleanup(srv.Close)
	return srv, &tokenCalls, &sendCalls, &last
}

func TestSendTextCachesToken(t *testing.T) {
	srv, tokenCalls, sendCalls, last := fakeLark(t, false)
	c := New(srv.URL, "cli_x", "sec")
	ctx := context.Background()
	for i := 0; i < 2; i++ {
		id, err := c.SendText(ctx, "oc_1", `<at user_id="ou_a">主编</at> 你好`, "csw-outbox-7")
		if err != nil || id != "om_1" {
			t.Fatalf("send %d: id=%s err=%v", i, id, err)
		}
	}
	if tokenCalls.Load() != 1 || sendCalls.Load() != 2 {
		t.Fatalf("token calls=%d send calls=%d", tokenCalls.Load(), sendCalls.Load())
	}
	body := last.Load().(map[string]string)
	if body["receive_id"] != "oc_1" || body["msg_type"] != "text" || body["uuid"] != "csw-outbox-7" || body["auth"] != "Bearer t-1" {
		t.Fatalf("request body: %+v", body)
	}
	var content map[string]string
	if err := json.Unmarshal([]byte(body["content"]), &content); err != nil || !strings.Contains(content["text"], `<at user_id="ou_a">主编</at>`) {
		t.Fatalf("content: %s err=%v", body["content"], err)
	}
}

func TestSendTextRefreshesInvalidToken(t *testing.T) {
	srv, tokenCalls, sendCalls, last := fakeLark(t, true)
	c := New(srv.URL, "cli_x", "sec")
	if _, err := c.SendText(context.Background(), "oc_1", "hi", "u1"); err != nil {
		t.Fatalf("send: %v", err)
	}
	if tokenCalls.Load() != 2 || sendCalls.Load() != 2 {
		t.Fatalf("want refresh once: token=%d send=%d", tokenCalls.Load(), sendCalls.Load())
	}
	if got := last.Load().(map[string]string)["auth"]; got != "Bearer t-2" {
		t.Fatalf("retry should use refreshed token, got %s", got)
	}
}

func TestSendTextBadCredentials(t *testing.T) {
	srv, _, sendCalls, _ := fakeLark(t, false)
	c := New(srv.URL, "cli_x", "wrong")
	_, err := c.SendText(context.Background(), "oc_1", "hi", "u1")
	if err == nil || !strings.Contains(err.Error(), "10014") || sendCalls.Load() != 0 {
		t.Fatalf("want token error without sending, got %v (sends=%d)", err, sendCalls.Load())
	}
}
