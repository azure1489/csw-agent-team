package sqlite

import (
	"context"
	"database/sql"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// isoUTC 与库内时间戳同格式（UTC 秒级 ISO8601），可直接字符串比较。
func isoUTC(t time.Time) string { return t.UTC().Format("2006-01-02T15:04:05Z") }

// InsertEventID 写进度事件并返回 id（需要挂 outbox 的事件用）。
func (q *Queries) InsertEventID(ctx context.Context, e domain.Event) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO events (run_id, task_id, deliverable_id, actor_id, type, detail_json) VALUES (?,?,?,?,?,?)`,
		nullI64(e.RunID), nullI64(e.TaskID), nullI64(e.DeliverableID), nullI64(e.ActorID), e.Type, e.DetailJSON)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// InsertOutbox 写一条待发通知（event_id 唯一：同一事件最多一行）。
func (q *Queries) InsertOutbox(ctx context.Context, o domain.Outbox) (int64, error) {
	if o.Channel == "" {
		o.Channel = "lark_group"
	}
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO outbox (event_id, event_type, run_id, task_id, deliverable_id, channel, target_role, cc_roles, payload_json)
		VALUES (?,?,?,?,?,?,?,?,?)`,
		o.EventID, o.EventType, nullI64(o.RunID), nullI64(o.TaskID), nullI64(o.DeliverableID), o.Channel,
		nullIfEmpty(o.TargetRole), nullIfEmpty(o.CCRoles), o.PayloadJSON)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

const outboxCols = `id, event_id, event_type, run_id, task_id, deliverable_id, channel, target_role, cc_roles,
	payload_json, status, attempts, claimed_at, created_at, last_error`

func scanOutbox(s interface{ Scan(...any) error }) (domain.Outbox, error) {
	var o domain.Outbox
	var run, task, deliv sql.NullInt64
	var target, cc, claimed, lastErr sql.NullString
	err := s.Scan(&o.ID, &o.EventID, &o.EventType, &run, &task, &deliv, &o.Channel, &target, &cc,
		&o.PayloadJSON, &o.Status, &o.Attempts, &claimed, &o.CreatedAt, &lastErr)
	o.RunID, o.TaskID, o.DeliverableID = ptrI64(run), ptrI64(task), ptrI64(deliv)
	o.TargetRole, o.CCRoles, o.ClaimedAt, o.LastError = target.String, cc.String, claimed.String, lastErr.String
	return o, err
}

// ClaimDueOutbox 领取到期的待发通知（claimed_at 做 CAS；领取超过 lease 未完结的视为失效，可重领）。须在事务内调用。
func (q *Queries) ClaimDueOutbox(ctx context.Context, now time.Time, limit int, lease time.Duration) ([]domain.Outbox, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+outboxCols+` FROM outbox
		WHERE status='pending' AND next_at <= ? AND (claimed_at IS NULL OR claimed_at <= ?)
		ORDER BY id LIMIT ?`, isoUTC(now), isoUTC(now.Add(-lease)), limit)
	if err != nil {
		return nil, err
	}
	var cand []domain.Outbox
	for rows.Next() {
		o, err := scanOutbox(rows)
		if err != nil {
			rows.Close()
			return nil, err
		}
		cand = append(cand, o)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}
	var out []domain.Outbox
	for _, o := range cand {
		res, err := q.ex.ExecContext(ctx,
			`UPDATE outbox SET claimed_at=? WHERE id=? AND status='pending' AND COALESCE(claimed_at,'')=?`,
			isoUTC(now), o.ID, o.ClaimedAt)
		if err != nil {
			return nil, err
		}
		if n, _ := res.RowsAffected(); n == 1 {
			out = append(out, o)
		}
	}
	return out, nil
}

// MarkOutboxSent 记已发送。
func (q *Queries) MarkOutboxSent(ctx context.Context, id int64, messageID string, now time.Time) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE outbox SET status='sent', sent_at=?, message_id=?, claimed_at=NULL, last_error=NULL WHERE id=?`,
		isoUTC(now), nullIfEmpty(messageID), id)
	return err
}

// MarkOutboxFailed 记发送失败：attempts 递增，按退避设下次时间；dead=true 时不再重试。
func (q *Queries) MarkOutboxFailed(ctx context.Context, id int64, attempts int, nextAt time.Time, lastErr string, dead bool) error {
	status := "pending"
	if dead {
		status = "dead"
	}
	_, err := q.ex.ExecContext(ctx,
		`UPDATE outbox SET status=?, attempts=?, next_at=?, last_error=?, claimed_at=NULL WHERE id=?`,
		status, attempts, isoUTC(nextAt), lastErr, id)
	return err
}

// CountPendingOutbox 待发通知条数（/healthz 展示）。
func (q *Queries) CountPendingOutbox(ctx context.Context) (int, error) {
	var n int
	err := q.ex.QueryRowContext(ctx, `SELECT count(*) FROM outbox WHERE status='pending'`).Scan(&n)
	return n, err
}

// GetRunProgressDigest 取 run 上次发进度卡时的摘要。
func (q *Queries) GetRunProgressDigest(ctx context.Context, runID int64) (string, error) {
	var d sql.NullString
	err := q.ex.QueryRowContext(ctx, `SELECT progress_digest FROM runs WHERE id=?`, runID).Scan(&d)
	return d.String, err
}

// SetRunProgressDigest 记 run 本次进度卡摘要。
func (q *Queries) SetRunProgressDigest(ctx context.Context, runID int64, digest string) error {
	_, err := q.ex.ExecContext(ctx, `UPDATE runs SET progress_digest=? WHERE id=?`, digest, runID)
	return err
}
