package sqlite

import (
	"path/filepath"
	"strings"
	"testing"
)

// TestMigrate 验证全部迁移可干净应用，并抽查关键表/约束存在。
func TestMigrate(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()

	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}

	// 20 张原始业务表 + 0008 花名册 2 张（chats / chat_members）+ 0010 run_authorizations；不含 goose 版本表。
	var n int
	if err := db.QueryRow(
		`SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name <> 'goose_db_version'`,
	).Scan(&n); err != nil {
		t.Fatalf("count tables: %v", err)
	}
	// 0014 outbox、0015 run_items、0020–0022 数据子系统 8 张、0028 采集留痕 3 张、
	// 0036 intake_judgements、0039 选题记忆 2 张、0042 review_reminders。
	if n != 40 {
		t.Fatalf("want 40 tables, got %d", n)
	}

	// 0009 新增列：阶段四列 + 任务快照三列。
	for _, tc := range []struct{ table, col string }{
		{"workflow_stages", "dispatch_mode"}, {"workflow_stages", "action_class"},
		{"workflow_stages", "sla_minutes"}, {"workflow_stages", "per_item"},
		{"tasks", "dispatch_mode"}, {"tasks", "action_class"}, {"tasks", "sla_minutes"},
		// 0011 tasks 重建带入的列。
		{"tasks", "item_key"}, {"tasks", "fail_reason"}, {"tasks", "last_activity_at"},
		{"tasks", "overdue_notified_at"}, {"tasks", "rework_pending"},
		// 0037 run_items 溯源两列、0038 台账两列。
		{"run_items", "origin"}, {"run_items", "first_seen_at"},
		{"ledger_published_posts", "publish_evidence"}, {"ledger_published_posts", "is_reference"},
		// 0012 deliverables 重建、0013 reviews 扩列。
		{"deliverables", "kind"}, {"deliverables", "affects_deliverable_id"}, {"deliverables", "edit_of"},
		{"deliverables", "diff_summary"}, {"deliverables", "collab"},
		{"reviews", "decision_type"}, {"reviews", "source_quote"}, {"reviews", "items_json"}, {"reviews", "expected_version"},
		{"runs", "target_count"}, {"tasks", "wait_item_stages"},
		// 0023 接续告警。
		{"workflow_stages", "ack_minutes"}, {"workflow_stages", "idle_minutes"}, {"tasks", "ack_minutes"}, {"tasks", "idle_minutes"},
		{"run_items", "discovered_via"}, {"run_items", "fetched_at"},
		{"run_items", "evidence_url"}, {"run_items", "dedup_note"},
		{"intake_sources", "required"}, {"intake_sweeps", "sweep_key"}, {"item_traces", "reason_code"},
		{"tasks", "ack_notified_at"}, {"tasks", "ack_escalated_at"}, {"tasks", "idle_notified_at"},
	} {
		var c int
		if err := db.QueryRow(`SELECT count(*) FROM pragma_table_info(?) WHERE name=?`, tc.table, tc.col).Scan(&c); err != nil || c != 1 {
			t.Fatalf("missing column %s.%s (n=%d err=%v)", tc.table, tc.col, c, err)
		}
	}

	// 抽查部分唯一索引（约束硬化）存在。
	for _, idx := range []string{"uq_wf_gates_default", "uq_wf_gates_override", "uq_wf_active"} {
		var name string
		if err := db.QueryRow(`SELECT name FROM sqlite_master WHERE type='index' AND name=?`, idx).Scan(&name); err != nil {
			t.Fatalf("missing index %s: %v", idx, err)
		}
	}
}

// TestSeedDailyNews 验证 daily_news 定义 seed 完整。
func TestSeedDailyNews(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}

	var wfID int
	var status string
	if err := db.QueryRow(`SELECT id, status FROM workflows WHERE wf_key='daily_news' AND version=1`).Scan(&wfID, &status); err != nil {
		t.Fatalf("daily_news workflow: %v", err)
	}
	if status != "archived" {
		t.Fatalf("v1 want archived after 0009, got %s", status)
	}

	// 10 阶段，且三段内容全部非空。
	var stages, empties int
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?`, wfID).Scan(&stages); err != nil {
		t.Fatalf("stages count: %v", err)
	}
	if stages != 10 {
		t.Fatalf("want 10 stages, got %d", stages)
	}
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?
		AND (instructions IS NULL OR instructions='' OR self_check_criteria IS NULL OR self_check_criteria=''
		     OR acceptance IS NULL OR acceptance='')`, wfID).Scan(&empties); err != nil {
		t.Fatalf("empty content count: %v", err)
	}
	if empties != 0 {
		t.Fatalf("want all stages with non-empty instructions/self_check/acceptance, got %d empty", empties)
	}

	// 11 条依赖边、2 个合流阶段、2 道默认闸。
	var deps, merges, gates int
	db.QueryRow(`SELECT count(*) FROM workflow_stage_deps WHERE workflow_id=?`, wfID).Scan(&deps)
	db.QueryRow(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND is_merge=1`, wfID).Scan(&merges)
	db.QueryRow(`SELECT count(*) FROM workflow_gates WHERE workflow_id=? AND stage_id IS NULL`, wfID).Scan(&gates)
	if deps != 11 {
		t.Fatalf("want 11 deps, got %d", deps)
	}
	if merges != 2 {
		t.Fatalf("want 2 merge stages, got %d", merges)
	}
	if gates != 2 {
		t.Fatalf("want 2 default gates, got %d", gates)
	}

	// 0007 · CAMPsomeWHERE 业务规范已同步进定义（抽查关键词，防文案迁移回归）。
	for _, tc := range []struct{ code, col, want string }{
		{"topic", "instructions", "传播潜力"},
		{"topic", "self_check_criteria", "S/A/B"},
		{"wx_content", "instructions", "三段式"},
		{"wx_content", "instructions", "推文标题方案"},
		{"wx_content", "acceptance", "无公关稿语言"},
		{"wx_visual", "instructions", "图片使用原则"},
		{"wx_visual", "acceptance", "禁止重绘装备主体"},
		{"xhs_text", "instructions", "户外内容平台 CAMPsomeWHERE"},
		{"xhs_text", "acceptance", "18 字以内"},
		{"xhs_visual", "instructions", "杂志感"},
		{"xhs_visual", "acceptance", "禁止 AI 生成不存在的产品"},
	} {
		var n int
		q := `SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND code=? AND instr(` + tc.col + `, ?) > 0`
		if err := db.QueryRow(q, wfID, tc.code, tc.want).Scan(&n); err != nil || n != 1 {
			t.Fatalf("0007 规范缺失：%s.%s 应含 %q (n=%d err=%v)", tc.code, tc.col, tc.want, n, err)
		}
	}
}

// TestSeedDailyNewsV2 验证 0009 seed（0024 起归档，结构不变）：十三阶段、逐阶段闸（无默认闸）、
// 派工模式覆盖、平台写与条目级标记、三个新角色与花名册转正。
func TestSeedDailyNewsV2(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}

	var wfID int
	var mode, status string
	if err := db.QueryRow(`SELECT id, dispatch_mode, status FROM workflows WHERE wf_key='daily_news' AND version=2`).
		Scan(&wfID, &mode, &status); err != nil {
		t.Fatalf("daily_news v2: %v", err)
	}
	if mode != "auto" || status != "archived" {
		t.Fatalf("want v2 auto archived (v3 active since 0024), got %s %s", mode, status)
	}

	count := func(q string, args ...any) int {
		t.Helper()
		var n int
		if err := db.QueryRow(q, args...).Scan(&n); err != nil {
			t.Fatalf("%s: %v", q, err)
		}
		return n
	}
	codes := func(cond string) string {
		t.Helper()
		var s string
		if err := db.QueryRow(`SELECT COALESCE(group_concat(code, ','), '') FROM
			(SELECT code FROM workflow_stages WHERE workflow_id=? AND `+cond+` ORDER BY seq)`, wfID).Scan(&s); err != nil {
			t.Fatalf("codes %s: %v", cond, err)
		}
		return s
	}

	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?`, wfID); n != 13 {
		t.Fatalf("want 13 stages, got %d", n)
	}
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?
		AND (COALESCE(instructions,'')='' OR COALESCE(self_check_criteria,'')='' OR COALESCE(acceptance,'')='')`, wfID); n != 0 {
		t.Fatalf("%d stages with empty text", n)
	}
	// 文本不粘贴方案文档：单段不超过 2500 字、不引用方案章节号。
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?
		AND (length(instructions)>2500 OR length(self_check_criteria)>2500 OR length(acceptance)>2500
		     OR instr(instructions,'§')>0 OR instr(self_check_criteria,'§')>0 OR instr(acceptance,'§')>0)`, wfID); n != 0 {
		t.Fatalf("%d stages with oversized text or section refs", n)
	}
	if n := count(`SELECT count(*) FROM workflows WHERE id=? AND (length(common_instructions)>2500 OR length(common_acceptance)>2500)`, wfID); n != 0 {
		t.Fatalf("common text oversized")
	}
	if n := count(`SELECT count(*) FROM workflow_stage_deps WHERE workflow_id=?`, wfID); n != 15 {
		t.Fatalf("want 15 deps, got %d", n)
	}
	if got := codes("is_merge=1"); got != "topic,fulltext" {
		t.Fatalf("merge stages: %s", got)
	}
	if n := count(`SELECT count(*) FROM workflow_gates WHERE workflow_id=? AND stage_id IS NULL`, wfID); n != 0 {
		t.Fatalf("want 0 default gates, got %d", n)
	}
	rows, err := db.Query(`SELECT s.code, count(*) FROM workflow_gates g JOIN workflow_stages s ON s.id=g.stage_id
		WHERE g.workflow_id=? GROUP BY s.code`, wfID)
	if err != nil {
		t.Fatalf("gates: %v", err)
	}
	gates := map[string]int{}
	for rows.Next() {
		var code string
		var n int
		if err := rows.Scan(&code, &n); err != nil {
			t.Fatalf("scan: %v", err)
		}
		gates[code] = n
	}
	rows.Close()
	want := map[string]int{"topic": 2, "write": 1, "fulltext": 2, "wx_layout": 1, "xhs_text": 2, "xhs_visual": 1, "xhs_package": 1}
	if len(gates) != len(want) {
		t.Fatalf("gate distribution %v, want %v", gates, want)
	}
	for k, v := range want {
		if gates[k] != v {
			t.Fatalf("gate distribution %v, want %v", gates, want)
		}
	}
	// Van 闸只在选题、全文、小红书文本三处，且都由中枢代录。
	if got := codes(`id IN (SELECT stage_id FROM workflow_gates WHERE reviewer_role='van' AND relayed_by_hub=1)`); got != "topic,fulltext,xhs_text" {
		t.Fatalf("van gates at: %s", got)
	}
	if got := codes("dispatch_mode='manual'"); got != "write,xhs_text" {
		t.Fatalf("manual stages: %s", got)
	}
	if got := codes("dispatch_mode IS NOT NULL AND dispatch_mode<>'manual'"); got != "" {
		t.Fatalf("unexpected auto overrides: %s", got)
	}
	if got := codes("action_class LIKE 'platform_write:%'"); got != "wx_save,xhs_save" {
		t.Fatalf("platform_write stages: %s", got)
	}
	if got := codes("per_item=1"); got != "write,material" {
		t.Fatalf("per_item stages: %s", got)
	}
	if n := count(`SELECT sla_minutes FROM workflow_stages WHERE workflow_id=? AND code='topic'`, wfID); n != 15 {
		t.Fatalf("topic sla want 15, got %d", n)
	}
	// 阶段名不能含路径分隔符（交付物路径按阶段名派生）。
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND (instr(name,'/')>0 OR instr(name,'\')>0)`, wfID); n != 0 {
		t.Fatalf("stage names must not contain path separators")
	}

	for _, r := range []string{"xhswriter", "reviewer", "analyst"} {
		if n := count(`SELECT count(*) FROM roles r JOIN agents a ON a.role_code=r.code WHERE r.code=? AND a.active=1`, r); n != 1 {
			t.Fatalf("role/agent %s: n=%d", r, n)
		}
	}
	if n := count(`SELECT count(*) FROM chat_members WHERE kind='mapped'`); n != 10 {
		t.Fatalf("want 10 mapped chat members, got %d", n)
	}
	if n := count(`SELECT count(*) FROM chat_members WHERE kind='reserved'`); n != 0 {
		t.Fatalf("want 0 reserved chat members, got %d", n)
	}
}

// TestSeedDailyNewsV3 验证 0024：v3 为唯一 active；Van 闸只在 03 / 08 / 12；组版前移（08 即完整审核稿）；
// 小红书 11 改为收集员选图包、与 10 同依赖 08；写作自动派工；时限与接续告警阈值。
func TestSeedDailyNewsV8(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	var wfID, ver int
	if err := db.QueryRow(`SELECT id, version FROM workflows WHERE wf_key='daily_news' AND status='active'`).Scan(&wfID, &ver); err != nil {
		t.Fatalf("active daily_news: %v", err)
	}
	if ver != 8 {
		t.Fatalf("want v8 active, got v%d", ver)
	}
	count := func(q string, args ...any) int {
		t.Helper()
		var n int
		if err := db.QueryRow(q, args...).Scan(&n); err != nil {
			t.Fatalf("%s: %v", q, err)
		}
		return n
	}
	codes := func(cond string) string {
		t.Helper()
		var s string
		if err := db.QueryRow(`SELECT COALESCE(group_concat(code), '') FROM (SELECT code FROM workflow_stages WHERE workflow_id=? AND `+cond+` ORDER BY seq)`, wfID).Scan(&s); err != nil {
			t.Fatalf("codes %s: %v", cond, err)
		}
		return s
	}
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?`, wfID); n != 13 {
		t.Fatalf("want 13 stages, got %d", n)
	}
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND (
		COALESCE(instructions,'')='' OR COALESCE(self_check_criteria,'')='' OR COALESCE(acceptance,'')=''
		OR length(instructions)>2500 OR length(self_check_criteria)>2500 OR length(acceptance)>2500)`, wfID); n != 0 {
		t.Fatalf("stage texts must be non-empty and <= 2500 chars, %d bad", n)
	}
	if n := count(`SELECT count(*) FROM workflow_stage_deps WHERE workflow_id=?`, wfID); n != 15 {
		t.Fatalf("want 15 deps, got %d", n)
	}
	if n := count(`SELECT count(*) FROM workflow_gates WHERE workflow_id=? AND stage_id IS NULL`, wfID); n != 0 {
		t.Fatalf("want 0 default gates, got %d", n)
	}
	for code, want := range map[string]int{"intake": 1, "topic": 2, "write": 1, "fulltext": 1, "wx_layout": 2, "xhs_text": 1, "xhs_package": 2} {
		if n := count(`SELECT count(*) FROM workflow_gates g JOIN workflow_stages s ON s.id=g.stage_id WHERE g.workflow_id=? AND s.code=?`, wfID, code); n != want {
			t.Fatalf("%s gates: %d, want %d", code, n, want)
		}
	}
	if n := count(`SELECT count(*) FROM workflow_gates WHERE workflow_id=?`, wfID); n != 10 {
		t.Fatalf("want 10 gates, got %d", n)
	}
	if got := codes(`id IN (SELECT stage_id FROM workflow_gates WHERE reviewer_role='van' AND relayed_by_hub=1)`); got != "topic,wx_layout,xhs_package" {
		t.Fatalf("van gates at: %s", got)
	}
	if got := codes("dispatch_mode='manual'"); got != "xhs_text,xhs_pick" {
		t.Fatalf("manual stages: %s", got)
	}
	if got := codes("dispatch_mode IS NOT NULL AND dispatch_mode<>'manual'"); got != "" {
		t.Fatalf("unexpected auto overrides: %s", got)
	}
	if got := codes("per_item=1"); got != "write,material" {
		t.Fatalf("per_item stages: %s", got)
	}
	if got := codes("action_class LIKE 'platform_write:%'"); got != "wx_save,xhs_save" {
		t.Fatalf("platform_write stages: %s", got)
	}
	if got := codes("role_code='collector'"); got != "intake,material,xhs_pick" {
		t.Fatalf("collector stages: %s", got)
	}
	if got := codes("id IN (SELECT stage_id FROM workflow_stage_deps d JOIN workflow_stages u ON u.id=d.depends_on_id WHERE u.code='wx_layout')"); got != "wx_save,xhs_text,xhs_pick" {
		t.Fatalf("downstream of wx_layout: %s", got)
	}
	for code, want := range map[string]int{"intake": 40, "topic": 15, "write": 50, "design_prep": 30, "fulltext": 15, "wx_layout": 15, "wx_save": 20} {
		if n := count(`SELECT COALESCE(sla_minutes,0) FROM workflow_stages WHERE workflow_id=? AND code=?`, wfID, code); n != want {
			t.Fatalf("%s sla: %d, want %d", code, n, want)
		}
	}
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND ack_minutes=5`, wfID); n != 13 {
		t.Fatalf("ack_minutes=5 on all stages, got %d", n)
	}
	if got := codes("idle_minutes=5"); got != "wx_layout,wx_save,xhs_package,xhs_save" {
		t.Fatalf("idle 5 stages: %s", got)
	}
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND (instr(name,'/')>0 OR instr(name,'\')>0 OR instr(name,':')>0)`, wfID); n != 0 {
		t.Fatalf("stage names must not contain path separators or colons")
	}
	// 旧形态的说法不能留在 v4 文本里。
	if n := count(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND (
		instr(instructions||self_check_criteria||acceptance,'07 完整审核稿')>0 OR instr(instructions||self_check_criteria||acceptance,'小红书视觉')>0
		OR instr(instructions||self_check_criteria||acceptance,'分页文案')>0 OR instr(instructions||self_check_criteria||acceptance,'§')>0)`, wfID); n != 0 {
		t.Fatalf("%d stage texts still describe the v2 shape", n)
	}
	if n := count(`SELECT instr(common_acceptance,'11-小红书选图包') FROM workflows WHERE id=?`, wfID); n == 0 {
		t.Fatal("common acceptance should list v3 stage names")
	}
}

// TestMigrateRollback 验证 0036–0039 能干净回滚再重上。
//
// 为什么专门测这个：0036 要重建 intake_sweeps（SQLite 改不了 CHECK），
// 重建路径上一旦漏列或漏索引，正向跑得通、回滚才炸——而回滚是出事那天才走的路，
// 不能等到那天才发现。切换失败时要能五分钟回到 v8，这条测试守的就是那五分钟。
func TestMigrateRollback(t *testing.T) {
	path := filepath.Join(t.TempDir(), "rollback.db")
	db, err := Open(path)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate up: %v", err)
	}

	// 塞几行真实形态的数据，确保回滚不是在空表上跑
	// seed 迁移已经插了 daily_news 的若干版本，取当前 active 的那一版挂 run
	var wfID int64
	if err := db.QueryRow(`SELECT id FROM workflows WHERE wf_key='daily_news' AND status='active'`).Scan(&wfID); err != nil {
		t.Fatalf("找不到 active 的 daily_news: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO runs (id, workflow_id, workflow_ver, subject, status)
		VALUES (1, ?, 8, '2026-09-22', 'active')`, wfID); err != nil {
		t.Fatalf("seed run: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO intake_sweeps (run_id, sweep_key, platform, tool, found, in_window)
		VALUES (1, 'csw-window', 'instagram', 'csw_api', 438, 358)`); err != nil {
		t.Fatalf("seed sweep: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO intake_judgements (run_id, candidate_key, tier, image_seen)
		VALUES (1, 'snowpeak-abc123', 'recommend', 1)`); err != nil {
		t.Fatalf("seed judgement: %v", err)
	}

	// 口径的硬规则要能在库层面拦住：没读到实图就不许落非待核档
	if _, err := db.Exec(`INSERT INTO intake_judgements (run_id, candidate_key, tier, image_seen)
		VALUES (1, 'x-000000', 'not_recommend', 0)`); err == nil {
		t.Fatal("image_seen=0 却落了 not_recommend，CHECK 没拦住")
	}

	if err := DownTo(db, 35); err != nil {
		t.Fatalf("migrate down: %v", err)
	}
	// 回滚后 csw_api 归到 other，行不能丢
	var tool string
	var n int
	if err := db.QueryRow(`SELECT tool FROM intake_sweeps WHERE sweep_key='csw-window'`).Scan(&tool); err != nil {
		t.Fatalf("回滚后采集轮丢了: %v", err)
	}
	if tool != "other" {
		t.Fatalf("回滚后 tool 应归到 other，实为 %q", tool)
	}
	if err := db.QueryRow(`SELECT count(*) FROM sqlite_master WHERE type='table' AND name='intake_judgements'`).Scan(&n); err != nil || n != 0 {
		t.Fatalf("回滚后 intake_judgements 应当没了，count=%d err=%v", n, err)
	}
	var integrity string
	if err := db.QueryRow(`PRAGMA integrity_check`).Scan(&integrity); err != nil || integrity != "ok" {
		t.Fatalf("回滚后 integrity_check=%q err=%v", integrity, err)
	}

	// 再上一次，回到最新
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate up again: %v", err)
	}
	if err := db.QueryRow(`SELECT count(*) FROM sqlite_master WHERE type='table' AND name='intake_judgements'`).Scan(&n); err != nil || n != 1 {
		t.Fatalf("重上后 intake_judgements 应当回来，count=%d err=%v", n, err)
	}
}

// TestDailyNewsV10Draft 验证 0041：v10 从最新版本（v9）复制、只是 draft、01 换成新口径。
func TestDailyNewsV10Draft(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	var status, intake string
	if err := db.QueryRow(`SELECT w.status, s.instructions FROM workflows w
		JOIN workflow_stages s ON s.workflow_id = w.id AND s.code = 'intake'
		WHERE w.wf_key='daily_news' AND w.version = 10`).Scan(&status, &intake); err != nil {
		t.Fatalf("v10: %v", err)
	}
	if status != "draft" {
		t.Fatalf("v10 必须停在 draft，got %s", status)
	}
	for _, want := range []string{"值不值得推荐给中国户外潮流读者", "查重未确认", "不写「从……变成……」", "影响选题判断", "推荐位按选题算"} {
		if !strings.Contains(intake, want) {
			t.Fatalf("v10 的 01 少了「%s」", want)
		}
	}
	// 0043：窗口按原始披露时间判（生产约定），首次入库只作抓取证据。
	// 草稿被人在后台改过时 REPLACE 会静默对不上，这里兜住
	if !strings.Contains(intake, "**窗口以原始披露时间为准**") || strings.Contains(intake, "首次进入来源库的时间为准") {
		t.Fatal("v10 的 01 窗口口径应是原始披露时间，不是首次进入来源库")
	}
	// 0044：01 主编审核只查格式与材料，内容意见随通过意见交 02（2026-10-10 分工调整）。
	// REPLACE 对不上会静默不改，逐处兜住
	var acc, sl, slAcc, gate string
	if err := db.QueryRow(`SELECT s.acceptance FROM workflow_stages s JOIN workflows w ON w.id=s.workflow_id
		WHERE w.wf_key='daily_news' AND w.version=10 AND s.code='intake'`).Scan(&acc); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`SELECT s.instructions, s.acceptance FROM workflow_stages s JOIN workflows w ON w.id=s.workflow_id
		WHERE w.wf_key='daily_news' AND w.version=10 AND s.code='shortlist'`).Scan(&sl, &slAcc); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`SELECT g.name FROM workflow_gates g JOIN workflow_stages s ON s.id=g.stage_id
		JOIN workflows w ON w.id=g.workflow_id WHERE w.wf_key='daily_news' AND w.version=10 AND s.code='intake'`).Scan(&gate); err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct{ got, want, what string }{
		{acc, "本闸只查格式与材料是否齐全", "01 验收标准"},
		{intake, "前一天 06:00 至当天 06:00", "01 窗口"},
		{intake, "工作台不按内容意见返工", "01 先交首批"},
		{sl, "【上游主编意见】是你的起点", "02 作业内容"},
		{slAcc, "已采用派工单里【上游主编意见】", "02 验收标准"},
		{gate, "主编格式核对", "01 闸名"},
	} {
		if !strings.Contains(c.got, c.want) {
			t.Fatalf("0044 没改到 v10 的%s：缺「%s」", c.what, c.want)
		}
	}
	if strings.Contains(intake, "周一覆盖最近 72 小时") || strings.Contains(sl, "01 的主编首批校准结论是你的起点") {
		t.Fatal("0044 应替换掉旧窗口与旧的 02 起点说法")
	}
	// 已激活的 v9 不动
	var v9 string
	if err := db.QueryRow(`SELECT s.instructions FROM workflow_stages s JOIN workflows w ON w.id=s.workflow_id
		WHERE w.wf_key='daily_news' AND w.version=9 AND s.code='intake'`).Scan(&v9); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(v9, "首次进入来源库的时间为准") {
		t.Fatal("0043 只改 v10 草稿，v9 不该被改")
	}
	// 从 v9 复制：阶段数与依赖数一样
	var n9, n10, d9, d10 int
	q := func(v int, sql string) int {
		var n int
		if err := db.QueryRow(sql, v).Scan(&n); err != nil {
			t.Fatal(err)
		}
		return n
	}
	stages := `SELECT COUNT(*) FROM workflow_stages WHERE workflow_id=(SELECT id FROM workflows WHERE wf_key='daily_news' AND version=?)`
	deps := `SELECT COUNT(*) FROM workflow_stage_deps WHERE workflow_id=(SELECT id FROM workflows WHERE wf_key='daily_news' AND version=?)`
	n9, n10, d9, d10 = q(9, stages), q(10, stages), q(9, deps), q(10, deps)
	if n9 == 0 || n9 != n10 || d9 != d10 {
		t.Fatalf("v10 应照 v9 复制：阶段 %d/%d，依赖 %d/%d", n9, n10, d9, d10)
	}
	gates := `SELECT COUNT(*) FROM workflow_gates WHERE workflow_id=(SELECT id FROM workflows WHERE wf_key='daily_news' AND version=?)`
	if g9, g10 := q(9, gates), q(10, gates); g9 == 0 || g9 != g10 {
		t.Fatalf("v10 应照 v9 复制闸：%d/%d", g9, g10)
	}
	var shortlist string
	if err := db.QueryRow(`SELECT s.instructions FROM workflow_stages s JOIN workflows w ON w.id=s.workflow_id
		WHERE w.wf_key='daily_news' AND w.version=10 AND s.code='shortlist'`).Scan(&shortlist); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(shortlist, "读判断台账（v10 起）") {
		t.Fatal("02 少了读台账那一段")
	}
}

// TestDailyNewsV11Draft 验证 0045：v11 去掉 02 / 03、01 加 Van 选题闸、只是 draft；REPLACE 对不上会静默不改，逐处兜住。
func TestDailyNewsV11Draft(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	var wfID int64
	var status, common string
	if err := db.QueryRow(`SELECT id, status, common_acceptance FROM workflows WHERE wf_key='daily_news' AND version=11`).Scan(&wfID, &status, &common); err != nil {
		t.Fatalf("v11: %v", err)
	}
	if status != "draft" {
		t.Fatalf("v11 必须停在 draft，got %s", status)
	}
	if strings.Contains(common, "02-价值初筛") || strings.Contains(common, "03-选题方案") {
		t.Fatal("v11 通用验收的阶段清单应去掉 02 / 03")
	}
	var n int
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND code IN ('shortlist','topic')`, wfID).Scan(&n); err != nil || n != 0 {
		t.Fatalf("v11 不应有 shortlist / topic：%d err=%v", n, err)
	}
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages WHERE workflow_id=?`, wfID).Scan(&n); err != nil || n != 11 {
		t.Fatalf("v11 应有 11 段：%d err=%v", n, err)
	}
	// 写作、配图、版式都依赖 01。
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stage_deps x
		JOIN workflow_stages s ON s.id=x.stage_id JOIN workflow_stages d ON d.id=x.depends_on_id
		WHERE x.workflow_id=? AND d.code='intake' AND s.code IN ('write','material','design_prep')`, wfID).Scan(&n); err != nil || n != 3 {
		t.Fatalf("04 / 05 / 06 应依赖 01：%d err=%v", n, err)
	}
	var gates string
	if err := db.QueryRow(`SELECT group_concat(g.reviewer_role || ':' || g.relayed_by_hub || ':' || g.name, ',') FROM
		(SELECT g.* FROM workflow_gates g JOIN workflow_stages s ON s.id=g.stage_id WHERE g.workflow_id=? AND s.code='intake' ORDER BY g.gate_order) g`, wfID).Scan(&gates); err != nil {
		t.Fatal(err)
	}
	if gates != "editor:0:主编格式核对,van:1:Van选题" {
		t.Fatalf("01 闸应是 主编格式核对 → Van选题：%s", gates)
	}
	var instr, self, acc string
	if err := db.QueryRow(`SELECT instructions, self_check_criteria, acceptance FROM workflow_stages WHERE workflow_id=? AND code='intake'`, wfID).Scan(&instr, &self, &acc); err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"### 一次交齐（v11 起）", "### 条目状态由你落定", "### 给 Van 的选题单", "### 缺量说明", "历史选题决定（v11 起"} {
		if !strings.Contains(instr, want) {
			t.Fatalf("v11 的 01 作业内容少了「%s」", want)
		}
	}
	for _, gone := range []string{"### 先交首批", "随 02 派工单", "03 阶段的历史决定"} {
		if strings.Contains(instr, gone) {
			t.Fatalf("v11 的 01 作业内容还留着「%s」", gone)
		}
	}
	if !strings.Contains(self, "**一次交齐**") || !strings.Contains(acc, "## 验收标准（Van选题 · 主编代录）") || strings.Contains(acc, "02 派工单") {
		t.Fatal("v11 的 01 自检 / 验收没改到")
	}
	var write, fulltext, ftSelf string
	if err := db.QueryRow(`SELECT instructions FROM workflow_stages WHERE workflow_id=? AND code='write'`, wfID).Scan(&write); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`SELECT instructions, self_check_criteria FROM workflow_stages WHERE workflow_id=? AND code='fulltext'`, wfID).Scan(&fulltext, &ftSelf); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(write, "短名单") || strings.Contains(fulltext, "选题方案") || strings.Contains(ftSelf, "选题方案") {
		t.Fatal("v11 的 04 / 07 还引用短名单或选题方案")
	}
}

// TestDailyNewsV12Draft v12 只把 01 时限 40 → 75（取消工作台预取轮，2026-10-24），其余与 v11 相同。
func TestDailyNewsV12Draft(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	var v11, v12 int64
	var status string
	if err := db.QueryRow(`SELECT id FROM workflows WHERE wf_key='daily_news' AND version=11`).Scan(&v11); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`SELECT id, status FROM workflows WHERE wf_key='daily_news' AND version=12`).Scan(&v12, &status); err != nil {
		t.Fatalf("v12: %v", err)
	}
	if status != "draft" {
		t.Fatalf("v12 必须停在 draft，got %s", status)
	}
	var sla int
	if err := db.QueryRow(`SELECT sla_minutes FROM workflow_stages WHERE workflow_id=? AND code='intake'`, v12).Scan(&sla); err != nil || sla != 75 {
		t.Fatalf("v12 01 时限应为 75：%d err=%v", sla, err)
	}
	count := func(q string, id int64) int {
		var n int
		if err := db.QueryRow(q, id).Scan(&n); err != nil {
			t.Fatal(err)
		}
		return n
	}
	for _, q := range []string{
		`SELECT count(*) FROM workflow_stages WHERE workflow_id=?`,
		`SELECT count(*) FROM workflow_stage_deps WHERE workflow_id=?`,
		`SELECT count(*) FROM workflow_gates WHERE workflow_id=?`,
		`SELECT count(*) FROM workflow_stages WHERE workflow_id=? AND code!='intake' AND sla_minutes IS NOT NULL`,
	} {
		if a, b := count(q, v11), count(q, v12); a != b {
			t.Fatalf("v12 与 v11 不同：%s v11=%d v12=%d", q, a, b)
		}
	}
	var diff int
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages a JOIN workflow_stages b ON b.code=a.code
		WHERE a.workflow_id=? AND b.workflow_id=? AND (a.instructions IS NOT b.instructions OR a.acceptance IS NOT b.acceptance
		OR (a.code!='intake' AND a.sla_minutes IS NOT b.sla_minutes))`, v11, v12).Scan(&diff); err != nil || diff != 0 {
		t.Fatalf("v12 除 01 时限外应与 v11 一致：差 %d 段 err=%v", diff, err)
	}
}

// TestDailyNewsV13Draft v13 只在文字里加「图片加工一律交设计师」（2026-10-25），结构、时限与 v12 相同。
func TestDailyNewsV13Draft(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	var v12, v13 int64
	var status string
	if err := db.QueryRow(`SELECT id FROM workflows WHERE wf_key='daily_news' AND version=12`).Scan(&v12); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`SELECT id, status FROM workflows WHERE wf_key='daily_news' AND version=13`).Scan(&v13, &status); err != nil {
		t.Fatalf("v13: %v", err)
	}
	if status != "draft" {
		t.Fatalf("v13 必须停在 draft，got %s", status)
	}
	count := func(q string, id int64) int {
		var n int
		if err := db.QueryRow(q, id).Scan(&n); err != nil {
			t.Fatal(err)
		}
		return n
	}
	for _, q := range []string{
		`SELECT count(*) FROM workflow_stages WHERE workflow_id=?`,
		`SELECT count(*) FROM workflow_stage_deps WHERE workflow_id=?`,
		`SELECT count(*) FROM workflow_gates WHERE workflow_id=?`,
	} {
		if a, b := count(q, v12), count(q, v13); a != b {
			t.Fatalf("v13 与 v12 结构不同：%s v12=%d v13=%d", q, a, b)
		}
	}
	var diff int
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages a JOIN workflow_stages b ON b.code=a.code
		WHERE a.workflow_id=? AND b.workflow_id=? AND (a.seq IS NOT b.seq OR a.role_code IS NOT b.role_code
		OR a.sla_minutes IS NOT b.sla_minutes OR a.action_class IS NOT b.action_class OR a.dispatch_mode IS NOT b.dispatch_mode)`, v12, v13).Scan(&diff); err != nil || diff != 0 {
		t.Fatalf("v13 的阶段属性应与 v12 一致：差 %d 段 err=%v", diff, err)
	}
	field := func(code, col string) string {
		var s string
		if err := db.QueryRow(`SELECT `+col+` FROM workflow_stages WHERE workflow_id=? AND code=?`, v13, code).Scan(&s); err != nil {
			t.Fatalf("%s.%s: %v", code, col, err)
		}
		return s
	}
	// 每处新增恰好出现一次；作业内容里新段落插在「交付物：」前，交付物仍只有一处
	want := map[[2]string]string{
		{"material", "instructions"}:           "只交原图：不裁切、不改尺寸、不加字",
		{"material", "self_check_criteria"}:    "交的都是原图",
		{"material", "acceptance"}:             "没有自行加工图片",
		{"design_prep", "instructions"}:        "图片加工都归你（2026-10-25 用户定）",
		{"design_prep", "self_check_criteria"}: "封面候选是可直接上传的成品文件",
		{"design_prep", "acceptance"}:          "封面候选为成品文件",
		{"fulltext", "instructions"}:           "封面以 06 设计师的成品文件为准",
		{"fulltext", "acceptance"}:             "封面：采用 06 设计师的成品文件",
		{"wx_layout", "instructions"}:          "图片只用成品",
		{"wx_layout", "self_check_criteria"}:   "封面是设计师交的成品文件",
		{"wx_layout", "acceptance"}:            "封面成品：",
		{"wx_save", "instructions"}:            "不加工图片：封面就是 08 字段清单里的成品文件",
		{"wx_save", "self_check_criteria"}:     "没有自行加工图片",
		{"xhs_pick", "instructions"}:           "只交原图：不裁切、不改尺寸；",
		{"xhs_package", "instructions"}:        "图片原样使用",
	}
	for k, s := range want {
		got := field(k[0], k[1])
		if n := strings.Count(got, s); n != 1 {
			t.Fatalf("v13 %s.%s 应恰好含一次 %q，实际 %d 次", k[0], k[1], s, n)
		}
		if k[1] == "instructions" {
			if n := strings.Count(got, "交付物："); n != 1 {
				t.Fatalf("v13 %s 的「交付物：」应只有一处，实际 %d", k[0], n)
			}
			if strings.Index(got, s) > strings.Index(got, "交付物：") {
				t.Fatalf("v13 %s 新段落应在「交付物：」之前", k[0])
			}
		}
	}
	// 没碰的阶段与 v12 逐字相同
	if err := db.QueryRow(`SELECT count(*) FROM workflow_stages a JOIN workflow_stages b ON b.code=a.code
		WHERE a.workflow_id=? AND b.workflow_id=? AND a.code NOT IN ('material','design_prep','fulltext','wx_layout','wx_save','xhs_pick','xhs_package')
		AND (a.instructions IS NOT b.instructions OR a.self_check_criteria IS NOT b.self_check_criteria OR a.acceptance IS NOT b.acceptance)`, v12, v13).Scan(&diff); err != nil || diff != 0 {
		t.Fatalf("没改的阶段应与 v12 逐字相同：差 %d 段 err=%v", diff, err)
	}
}
