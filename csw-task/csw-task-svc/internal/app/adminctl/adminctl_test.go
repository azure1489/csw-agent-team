package adminctl

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func TestActivateWorkflowNeedsConfirmAndReason(t *testing.T) {
	ctx := context.Background()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatal(err)
	}
	st := sqlite.New(db)
	status := func(v int) string {
		var s string
		if err := db.QueryRow(`SELECT status FROM workflows WHERE wf_key='daily_news' AND version=?`, v).Scan(&s); err != nil {
			t.Fatal(err)
		}
		return s
	}
	if status(10) != "draft" {
		t.Fatalf("迁移后 v10 应是 draft：%s", status(10))
	}
	// 版本号确认不对、没写原因：都不激活
	if rc := activateWorkflow(ctx, st, []string{"daily_news", "10", "--confirm", "9", "--reason", "x"}); rc == 0 {
		t.Fatal("confirm 不对也激活了")
	}
	if rc := activateWorkflow(ctx, st, []string{"daily_news", "10", "--confirm", "10"}); rc == 0 {
		t.Fatal("没写原因也激活了")
	}
	if status(10) != "draft" {
		t.Fatal("没通过的调用不该改状态")
	}
	// 写对了：v10 是唯一 active（原 active 归档），并留审计
	if rc := activateWorkflow(ctx, st, []string{"daily_news", "10", "--confirm", "10", "--reason", "01 只审格式"}); rc != 0 {
		t.Fatalf("应激活成功，rc=%d", rc)
	}
	var active int
	if err := db.QueryRow(`SELECT COUNT(*) FROM workflows WHERE wf_key='daily_news' AND status='active'`).Scan(&active); err != nil {
		t.Fatal(err)
	}
	if status(10) != "active" || active != 1 {
		t.Fatalf("激活后 v10 应是唯一 active：v10=%s，active 共 %d", status(10), active)
	}
	var n int
	if err := db.QueryRow(`SELECT COUNT(*) FROM admin_audit WHERE action='workflow_activate' AND target='daily_news@10'`).Scan(&n); err != nil {
		t.Fatal(err)
	}
	if n != 1 {
		t.Fatalf("应有一条激活审计，got %d", n)
	}
	// 已激活的不能再激活
	if rc := activateWorkflow(ctx, st, []string{"daily_news", "10", "--confirm", "10", "--reason", "again"}); rc == 0 {
		t.Fatal("非草稿不该能激活")
	}
}
