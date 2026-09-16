package notifier

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

type sent struct{ chat, text, uuid string }

type fakeSender struct {
	msgs []sent
	mu   sync.Mutex
	fail int
}

func (f *fakeSender) SendText(_ context.Context, chat, text, uuid string) (string, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.fail > 0 {
		f.fail--
		return "", errors.New("lark down")
	}
	f.msgs = append(f.msgs, sent{chat, text, uuid})
	return "om_x", nil
}

func (f *fakeSender) texts() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	out := make([]string, 0, len(f.msgs))
	for _, m := range f.msgs {
		out = append(out, m.text)
	}
	return out
}

const (
	openCollector = "ou_4df246d7bdd235a7337bfb453b38da5d"
	openEditor    = "ou_e8b7e1e4076dbfbaf88c55078794b5eb"
	openVan       = "ou_1fca25427af132224142dd3abdda906d"
)

type rig struct {
	st     *sqlite.Store
	eng    *engine.Engine
	sender *fakeSender
	n      *Notifier
	now    time.Time
	runID  int64
}

func newRig(t *testing.T, opt Options) *rig {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "n.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatal(err)
	}
	r := &rig{st: sqlite.New(db), sender: &fakeSender{}, now: time.Now()}
	r.eng = engine.New(r.st)
	opt.Now = func() time.Time { return r.now }
	r.n = New(r.st, r.eng, r.sender, opt, slog.New(slog.NewTextHandler(io.Discard, nil)))
	return r
}

func (r *rig) agent(t *testing.T, role string) (domain.Agent, domain.Role) {
	t.Helper()
	ctx := context.Background()
	a, err := r.st.Q().ActiveAgentByRole(ctx, role)
	if err != nil {
		t.Fatal(err)
	}
	ro, _ := r.st.Q().GetRole(ctx, role)
	return a, ro
}

func (r *rig) trigger(t *testing.T) {
	t.Helper()
	editor, role := r.agent(t, "editor")
	res, err := r.eng.Trigger(context.Background(), editor, role, "daily_news", "2026-09-16", "", "")
	if err != nil {
		t.Fatal(err)
	}
	r.runID = res.Run.ID
}

func (r *rig) task(t *testing.T, code string) domain.Task {
	t.Helper()
	ts, _ := r.st.Q().ListTasksByRun(context.Background(), r.runID)
	for _, tk := range ts {
		if tk.StageCode == code {
			return tk
		}
	}
	t.Fatalf("no task %s", code)
	return domain.Task{}
}

func (r *rig) flush(t *testing.T) int {
	t.Helper()
	n, err := r.n.Flush(context.Background())
	if err != nil {
		t.Fatalf("flush: %v", err)
	}
	return n
}

func TestFlushSendsDispatchWithAtTagOnce(t *testing.T) {
	r := newRig(t, Options{})
	r.trigger(t) // intake 自动派工 → dispatched 通知情报收集员
	if got := r.flush(t); got != 1 {
		t.Fatalf("want 1 sent, got %d", got)
	}
	m := r.sender.msgs[0]
	if m.chat != "oc_c7484358fbf366ba1c806e9d65db3103" || !strings.HasPrefix(m.uuid, "csw-outbox-") {
		t.Fatalf("chat/uuid: %+v", m)
	}
	if !strings.Contains(m.text, `<at user_id="`+openCollector+`">情报收集员</at>`) || !strings.Contains(m.text, "新派工") {
		t.Fatalf("text: %s", m.text)
	}
	if got := r.flush(t); got != 0 {
		t.Fatalf("same event must not be sent twice, got %d", got)
	}
}

func TestFlushRetriesWithBackoffThenDead(t *testing.T) {
	r := newRig(t, Options{MaxAttempts: 2})
	r.trigger(t)
	r.sender.fail = 5
	if got := r.flush(t); got != 0 {
		t.Fatalf("failed send counted: %d", got)
	}
	if got := r.flush(t); got != 0 { // 未到退避时间，不重领
		t.Fatalf("should wait for backoff, sent %d", got)
	}
	r.now = r.now.Add(10 * time.Second)
	r.flush(t) // 第二次失败 → 达上限转 dead
	var status string
	var attempts int
	if err := r.st.DB().QueryRow(`SELECT status, attempts FROM outbox ORDER BY id LIMIT 1`).Scan(&status, &attempts); err != nil {
		t.Fatal(err)
	}
	if status != "dead" || attempts != 2 {
		t.Fatalf("want dead after 2 attempts, got %s/%d", status, attempts)
	}
}

func TestFlushRetrySucceeds(t *testing.T) {
	r := newRig(t, Options{})
	r.trigger(t)
	r.sender.fail = 1
	r.flush(t)
	r.now = r.now.Add(6 * time.Second)
	if got := r.flush(t); got != 1 {
		t.Fatalf("retry should send, got %d", got)
	}
}

func TestDryRunDoesNotSend(t *testing.T) {
	r := newRig(t, Options{DryRun: true})
	r.trigger(t)
	if got := r.flush(t); got != 1 || len(r.sender.msgs) != 0 {
		t.Fatalf("dry-run: counted=%d sent=%d", got, len(r.sender.msgs))
	}
}

func TestStaleBacklogNotSent(t *testing.T) {
	r := newRig(t, Options{})
	r.trigger(t)
	r.now = r.now.Add(2 * time.Hour)
	if got := r.flush(t); got != 0 || len(r.sender.msgs) != 0 {
		t.Fatalf("stale backlog must not be sent: %d", got)
	}
	var status string
	_ = r.st.DB().QueryRow(`SELECT status FROM outbox ORDER BY id LIMIT 1`).Scan(&status)
	if status != "dead" {
		t.Fatalf("stale row want dead, got %s", status)
	}
}

func TestVanTargetOnlyFromGate(t *testing.T) {
	r := newRig(t, Options{})
	rs := roster{"editor": {OpenID: openEditor, Name: "主编"}, "van": {OpenID: openVan, Name: "Van"}}
	bad := domain.Outbox{EventType: domain.EvtDispatched, TargetRole: "van", PayloadJSON: `{"hub_role":"editor","run_id":1,"stage_name":"x","task_id":2}`}
	text, down := render(bad, rs)
	if !down || strings.Contains(text, openVan) || !strings.Contains(text, openEditor) {
		t.Fatalf("non-gate van target must downgrade to hub: %s", text)
	}
	ok := domain.Outbox{EventType: domain.EvtGatePassed, TargetRole: "van", CCRoles: "editor",
		PayloadJSON: `{"hub_role":"editor","run_id":1,"stage_name":"07-完整审核稿","version":2,"next_reviewer":"van","next_gate_name":"Van全文","passed_gate_name":"主编定点编辑","download_url":"https://x/v2.zip"}`}
	text, down = render(ok, rs)
	if down || !strings.Contains(text, `<at user_id="`+openVan+`">Van</at>`) || !strings.Contains(text, "抄送 <at user_id=\""+openEditor) {
		t.Fatalf("gate van target: %s", text)
	}
	_ = r
}

func TestRosterMissFallsBackToHub(t *testing.T) {
	rs := roster{"editor": {OpenID: openEditor, Name: "主编"}}
	o := domain.Outbox{EventType: domain.EvtDispatched, TargetRole: "xhswriter", PayloadJSON: `{"hub_role":"editor","run_id":1,"stage_name":"10-小红书改编","task_id":9}`}
	text, _ := render(o, rs)
	if !strings.Contains(text, "花名册无此角色") || !strings.Contains(text, openEditor) {
		t.Fatalf("roster miss should cc hub: %s", text)
	}
}

func TestOverdueScanNotifiesOnce(t *testing.T) {
	r := newRig(t, Options{})
	r.trigger(t)
	r.flush(t)
	intake := r.task(t, "intake")
	if _, err := r.st.DB().Exec(`UPDATE tasks SET due_at='2000-01-01T00:00:00Z' WHERE id=?`, intake.ID); err != nil {
		t.Fatal(err)
	}
	if n, err := r.n.ScanOverdue(context.Background()); err != nil || n != 1 {
		t.Fatalf("scan: n=%d err=%v", n, err)
	}
	if n, _ := r.n.ScanOverdue(context.Background()); n != 0 {
		t.Fatalf("overdue must be notified once, got %d", n)
	}
	if got := r.flush(t); got != 1 {
		t.Fatalf("overdue message: %d", got)
	}
	last := r.sender.texts()[len(r.sender.msgs)-1]
	if !strings.Contains(last, "已逾期") || !strings.Contains(last, openCollector) || !strings.Contains(last, "抄送 <at user_id=\""+openEditor) {
		t.Fatalf("overdue text: %s", last)
	}
}

func TestProgressCardAfterStagePassedOnce(t *testing.T) {
	r := newRig(t, Options{})
	r.trigger(t)
	r.flush(t)
	collector, _ := r.agent(t, "collector")
	if _, err := r.eng.Submit(context.Background(), collector, r.task(t, "intake").ID, engine.SubmitInput{DownloadURL: "http://x/intake"}); err != nil {
		t.Fatal(err)
	}
	// v4 起 01 有主编首批校准闸：先过闸，才会 stage_passed 并派 02。
	editor, editorRole := r.agent(t, "editor")
	intake := r.task(t, "intake")
	dl, err := r.st.Q().ListDeliverablesByTask(context.Background(), intake.ID)
	if err != nil {
		t.Fatal(err)
	}
	var out domain.Deliverable
	for _, d := range dl {
		if d.Kind != domain.KindDispatch {
			out = d
		}
	}
	if _, err := r.eng.Review(context.Background(), editor, editorRole, out.ID,
		engine.ReviewInput{Verdict: domain.VerdictPass, Comment: "方向可以"}); err != nil {
		t.Fatal(err)
	}
	r.flush(t) // stage_passed + shortlist dispatched → 一张进度卡
	cards := 0
	for _, s := range r.sender.texts() {
		if strings.Contains(s, "进度】") {
			cards++
			if !strings.Contains(s, "已交付 1/11") {
				t.Fatalf("progress card: %s", s)
			}
		}
	}
	if cards != 1 {
		t.Fatalf("want exactly 1 progress card, got %d: %v", cards, r.sender.texts())
	}
	// 摘要未变不再发。
	n := len(r.sender.msgs)
	r.n.progressCard(context.Background(), domain.Chat{ChatKey: "oc"}, r.runID)
	if len(r.sender.msgs) != n {
		t.Fatalf("unchanged digest must not resend progress card")
	}
}

func TestStallScanRemindsThenEscalates(t *testing.T) {
	r := newRig(t, Options{})
	r.trigger(t)
	r.flush(t)
	intake := r.task(t, "intake")
	if _, err := r.st.DB().Exec(`UPDATE tasks SET dispatched_at=strftime('%Y-%m-%dT%H:%M:%SZ','now','-20 minutes') WHERE id=?`, intake.ID); err != nil {
		t.Fatal(err)
	}
	// 第一轮只提醒执行者（升级须在提醒之后）；第二轮升级主编；之后不再重复。
	for i, want := range []int{1, 1, 0} {
		if n, err := r.n.ScanStalls(context.Background()); err != nil || n != want {
			t.Fatalf("scan %d: n=%d want %d err=%v", i+1, n, want, err)
		}
	}
	if got := r.flush(t); got != 2 {
		t.Fatalf("stall messages: %d", got)
	}
	texts := r.sender.texts()
	remind, escalate := texts[len(texts)-2], texts[len(texts)-1]
	if !strings.Contains(remind, "仍未接单：请先接单") || !strings.Contains(remind, openCollector) {
		t.Fatalf("remind text: %s", remind)
	}
	if !strings.Contains(escalate, "请改派、重开或取消") || !strings.Contains(escalate, openEditor) {
		t.Fatalf("escalate text: %s", escalate)
	}
}
