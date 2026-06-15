package sqlite

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func rosterStore(t *testing.T) *Store {
	t.Helper()
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	return New(db)
}

func TestRosterCRUD(t *testing.T) {
	st := rosterStore(t)
	ctx := context.Background()
	q := st.Q()

	// 新建群 + mapped/reserved 成员（role 复用 seed 的 roles：editor 等已存在）。
	chatID, err := q.CreateChat(ctx, "oc_test", "测试群", "note")
	if err != nil {
		t.Fatalf("CreateChat: %v", err)
	}
	rc := "editor"
	if _, err := q.CreateChatMember(ctx, domain.ChatMember{
		ChatID: chatID, Kind: "mapped", RoleCode: &rc, OpenID: "ou_a", DisplayName: "主编", BotName: "主编", Sort: 1,
	}); err != nil {
		t.Fatalf("CreateChatMember mapped: %v", err)
	}
	if _, err := q.CreateChatMember(ctx, domain.ChatMember{
		ChatID: chatID, Kind: "reserved", OpenID: "ou_b", DisplayName: "预留bot", BotName: "预留bot", Sort: 2,
	}); err != nil {
		t.Fatalf("CreateChatMember reserved: %v", err)
	}

	members, err := q.ListChatMembers(ctx, chatID)
	if err != nil || len(members) != 2 {
		t.Fatalf("ListChatMembers: n=%d err=%v", len(members), err)
	}

	// 唯一约束：同群同 role_code 第二次应失败。
	if _, err := q.CreateChatMember(ctx, domain.ChatMember{
		ChatID: chatID, Kind: "mapped", RoleCode: &rc, OpenID: "ou_c", DisplayName: "x", BotName: "x2", Sort: 3,
	}); err == nil {
		t.Fatalf("重复 role_code 应触发唯一约束")
	}

	// 改成员。
	m := members[0]
	m.OpenID = "ou_updated"
	if err := q.UpdateChatMember(ctx, m); err != nil {
		t.Fatalf("UpdateChatMember: %v", err)
	}
	got, _ := q.GetChatMember(ctx, m.ID)
	if got.OpenID != "ou_updated" {
		t.Fatalf("update 未生效: %s", got.OpenID)
	}

	// 删成员。
	if err := q.DeleteChatMember(ctx, members[1].ID); err != nil {
		t.Fatalf("DeleteChatMember: %v", err)
	}

	// 删群 → 成员级联删。
	if err := q.DeleteChat(ctx, chatID); err != nil {
		t.Fatalf("DeleteChat: %v", err)
	}
	after, _ := q.ListChatMembers(ctx, chatID)
	if len(after) != 0 {
		t.Fatalf("删群后成员应级联删，剩 %d", len(after))
	}
}
