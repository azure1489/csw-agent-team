package sqlite

import (
	"context"
	"database/sql"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── 采集与判断过程留痕：来源台账 / 采集轮 / 判断轨迹 ──

const sourceCols = `id, platform, source_key, name, entry_url, enabled, required,
	last_ok_at, added_by, source_ref, note, created_at, updated_at`

func scanSource(s interface{ Scan(...any) error }) (domain.IntakeSource, error) {
	var src domain.IntakeSource
	var entry, lastOK, addedBy, ref, note sql.NullString
	var enabled, required int
	err := s.Scan(&src.ID, &src.Platform, &src.SourceKey, &src.Name, &entry, &enabled, &required,
		&lastOK, &addedBy, &ref, &note, &src.CreatedAt, &src.UpdatedAt)
	src.EntryURL, src.LastOKAt, src.AddedBy, src.SourceRef, src.Note = entry.String, lastOK.String, addedBy.String, ref.String, note.String
	src.Enabled, src.Required = enabled == 1, required == 1
	return src, err
}

// UpsertIntakeSource 登记或更新一条来源台账（人工维护：API 路径不写这张表，
// 否则「扫过的源」自动成为基准，覆盖率就会自我实现，恒为 100%）。
func (q *Queries) UpsertIntakeSource(ctx context.Context, src domain.IntakeSource) error {
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO intake_sources (platform, source_key, name, entry_url, enabled, required, added_by, source_ref, note)
		VALUES (?,?,?,?,?,?,?,?,?)
		ON CONFLICT(platform, source_key) DO UPDATE SET
			name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE intake_sources.name END,
			entry_url = COALESCE(excluded.entry_url, intake_sources.entry_url),
			enabled = excluded.enabled,
			required = excluded.required,
			added_by = COALESCE(excluded.added_by, intake_sources.added_by),
			source_ref = COALESCE(excluded.source_ref, intake_sources.source_ref),
			note = COALESCE(excluded.note, intake_sources.note),
			updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')`,
		src.Platform, src.SourceKey, src.Name, nullIfEmpty(src.EntryURL), boolToInt(src.Enabled), boolToInt(src.Required),
		nullIfEmpty(src.AddedBy), nullIfEmpty(src.SourceRef), nullIfEmpty(src.Note))
	return err
}

// ListIntakeSources 列来源台账；enabledOnly 时只给启用中的。按平台、来源键排序。
func (q *Queries) ListIntakeSources(ctx context.Context, enabledOnly bool) ([]domain.IntakeSource, error) {
	where := ""
	if enabledOnly {
		where = ` WHERE enabled = 1`
	}
	rows, err := q.ex.QueryContext(ctx, `SELECT `+sourceCols+` FROM intake_sources`+where+` ORDER BY platform, source_key`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := make([]domain.IntakeSource, 0, 16)
	for rows.Next() {
		src, err := scanSource(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, src)
	}
	return out, rows.Err()
}

// SetIntakeSourceEnabled 启用或停用一条来源（停用后不再计入覆盖率分母）。
func (q *Queries) SetIntakeSourceEnabled(ctx context.Context, platform, key string, enabled bool) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`UPDATE intake_sources SET enabled=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE platform=? AND source_key=?`,
		boolToInt(enabled), platform, key)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}

// TouchIntakeSourceOK 记一次成功采集的时间（体检与回填用）。
func (q *Queries) TouchIntakeSourceOK(ctx context.Context, platform, key string) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE intake_sources SET last_ok_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE platform=? AND source_key=?`, platform, key)
	return err
}

const sweepCols = `id, run_id, task_id, sweep_key, platform, source_key, tool, query, started_at, ended_at,
	window_from, window_to, found, in_window, registered, result, error, paged_to_end,
	actor_id, role_code, created_at, updated_at, fetched_unique, reviewed, unreviewed`

func scanSweep(s interface{ Scan(...any) error }) (domain.IntakeSweep, error) {
	var sw domain.IntakeSweep
	var query, startedAt, endedAt, winFrom, winTo, errText sql.NullString
	var taskID, actorID sql.NullInt64
	var paged int
	err := s.Scan(&sw.ID, &sw.RunID, &taskID, &sw.SweepKey, &sw.Platform, &sw.SourceKey, &sw.Tool, &query,
		&startedAt, &endedAt, &winFrom, &winTo, &sw.Found, &sw.InWindow, &sw.Registered, &sw.Result, &errText,
		&paged, &actorID, &sw.RoleCode, &sw.CreatedAt, &sw.UpdatedAt,
		&sw.FetchedUnique, &sw.Reviewed, &sw.Unreviewed)
	sw.Query, sw.StartedAt, sw.EndedAt = query.String, startedAt.String, endedAt.String
	sw.WindowFrom, sw.WindowTo, sw.Error = winFrom.String, winTo.String, errText.String
	sw.TaskID, sw.ActorID, sw.PagedToEnd = ptrI64(taskID), ptrI64(actorID), paged == 1
	return sw, err
}

// UpsertSweep 上报一轮采集；(run_id, sweep_key) 幂等，同键重报即更新计数与结果。
func (q *Queries) UpsertSweep(ctx context.Context, sw domain.IntakeSweep) error {
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO intake_sweeps (run_id, task_id, sweep_key, platform, source_key, tool, query, started_at, ended_at,
			window_from, window_to, found, in_window, registered, result, error, paged_to_end, actor_id, role_code,
			fetched_unique, reviewed, unreviewed)
		VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
		ON CONFLICT(run_id, sweep_key) DO UPDATE SET
			task_id = COALESCE(excluded.task_id, intake_sweeps.task_id),
			platform = excluded.platform,
			source_key = excluded.source_key,
			tool = excluded.tool,
			query = COALESCE(excluded.query, intake_sweeps.query),
			started_at = COALESCE(excluded.started_at, intake_sweeps.started_at),
			ended_at = COALESCE(excluded.ended_at, intake_sweeps.ended_at),
			window_from = COALESCE(excluded.window_from, intake_sweeps.window_from),
			window_to = COALESCE(excluded.window_to, intake_sweeps.window_to),
			found = excluded.found,
			in_window = excluded.in_window,
			registered = excluded.registered,
			fetched_unique = excluded.fetched_unique,
			reviewed = excluded.reviewed,
			unreviewed = excluded.unreviewed,
			result = excluded.result,
			error = COALESCE(excluded.error, intake_sweeps.error),
			paged_to_end = excluded.paged_to_end,
			updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')`,
		sw.RunID, nullI64(sw.TaskID), sw.SweepKey, sw.Platform, sw.SourceKey, sw.Tool, nullIfEmpty(sw.Query),
		nullIfEmpty(sw.StartedAt), nullIfEmpty(sw.EndedAt), nullIfEmpty(sw.WindowFrom), nullIfEmpty(sw.WindowTo),
		sw.Found, sw.InWindow, sw.Registered, sw.Result, nullIfEmpty(sw.Error), boolToInt(sw.PagedToEnd),
		nullI64(sw.ActorID), sw.RoleCode, sw.FetchedUnique, sw.Reviewed, sw.Unreviewed)
	return err
}

// ListSweepsByRun 列某 run 的全部采集轮（按上报顺序）。
func (q *Queries) ListSweepsByRun(ctx context.Context, runID int64) ([]domain.IntakeSweep, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+sweepCols+` FROM intake_sweeps WHERE run_id=? ORDER BY id`, runID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := make([]domain.IntakeSweep, 0, 16)
	for rows.Next() {
		sw, err := scanSweep(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, sw)
	}
	return out, rows.Err()
}

const traceCols = `id, run_id, item_key, from_status, to_status, reason_code, reason, actor_id, actor_role, quote_ref, created_at`

func scanTrace(s interface{ Scan(...any) error }) (domain.ItemTrace, error) {
	var tr domain.ItemTrace
	var from, reason, quote sql.NullString
	var actorID sql.NullInt64
	err := s.Scan(&tr.ID, &tr.RunID, &tr.ItemKey, &from, &tr.ToStatus, &tr.ReasonCode, &reason,
		&actorID, &tr.ActorRole, &quote, &tr.CreatedAt)
	tr.FromStatus, tr.Reason, tr.QuoteRef = from.String, reason.String, quote.String
	tr.ActorID = ptrI64(actorID)
	return tr, err
}

// InsertItemTrace 记一次条目判断（状态变化或理由变化时才写，见 engine 的去重）。
func (q *Queries) InsertItemTrace(ctx context.Context, tr domain.ItemTrace) error {
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO item_traces (run_id, item_key, from_status, to_status, reason_code, reason, actor_id, actor_role, quote_ref)
		VALUES (?,?,?,?,?,?,?,?,?)`,
		tr.RunID, tr.ItemKey, nullIfEmpty(tr.FromStatus), tr.ToStatus, tr.ReasonCode, nullIfEmpty(tr.Reason),
		nullI64(tr.ActorID), tr.ActorRole, nullIfEmpty(tr.QuoteRef))
	return err
}

// ListTracesByRun 列某 run 的全部判断轨迹（按发生顺序）。
func (q *Queries) ListTracesByRun(ctx context.Context, runID int64) ([]domain.ItemTrace, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+traceCols+` FROM item_traces WHERE run_id=? ORDER BY id`, runID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := make([]domain.ItemTrace, 0, 16)
	for rows.Next() {
		tr, err := scanTrace(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, tr)
	}
	return out, rows.Err()
}

// LastTraceForItem 取某条目最近一条轨迹；没有时返回 sql.ErrNoRows。
func (q *Queries) LastTraceForItem(ctx context.Context, runID int64, key string) (domain.ItemTrace, error) {
	return scanTrace(q.ex.QueryRowContext(ctx,
		`SELECT `+traceCols+` FROM item_traces WHERE run_id=? AND item_key=? ORDER BY id DESC LIMIT 1`, runID, key))
}

func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
