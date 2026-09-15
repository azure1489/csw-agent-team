package sqlite

import (
	"context"
	"database/sql"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── run 授权（平台写操作护栏）──

// authActiveCond 有效授权：未撤销且未过期（expires_at 统一存 UTC 秒级 ISO8601，可直接字符串比较）。
const authActiveCond = `revoked_at IS NULL AND (expires_at IS NULL OR expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now'))`

const authCols = `id, run_id, scope, granted_by, source_quote, granted_at, expires_at, revoked_at, revoked_by`

func scanAuthorization(s interface{ Scan(...any) error }) (domain.RunAuthorization, error) {
	var a domain.RunAuthorization
	var scope string
	var grantedBy, revokedBy sql.NullInt64
	var expires, revoked sql.NullString
	err := s.Scan(&a.ID, &a.RunID, &scope, &grantedBy, &a.SourceQuote, &a.GrantedAt, &expires, &revoked, &revokedBy)
	a.Scope = domain.AuthScope(scope)
	a.GrantedBy, a.RevokedBy = ptrI64(grantedBy), ptrI64(revokedBy)
	a.ExpiresAt, a.RevokedAt = expires.String, revoked.String
	return a, err
}

func (q *Queries) listAuthorizations(ctx context.Context, where string, args ...any) ([]domain.RunAuthorization, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+authCols+` FROM run_authorizations WHERE `+where+` ORDER BY id`, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.RunAuthorization
	for rows.Next() {
		a, err := scanAuthorization(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, a)
	}
	return out, rows.Err()
}

// InsertAuthorization 写一条授权，返回 id。
func (q *Queries) InsertAuthorization(ctx context.Context, a domain.RunAuthorization) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO run_authorizations (run_id, scope, granted_by, source_quote, expires_at) VALUES (?,?,?,?,?)`,
		a.RunID, string(a.Scope), nullI64(a.GrantedBy), a.SourceQuote, nullIfEmpty(a.ExpiresAt))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// GetAuthorization 取授权。
func (q *Queries) GetAuthorization(ctx context.Context, id int64) (domain.RunAuthorization, error) {
	return scanAuthorization(q.ex.QueryRowContext(ctx, `SELECT `+authCols+` FROM run_authorizations WHERE id=?`, id))
}

// ListAuthorizations 列某 run 全部授权记录（含已撤销、已过期）。
func (q *Queries) ListAuthorizations(ctx context.Context, runID int64) ([]domain.RunAuthorization, error) {
	return q.listAuthorizations(ctx, `run_id=?`, runID)
}

// ListActiveAuthorizations 列某 run 当前有效授权。
func (q *Queries) ListActiveAuthorizations(ctx context.Context, runID int64) ([]domain.RunAuthorization, error) {
	return q.listAuthorizations(ctx, `run_id=? AND `+authActiveCond, runID)
}

// ActiveAuthorizationByScope 取某 run 某范围的有效授权（最近一条）；无则 sql.ErrNoRows。
func (q *Queries) ActiveAuthorizationByScope(ctx context.Context, runID int64, scope domain.AuthScope) (domain.RunAuthorization, error) {
	return scanAuthorization(q.ex.QueryRowContext(ctx,
		`SELECT `+authCols+` FROM run_authorizations WHERE run_id=? AND scope=? AND `+authActiveCond+` ORDER BY id DESC LIMIT 1`,
		runID, string(scope)))
}

// HasActiveAuthorization 某 run 是否存在任一给定范围的有效授权。
func (q *Queries) HasActiveAuthorization(ctx context.Context, runID int64, scopes []domain.AuthScope) (bool, error) {
	if len(scopes) == 0 {
		return false, nil
	}
	args := []any{runID}
	for _, s := range scopes {
		args = append(args, string(s))
	}
	var n int
	err := q.ex.QueryRowContext(ctx,
		`SELECT count(*) FROM run_authorizations WHERE run_id=? AND scope IN (?`+strings.Repeat(",?", len(scopes)-1)+`) AND `+authActiveCond,
		args...).Scan(&n)
	return n > 0, err
}

// RevokeAuthorizations 撤销某 run 某范围尚未撤销的授权，返回撤销条数。
func (q *Queries) RevokeAuthorizations(ctx context.Context, runID int64, scope domain.AuthScope, revokedBy *int64) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `
		UPDATE run_authorizations SET revoked_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), revoked_by=?
		WHERE run_id=? AND scope=? AND revoked_at IS NULL`, nullI64(revokedBy), runID, string(scope))
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}
