package sqlite

import (
	"context"
	"database/sql"
	"fmt"
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
	err := s.Scan(&r.ID, &r.WorkflowID, &r.WorkflowVer, &r.Subject, &title, &status, &createdBy, &r.TargetCount)
	r.Title, r.Status, r.CreatedBy = title.String, domain.RunStatus(status), ptrI64(createdBy)
	return r, err
}

const runCols = `id, workflow_id, workflow_ver, subject, title, status, created_by, target_count`

// runColsR 同 runCols，但带 r. 前缀，供带 JOIN / 别名的查询使用。
const runColsR = `r.id, r.workflow_id, r.workflow_ver, r.subject, r.title, r.status, r.created_by, r.target_count`

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
		   instructions, self_check_criteria, acceptance, assignee_id, status, cur_version,
		   dispatch_mode, action_class, sla_minutes, item_key, wait_item_stages, ack_minutes, idle_minutes)
		VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
		t.RunID, t.StageCode, t.StageName, t.Seq, t.RoleCode, b2i(t.IsMerge), t.OutputType,
		t.Instructions, t.SelfCheckCriteria, t.Acceptance, nullI64(t.AssigneeID), string(t.Status), t.CurVersion,
		nullIfEmpty(string(t.DispatchMode)), actionOrRead(t.ActionClass), nullIfNonPos(t.SLAMinutes), t.ItemKey, nullIfEmpty(t.WaitItemStages),
		nullIfNonPos(t.AckMinutes), nullIfNonPos(t.IdleMinutes))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

const taskCols = `id, run_id, stage_code, stage_name, seq, role_code, is_merge, output_type,
	instructions, self_check_criteria, acceptance, assignee_id, status, cur_version,
	dispatch_mode, action_class, sla_minutes,
	item_key, fail_reason, due_at, dispatched_at, started_at, completed_at, last_activity_at, rework_pending,
	wait_item_stages, ack_minutes, idle_minutes`

// 带表别名 t. 的列，用于含 JOIN 的查询（避免 id 等与 runs 列歧义）。
const taskColsT = `t.id, t.run_id, t.stage_code, t.stage_name, t.seq, t.role_code, t.is_merge, t.output_type,
	t.instructions, t.self_check_criteria, t.acceptance, t.assignee_id, t.status, t.cur_version,
	t.dispatch_mode, t.action_class, t.sla_minutes,
	t.item_key, t.fail_reason, t.due_at, t.dispatched_at, t.started_at, t.completed_at, t.last_activity_at, t.rework_pending,
	t.wait_item_stages, t.ack_minutes, t.idle_minutes`

func scanTask(s interface{ Scan(...any) error }) (domain.Task, error) {
	var t domain.Task
	var merge int
	var outType, ins, sc, ac, dmode, failR, due, disp, start, done, act, wait sql.NullString
	var assignee, sla, ack, idle sql.NullInt64
	var status string
	var rework int
	err := s.Scan(&t.ID, &t.RunID, &t.StageCode, &t.StageName, &t.Seq, &t.RoleCode, &merge, &outType,
		&ins, &sc, &ac, &assignee, &status, &t.CurVersion, &dmode, &t.ActionClass, &sla,
		&t.ItemKey, &failR, &due, &disp, &start, &done, &act, &rework, &wait, &ack, &idle)
	t.WaitItemStages = wait.String
	t.AckMinutes, t.IdleMinutes = int(ack.Int64), int(idle.Int64)
	t.DispatchMode, t.SLAMinutes = domain.DispatchMode(dmode.String), int(sla.Int64)
	t.FailReason, t.DueAt, t.DispatchedAt = failR.String, due.String, disp.String
	t.StartedAt, t.CompletedAt, t.LastActivityAt = start.String, done.String, act.String
	t.ReworkPending = rework == 1
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

// ListMyTasks 列某 assignee 的任务；openOnly=true 时只列待处理的（排除已通过、已取消、已报告失败）。
func (q *Queries) ListMyTasks(ctx context.Context, assigneeID int64, openOnly bool) ([]domain.Task, error) {
	query := `SELECT ` + taskCols + ` FROM tasks WHERE assignee_id=?`
	if openOnly {
		query += ` AND status NOT IN ('passed','cancelled','failed')`
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
	return q.ListActiveRunTasksByStatus(ctx, domain.TaskReady)
}

// ListActiveRunTasksByStatus 列进行中 run 里某状态的全部任务。
func (q *Queries) ListActiveRunTasksByStatus(ctx context.Context, status domain.TaskStatus) ([]domain.Task, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT `+taskColsT+` FROM tasks t
		JOIN runs r ON r.id = t.run_id
		WHERE t.status=? AND r.status='active' ORDER BY t.run_id, t.seq`, string(status))
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
			dispatched_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), ack_notified_at=NULL, ack_escalated_at=NULL,
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

// SetTaskInProgress 接单：dispatched → in_progress，记 started_at 与活动时间。
func (q *Queries) SetTaskInProgress(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status='in_progress',
			started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%SZ','now')),
			last_activity_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), idle_notified_at=NULL,
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, id)
	return err
}

// TouchTaskActivity 刷新最近活动时间（接单心跳 / 提交）。
func (q *Queries) TouchTaskActivity(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE tasks SET last_activity_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), idle_notified_at=NULL WHERE id=?`, id)
	return err
}

// SetTaskFailed 执行者报告无法完成。
func (q *Queries) SetTaskFailed(ctx context.Context, id int64, reason string) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status='failed', fail_reason=?,
			last_activity_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, reason, id)
	return err
}

// SetTaskCancelled 中枢取消。
func (q *Queries) SetTaskCancelled(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status='cancelled', completed_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, id)
	return err
}

// ReopenTask 中枢重开：置 ready 或 blocked，清失败原因与时限（再次派工时重新计时）。
func (q *Queries) ReopenTask(ctx context.Context, id int64, status domain.TaskStatus) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET status=?, fail_reason=NULL, completed_at=NULL, due_at=NULL, overdue_notified_at=NULL,
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, string(status), id)
	return err
}

// SetTaskDue 按时限（分钟）从现在起算 due_at，并清逾期提醒标记。
func (q *Queries) SetTaskDue(ctx context.Context, id int64, minutes int) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET due_at=strftime('%Y-%m-%dT%H:%M:%SZ','now', ?), overdue_notified_at=NULL WHERE id=?`,
		fmt.Sprintf("+%d minutes", minutes), id)
	return err
}

// ClearOverdueNotified 清逾期提醒标记（提交后再逾期可再提醒一次）。
func (q *Queries) ClearOverdueNotified(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `UPDATE tasks SET overdue_notified_at=NULL WHERE id=?`, id)
	return err
}

// ListOverdueTasks 列进行中 run 里已逾期且尚未提醒过的执行中任务。
func (q *Queries) ListOverdueTasks(ctx context.Context) ([]domain.Task, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT `+taskColsT+` FROM tasks t
		JOIN runs r ON r.id = t.run_id
		WHERE r.status='active' AND t.status IN ('dispatched','in_progress','returned')
		  AND t.due_at IS NOT NULL AND t.due_at <= strftime('%Y-%m-%dT%H:%M:%SZ','now')
		  AND t.overdue_notified_at IS NULL
		ORDER BY t.due_at`)
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

// MarkOverdueNotified 条件标记「已提醒逾期」：仍在执行中、已逾期且尚未提醒时才标，返回是否本次标上。
// 同一次逾期只提醒一次；并发扫描时只有一方拿到。
func (q *Queries) MarkOverdueNotified(ctx context.Context, id int64) (bool, error) {
	res, err := q.ex.ExecContext(ctx, `
		UPDATE tasks SET overdue_notified_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=? AND overdue_notified_at IS NULL AND due_at IS NOT NULL
		  AND due_at <= strftime('%Y-%m-%dT%H:%M:%SZ','now')
		  AND status IN ('dispatched','in_progress','returned')`, id)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}

// 接续告警的到期条件（阈值为空时取默认；未接单条件带倍数，升级用 AckEscalateFactor）。
// 只看最近 24 小时内开始的停滞：更早的只可能是历史遗留或 notifier 长时间停机，补发只会刷屏。
const (
	condAckDue = `t.status='dispatched' AND t.dispatched_at IS NOT NULL
		AND t.dispatched_at >= strftime('%Y-%m-%dT%H:%M:%SZ','now','-24 hours')
		AND strftime('%Y-%m-%dT%H:%M:%SZ', t.dispatched_at, '+' || (COALESCE(t.ack_minutes, ?) * ?) || ' minutes')
		    <= strftime('%Y-%m-%dT%H:%M:%SZ','now')`
	condIdleDue = `t.status='in_progress' AND t.last_activity_at IS NOT NULL
		AND t.last_activity_at >= strftime('%Y-%m-%dT%H:%M:%SZ','now','-24 hours')
		AND strftime('%Y-%m-%dT%H:%M:%SZ', t.last_activity_at, '+' || COALESCE(t.idle_minutes, ?) || ' minutes')
		    <= strftime('%Y-%m-%dT%H:%M:%SZ','now')`
)

// stallSQL 某类接续告警「已到期且尚未提醒」的条件、参数与标记列。升级只在已提醒过执行者之后发生。
func stallSQL(k domain.StallKind) (cond string, args []any, col string) {
	switch k {
	case domain.StallAck:
		return condAckDue + ` AND t.ack_notified_at IS NULL`, []any{domain.DefaultAckMinutes, 1}, "ack_notified_at"
	case domain.StallEscalate:
		return condAckDue + ` AND t.ack_notified_at IS NOT NULL AND t.ack_escalated_at IS NULL`,
			[]any{domain.DefaultAckMinutes, domain.AckEscalateFactor}, "ack_escalated_at"
	default:
		return condIdleDue + ` AND t.idle_notified_at IS NULL`, []any{domain.DefaultIdleMinutes}, "idle_notified_at"
	}
}

// ListStalledTasks 列某类接续告警到期且尚未提醒的任务（只看进行中的 run）。
func (q *Queries) ListStalledTasks(ctx context.Context, k domain.StallKind) ([]domain.Task, error) {
	cond, args, _ := stallSQL(k)
	rows, err := q.ex.QueryContext(ctx, `SELECT `+taskColsT+` FROM tasks t JOIN runs r ON r.id = t.run_id
		WHERE r.status='active' AND `+cond+` ORDER BY t.id`, args...)
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

// MarkStallNotified 条件标记某类接续告警「已提醒」：仍到期且尚未提醒时才标，返回是否本次标上。
// 同一次停滞只提醒一次；并发扫描时只有一方拿到。
func (q *Queries) MarkStallNotified(ctx context.Context, id int64, k domain.StallKind) (bool, error) {
	cond, args, col := stallSQL(k)
	res, err := q.ex.ExecContext(ctx, `UPDATE tasks AS t SET `+col+`=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE t.id=? AND `+cond, append([]any{id}, args...)...)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}

// condRunStalled 整期停滞：run 仍 active、没有任何已派工/进行中/待审的任务、但还有没走完的阶段，
// 且距最近一次**业务**事件已超过阈值。「业务事件」不含各类提醒本身（run_stalled、逾期、未接单、
// 无活动、待审提醒）——否则提醒一发，「最近事件」就被刷成当下，再也判不出停滞（自我抵消）。
// 同时只看最近 24 小时内还有动静的 run：与任务级 condAckDue / condIdleDue 一致。
// 漏了这条的代价已经付过——6 月以来 39 个一直挂着的旧 v1 run 一次性全被判停滞并 @ 了中枢。中枢报失败后正是这种状态——任务 failed、下游 blocked，任务级告警全都不触发。
//
// **同一次停滞只提醒一次，有了新的业务事件就算新的一次**：比较提醒时刻与最近业务事件，
// 不靠清标记。原先的 ClearRunStallNotified 从没被调用过，r56 在 05:30 报过一次后就再也不会报（09-28）。
// 有任务在待审的不算：那是审核方没动，由待审提醒（review_waiting）按档追，这里不重复 @。
const runStallLastBusiness = `IFNULL((SELECT MAX(e.created_at) FROM events e WHERE e.run_id=r.id
	AND e.type NOT IN ('run_stalled','overdue','ack_overdue','ack_escalated','task_idle','review_waiting','review_escalated')), r.created_at)`

const condRunStalled = `r.status='active'
	AND (r.stalled_notified_at IS NULL OR r.stalled_notified_at < ` + runStallLastBusiness + `)
	AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.run_id=r.id AND t.status IN ('dispatched','in_progress','review'))
	AND EXISTS (SELECT 1 FROM tasks t WHERE t.run_id=r.id AND t.status IN ('ready','blocked','failed','returned'))
	AND ` + runStallLastBusiness + ` <= strftime('%Y-%m-%dT%H:%M:%SZ','now',-? || ' minutes')
	AND ` + runStallLastBusiness + ` >= strftime('%Y-%m-%dT%H:%M:%SZ','now','-24 hours')`

// ListStalledRuns 列整期停滞且尚未提醒的 run。
func (q *Queries) ListStalledRuns(ctx context.Context) ([]domain.Run, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT `+runColsR+` FROM runs r WHERE `+condRunStalled+` ORDER BY r.id`, domain.RunStallMinutes)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Run
	for rows.Next() {
		r, err := scanRun(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

// MarkRunStallNotified 条件标记整期停滞「已提醒」：仍停滞且未提醒时才标，返回是否本次标上（同一次停滞只提醒一次）。
func (q *Queries) MarkRunStallNotified(ctx context.Context, runID int64) (bool, error) {
	res, err := q.ex.ExecContext(ctx,
		`UPDATE runs AS r SET stalled_notified_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		 WHERE r.id=? AND `+condRunStalled, runID, domain.RunStallMinutes)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}

// RunStallSummary 停滞时的卡点摘要：还差哪些阶段、各是什么状态。
func (q *Queries) RunStallSummary(ctx context.Context, runID int64) (string, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT stage_name, status FROM tasks WHERE run_id=? AND status IN ('ready','blocked','failed','returned') ORDER BY seq`, runID)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	parts := make([]string, 0, 8)
	for rows.Next() {
		var name, st string
		if err := rows.Scan(&name, &st); err != nil {
			return "", err
		}
		parts = append(parts, name+"("+st+")")
	}
	return strings.Join(parts, "、"), rows.Err()
}

// ListDirectDownstream 列直接依赖某任务的下游任务。
func (q *Queries) ListDirectDownstream(ctx context.Context, taskID int64) ([]domain.Task, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT `+taskColsT+` FROM task_deps td JOIN tasks t ON t.id = td.task_id
		WHERE td.depends_on_id=? ORDER BY t.seq`, taskID)
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
		       (SELECT t.stage_name FROM tasks t WHERE t.run_id=r.id AND t.status NOT IN ('passed','cancelled') ORDER BY t.seq LIMIT 1) AS cur_stage,
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
