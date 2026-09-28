package sqlite

import (
	"context"
	"database/sql"
)

// ReviewWait 一份停在某道闸上等审的交付物：从什么时候开始等、提醒过几次、升级过没有。
type ReviewWait struct {
	Since          string // 到达这道闸的时间：首道闸 = 提交时间，之后 = 上一道闸通过的时间
	ReviewerRole   string
	GateName       string
	StageName      string
	LastRemindedAt string // 空 = 还没提醒过
	EscalatedAt    string // 空 = 还没升级
	DeliverableID  int64
	TaskID         int64
	RunID          int64
	Version        int
	GateOrder      int
	Reminded       int
	RelayedByHub   bool
}

// ListReviewWaits 列进行中 run 里全部待审交付物，带等待起点与提醒记录。是否到点由调用方按时间判定。
// 只看最近 48 小时内开始的等待：更早的只可能是历史遗留，上线时一次补发只会刷屏。
func (q *Queries) ListReviewWaits(ctx context.Context) ([]ReviewWait, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT d.id, d.task_id, t.run_id, t.stage_name, d.version,
		       tg.gate_order, tg.reviewer_role, tg.relayed_by_hub, tg.name,
		       MAX(d.created_at, IFNULL((SELECT MAX(rv.created_at) FROM reviews rv WHERE rv.deliverable_id = d.id), '')) AS since,
		       IFNULL(rr.reminded, 0), IFNULL(rr.last_reminded_at, ''), IFNULL(rr.escalated_at, '')
		FROM deliverables d
		JOIN tasks t ON t.id = d.task_id
		JOIN runs  r ON r.id = t.run_id
		JOIN task_gates tg ON tg.task_id = d.task_id AND tg.gate_order = d.cur_gate + 1
		LEFT JOIN review_reminders rr ON rr.deliverable_id = d.id AND rr.gate_order = tg.gate_order
		WHERE d.kind IN ('output','edit')
		  AND d.status IN ('submitted','in_review')
		  AND t.status = 'review'
		  AND r.status = 'active'
		  AND MAX(d.created_at, IFNULL((SELECT MAX(rv.created_at) FROM reviews rv WHERE rv.deliverable_id = d.id), ''))
		      >= strftime('%Y-%m-%dT%H:%M:%SZ','now','-48 hours')
		ORDER BY t.run_id, t.seq`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []ReviewWait
	for rows.Next() {
		var w ReviewWait
		var relayed int
		var name sql.NullString
		if err := rows.Scan(&w.DeliverableID, &w.TaskID, &w.RunID, &w.StageName, &w.Version,
			&w.GateOrder, &w.ReviewerRole, &relayed, &name, &w.Since,
			&w.Reminded, &w.LastRemindedAt, &w.EscalatedAt); err != nil {
			return nil, err
		}
		w.RelayedByHub = relayed == 1
		w.GateName = name.String
		out = append(out, w)
	}
	return out, rows.Err()
}

// MarkReviewReminded 条件记一次待审提醒：提醒次数仍是 seen 时才加一，返回是否本次记上。
// 并发扫描时只有一方拿到，同一档不会提醒两次。
func (q *Queries) MarkReviewReminded(ctx context.Context, deliverableID int64, gateOrder, seen int) (bool, error) {
	var res sql.Result
	var err error
	if seen == 0 {
		res, err = q.ex.ExecContext(ctx, `
			INSERT OR IGNORE INTO review_reminders (deliverable_id, gate_order, reminded, last_reminded_at)
			VALUES (?, ?, 1, strftime('%Y-%m-%dT%H:%M:%SZ','now'))`, deliverableID, gateOrder)
	} else {
		res, err = q.ex.ExecContext(ctx, `
			UPDATE review_reminders SET reminded = reminded + 1,
			       last_reminded_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
			WHERE deliverable_id = ? AND gate_order = ? AND reminded = ?`, deliverableID, gateOrder, seen)
	}
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}

// MarkReviewEscalated 条件记升级：同一次等待只升级一次，返回是否本次记上。
func (q *Queries) MarkReviewEscalated(ctx context.Context, deliverableID int64, gateOrder int) (bool, error) {
	res, err := q.ex.ExecContext(ctx, `
		UPDATE review_reminders SET escalated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE deliverable_id = ? AND gate_order = ? AND escalated_at IS NULL`, deliverableID, gateOrder)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}
