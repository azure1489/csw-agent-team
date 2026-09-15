package ledger

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func TestSplitWechat(t *testing.T) {
	body := strings.Join([]string{
		"# 营事编集室 vol.12", "01｜HxO｜折叠木椅重新理解尺寸", "02｜NANGA｜羽绒进入城市通勤",
		"### HxO｜折叠木椅重新理解尺寸", "![](images/1.jpg)", "HxO 来自北欧，这次发布了……",
		"**NANGA｜羽绒进入城市通勤**", "正文……", "Coleman｜透明幕帘帐篷", "正文……",
	}, "\n")
	items := SplitWechat(body)
	if len(items) != 3 || items[0].Brand != "HxO" || items[1].Brand != "NANGA" || items[2].Title != "透明幕帘帐篷" {
		t.Fatalf("split: %+v", items)
	}
}

func TestParseNotesFlags(t *testing.T) {
	csv := "\ufeff笔记标题,发布时间,曝光,观看,点赞,评论,收藏,分享,涨粉,平均观看时长\n" +
		"On 穆勒鞋,2026-09-01 20:00,61200,7465,300,20,24,9,26,7\n" +
		"数字游民 OG,2026-08-20 19:00,40100,4261,500,40,181,30,18,36\n" +
		"新笔记,2026-09-14 21:00,0,120,5,1,2,0,0,0\n"
	rows, err := ParseNotes(strings.NewReader(csv))
	if err != nil || len(rows) != 3 {
		t.Fatalf("rows=%d err=%v", len(rows), err)
	}
	if rows[0].Metrics["views"] != 7465.0 || rows[0].Post.PostID == "" || !strings.HasPrefix(rows[0].Post.PostID, "csv-") {
		t.Fatalf("row0: %+v", rows[0])
	}
	if len(rows[0].Flags) != 0 || len(rows[2].Flags) != 2 || !strings.HasPrefix(rows[2].Flags[0], "refresh_pending:") {
		t.Fatalf("flags: %v / %v", rows[0].Flags, rows[2].Flags)
	}
	again, _ := ParseNotes(strings.NewReader(csv))
	if again[1].Post.PostID != rows[1].Post.PostID {
		t.Fatal("derived id must be stable across imports")
	}
}

func TestParseAccountMismatch(t *testing.T) {
	csv := "日期,曝光,观看,新增关注,取消关注,净涨粉\n" +
		"2026-08-15,15000,1800,200,30,172\n" +
		"2026-09-13,20000,2100,194,28,168\n"
	s, err := ParseAccount(strings.NewReader(csv))
	if err != nil {
		t.Fatal(err)
	}
	if s.WindowFrom != "2026-08-15" || s.WindowTo != "2026-09-13" || s.Totals["新增关注"] != 394 || s.Totals["净涨粉"] != 340 {
		t.Fatalf("summary: %+v", s)
	}
	if len(s.Flags) != 1 || !strings.Contains(s.Flags[0], "差 4") || !strings.HasPrefix(s.Flags[0], "definition_pending:") {
		t.Fatalf("net mismatch must be kept and flagged: %v", s.Flags)
	}
}

// fakeScript 模拟平台读取命令：列表含一篇已发合集和一篇草稿；账号趋势不可得（退出码 3）。
func fakeScript(t *testing.T) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "adapter.sh")
	script := `#!/bin/sh
case "$1" in
  list) printf '%s\n' '{"posts":[{"post_id":"p1","title":"营事编集室 vol.12","published_at":"2026-09-10 08:00","state":"published"},{"post_id":"d1","title":"草稿","state":"draft"}]}' ;;
  get) printf '%s\n' '{"post":{"post_id":"'"$2"'","title":"营事编集室 vol.12","body":"### HxO｜折叠木椅\n正文\n### NANGA｜羽绒进城\n正文"}}' ;;
  metrics) printf '%s\n' '{"metrics":{"views":100,"impressions":0},"definitions":{"impressions":"曝光：笔记被展示的次数"}}' ;;
  account) printf '%s\n' '{"unavailable":"账号趋势需要运营后台权限"}'; exit 3 ;;
  *) exit 1 ;;
esac
`
	if err := os.WriteFile(p, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestCommandAdapterAndProbe(t *testing.T) {
	a := &CommandAdapter{Name: "wechat", Cmd: fakeScript(t)}
	ctx := context.Background()
	if _, err := a.Account(ctx, 30); !errors.Is(err, ErrUnavailable) || !strings.Contains(err.Error(), "运营后台权限") {
		t.Fatalf("account should be unavailable, got %v", err)
	}
	rep := Probe(ctx, a, "p1")
	if rep.ExitCode() != 3 {
		t.Fatalf("partial probe should exit 3: %+v", rep)
	}
	var statuses []string
	for _, c := range rep.Checks {
		statuses = append(statuses, c.Name+"="+c.Status)
	}
	joined := strings.Join(statuses, " ")
	for _, want := range []string{"包含已知人工发布=ok", "草稿与已发分列=ok", "单篇正文=ok", "单篇指标=ok", "账号 30 天趋势=unavailable"} {
		if !strings.Contains(joined, want) {
			t.Fatalf("probe checks missing %s: %s", want, joined)
		}
	}
	if (&CommandAdapter{Name: "xhs"}).Platform() != "xhs" {
		t.Fatal("platform")
	}
	if _, err := (&CommandAdapter{Name: "xhs"}).List(ctx, TimeZero()); err == nil || !strings.Contains(err.Error(), "未配置") {
		t.Fatalf("missing command must error clearly: %v", err)
	}
}

func TestSyncAndSnapshot(t *testing.T) {
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "l.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if err := sqlite.Migrate(db); err != nil {
		t.Fatal(err)
	}
	st := sqlite.New(db)
	ctx := context.Background()
	a := &CommandAdapter{Name: "wechat", Cmd: fakeScript(t)}
	res, err := Sync(ctx, st, a, "营事编集室", "backfill", TimeZero())
	if err != nil || res.Fetched != 2 || res.Inserted != 2 || res.Split != 2 {
		t.Fatalf("backfill: %+v err=%v", res, err)
	}
	p, err := st.Q().GetLedgerPost(ctx, "wechat", "营事编集室", "p1")
	if err != nil || p.PublishedAt != "2026-09-10T00:00:00Z" || p.State != "published" {
		t.Fatalf("post: %+v err=%v", p, err)
	}
	items, _ := st.Q().ListPostItems(ctx, p.ID)
	if len(items) != 2 || items[1].Brand != "NANGA" || items[0].SplitBy != "auto" {
		t.Fatalf("auto split: %+v", items)
	}
	if d, _ := st.Q().GetLedgerPost(ctx, "wechat", "营事编集室", "d1"); d.State != "draft" {
		t.Fatalf("draft must stay draft, got %s", d.State)
	}
	res, err = Sync(ctx, st, a, "营事编集室", "sync", TimeZero())
	if err != nil || res.Inserted != 0 || res.Updated != 2 || res.Split != 0 {
		t.Fatalf("resync must not duplicate or re-split: %+v err=%v", res, err)
	}
	n, gaps, err := Snapshot(ctx, st, a, "24h")
	if err != nil || n != 1 || len(gaps) != 0 {
		t.Fatalf("snapshot: n=%d gaps=%v err=%v", n, gaps, err)
	}
	snaps, _ := st.Q().ListMetricSnapshots(ctx, p.ID)
	if len(snaps) != 1 || !strings.Contains(snaps[0].FlagsJSON, "refresh_pending") || !strings.Contains(snaps[0].RawJSON, `"impressions":0`) ||
		!strings.Contains(snaps[0].DefinitionsJSON, "曝光：笔记被展示的次数") {
		t.Fatalf("snapshot keeps raw zero and flags it: %+v", snaps)
	}
	if n, _, _ := Snapshot(ctx, st, a, "24h"); n != 0 {
		t.Fatalf("same bucket must be taken once, got %d", n)
	}
	cov, _ := st.Q().LedgerCoverage(ctx)
	if len(cov) != 1 || cov[0].Kind != "sync" || cov[0].Platform != "wechat" {
		t.Fatalf("coverage: %+v", cov)
	}
}

func scriptAdapter(t *testing.T, name, body string) *CommandAdapter {
	t.Helper()
	p := filepath.Join(t.TempDir(), name+".sh")
	if err := os.WriteFile(p, []byte("#!/bin/sh\ncase \"$1\" in\n"+body+"\n  *) exit 1 ;;\nesac\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	return &CommandAdapter{Name: "xhs", Cmd: p}
}

func ledgerStore(t *testing.T) *sqlite.Store {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "l.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatal(err)
	}
	return sqlite.New(db)
}

// TestSnapshotGapKinds 快照缺口分「平台不提供」与「采集失败」两类，都不写快照。
func TestSnapshotGapKinds(t *testing.T) {
	st, ctx := ledgerStore(t), context.Background()
	a := scriptAdapter(t, "gaps", `  list) printf '%s\n' '{"posts":[{"post_id":"p1","title":"A","published_at":"2026-09-01 08:00","state":"published"},{"post_id":"p2","title":"B","published_at":"2026-09-01 09:00","state":"published"}]}' ;;
  get) printf '%s\n' '{"post":{"post_id":"'"$2"'","title":"A","body":"正文"}}' ;;
  metrics) if [ "$2" = "p1" ]; then printf '%s\n' '{"unavailable":"该笔记不提供曝光"}'; exit 3; else echo boom >&2; exit 1; fi ;;`)
	if _, err := Sync(ctx, st, a, "营事编集室", "backfill", TimeZero()); err != nil {
		t.Fatalf("backfill: %v", err)
	}
	n, gaps, err := Snapshot(ctx, st, a, "24h")
	if err != nil || n != 0 || len(gaps) != 2 {
		t.Fatalf("snapshot: n=%d gaps=%v err=%v", n, gaps, err)
	}
	if k := GapKinds(gaps); k[GapUnavailable] != 1 || k[GapFetchFailed] != 1 {
		t.Fatalf("gap kinds: %v (%v)", k, gaps)
	}
}

// TestSnapshotAccountKeepsWindowAndDefinitions 账号级快照：区间、原值与指标定义原样保存，净涨粉口径不符标 definition_pending；
// 读不到时不写占位数据。
func TestSnapshotAccountKeepsWindowAndDefinitions(t *testing.T) {
	st, ctx := ledgerStore(t), context.Background()
	a := scriptAdapter(t, "acct", `  account) printf '%s\n' '{"window_from":"2026-08-15","window_to":"2026-09-13","raw":{"新增关注":394,"取消关注":58,"净涨粉":340,"曝光":35000},"definitions":{"净涨粉":"区间内关注数净变化（平台口径）"}}' ;;`)
	s, ok, err := SnapshotAccount(ctx, st, a, "营事编集室", 30)
	if err != nil || !ok {
		t.Fatalf("account snapshot: ok=%v err=%v", ok, err)
	}
	if s.WindowFrom != "2026-08-15" || s.WindowTo != "2026-09-13" || !strings.Contains(s.RawJSON, `"净涨粉":340`) ||
		!strings.Contains(s.DefinitionsJSON, "平台口径") || !strings.Contains(s.FlagsJSON, "definition_pending") || !strings.Contains(s.FlagsJSON, "差 4") {
		t.Fatalf("account snapshot: %+v", s)
	}
	if _, ok, err := SnapshotAccount(ctx, st, &CommandAdapter{Name: "wechat", Cmd: fakeScript(t)}, "营事编集室", 30); ok || !errors.Is(err, ErrUnavailable) {
		t.Fatalf("unavailable account must not write: ok=%v err=%v", ok, err)
	}
}
