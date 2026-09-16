package sqlite

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func adminSetup(t *testing.T) *Store {
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

func TestAdminUserAndRefresh(t *testing.T) {
	st := adminSetup(t)
	ctx := context.Background()
	q := st.Q()

	id, err := q.CreateUser(ctx, "admin", "hash1", "管理员", domain.AdminSuperadmin)
	if err != nil {
		t.Fatalf("create user: %v", err)
	}
	u, hash, err := q.GetUserByUsername(ctx, "admin")
	if err != nil || hash != "hash1" || u.ID != id || u.Role != domain.AdminSuperadmin {
		t.Fatalf("get user: u=%+v hash=%s err=%v", u, hash, err)
	}
	if n, _ := q.CountActiveSuperadmins(ctx); n != 1 {
		t.Fatalf("want 1 active superadmin, got %d", n)
	}

	// refresh 轮换：insert→byhash→revoke→byhash 失败。
	exp := time.Now().UTC().Add(time.Hour).Format(time.RFC3339)
	rid, err := q.InsertRefresh(ctx, id, "rhash", exp, "ua", "ip")
	if err != nil {
		t.Fatalf("insert refresh: %v", err)
	}
	gotID, gotUser, err := q.RefreshByHash(ctx, "rhash")
	if err != nil || gotID != rid || gotUser != id {
		t.Fatalf("refresh by hash: id=%d user=%d err=%v", gotID, gotUser, err)
	}
	if err := q.RevokeRefresh(ctx, rid); err != nil {
		t.Fatalf("revoke: %v", err)
	}
	if _, _, err := q.RefreshByHash(ctx, "rhash"); err == nil {
		t.Fatalf("revoked refresh should not resolve")
	}

	// 禁用 + 审计。
	if err := q.UpdateUser(ctx, id, "管理员", domain.AdminViewer, "disabled"); err != nil {
		t.Fatalf("update user: %v", err)
	}
	if n, _ := q.CountActiveSuperadmins(ctx); n != 0 {
		t.Fatalf("want 0 active superadmin after downgrade, got %d", n)
	}
	if err := q.InsertAudit(ctx, &id, "login", "", ""); err != nil {
		t.Fatalf("audit: %v", err)
	}
	if entries, err := q.ListAudit(ctx, nil, "", "", 10); err != nil || len(entries) != 1 {
		t.Fatalf("list audit: n=%d err=%v", len(entries), err)
	}
}

func TestWorkflowActivateArchivesOld(t *testing.T) {
	st := adminSetup(t)
	ctx := context.Background()
	q := st.Q()

	// seed: daily_news v1–v5 archived，v6 active。新建草稿即 v7。
	v2, err := q.CreateWorkflowDraft(ctx, "daily_news", "资讯日更", "editor", "manual", "editor,scheduler")
	if err != nil {
		t.Fatalf("create draft: %v", err)
	}
	wf2, _ := q.GetWorkflow(ctx, v2)
	if wf2.Version != 7 || wf2.Status != domain.WfDraft {
		t.Fatalf("new draft want version=7 draft, got v%d %s", wf2.Version, wf2.Status)
	}

	// 激活 v2：先归档同 key 其他 active，再置 active（uq_wf_active 不冲突）。
	err = st.Tx(ctx, func(qq *Queries) error {
		if err := qq.ArchiveOtherActive(ctx, "daily_news", v2); err != nil {
			return err
		}
		return qq.SetWorkflowStatus(ctx, v2, domain.WfActive)
	})
	if err != nil {
		t.Fatalf("activate v2: %v", err)
	}

	active, err := q.ActiveWorkflowByKey(ctx, "daily_news")
	if err != nil || active.ID != v2 {
		t.Fatalf("active should be v2, got id=%d err=%v", active.ID, err)
	}
	if n, _ := q.CountWorkflowsByStatus(ctx, "active"); n != 1 {
		t.Fatalf("want exactly 1 active daily_news, got %d", n)
	}

	// 整份子表 delete+reinsert 原语。
	err = st.Tx(ctx, func(qq *Queries) error {
		if err := qq.DeleteWorkflowChildren(ctx, v2); err != nil {
			return err
		}
		sid, err := qq.InsertStage(ctx, domain.Stage{WorkflowID: v2, Seq: 1, Code: "only", Name: "唯一", RoleCode: "collector",
			Instructions: "i", SelfCheckCriteria: "s", Acceptance: "a"})
		if err != nil {
			return err
		}
		return qq.InsertGate(ctx, domain.Gate{WorkflowID: v2, StageID: &sid, GateOrder: 1, ReviewerRole: "editor", Name: "审"})
	})
	if err != nil {
		t.Fatalf("replace children: %v", err)
	}
	stages, _ := q.ListStages(ctx, v2)
	if len(stages) != 1 || stages[0].Code != "only" {
		t.Fatalf("want 1 stage 'only', got %+v", stages)
	}
}
