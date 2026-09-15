package sqlite

import (
	"context"
	"database/sql"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── 发布记录 ──

const postCols = `id, platform, account, post_id, url, published_at, title, body_text, template_ver, source, state, run_id,
	raw_json, first_seen_at, updated_at`

func scanPost(s interface{ Scan(...any) error }) (domain.LedgerPost, error) {
	var p domain.LedgerPost
	var url, pub, body, tpl, raw sql.NullString
	var run sql.NullInt64
	err := s.Scan(&p.ID, &p.Platform, &p.Account, &p.PostID, &url, &pub, &p.Title, &body, &tpl, &p.Source, &p.State, &run,
		&raw, &p.FirstSeenAt, &p.UpdatedAt)
	p.URL, p.PublishedAt, p.BodyText, p.TemplateVer, p.RawJSON, p.RunID = url.String, pub.String, body.String, tpl.String, raw.String, ptrI64(run)
	return p, err
}

// UpsertLedgerPost 按 (platform, account, post_id) 写入或更新；空字段不覆盖已有值。返回 id 与是否新建。
func (q *Queries) UpsertLedgerPost(ctx context.Context, p domain.LedgerPost) (int64, bool, error) {
	if p.Source == "" {
		p.Source = "sync"
	}
	if p.State == "" {
		p.State = "published"
	}
	var id int64
	err := q.ex.QueryRowContext(ctx, `SELECT id FROM ledger_published_posts WHERE platform=? AND account=? AND post_id=?`,
		p.Platform, p.Account, p.PostID).Scan(&id)
	if err == sql.ErrNoRows {
		res, err := q.ex.ExecContext(ctx, `
			INSERT INTO ledger_published_posts (platform, account, post_id, url, published_at, title, body_text, template_ver, source, state, run_id, raw_json)
			VALUES (?,?,?,?,?,?,?,?,?,?,?,?)`,
			p.Platform, p.Account, p.PostID, nullIfEmpty(p.URL), nullIfEmpty(p.PublishedAt), p.Title, nullIfEmpty(p.BodyText),
			nullIfEmpty(p.TemplateVer), p.Source, p.State, nullI64(p.RunID), nullIfEmpty(p.RawJSON))
		if err != nil {
			return 0, false, err
		}
		id, err = res.LastInsertId()
		return id, true, err
	}
	if err != nil {
		return 0, false, err
	}
	_, err = q.ex.ExecContext(ctx, `
		UPDATE ledger_published_posts SET
			url=COALESCE(?, url), published_at=COALESCE(?, published_at),
			title=CASE WHEN ?<>'' THEN ? ELSE title END, body_text=COALESCE(?, body_text),
			template_ver=COALESCE(?, template_ver), state=?, raw_json=COALESCE(?, raw_json),
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`,
		nullIfEmpty(p.URL), nullIfEmpty(p.PublishedAt), p.Title, p.Title, nullIfEmpty(p.BodyText),
		nullIfEmpty(p.TemplateVer), p.State, nullIfEmpty(p.RawJSON), id)
	return id, false, err
}

// GetLedgerPost 取一篇记录。
func (q *Queries) GetLedgerPost(ctx context.Context, platform, account, postID string) (domain.LedgerPost, error) {
	return scanPost(q.ex.QueryRowContext(ctx, `SELECT `+postCols+` FROM ledger_published_posts WHERE platform=? AND account=? AND post_id=?`,
		platform, account, postID))
}

// LedgerFilter 发布记录查询条件。
type LedgerFilter struct {
	Since    string // UTC 时间下限（published_at >=）
	Platform string
	Brand    string
	State    string
	Q        string
	Limit    int
}

// ListLedgerPosts 按条件查发布记录（新到旧）。Brand 匹配拆条品牌或标题；Q 匹配标题或正文。
func (q *Queries) ListLedgerPosts(ctx context.Context, f LedgerFilter) ([]domain.LedgerPost, error) {
	var where []string
	var args []any
	if f.Since != "" {
		where = append(where, "published_at >= ?")
		args = append(args, f.Since)
	}
	if f.Platform != "" {
		where = append(where, "platform = ?")
		args = append(args, f.Platform)
	}
	if f.State != "" {
		where = append(where, "state = ?")
		args = append(args, f.State)
	}
	if f.Brand != "" {
		where = append(where, `(lower(title) LIKE ? OR EXISTS (SELECT 1 FROM ledger_post_items i WHERE i.post_ref = ledger_published_posts.id AND lower(i.brand) LIKE ?))`)
		b := "%" + strings.ToLower(f.Brand) + "%"
		args = append(args, b, b)
	}
	if f.Q != "" {
		where = append(where, "(title LIKE ? OR body_text LIKE ?)")
		args = append(args, "%"+f.Q+"%", "%"+f.Q+"%")
	}
	query := `SELECT ` + postCols + ` FROM ledger_published_posts`
	if len(where) > 0 {
		query += " WHERE " + strings.Join(where, " AND ")
	}
	limit := f.Limit
	if limit <= 0 || limit > 500 {
		limit = 200
	}
	query += " ORDER BY published_at DESC, id DESC LIMIT ?"
	args = append(args, limit)
	rows, err := q.ex.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.LedgerPost
	for rows.Next() {
		p, err := scanPost(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, p)
	}
	return out, rows.Err()
}

// ReplacePostItems 整体替换一篇的拆条结果。
func (q *Queries) ReplacePostItems(ctx context.Context, postRef int64, items []domain.LedgerPostItem, splitBy, checkedBy string) error {
	if _, err := q.ex.ExecContext(ctx, `DELETE FROM ledger_post_items WHERE post_ref=?`, postRef); err != nil {
		return err
	}
	for i, it := range items {
		var checkedAt any
		if checkedBy != "" {
			checkedAt = time.Now().UTC().Format("2006-01-02T15:04:05Z")
		}
		if _, err := q.ex.ExecContext(ctx, `
			INSERT INTO ledger_post_items (post_ref, seq, item_key, brand, product, angle, title, source_url, split_by, checked_by, checked_at)
			VALUES (?,?,?,?,?,?,?,?,?,?,?)`,
			postRef, i+1, nullIfEmpty(it.ItemKey), nullIfEmpty(it.Brand), nullIfEmpty(it.Product), nullIfEmpty(it.Angle),
			nullIfEmpty(it.Title), nullIfEmpty(it.SourceURL), splitBy, nullIfEmpty(checkedBy), checkedAt); err != nil {
			return err
		}
	}
	return nil
}

// ListPostItems 列一篇的拆条。
func (q *Queries) ListPostItems(ctx context.Context, postRef int64) ([]domain.LedgerPostItem, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT id, post_ref, seq, item_key, brand, product, angle, title, source_url, split_by, checked_by, checked_at
		FROM ledger_post_items WHERE post_ref=? ORDER BY seq`, postRef)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.LedgerPostItem
	for rows.Next() {
		var it domain.LedgerPostItem
		var key, brand, product, angle, title, url, by, at sql.NullString
		if err := rows.Scan(&it.ID, &it.PostRef, &it.Seq, &key, &brand, &product, &angle, &title, &url, &it.SplitBy, &by, &at); err != nil {
			return nil, err
		}
		it.ItemKey, it.Brand, it.Product, it.Angle, it.Title, it.SourceURL = key.String, brand.String, product.String, angle.String, title.String, url.String
		it.CheckedBy, it.CheckedAt = by.String, at.String
		out = append(out, it)
	}
	return out, rows.Err()
}

// ── 同步批次 ──

// StartSyncRun 记一次批次开始。
func (q *Queries) StartSyncRun(ctx context.Context, platform, kind, windowFrom, windowTo string) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `INSERT INTO ledger_sync_runs (platform, kind, window_from, window_to) VALUES (?,?,?,?)`,
		platform, kind, nullIfEmpty(windowFrom), nullIfEmpty(windowTo))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// FinishSyncRun 记批次结果。
func (q *Queries) FinishSyncRun(ctx context.Context, r domain.LedgerSyncRun) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE ledger_sync_runs SET finished_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), ok=?, fetched=?, inserted=?, updated=?,
			gap_note=?, error=?, window_from=COALESCE(?, window_from), window_to=COALESCE(?, window_to)
		WHERE id=?`, b2i(r.OK), r.Fetched, r.Inserted, r.Updated, nullIfEmpty(r.GapNote), nullIfEmpty(r.Error),
		nullIfEmpty(r.WindowFrom), nullIfEmpty(r.WindowTo), r.ID)
	return err
}

// LedgerCoverage 各平台最近一次成功的同步 / 回填 / 导入（覆盖范围说明用）。
func (q *Queries) LedgerCoverage(ctx context.Context) ([]domain.LedgerSyncRun, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT id, platform, kind, started_at, COALESCE(finished_at,''), COALESCE(window_from,''), COALESCE(window_to,''),
		       COALESCE(gap_note,''), fetched, inserted, updated
		FROM ledger_sync_runs r
		WHERE ok=1 AND kind IN ('backfill','sync','import')
		  AND id = (SELECT MAX(id) FROM ledger_sync_runs x WHERE x.platform=r.platform AND x.ok=1 AND x.kind IN ('backfill','sync','import'))
		ORDER BY platform`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.LedgerSyncRun
	for rows.Next() {
		var r domain.LedgerSyncRun
		if err := rows.Scan(&r.ID, &r.Platform, &r.Kind, &r.StartedAt, &r.FinishedAt, &r.WindowFrom, &r.WindowTo, &r.GapNote,
			&r.Fetched, &r.Inserted, &r.Updated); err != nil {
			return nil, err
		}
		r.OK = true
		out = append(out, r)
	}
	return out, rows.Err()
}

// LastOKSyncWindowTo 某平台最近一次成功同步覆盖到的时间（增量同步起点）。
func (q *Queries) LastOKSyncWindowTo(ctx context.Context, platform string) (string, error) {
	var s sql.NullString
	err := q.ex.QueryRowContext(ctx, `SELECT window_to FROM ledger_sync_runs WHERE platform=? AND ok=1 AND kind IN ('backfill','sync')
		ORDER BY id DESC LIMIT 1`, platform).Scan(&s)
	if err == sql.ErrNoRows {
		return "", nil
	}
	return s.String, err
}

// ── 指标快照 ──

// InsertMetricSnapshot 写单篇快照（同一篇同一龄期同一采集时间只一条），返回是否新写入。
func (q *Queries) InsertMetricSnapshot(ctx context.Context, s domain.MetricSnapshot) (bool, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO ledger_metrics_snapshots (post_ref, platform, age_bucket, collected_at, window_from, window_to, raw_json, flags_json, definitions_json)
		VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(post_ref, age_bucket, collected_at) DO NOTHING`,
		s.PostRef, s.Platform, s.AgeBucket, s.CollectedAt, nullIfEmpty(s.WindowFrom), nullIfEmpty(s.WindowTo), s.RawJSON, nullIfEmpty(s.FlagsJSON),
		nullIfEmpty(s.DefinitionsJSON))
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}

// PostsDueForSnapshot 已发布且发布满 age、该龄期尚无快照的记录。
func (q *Queries) PostsDueForSnapshot(ctx context.Context, platform, bucket string, cutoff string) ([]domain.LedgerPost, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+postCols+` FROM ledger_published_posts p
		WHERE p.platform=? AND p.state='published' AND p.published_at IS NOT NULL AND p.published_at <= ?
		  AND NOT EXISTS (SELECT 1 FROM ledger_metrics_snapshots s WHERE s.post_ref=p.id AND s.age_bucket=?)
		ORDER BY p.published_at`, platform, cutoff, bucket)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.LedgerPost
	for rows.Next() {
		p, err := scanPost(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, p)
	}
	return out, rows.Err()
}

// ListMetricSnapshots 列一篇的快照（按采集时间）。
func (q *Queries) ListMetricSnapshots(ctx context.Context, postRef int64) ([]domain.MetricSnapshot, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT id, post_ref, platform, age_bucket, collected_at, COALESCE(window_from,''), COALESCE(window_to,''),
		raw_json, COALESCE(flags_json,''), COALESCE(definitions_json,'') FROM ledger_metrics_snapshots WHERE post_ref=? ORDER BY collected_at`, postRef)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.MetricSnapshot
	for rows.Next() {
		var s domain.MetricSnapshot
		if err := rows.Scan(&s.ID, &s.PostRef, &s.Platform, &s.AgeBucket, &s.CollectedAt, &s.WindowFrom, &s.WindowTo, &s.RawJSON, &s.FlagsJSON, &s.DefinitionsJSON); err != nil {
			return nil, err
		}
		out = append(out, s)
	}
	return out, rows.Err()
}

// InsertAccountSnapshot 写账号区间快照（同一区间同一采集时间只一条）。
func (q *Queries) InsertAccountSnapshot(ctx context.Context, s domain.AccountSnapshot) (bool, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO ledger_account_snapshots (platform, account, window_from, window_to, collected_at, raw_json, flags_json, definitions_json)
		VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(platform, account, window_from, window_to, collected_at) DO NOTHING`,
		s.Platform, s.Account, nullIfEmpty(s.WindowFrom), nullIfEmpty(s.WindowTo), s.CollectedAt, s.RawJSON, nullIfEmpty(s.FlagsJSON),
		nullIfEmpty(s.DefinitionsJSON))
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n == 1, err
}

// ── 编辑反馈 ──

// InsertFeedback 写一条反馈与标签。带 review_id 且已存在时不重复写入（返回已有 id，created=false）。
// 带 supersedes_id 时把被取代的记录置 superseded。
func (q *Queries) InsertFeedback(ctx context.Context, r domain.FeedbackRecord) (int64, bool, error) {
	if r.ReviewID != nil {
		var id int64
		err := q.ex.QueryRowContext(ctx, `SELECT id FROM ledger_feedback_records WHERE review_id=?`, *r.ReviewID).Scan(&id)
		if err == nil {
			return id, false, nil
		}
		if err != sql.ErrNoRows {
			return 0, false, err
		}
	}
	if r.Kind == "" {
		r.Kind = "recent"
	}
	if r.Stance == "" {
		r.Stance = "explicit"
	}
	if r.Status == "" {
		r.Status = "active"
	}
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO ledger_feedback_records (quote, said_at, source_ref, object_ref, object_version, image_refs_json, kind, stance, status,
			supersedes_id, expires_at, curated_by, review_id)
		VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)`,
		r.Quote, nullIfEmpty(r.SaidAt), nullIfEmpty(r.SourceRef), nullIfEmpty(r.ObjectRef), nullIfEmpty(r.ObjectVersion),
		nullIfEmpty(r.ImageRefsJSON), r.Kind, r.Stance, r.Status, nullI64(r.SupersedesID), nullIfEmpty(r.ExpiresAt),
		nullIfEmpty(r.CuratedBy), nullI64(r.ReviewID))
	if err != nil {
		return 0, false, err
	}
	id, err := res.LastInsertId()
	if err != nil {
		return 0, false, err
	}
	for _, t := range r.Tags {
		if t = strings.ToLower(strings.TrimSpace(t)); t == "" {
			continue
		}
		if _, err := q.ex.ExecContext(ctx, `INSERT OR IGNORE INTO ledger_feedback_tags (record_id, tag) VALUES (?,?)`, id, t); err != nil {
			return 0, false, err
		}
	}
	if r.SupersedesID != nil {
		if _, err := q.ex.ExecContext(ctx, `UPDATE ledger_feedback_records SET status='superseded' WHERE id=?`, *r.SupersedesID); err != nil {
			return 0, false, err
		}
	}
	return id, true, nil
}

// ExpireFeedback 把已过期的 temporary 记录置 expired，返回条数。
func (q *Queries) ExpireFeedback(ctx context.Context) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `UPDATE ledger_feedback_records SET status='expired'
		WHERE status='active' AND expires_at IS NOT NULL AND expires_at <= strftime('%Y-%m-%dT%H:%M:%SZ','now')`)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}

// ListActiveFeedback 有效反馈（active、未过期），给了标签时至少命中一个，按命中标签数与新近排序。
func (q *Queries) ListActiveFeedback(ctx context.Context, tags []string, limit int) ([]domain.FeedbackRecord, error) {
	if limit <= 0 || limit > 50 {
		limit = 20
	}
	args := []any{}
	match := "0"
	filter := ""
	var clean []string
	for _, t := range tags {
		if t = strings.ToLower(strings.TrimSpace(t)); t != "" {
			clean = append(clean, t)
		}
	}
	if len(clean) > 0 {
		ph := "?" + strings.Repeat(",?", len(clean)-1)
		match = `(SELECT count(*) FROM ledger_feedback_tags t WHERE t.record_id=r.id AND t.tag IN (` + ph + `))`
		for _, t := range clean {
			args = append(args, t)
		}
		filter = ` AND EXISTS (SELECT 1 FROM ledger_feedback_tags t WHERE t.record_id=r.id AND t.tag IN (` + ph + `))`
		for _, t := range clean {
			args = append(args, t)
		}
	}
	args = append(args, limit)
	rows, err := q.ex.QueryContext(ctx, `
		SELECT r.id, r.quote, COALESCE(r.said_at,''), COALESCE(r.source_ref,''), COALESCE(r.object_ref,''), COALESCE(r.object_version,''),
		       COALESCE(r.image_refs_json,''), r.kind, r.stance, r.status, COALESCE(r.expires_at,''), COALESCE(r.curated_by,''),
		       r.created_at, r.supersedes_id, r.review_id, `+match+` AS hits
		FROM ledger_feedback_records r
		WHERE r.status='active' AND (r.expires_at IS NULL OR r.expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now'))`+filter+`
		ORDER BY hits DESC, CASE r.stance WHEN 'explicit' THEN 0 ELSE 1 END, r.id DESC LIMIT ?`, args...)
	if err != nil {
		return nil, err
	}
	var out []domain.FeedbackRecord
	for rows.Next() {
		var r domain.FeedbackRecord
		var sup, rev sql.NullInt64
		var hits int
		if err := rows.Scan(&r.ID, &r.Quote, &r.SaidAt, &r.SourceRef, &r.ObjectRef, &r.ObjectVersion, &r.ImageRefsJSON, &r.Kind,
			&r.Stance, &r.Status, &r.ExpiresAt, &r.CuratedBy, &r.CreatedAt, &sup, &rev, &hits); err != nil {
			rows.Close()
			return nil, err
		}
		r.SupersedesID, r.ReviewID = ptrI64(sup), ptrI64(rev)
		out = append(out, r)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}
	for i := range out {
		trs, err := q.ex.QueryContext(ctx, `SELECT tag FROM ledger_feedback_tags WHERE record_id=? ORDER BY tag`, out[i].ID)
		if err != nil {
			return nil, err
		}
		for trs.Next() {
			var t string
			if err := trs.Scan(&t); err != nil {
				trs.Close()
				return nil, err
			}
			out[i].Tags = append(out[i].Tags, t)
		}
		trs.Close()
	}
	return out, nil
}
