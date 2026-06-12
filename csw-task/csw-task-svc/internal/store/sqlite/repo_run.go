package sqlite

import (
	"context"
	"database/sql"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── runs ──

// InsertRun 建实例，返回 run id。
func (q *Queries) InsertRun(ctx context.Context, workflowID int64, ver int, subject, title string, createdBy *int64) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO runs (workflow_id, workflow_ver, subject, title, created_by) VALUES (?,?,?,?,?)`,
		workflowID, ver, subject, title, nullI64(createdBy))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

func scanRun(s interface{ Scan(...any) error }) (domain.Run, error) {
	var r domain.Run
	var title sql.NullString
	var createdBy sql.NullInt64
	var status string
	err := s.Scan(&r.ID, &r.WorkflowID, &r.WorkflowVer, &r.Subject, &title, &status, &createdBy)
	r.Title, r.Status, r.CreatedBy = title.String, domain.RunStatus(status), ptrI64(createdBy)
	return r, err
}

const runCols = `id, workflow_id, workflow_ver, subject, title, status, created_by`

// GetRun 取实例。
func (q *Queries) GetRun(ctx context.Context, id int64) (domain.Run, error) {
	return scanRun(q.ex.QueryRowContext(ctx, `SELECT `+runCols+` FROM runs WHERE id=?`, id))
}

// RunByWorkflowSubject 据 (workflow, subject) 取实例（触发去重 / UNIQUE 命中）。
func (q *Queries) RunByWorkflowSubject(ctx context.Context, workflowID int64, subject string) (domain.Run, error) {
	return scanRun(q.ex.QueryRowContext(ctx,
		`SELECT `+runCols+` FROM runs WHERE workflow_id=? AND subject=?`, workflowID, subject))
}

// SetRunStatus 改实例状态。
func (q *Queries) SetRunStatus(ctx context.Context, id int64, status domain.RunStatus) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE runs SET status=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`, string(status), id)
	return err
}

// ── tasks ──

// InsertTask 建阶段任务（从 stage 快照），返回 task id。
func (q *Queries) InsertTask(ctx context.Context, t domain.Task) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO tasks
		  (run_id, stage_code, stage_name, seq, role_code, is_merge, output_type,
		   instructions, self_check_criteria, acceptance, assignee_id, status, cur_version)
		VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)`,
		t.RunID, t.StageCode, t.StageName, t.Seq, t.RoleCode, b2i(t.IsMerge), t.OutputType,
		t.Instructions, t.SelfCheckCriteria, t.Acceptance, nullI64(t.AssigneeID), string(t.Status), t.CurVersion)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

const taskCols = `id, run_id, stage_code, stage_name, seq, role_code, is_merge, output_type,
	instructions, self_check_criteria, acceptance, assignee_id, status, cur_version`

// 带表别名 t. 的列，用于含 JOIN 的查询（避免 id 等与 runs 列歧义）。
const taskColsT = `t.id, t.run_id, t.stage_code, t.stage_name, t.seq, t.role_code, t.is_merge, t.output_type,
	t.instructions, t.self_check_criteria, t.acceptance, t.assignee_id, t.status, t.cur_version`

func scanTask(s interface{ Scan(...any) error }) (domain.Task, error) {
	var t domain.Task
	var merge int
	var outType, ins, sc, ac sql.NullString
	var assignee sql.NullInt64
	var status string
	err := s.Scan(&t.ID, &t.RunID, &t.StageCode, &t.StageName, &t.Seq, &t.RoleCode, &merge, &outType,
		&ins, &sc, &ac, &assignee, &status, &t.CurVersion)
	t.IsMerge = merge == 1
	t.OutputType = outType.String
	t.Instructions, t.SelfCheckCriteria, t.Acceptance = ins.String, sc.String, ac.String
	t.AssigneeID = ptrI64(assignee)
	t.Status = domain.TaskStatus(status)
	return t, err
}

// GetTask 取任务。
func (q *Queries) GetTask(ctx context.Context, id int64) (domain.Task, error) {
	return scanTask(q.ex.QueryRowContext(ctx, `SELECT `+taskCols+` FROM tasks WHERE id=?`, id))
}

// ListTasksByRun 列实例全部任务（按 seq）。
func (q *Queries) ListTasksByRun(ctx context.Context, runID int64) ([]domain.Task, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+taskCols+` FROM tasks WHERE run_id=? ORDER BY seq`, runID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Task
	for rows.Next() {
		t, err := scanTask(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, t)
	}
	return out, rows.Err()
}

// ListMyTasks 列某 assignee 的任务；openOnly=true 时排除 passed。
func (q *Queries) ListMyTasks(ctx context.Context, assigneeID int64, openOnly bool) ([]domain.Task, error) {
	query := `SELECT ` + taskCols + ` FROM tasks WHERE assignee_id=?`
	if openOnly {
		query += ` AND status <> 'passed'`
	}
	query += ` ORDER BY run_id, seq`
	rows, err := q.ex.QueryContext(ctx, query, assigneeID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Task
	for rows.Next() {
		t, err := scanTask(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, t)
	}
	return out, rows.Err()
}

// ListReadyTasks 列全部 ready 任务（active run），即中枢派工队列。
func (q *Queries) ListReadyTasks(ctx context.Context) ([]domain.Task, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT `+taskColsT+` FROM tasks t
		JOIN runs r ON r.id = t.run_id
		WHERE t.status='ready' AND r.status='active' ORDER BY t.run_id, t.seq`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Task
	for rows.Next() {
		t, err := scanTask(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, t)
	}
	return out, rows.Err()
}

// SetTaskAssignee 设置任务负责 agent。
func (q *Queries) SetTaskAssignee(ctx context.Context, id int64, assigneeID *int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE tasks SET assignee_id=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`,
		nullI64(assigneeID), id)
	return err
}

// SetTaskReady 置 ready。
func (q *Queries) SetTaskReady(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE tasks SET status='ready', updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`, id)
	return err
}

// SetTaskDispatched 置 dispatched + dispatched_at。
func (q *Queries) SetTaskDispatched(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status='dispatched',
			dispatched_at=COALESCE(dispatched_at, strftime('%Y-%m-%dT%H:%M:%SZ','now')),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, id)
	return err
}

// SetTaskReview 提交产出后置 review + cur_version + started_at。
func (q *Queries) SetTaskReview(ctx context.Context, id int64, curVersion int) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status='review', cur_version=?,
			started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%SZ','now')),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, curVersion, id)
	return err
}

// SetTaskReturned 置 returned。
func (q *Queries) SetTaskReturned(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE tasks SET status='returned', updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`, id)
	return err
}

// SetTaskPassed 置 passed + completed_at + cur_version。
func (q *Queries) SetTaskPassed(ctx context.Context, id int64, curVersion int) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status='passed', cur_version=?,
			completed_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, curVersion, id)
	return err
}

// ── task_deps ──

// InsertTaskDep 写依赖边。
func (q *Queries) InsertTaskDep(ctx context.Context, taskID, dependsOnID int64) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO task_deps (task_id, depends_on_id) VALUES (?,?)`, taskID, dependsOnID)
	return err
}

// ListTaskDepsByRun 列实例全部依赖边。
func (q *Queries) ListTaskDepsByRun(ctx context.Context, runID int64) ([]domain.TaskDep, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT td.task_id, td.depends_on_id
		FROM task_deps td JOIN tasks t ON t.id = td.task_id
		WHERE t.run_id=?`, runID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.TaskDep
	for rows.Next() {
		var d domain.TaskDep
		if err := rows.Scan(&d.TaskID, &d.DependsOnID); err != nil {
			return nil, err
		}
		out = append(out, d)
	}
	return out, rows.Err()
}

// TaskDepIDs 取某任务直接依赖的上游 task id 列表。
func (q *Queries) TaskDepIDs(ctx context.Context, taskID int64) ([]int64, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT depends_on_id FROM task_deps WHERE task_id=? ORDER BY depends_on_id`, taskID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []int64
	for rows.Next() {
		var id int64
		if err := rows.Scan(&id); err != nil {
			return nil, err
		}
		out = append(out, id)
	}
	return out, rows.Err()
}

// ── task_gates ──

// InsertTaskGate 写任务闸。
func (q *Queries) InsertTaskGate(ctx context.Context, taskID int64, order int, reviewerRole string, relayed bool, name string) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO task_gates (task_id, gate_order, reviewer_role, relayed_by_hub, name) VALUES (?,?,?,?,?)`,
		taskID, order, reviewerRole, b2i(relayed), name)
	return err
}

func scanTaskGate(s interface{ Scan(...any) error }) (domain.TaskGate, error) {
	var g domain.TaskGate
	var relayed int
	var name sql.NullString
	err := s.Scan(&g.ID, &g.TaskID, &g.GateOrder, &g.ReviewerRole, &relayed, &name)
	g.RelayedByHub = relayed == 1
	g.Name = name.String
	return g, err
}

// ListTaskGates 列任务全部闸（按 order）。
func (q *Queries) ListTaskGates(ctx context.Context, taskID int64) ([]domain.TaskGate, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT id, task_id, gate_order, reviewer_role, relayed_by_hub, name
		 FROM task_gates WHERE task_id=? ORDER BY gate_order`, taskID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.TaskGate
	for rows.Next() {
		g, err := scanTaskGate(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, g)
	}
	return out, rows.Err()
}

// TaskGateByOrder 取任务某道闸。
func (q *Queries) TaskGateByOrder(ctx context.Context, taskID int64, order int) (domain.TaskGate, error) {
	return scanTaskGate(q.ex.QueryRowContext(ctx,
		`SELECT id, task_id, gate_order, reviewer_role, relayed_by_hub, name
		 FROM task_gates WHERE task_id=? AND gate_order=?`, taskID, order))
}

func b2i(b bool) int {
	if b {
		return 1
	}
	return 0
}

// ── 监控（管理后台只读）──

// RunListItem 运行列表项（含工作流名/当前阶段/触发者）。
type RunListItem struct {
	Subject      string
	Status       string
	WorkflowName string
	WorkflowKey  string
	CurrentStage string
	TriggerName  string
	CreatedAt    string
	ID           int64
	WorkflowID   int64
}

// ListRuns 列运行实例，可按 wf_key/status/创建日期前缀过滤；limit<=0 不限。
func (q *Queries) ListRuns(ctx context.Context, wfKey, status, datePrefix string, limit int) ([]RunListItem, error) {
	query := `
		SELECT r.id, r.workflow_id, w.name, w.wf_key, r.subject, r.status, r.created_at,
		       (SELECT t.stage_name FROM tasks t WHERE t.run_id=r.id AND t.status<>'passed' ORDER BY t.seq LIMIT 1) AS cur_stage,
		       (SELECT a.name FROM agents a WHERE a.id = r.created_by) AS trigger_name
		FROM runs r JOIN workflows w ON w.id = r.workflow_id`
	var where []string
	var args []any
	if wfKey != "" {
		where = append(where, "w.wf_key=?")
		args = append(args, wfKey)
	}
	if status != "" {
		where = append(where, "r.status=?")
		args = append(args, status)
	}
	if datePrefix != "" {
		where = append(where, "r.created_at LIKE ?")
		args = append(args, datePrefix+"%")
	}
	if len(where) > 0 {
		query += " WHERE " + strings.Join(where, " AND ")
	}
	query += " ORDER BY r.id DESC"
	if limit > 0 {
		query += " LIMIT ?"
		args = append(args, limit)
	}

	rows, err := q.ex.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []RunListItem
	for rows.Next() {
		var it RunListItem
		var curStage, trigger sql.NullString
		if err := rows.Scan(&it.ID, &it.WorkflowID, &it.WorkflowName, &it.WorkflowKey,
			&it.Subject, &it.Status, &it.CreatedAt, &curStage, &trigger); err != nil {
			return nil, err
		}
		it.CurrentStage, it.TriggerName = curStage.String, trigger.String
		out = append(out, it)
	}
	return out, rows.Err()
}

// CountWorkflowsByStatus 统计某状态工作流数。
func (q *Queries) CountWorkflowsByStatus(ctx context.Context, status string) (int, error) {
	var n int
	err := q.ex.QueryRowContext(ctx, `SELECT count(*) FROM workflows WHERE status=?`, status).Scan(&n)
	return n, err
}

// CountRunsByStatus 统计某状态实例数。
func (q *Queries) CountRunsByStatus(ctx context.Context, status string) (int, error) {
	var n int
	err := q.ex.QueryRowContext(ctx, `SELECT count(*) FROM runs WHERE status=?`, status).Scan(&n)
	return n, err
}

// CountRunsDoneOn 统计某日期前缀完成的实例数（updated_at 落在该日）。
func (q *Queries) CountRunsDoneOn(ctx context.Context, datePrefix string) (int, error) {
	var n int
	err := q.ex.QueryRowContext(ctx,
		`SELECT count(*) FROM runs WHERE status='done' AND updated_at LIKE ?`, datePrefix+"%").Scan(&n)
	return n, err
}
