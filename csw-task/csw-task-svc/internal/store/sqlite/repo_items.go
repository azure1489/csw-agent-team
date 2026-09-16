package sqlite

import (
	"context"
	"database/sql"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── run_items（条目级推进）──

const itemCols = `id, run_id, item_key, title, brand, product, source_url, published_at, status, "rank",
	decided_by, decided_at, decision_source, created_at, updated_at`

func scanItem(s interface{ Scan(...any) error }) (domain.RunItem, error) {
	var it domain.RunItem
	var brand, product, url, pub, rank, decAt, decSrc sql.NullString
	var decBy sql.NullInt64
	var status string
	err := s.Scan(&it.ID, &it.RunID, &it.ItemKey, &it.Title, &brand, &product, &url, &pub, &status, &rank,
		&decBy, &decAt, &decSrc, &it.CreatedAt, &it.UpdatedAt)
	it.Rank = rank.String
	it.Brand, it.Product, it.SourceURL, it.PublishedAt = brand.String, product.String, url.String, pub.String
	it.Status, it.DecidedBy, it.DecidedAt, it.DecisionSource = domain.ItemStatus(status), ptrI64(decBy), decAt.String, decSrc.String
	return it, err
}

// UpsertItem 登记或更新一条条目的描述字段；已存在时只在「线索 / 待核 / 成熟」之间改状态，不覆盖中枢的决定。
func (q *Queries) UpsertItem(ctx context.Context, it domain.RunItem) error {
	if it.Status == "" {
		it.Status = domain.ItemCandidate
	}
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO run_items (run_id, item_key, title, brand, product, source_url, published_at, status, "rank")
		VALUES (?,?,?,?,?,?,?,?,?)
		ON CONFLICT(run_id, item_key) DO UPDATE SET
			title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE run_items.title END,
			brand = COALESCE(excluded.brand, run_items.brand),
			product = COALESCE(excluded.product, run_items.product),
			source_url = COALESCE(excluded.source_url, run_items.source_url),
			published_at = COALESCE(excluded.published_at, run_items.published_at),
			status = CASE WHEN run_items.status IN ('candidate','pending_check','shortlisted') THEN excluded.status ELSE run_items.status END,
			"rank" = COALESCE(excluded."rank", run_items."rank"),
			updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')`,
		it.RunID, it.ItemKey, it.Title, nullIfEmpty(it.Brand), nullIfEmpty(it.Product), nullIfEmpty(it.SourceURL),
		nullIfEmpty(it.PublishedAt), string(it.Status), nullIfEmpty(it.Rank))
	return err
}

// GetItem 取某 run 的某条目。
func (q *Queries) GetItem(ctx context.Context, runID int64, key string) (domain.RunItem, error) {
	return scanItem(q.ex.QueryRowContext(ctx, `SELECT `+itemCols+` FROM run_items WHERE run_id=? AND item_key=?`, runID, key))
}

// ListItems 列某 run 的全部条目（按登记顺序）。
func (q *Queries) ListItems(ctx context.Context, runID int64) ([]domain.RunItem, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+itemCols+` FROM run_items WHERE run_id=? ORDER BY id`, runID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.RunItem
	for rows.Next() {
		it, err := scanItem(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, it)
	}
	return out, rows.Err()
}

// SetItemDecision 记中枢的条目决定（附 Van 原话）。
func (q *Queries) SetItemDecision(ctx context.Context, runID int64, key string, status domain.ItemStatus, by *int64, source string) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE run_items SET status=?, decided_by=?, decided_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), decision_source=?,
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE run_id=? AND item_key=?`, string(status), nullI64(by), nullIfEmpty(source), runID, key)
	return err
}

// SetItemStatus 只改条目状态（如逐条任务全部通过后置 written）。
func (q *Queries) SetItemStatus(ctx context.Context, runID int64, key string, status domain.ItemStatus) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE run_items SET status=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE run_id=? AND item_key=?`,
		string(status), runID, key)
	return err
}

// SetRunTargetCount 设整期目标条数。
func (q *Queries) SetRunTargetCount(ctx context.Context, runID int64, n int) error {
	_, err := q.ex.ExecContext(ctx, `UPDATE runs SET target_count=? WHERE id=?`, n, runID)
	return err
}

// DeleteTaskDep 删一条依赖边（条目被暂缓 / 否决时摘掉合流任务对它的等待）。
func (q *Queries) DeleteTaskDep(ctx context.Context, taskID, dependsOnID int64) error {
	_, err := q.ex.ExecContext(ctx, `DELETE FROM task_deps WHERE task_id=? AND depends_on_id=?`, taskID, dependsOnID)
	return err
}

// RecentItemDecision 最近几期被否决或暂缓的条目（跨 run），供 01 / 02 / 03 的任务详情随附，避免换说法重报。
type RecentItemDecision struct {
	Subject        string
	ItemKey        string
	Title          string
	Brand          string
	Status         string
	DecisionSource string
	DecidedAt      string
}

// ListRecentItemDecisions 列出 since（UTC，YYYY-MM-DD）之后被否决 / 暂缓的条目，新到旧。
func (q *Queries) ListRecentItemDecisions(ctx context.Context, since string, limit int) ([]RecentItemDecision, error) {
	if limit <= 0 {
		limit = 20
	}
	rows, err := q.ex.QueryContext(ctx, `
		SELECT r.subject, i.item_key, i.title, IFNULL(i.brand,''), i.status, IFNULL(i.decision_source,''), IFNULL(i.decided_at,'')
		FROM run_items i JOIN runs r ON r.id = i.run_id
		WHERE i.status IN ('rejected','deferred') AND IFNULL(i.decided_at, i.updated_at) >= ?
		ORDER BY IFNULL(i.decided_at, i.updated_at) DESC LIMIT ?`, since, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := make([]RecentItemDecision, 0, limit)
	for rows.Next() {
		var d RecentItemDecision
		if err := rows.Scan(&d.Subject, &d.ItemKey, &d.Title, &d.Brand, &d.Status, &d.DecisionSource, &d.DecidedAt); err != nil {
			return nil, err
		}
		out = append(out, d)
	}
	return out, rows.Err()
}
