package sqlite

import (
	"context"
	"database/sql"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── run_items（条目级推进）──

const itemCols = `id, run_id, item_key, title, brand, product, source_url, published_at, status,
	decided_by, decided_at, decision_source, created_at, updated_at`

func scanItem(s interface{ Scan(...any) error }) (domain.RunItem, error) {
	var it domain.RunItem
	var brand, product, url, pub, decAt, decSrc sql.NullString
	var decBy sql.NullInt64
	var status string
	err := s.Scan(&it.ID, &it.RunID, &it.ItemKey, &it.Title, &brand, &product, &url, &pub, &status,
		&decBy, &decAt, &decSrc, &it.CreatedAt, &it.UpdatedAt)
	it.Brand, it.Product, it.SourceURL, it.PublishedAt = brand.String, product.String, url.String, pub.String
	it.Status, it.DecidedBy, it.DecidedAt, it.DecisionSource = domain.ItemStatus(status), ptrI64(decBy), decAt.String, decSrc.String
	return it, err
}

// UpsertItem 登记或更新一条条目的描述字段；已存在时只在「candidate ↔ shortlisted」之间改状态，不覆盖中枢的决定。
func (q *Queries) UpsertItem(ctx context.Context, it domain.RunItem) error {
	if it.Status == "" {
		it.Status = domain.ItemCandidate
	}
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO run_items (run_id, item_key, title, brand, product, source_url, published_at, status)
		VALUES (?,?,?,?,?,?,?,?)
		ON CONFLICT(run_id, item_key) DO UPDATE SET
			title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE run_items.title END,
			brand = COALESCE(excluded.brand, run_items.brand),
			product = COALESCE(excluded.product, run_items.product),
			source_url = COALESCE(excluded.source_url, run_items.source_url),
			published_at = COALESCE(excluded.published_at, run_items.published_at),
			status = CASE WHEN run_items.status IN ('candidate','shortlisted') THEN excluded.status ELSE run_items.status END,
			updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')`,
		it.RunID, it.ItemKey, it.Title, nullIfEmpty(it.Brand), nullIfEmpty(it.Product), nullIfEmpty(it.SourceURL),
		nullIfEmpty(it.PublishedAt), string(it.Status))
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
