package sqlite

import (
	"path/filepath"
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
	// 0014 outbox、0015 run_items、0020–0022 数据子系统 8 张、0028 采集留痕 3 张。
	if n != 36 {
		t.Fatalf("want 36 tables, got %d", n)
	}

	// 0009 新增列：阶段四列 + 任务快照三列。
	for _, tc := range []struct{ table, col string }{
		{"workflow_stages", "dispatch_mode"}, {"workflow_stages", "action_class"},
		{"workflow_stages", "sla_minutes"}, {"workflow_stages", "per_item"},
		{"tasks", "dispatch_mode"}, {"tasks", "action_class"}, {"tasks", "sla_minutes"},
		// 0011 tasks 重建带入的列。
		{"tasks", "item_key"}, {"tasks", "fail_reason"}, {"tasks", "last_activity_at"},
		{"tasks", "overdue_notified_at"}, {"tasks", "rework_pending"},
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
