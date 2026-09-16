package wfctl

import (
	"bytes"
	"context"
	"io"
	"log/slog"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/adminsrv"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/wfdef"
)

func adminServer(t *testing.T) (*httptest.Server, *sqlite.Store) {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "w.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatal(err)
	}
	st := sqlite.New(db)
	hash, _ := auth.HashPassword("pw-123456")
	if _, err := st.Q().CreateUser(context.Background(), "op", hash, "操作员", "operator"); err != nil {
		t.Fatal(err)
	}
	cfg := config.Config{JWTSecret: []byte("s"), AccessTTL: time.Minute, RefreshTTL: time.Hour, CORSOrigins: []string{"http://localhost"}}
	ts := httptest.NewServer(adminsrv.New(st, cfg, slog.New(slog.NewTextHandler(io.Discard, nil))).Router())
	t.Cleanup(ts.Close)
	return ts, st
}

func run(t *testing.T, ts *httptest.Server, stdin string, tty bool, args ...string) (int, string) {
	t.Helper()
	var out bytes.Buffer
	code := RunWith(args, Env{In: strings.NewReader(stdin), Out: &out, URL: ts.URL, User: "op", Pass: "pw-123456", IsTTY: tty, AllowY: true})
	return code, out.String()
}

func TestWfctlEndToEnd(t *testing.T) {
	ts, st := adminServer(t)
	dir := t.TempDir()
	file := filepath.Join(dir, "daily_news_v4.yaml")

	if code, out := run(t, ts, "", false, "export", "daily_news", "-o", dir); code != 0 || !strings.Contains(out, "daily_news@v4") {
		t.Fatalf("export: %d %s", code, out)
	}
	if _, err := os.Stat(filepath.Join(dir, "daily_news_v4", "intake.instructions.md")); err != nil {
		t.Fatalf("texts should be externalized: %v", err)
	}
	if code, out := run(t, ts, "", false, "diff", file); code != 0 || !strings.Contains(out, "无差异") {
		t.Fatalf("diff after export: %d %s", code, out)
	}

	// 改：选题时限 15→20；小红书选图包额外依赖内容整合稿。
	def, err := wfdef.Load(file)
	if err != nil {
		t.Fatal(err)
	}
	for i := range def.Stages {
		switch def.Stages[i].Code {
		case "topic":
			def.Stages[i].SLAMinutes = 20
		case "xhs_pick":
			def.Stages[i].Deps = append(def.Stages[i].Deps, "fulltext")
		}
	}
	edited := filepath.Join(dir, "edited.yaml")
	if err := wfdef.Save(def, edited, "edited"); err != nil {
		t.Fatal(err)
	}
	if code, out := run(t, ts, "", false, "diff", edited); code != 0 || !strings.Contains(out, "时限：15 → 20") || !strings.Contains(out, "+fulltext") {
		t.Fatalf("diff edited: %d %s", code, out)
	}
	if code, out := run(t, ts, "", false, "lint", edited); code != 0 {
		t.Fatalf("lint edited: %d %s", code, out)
	}
	if code, _ := run(t, ts, "", false, "push", edited, "--reason", "x"); code != 2 {
		t.Fatalf("push without --draft must be usage error, got %d", code)
	}
	code, out := run(t, ts, "", false, "push", edited, "--draft", "--reason", "Van：选题时限放宽到 20 分钟")
	if code != 0 || !strings.Contains(out, "已建草稿 daily_news@v5") {
		t.Fatalf("push: %d %s", code, out)
	}
	if code, out := run(t, ts, "", false, "validate", "daily_news@5"); code != 0 {
		t.Fatalf("validate: %d %s", code, out)
	}
	if code, out := run(t, ts, "", false, "activate", "daily_news@5", "--reason", "Van：激活"); code != 2 || !strings.Contains(out, "终端") {
		t.Fatalf("activate without tty must require a human: %d %s", code, out)
	}
	if code, _ := run(t, ts, "daily_news@v9\n", true, "activate", "daily_news@5", "--reason", "Van：激活"); code != 2 {
		t.Fatalf("wrong confirmation must not activate, got %d", code)
	}
	if code, out := run(t, ts, "daily_news@v5\n", true, "activate", "daily_news@5", "--reason", "Van：激活"); code != 0 || !strings.Contains(out, "已激活") {
		t.Fatalf("activate with confirmation: %d %s", code, out)
	}
	active, _ := st.Q().ActiveWorkflowByKey(context.Background(), "daily_news")
	if active.Version != 5 {
		t.Fatalf("active want v5, got v%d", active.Version)
	}
	if code, out := run(t, ts, "", false, "history", "daily_news"); code != 0 || !strings.Contains(out, "workflow_activate") || !strings.Contains(out, "Van：选题时限放宽到 20 分钟") {
		t.Fatalf("history: %d %s", code, out)
	}
	if code, out := run(t, ts, "", false, "rollback", "daily_news@4", "--reason", "回到原时限"); code != 0 || !strings.Contains(out, "草稿 v6") {
		t.Fatalf("rollback: %d %s", code, out)
	}
	v4, _ := st.Q().ListWorkflowsAll(context.Background(), "daily_news", "draft")
	if len(v4) != 1 || v4[0].Version != 6 {
		t.Fatalf("rollback draft: %+v", v4)
	}
	stages, _ := st.Q().ListStages(context.Background(), v4[0].ID)
	for _, s := range stages {
		if s.Code == "topic" && s.SLAMinutes != 15 {
			t.Fatalf("rollback should carry v4 content, topic sla=%d", s.SLAMinutes)
		}
	}
	if code, _ := run(t, ts, "", false, "activate", "daily_news@4", "--reason", "x", "--yes"); code != 1 {
		t.Fatalf("archived version must not activate, got %d", code)
	}
	if code, out := run(t, ts, "", false, "roles"); code != 0 || !strings.Contains(out, "xhswriter") {
		t.Fatalf("roles: %d %s", code, out)
	}
}

func TestWfctlLintBlocksPush(t *testing.T) {
	ts, _ := adminServer(t)
	dir := t.TempDir()
	if code, out := run(t, ts, "", false, "export", "daily_news", "-o", dir); code != 0 {
		t.Fatalf("export: %d %s", code, out)
	}
	file := filepath.Join(dir, "daily_news_v4.yaml")
	def, _ := wfdef.Load(file)
	for i := range def.Stages {
		switch def.Stages[i].Code {
		case "intake":
			def.Stages[i].Gates = nil
		case "wx_save":
			def.Stages[i].ActionClass = "platform_write:local_drill"
		}
	}
	bad := filepath.Join(dir, "bad.yaml")
	if err := wfdef.Save(def, bad, "bad"); err != nil {
		t.Fatal(err)
	}
	code, out := run(t, ts, "", false, "lint", bad)
	if code != 1 || !strings.Contains(out, "[L1]") || !strings.Contains(out, "动作类别合法") {
		t.Fatalf("lint errors: %d %s", code, out)
	}
	if code, out := run(t, ts, "", false, "push", bad, "--draft", "--reason", "x"); code != 1 || !strings.Contains(out, "未写入") {
		t.Fatalf("push with lint errors: %d %s", code, out)
	}
}
