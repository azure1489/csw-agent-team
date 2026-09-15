package sqlite

import (
	"path/filepath"
	"testing"

	"github.com/pressly/goose/v3"
)

// TestTasksRebuildKeepsData 0011 重建 tasks：存量行（含 id）、外键引用与快照列原样保留，新列取默认值。
func TestTasksRebuildKeepsData(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "rebuild.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()
	goose.SetBaseFS(migrationsFS)
	if err := goose.SetDialect("sqlite3"); err != nil {
		t.Fatal(err)
	}
	if err := goose.UpTo(db, "migrations", 10); err != nil {
		t.Fatalf("up to 10: %v", err)
	}

	// 0010 形态下造一条 run + 两个任务 + 依赖 + 闸 + 交付物 + 事件。
	for _, q := range []string{
		`INSERT INTO runs (id, workflow_id, workflow_ver, subject) SELECT 7, id, version, '2026-09-12' FROM workflows WHERE wf_key='daily_news' AND version=1`,
		`INSERT INTO tasks (id, run_id, stage_code, stage_name, seq, role_code, status, cur_version, output_type, action_class, due_at)
		 VALUES (41, 7, 'collect', '01-采集', 1, 'collector', 'passed', 2, '资讯包', 'read', '2026-09-12T02:00:00Z'),
		        (42, 7, 'topic',   '02-选题', 2, 'researcher', 'review', 1, '选题成品', 'read', NULL)`,
		`INSERT INTO task_deps (task_id, depends_on_id) VALUES (42, 41)`,
		`INSERT INTO task_gates (task_id, gate_order, reviewer_role, name) VALUES (42, 1, 'editor', '主编审')`,
		`INSERT INTO deliverables (id, task_id, version, doc_type, status) VALUES (90, 42, 1, '选题成品', 'submitted')`,
		`INSERT INTO deliverables (id, task_id, is_dispatch, version, doc_type, status) VALUES (91, 42, 1, 1, '派工单', 'issued')`,
		`INSERT INTO deliverable_upstreams (deliverable_id, label, upstream_url, upstream_id) VALUES (91, '01-采集', 'http://x/1', 90)`,
		`INSERT INTO reviews (deliverable_id, task_gate_id, verdict) SELECT 90, id, 'pass' FROM task_gates WHERE task_id=42`,
		`INSERT INTO events (run_id, task_id, type) VALUES (7, 42, 'submitted')`,
	} {
		if _, err := db.Exec(q); err != nil {
			t.Fatalf("seed: %v\n%s", err, q)
		}
	}

	if err := Migrate(db); err != nil {
		t.Fatalf("migrate to latest: %v", err)
	}

	var n int
	if err := db.QueryRow(`SELECT count(*) FROM tasks WHERE run_id=7`).Scan(&n); err != nil || n != 2 {
		t.Fatalf("tasks kept: n=%d err=%v", n, err)
	}
	var status, itemKey, due, action string
	var rework int
	if err := db.QueryRow(`SELECT status, item_key, COALESCE(due_at,''), action_class, rework_pending FROM tasks WHERE id=41`).
		Scan(&status, &itemKey, &due, &action, &rework); err != nil {
		t.Fatalf("task 41: %v", err)
	}
	if status != "passed" || itemKey != "" || due != "2026-09-12T02:00:00Z" || action != "read" || rework != 0 {
		t.Fatalf("task 41 columns: %s %q %s %s %d", status, itemKey, due, action, rework)
	}
	rows, err := db.Query(`PRAGMA foreign_key_check`)
	if err != nil {
		t.Fatal(err)
	}
	if rows.Next() {
		rows.Close()
		t.Fatalf("foreign_key_check reports violations after rebuild")
	}
	rows.Close()
	var ok string
	if err := db.QueryRow(`PRAGMA integrity_check`).Scan(&ok); err != nil || ok != "ok" {
		t.Fatalf("integrity_check: %s %v", ok, err)
	}
	var fk int
	if err := db.QueryRow(`PRAGMA foreign_keys`).Scan(&fk); err != nil || fk != 1 {
		t.Fatalf("foreign_keys should be back on, got %d", fk)
	}

	// 0012：kind 由 is_dispatch 推出；三条版本流各自唯一。
	var k90, k91 string
	if err := db.QueryRow(`SELECT (SELECT kind FROM deliverables WHERE id=90), (SELECT kind FROM deliverables WHERE id=91)`).Scan(&k90, &k91); err != nil || k90 != "output" || k91 != "dispatch" {
		t.Fatalf("kinds: %s %s %v", k90, k91, err)
	}
	if _, err := db.Exec(`INSERT INTO deliverables (task_id, kind, version, doc_type, status, affects_deliverable_id) VALUES (41, 'supplement', 1, '资讯包', 'passed', NULL)`); err != nil {
		t.Fatalf("supplement v1 should coexist with output v1: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO deliverables (task_id, kind, version, doc_type) VALUES (42, 'edit', 1, '选题成品')`); err == nil {
		t.Fatalf("edit shares the output version stream: v1 should conflict")
	}

	// 新约束：同 run 同阶段按 item_key 区分；新状态可写入；非法状态被拒。
	if _, err := db.Exec(`INSERT INTO tasks (run_id, stage_code, item_key, stage_name, seq, role_code, status) VALUES (7, 'topic', 'hxo-1a2b', '02-选题', 2, 'researcher', 'cancelled')`); err != nil {
		t.Fatalf("item task insert: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO tasks (run_id, stage_code, item_key, stage_name, seq, role_code) VALUES (7, 'topic', 'hxo-1a2b', '02-选题', 2, 'researcher')`); err == nil {
		t.Fatalf("duplicate (run, stage, item_key) should violate unique")
	}
	if _, err := db.Exec(`INSERT INTO tasks (run_id, stage_code, item_key, stage_name, seq, role_code, status) VALUES (7, 'x', '', 'x', 9, 'writer', 'weird')`); err == nil {
		t.Fatalf("invalid status should violate check")
	}
}
