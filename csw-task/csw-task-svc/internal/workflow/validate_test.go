package workflow

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func setupStore(t *testing.T) (*sqlite.Store, domain.Workflow) {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	st := sqlite.New(db)
	wf, err := st.Q().ActiveWorkflowByKey(context.Background(), "daily_news")
	if err != nil {
		t.Fatalf("active daily_news: %v", err)
	}
	return st, wf
}

func checkByName(rep Report, name string) (Check, bool) {
	for _, c := range rep.Checks {
		if c.Name == name {
			return c, true
		}
	}
	return Check{}, false
}

func TestValidateDailyNewsV2(t *testing.T) {
	st, wf := setupStore(t)
	rep, err := Validate(context.Background(), st.Q(), wf)
	if err != nil {
		t.Fatalf("validate: %v", err)
	}
	if len(rep.Checks) != 10 {
		t.Fatalf("want 10 checks, got %d", len(rep.Checks))
	}
	for _, c := range rep.Checks {
		if !c.OK {
			t.Fatalf("check %q failed: %s", c.Name, c.Detail)
		}
	}
}

func TestValidateRejectsBadActionClass(t *testing.T) {
	st, wf := setupStore(t)
	// local_drill 是 run 授权范围，不是平台写动作。
	if _, err := st.DB().Exec(`UPDATE workflow_stages SET action_class='platform_write:local_drill' WHERE workflow_id=? AND code='wx_save'`, wf.ID); err != nil {
		t.Fatal(err)
	}
	rep, err := Validate(context.Background(), st.Q(), wf)
	if err != nil {
		t.Fatal(err)
	}
	if c, ok := checkByName(rep, "动作类别合法"); !ok || c.OK {
		t.Fatalf("bad action_class should fail: %+v", c)
	}
}

func TestValidateRejectsPerItemWithoutMerge(t *testing.T) {
	st, wf := setupStore(t)
	// xhs_save 是终点，下游没有合流阶段。
	if _, err := st.DB().Exec(`UPDATE workflow_stages SET per_item=1 WHERE workflow_id=? AND code='xhs_save'`, wf.ID); err != nil {
		t.Fatal(err)
	}
	rep, err := Validate(context.Background(), st.Q(), wf)
	if err != nil {
		t.Fatal(err)
	}
	if c, ok := checkByName(rep, "条目级阶段下游有合流"); !ok || c.OK {
		t.Fatalf("per_item without merge downstream should fail: %+v", c)
	}
}
