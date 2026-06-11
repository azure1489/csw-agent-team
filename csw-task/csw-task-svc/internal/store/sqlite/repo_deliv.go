package sqlite

import (
	"context"
	"database/sql"
	"errors"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── files ──

// FileBySHA 据内容哈希查文件（去重）。无则 sql.ErrNoRows。
func (q *Queries) FileBySHA(ctx context.Context, sha string) (domain.File, error) {
	return scanFile(q.ex.QueryRowContext(ctx, `SELECT `+fileCols+` FROM files WHERE sha256=? LIMIT 1`, sha))
}

// GetFile 取文件。
func (q *Queries) GetFile(ctx context.Context, id int64) (domain.File, error) {
	return scanFile(q.ex.QueryRowContext(ctx, `SELECT `+fileCols+` FROM files WHERE id=?`, id))
}

const fileCols = `id, sha256, filename, byte_size, content_type, storage_path, uploaded_by`

func scanFile(s interface{ Scan(...any) error }) (domain.File, error) {
	var f domain.File
	var ct sql.NullString
	var by sql.NullInt64
	err := s.Scan(&f.ID, &f.SHA256, &f.Filename, &f.ByteSize, &ct, &f.StoragePath, &by)
	f.ContentType, f.UploadedBy = ct.String, ptrI64(by)
	return f, err
}

// InsertFile 写文件元数据，返回 file id。
func (q *Queries) InsertFile(ctx context.Context, f domain.File) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO files (sha256, filename, byte_size, content_type, storage_path, uploaded_by) VALUES (?,?,?,?,?,?)`,
		f.SHA256, f.Filename, f.ByteSize, f.ContentType, f.StoragePath, nullI64(f.UploadedBy))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// ── deliverables ──

// MaxDeliverableVersion 取 task 下某类交付物（派工单/产出）的最大版本号，无则 0。
func (q *Queries) MaxDeliverableVersion(ctx context.Context, taskID int64, isDispatch bool) (int, error) {
	var v sql.NullInt64
	err := q.ex.QueryRowContext(ctx,
		`SELECT MAX(version) FROM deliverables WHERE task_id=? AND is_dispatch=?`, taskID, b2i(isDispatch)).Scan(&v)
	if err != nil {
		return 0, err
	}
	if !v.Valid {
		return 0, nil
	}
	return int(v.Int64), nil
}

const delivCols = `id, task_id, is_dispatch, version, doc_type, producer_id, file_id, download_url,
	filename, title, summary, meta_json, self_check, editor_note, cur_gate, returned_at_gate, status`

func scanDeliverable(s interface{ Scan(...any) error }) (domain.Deliverable, error) {
	var d domain.Deliverable
	var isDispatch int
	var producer, fileID sql.NullInt64
	var url, fn, title, sum, meta, sc, note sql.NullString
	var retGate sql.NullInt64
	var status string
	err := s.Scan(&d.ID, &d.TaskID, &isDispatch, &d.Version, &d.DocType, &producer, &fileID, &url,
		&fn, &title, &sum, &meta, &sc, &note, &d.CurGate, &retGate, &status)
	d.IsDispatch = isDispatch == 1
	d.ProducerID, d.FileID = ptrI64(producer), ptrI64(fileID)
	d.DownloadURL, d.Filename, d.Title, d.Summary = url.String, fn.String, title.String, sum.String
	d.MetaJSON, d.SelfCheck, d.EditorNote = meta.String, sc.String, note.String
	d.ReturnedAtGate = ptrInt(retGate)
	d.Status = domain.DeliverableStatus(status)
	return d, err
}

// InsertDeliverable 写交付物，返回 id。
func (q *Queries) InsertDeliverable(ctx context.Context, d domain.Deliverable) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO deliverables
		  (task_id, is_dispatch, version, doc_type, producer_id, file_id, download_url,
		   filename, title, summary, meta_json, self_check, editor_note, cur_gate, returned_at_gate, status)
		VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
		d.TaskID, b2i(d.IsDispatch), d.Version, d.DocType, nullI64(d.ProducerID), nullI64(d.FileID), d.DownloadURL,
		d.Filename, d.Title, d.Summary, d.MetaJSON, d.SelfCheck, d.EditorNote, d.CurGate, nullInt(d.ReturnedAtGate), string(d.Status))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// GetDeliverable 取交付物。
func (q *Queries) GetDeliverable(ctx context.Context, id int64) (domain.Deliverable, error) {
	return scanDeliverable(q.ex.QueryRowContext(ctx, `SELECT `+delivCols+` FROM deliverables WHERE id=?`, id))
}

// ListDeliverablesByTask 列 task 下全部交付物（派工单 + 产出，按 id）。
func (q *Queries) ListDeliverablesByTask(ctx context.Context, taskID int64) ([]domain.Deliverable, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+delivCols+` FROM deliverables WHERE task_id=? ORDER BY id`, taskID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Deliverable
	for rows.Next() {
		d, err := scanDeliverable(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, d)
	}
	return out, rows.Err()
}

// LatestPassedDeliverable 取 task 已通过的最新产出（is_dispatch=0, passed, 最大 version）——用于上游预填。
func (q *Queries) LatestPassedDeliverable(ctx context.Context, taskID int64) (domain.Deliverable, error) {
	return scanDeliverable(q.ex.QueryRowContext(ctx, `SELECT `+delivCols+`
		FROM deliverables WHERE task_id=? AND is_dispatch=0 AND status='passed'
		ORDER BY version DESC LIMIT 1`, taskID))
}

// SetDeliverableGate 更新交付物闸进度/状态/退回闸位。
func (q *Queries) SetDeliverableGate(ctx context.Context, id int64, curGate int, status domain.DeliverableStatus, returnedAtGate *int) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE deliverables SET cur_gate=?, status=?, returned_at_gate=? WHERE id=?`,
		curGate, string(status), nullInt(returnedAtGate), id)
	return err
}

// ── upstreams ──

// InsertUpstream 写一条上游链接。
func (q *Queries) InsertUpstream(ctx context.Context, deliverableID int64, label, url string, upstreamID *int64) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO deliverable_upstreams (deliverable_id, label, upstream_url, upstream_id) VALUES (?,?,?,?)`,
		deliverableID, label, url, nullI64(upstreamID))
	return err
}

// ListUpstreams 列某交付物的上游。
func (q *Queries) ListUpstreams(ctx context.Context, deliverableID int64) ([]domain.Upstream, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT label, upstream_url, upstream_id FROM deliverable_upstreams WHERE deliverable_id=? ORDER BY rowid`, deliverableID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Upstream
	for rows.Next() {
		var u domain.Upstream
		var label sql.NullString
		var upID sql.NullInt64
		if err := rows.Scan(&label, &u.UpstreamURL, &upID); err != nil {
			return nil, err
		}
		u.Label, u.UpstreamID = label.String, ptrI64(upID)
		out = append(out, u)
	}
	return out, rows.Err()
}

// ── reviews ──

// InsertReview 写一条审核，返回 id。
func (q *Queries) InsertReview(ctx context.Context, r domain.Review) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO reviews (deliverable_id, task_gate_id, reviewer_id, verdict, comment, return_direction, return_location)
		VALUES (?,?,?,?,?,?,?)`,
		r.DeliverableID, r.TaskGateID, nullI64(r.ReviewerID), string(r.Verdict), r.Comment, r.ReturnDirection, r.ReturnLocation)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// ListReviewsByDeliverable 列某交付物的审核记录。
func (q *Queries) ListReviewsByDeliverable(ctx context.Context, deliverableID int64) ([]domain.Review, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT id, deliverable_id, task_gate_id, reviewer_id, verdict, comment, return_direction, return_location
		FROM reviews WHERE deliverable_id=? ORDER BY id`, deliverableID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Review
	for rows.Next() {
		var r domain.Review
		var reviewer sql.NullInt64
		var comment, dir, loc sql.NullString
		var verdict string
		if err := rows.Scan(&r.ID, &r.DeliverableID, &r.TaskGateID, &reviewer, &verdict, &comment, &dir, &loc); err != nil {
			return nil, err
		}
		r.ReviewerID = ptrI64(reviewer)
		r.Verdict = domain.Verdict(verdict)
		r.Comment, r.ReturnDirection, r.ReturnLocation = comment.String, dir.String, loc.String
		out = append(out, r)
	}
	return out, rows.Err()
}

// ── events ──

// InsertEvent 写进度事件。
func (q *Queries) InsertEvent(ctx context.Context, e domain.Event) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO events (run_id, task_id, deliverable_id, actor_id, type, detail_json) VALUES (?,?,?,?,?,?)`,
		nullI64(e.RunID), nullI64(e.TaskID), nullI64(e.DeliverableID), nullI64(e.ActorID), e.Type, e.DetailJSON)
	return err
}

// ListEventsByRun 列实例事件流水（时间线）。
func (q *Queries) ListEventsByRun(ctx context.Context, runID int64) ([]domain.Event, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT id, run_id, task_id, deliverable_id, actor_id, type, detail_json, created_at
		 FROM events WHERE run_id=? ORDER BY id`, runID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Event
	for rows.Next() {
		var e domain.Event
		var run, task, deliv, actor sql.NullInt64
		var detail sql.NullString
		if err := rows.Scan(&e.ID, &run, &task, &deliv, &actor, &e.Type, &detail, &e.CreatedAt); err != nil {
			return nil, err
		}
		e.RunID, e.TaskID, e.DeliverableID, e.ActorID = ptrI64(run), ptrI64(task), ptrI64(deliv), ptrI64(actor)
		e.DetailJSON = detail.String
		out = append(out, e)
	}
	return out, rows.Err()
}

// ── idempotency ──

// Idempotency 取幂等键的首次响应。found=false 表示未命中。
func (q *Queries) Idempotency(ctx context.Context, key string) (string, bool, error) {
	var resp sql.NullString
	err := q.ex.QueryRowContext(ctx, `SELECT response_json FROM idempotency_keys WHERE key=?`, key).Scan(&resp)
	if errors.Is(err, sql.ErrNoRows) {
		return "", false, nil
	}
	if err != nil {
		return "", false, err
	}
	return resp.String, true, nil
}

// PutIdempotency 记录幂等键 + 首次响应。
func (q *Queries) PutIdempotency(ctx context.Context, key string, agentID *int64, endpoint, responseJSON string) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT OR IGNORE INTO idempotency_keys (key, agent_id, endpoint, response_json) VALUES (?,?,?,?)`,
		key, nullI64(agentID), endpoint, responseJSON)
	return err
}

// ── 审核队列（中枢 inbox）──

// PendingReview 一条待审：交付物 + 下一道闸信息。
type PendingReview struct {
	StageName     string
	ReviewerRole  string
	GateName      string
	DeliverableID int64
	TaskID        int64
	RunID         int64
	Version       int
	NextGate      int
	RelayedByHub  bool
}

// ListPendingReviews 列全部待审交付物 + 其下一道闸（active run）。调用方按 reviewer_role/中枢身份过滤。
func (q *Queries) ListPendingReviews(ctx context.Context) ([]PendingReview, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT d.id, d.task_id, t.run_id, t.stage_name, d.version,
		       tg.gate_order, tg.reviewer_role, tg.relayed_by_hub, tg.name
		FROM deliverables d
		JOIN tasks t ON t.id = d.task_id
		JOIN runs  r ON r.id = t.run_id
		JOIN task_gates tg ON tg.task_id = d.task_id AND tg.gate_order = d.cur_gate + 1
		WHERE d.is_dispatch = 0
		  AND d.status IN ('submitted','in_review')
		  AND r.status = 'active'
		ORDER BY t.run_id, t.seq`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []PendingReview
	for rows.Next() {
		var p PendingReview
		var relayed int
		var name sql.NullString
		if err := rows.Scan(&p.DeliverableID, &p.TaskID, &p.RunID, &p.StageName, &p.Version,
			&p.NextGate, &p.ReviewerRole, &relayed, &name); err != nil {
			return nil, err
		}
		p.RelayedByHub = relayed == 1
		p.GateName = name.String
		out = append(out, p)
	}
	return out, rows.Err()
}
