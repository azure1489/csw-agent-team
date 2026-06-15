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

	// 20 张原始业务表 + 0008 花名册 2 张（chats / chat_members）+ goose 版本表。
	var n int
	if err := db.QueryRow(
		`SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name <> 'goose_db_version'`,
	).Scan(&n); err != nil {
		t.Fatalf("count tables: %v", err)
	}
	if n != 22 {
		t.Fatalf("want 22 tables, got %d", n)
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
	if status != "active" {
		t.Fatalf("want active, got %s", status)
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
