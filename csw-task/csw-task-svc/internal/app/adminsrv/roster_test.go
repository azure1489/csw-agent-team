package adminsrv

import (
	"io"
	"log/slog"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
)

// TestRouterBuilds 确认含花名册路由的 Router() 注册无冲突（不 panic）。
func TestRouterBuilds(t *testing.T) {
	defer func() {
		if r := recover(); r != nil {
			t.Fatalf("Router() panic（路由冲突？）：%v", r)
		}
	}()
	s := New(nil, config.Config{CORSOrigins: []string{"http://localhost:5173"}}, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if s.Router() == nil {
		t.Fatal("nil router")
	}
}

// TestMemberReqValidation 校验 mapped 必填 role_code、reserved 不带 role_code、空串转 nil。
func TestMemberReqValidation(t *testing.T) {
	// mapped 缺 role_code → 报错
	if _, err := (memberReq{Kind: "mapped", OpenID: "ou_a", DisplayName: "x", BotName: "x"}).toMember(1); err == nil {
		t.Fatal("mapped 缺 role_code 应报错")
	}
	// 非法 kind → 报错
	if _, err := (memberReq{Kind: "weird", OpenID: "ou_a", DisplayName: "x", BotName: "x"}).toMember(1); err == nil {
		t.Fatal("非法 kind 应报错")
	}
	// mapped 正常
	m, err := (memberReq{Kind: "mapped", RoleCode: "editor", OpenID: "ou_a", DisplayName: "主编", BotName: "主编"}).toMember(1)
	if err != nil || m.RoleCode == nil || *m.RoleCode != "editor" || m.UserID != nil {
		t.Fatalf("mapped: m=%+v err=%v", m, err)
	}
	// reserved：role_code 强制 nil，即便传了
	r, err := (memberReq{Kind: "reserved", RoleCode: "editor", OpenID: "ou_b", DisplayName: "bot", BotName: "bot"}).toMember(1)
	if err != nil || r.RoleCode != nil {
		t.Fatalf("reserved 应忽略 role_code: r=%+v err=%v", r, err)
	}
	// kind 缺省=mapped；user_id 非空保留
	v, err := (memberReq{RoleCode: "van", OpenID: "ou_v", UserID: "ag1", DisplayName: "Van", BotName: "Van", IsHuman: true}).toMember(1)
	if err != nil || v.Kind != "mapped" || v.UserID == nil || *v.UserID != "ag1" {
		t.Fatalf("van: v=%+v err=%v", v, err)
	}
}
