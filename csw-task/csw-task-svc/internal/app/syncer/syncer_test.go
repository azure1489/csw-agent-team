package syncer

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func write(t *testing.T, dir, name, body string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestSyncerImportSplitFeedback(t *testing.T) {
	dir := t.TempDir()
	cfg := config.Config{DBPath: filepath.Join(dir, "s.db"), SyncWxAccount: "营事编集室", SyncXhsAccount: "CAMPsomeWHERE"}
	run := func(args ...string) (int, string) {
		var out bytes.Buffer
		code := RunWith(args, cfg, &out, nil)
		return code, out.String()
	}
	notes := write(t, dir, "notes.csv", "笔记ID,笔记标题,发布时间,曝光,观看,收藏,涨粉\n"+
		"n1,On 穆勒鞋,2026-09-01 20:00,61200,7465,24,26\n"+
		"n2,KEEN 新配色,2026-09-05 20:00,50000,5482,25,3\n"+
		"n3,09-14 新笔记,2026-09-14 21:00,0,120,2,0\n")
	if code, out := run("import-csv", "--platform", "xhs", "--kind", "notes", "--file", notes); code != 0 || !strings.Contains(out, "导入 3 条") || !strings.Contains(out, "1 条带异常标记") {
		t.Fatalf("import notes: %d %s", code, out)
	}
	if code, out := run("import-csv", "--platform", "xhs", "--kind", "notes", "--file", notes); code != 0 || !strings.Contains(out, "新增 0，更新 3") {
		t.Fatalf("re-import must not duplicate: %d %s", code, out)
	}
	acct := write(t, dir, "acct.csv", "日期,新增关注,取消关注,净涨粉\n2026-08-15,200,30,172\n2026-09-13,194,28,168\n")
	if code, out := run("import-csv", "--platform", "xhs", "--kind", "account", "--file", acct); code != 0 || !strings.Contains(out, "差 4") {
		t.Fatalf("import account: %d %s", code, out)
	}
	if code, out := run("split", "--platform", "xhs", "--post", "n1", "--manual", write(t, dir, "items.json", `[{"brand":"On","title":"穆勒鞋"}]`), "--by", "主编"); code != 0 {
		t.Fatalf("manual split: %d %s", code, out)
	}
	fb := write(t, dir, "fb.jsonl", `{"quote":"Coleman 这个透明幕帘不好看","kind":"case","tags":["brand:coleman","stage:topic"],"source_ref":"9/14 群聊"}`+"\n"+
		`{"quote":"本周先别推 NANGA","kind":"temporary","expires_at":"2026-09-01T00:00:00Z","tags":["brand:nanga"]}`+"\n"+"not json\n")
	if code, out := run("feedback", "import", fb); code != 0 || !strings.Contains(out, "导入编辑反馈 2 条") || !strings.Contains(out, "第 3 行无效") {
		t.Fatalf("feedback import: %d %s", code, out)
	}
	if code, out := run("feedback", "expire"); code != 0 || !strings.Contains(out, "已置过期 1 条") {
		t.Fatalf("feedback expire: %d %s", code, out)
	}
	db, _ := sqlite.Open(cfg.DBPath)
	defer db.Close()
	q := sqlite.New(db).Q()
	fs, _ := q.ListActiveFeedback(context.Background(), []string{"brand:coleman", "brand:nanga"}, 10)
	if len(fs) != 1 || fs[0].Kind != "case" {
		t.Fatalf("expired temporary feedback must not be served: %+v", fs)
	}
	posts, _ := q.ListLedgerPosts(context.Background(), sqlite.LedgerFilter{Platform: "xhs"})
	if len(posts) != 3 {
		t.Fatalf("xhs posts: %d", len(posts))
	}
	if code, _ := run("backfill", "--platform", "xhs"); code != 1 {
		t.Fatalf("backfill without adapter command must fail clearly, got %d", code)
	}
	if code, _ := run("nope"); code != 2 {
		t.Fatalf("unknown command must be usage error")
	}
}

// TestSyncerKBImport 范例导入：进发布记录表但标 is_reference=1，拆条按人标的写。
//
// 为什么要标出来：「正式已发布」与「范例」是判断时的两类对照材料，
// 混成一类就分不清「这事我们发过」和「这种写法我们认可」。
func TestSyncerKBImport(t *testing.T) {
	dir := t.TempDir()
	cfg := config.Config{DBPath: filepath.Join(dir, "kb.db")}
	run := func(args ...string) (int, string) {
		var out bytes.Buffer
		code := RunWith(args, cfg, &out, nil)
		return code, out.String()
	}
	jsonl := strings.Join([]string{
		`{"platform":"wechat","account":"营事编集室","post_id":"abc123","url":"https://mp.weixin.qq.com/s/abc123",` +
			`"published_at":"2026-09-03","title":"范例一","body_text":"正文内容够长可以入库",` +
			`"items":[{"seq":1,"brand":"Snow Peak","product":"Capture L","angle":"联名"},` +
			`{"seq":2,"brand":"KEEN","product":"凉鞋","angle":"改款"}]}`,
		`{"platform":"wechat","account":"营事编集室","post_id":"def456","title":"范例二","body_text":"另一篇正文"}`,
		`   `,
		`{"platform":"wechat","post_id":"","body_text":"缺 post_id"}`,
	}, "\n")
	path := write(t, dir, "kb_import.jsonl", jsonl)

	code, out := run("kb-import", path)
	if code != 0 || !strings.Contains(out, "2 篇、2 条拆条") {
		t.Fatalf("kb-import: %d %s", code, out)
	}
	if !strings.Contains(out, "跳过 1 行") {
		t.Fatalf("缺 post_id 的那行应当被跳过并计数：%s", out)
	}

	db, err := sqlite.Open(cfg.DBPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	st := sqlite.New(db)
	ctx := context.Background()
	p, err := st.Q().GetLedgerPost(ctx, "wechat", "营事编集室", "abc123")
	if err != nil {
		t.Fatalf("get post: %v", err)
	}
	if !p.IsReference {
		t.Fatal("范例应当标 is_reference")
	}
	if p.Source != "import" || p.PublishEvidence == "" {
		t.Fatalf("来源与发布凭据不对：%+v", p)
	}
	items, _ := st.Q().ListPostItems(ctx, p.ID)
	if len(items) != 2 || items[0].SplitBy != "manual" {
		t.Fatalf("拆条应是人标的两条：%+v", items)
	}

	// 幂等：再跑一次不重复建、不重复拆
	if code, _ := run("kb-import", path); code != 0 {
		t.Fatal("重跑应当成功")
	}
	var posts, its int
	_ = db.QueryRow(`SELECT count(*) FROM ledger_published_posts`).Scan(&posts)
	_ = db.QueryRow(`SELECT count(*) FROM ledger_post_items`).Scan(&its)
	if posts != 2 || its != 2 {
		t.Fatalf("重跑后应仍是 2 篇 2 条，实为 %d 篇 %d 条", posts, its)
	}
}
