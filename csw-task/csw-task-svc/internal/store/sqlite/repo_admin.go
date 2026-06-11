package sqlite

import (
	"context"
	"database/sql"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── admin_users ──

const adminUserCols = `id, username, display_name, role, status, last_login_at, created_at`

func scanAdminUser(s interface{ Scan(...any) error }) (domain.AdminUser, error) {
	var u domain.AdminUser
	var disp, last sql.NullString
	err := s.Scan(&u.ID, &u.Username, &disp, &u.Role, &u.Status, &last, &u.CreatedAt)
	u.DisplayName, u.LastLoginAt = disp.String, last.String
	return u, err
}

// CreateUser 建后台用户，返回 id。
func (q *Queries) CreateUser(ctx context.Context, username, passwordHash, displayName, role string) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO admin_users (username, password_hash, display_name, role) VALUES (?,?,?,?)`,
		username, passwordHash, displayName, role)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// GetUserByUsername 据用户名取用户 + password_hash（登录用）。
func (q *Queries) GetUserByUsername(ctx context.Context, username string) (domain.AdminUser, string, error) {
	var u domain.AdminUser
	var disp, last sql.NullString
	var hash string
	err := q.ex.QueryRowContext(ctx,
		`SELECT id, username, display_name, role, status, last_login_at, created_at, password_hash
		 FROM admin_users WHERE username=?`, username).
		Scan(&u.ID, &u.Username, &disp, &u.Role, &u.Status, &last, &u.CreatedAt, &hash)
	u.DisplayName, u.LastLoginAt = disp.String, last.String
	return u, hash, err
}

// GetUser 据 id 取用户。
func (q *Queries) GetUser(ctx context.Context, id int64) (domain.AdminUser, error) {
	return scanAdminUser(q.ex.QueryRowContext(ctx, `SELECT `+adminUserCols+` FROM admin_users WHERE id=?`, id))
}

// ListUsers 列全部后台用户。
func (q *Queries) ListUsers(ctx context.Context) ([]domain.AdminUser, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+adminUserCols+` FROM admin_users ORDER BY id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.AdminUser
	for rows.Next() {
		u, err := scanAdminUser(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, u)
	}
	return out, rows.Err()
}

// UpdateUser 改 display_name/role/status。
func (q *Queries) UpdateUser(ctx context.Context, id int64, displayName, role, status string) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE admin_users SET display_name=?, role=?, status=?,
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, displayName, role, status, id)
	return err
}

// UpdatePassword 改密码哈希。
func (q *Queries) UpdatePassword(ctx context.Context, id int64, passwordHash string) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE admin_users SET password_hash=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`, passwordHash, id)
	return err
}

// UpdateLastLogin 记录最近登录时间。
func (q *Queries) UpdateLastLogin(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE admin_users SET last_login_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`, id)
	return err
}

// CountActiveSuperadmins 统计启用的 superadmin 数（防自锁/保 ≥1）。
func (q *Queries) CountActiveSuperadmins(ctx context.Context) (int, error) {
	var n int
	err := q.ex.QueryRowContext(ctx,
		`SELECT count(*) FROM admin_users WHERE role='superadmin' AND status='active'`).Scan(&n)
	return n, err
}

// ── admin_refresh_tokens ──

// InsertRefresh 写 refresh 哈希，返回 id。
func (q *Queries) InsertRefresh(ctx context.Context, userID int64, tokenHash, expiresAt, userAgent, ip string) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO admin_refresh_tokens (user_id, token_hash, expires_at, user_agent, ip) VALUES (?,?,?,?,?)`,
		userID, tokenHash, expiresAt, userAgent, ip)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// RefreshByHash 据哈希取有效（未吊销、未过期）refresh，返回 (id, userID)。
func (q *Queries) RefreshByHash(ctx context.Context, tokenHash string) (int64, int64, error) {
	var id, userID int64
	err := q.ex.QueryRowContext(ctx, `
		SELECT id, user_id FROM admin_refresh_tokens
		WHERE token_hash=? AND revoked=0 AND expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now')`,
		tokenHash).Scan(&id, &userID)
	return id, userID, err
}

// RevokeRefresh 吊销某 refresh。
func (q *Queries) RevokeRefresh(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `UPDATE admin_refresh_tokens SET revoked=1 WHERE id=?`, id)
	return err
}

// RevokeAllRefreshForUser 吊销某用户全部 refresh（禁用/改密后）。
func (q *Queries) RevokeAllRefreshForUser(ctx context.Context, userID int64) error {
	_, err := q.ex.ExecContext(ctx, `UPDATE admin_refresh_tokens SET revoked=1 WHERE user_id=?`, userID)
	return err
}

// ── admin_audit ──

// InsertAudit 写一条后台审计。
func (q *Queries) InsertAudit(ctx context.Context, userID *int64, action, target, detailJSON string) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO admin_audit (user_id, action, target, detail_json) VALUES (?,?,?,?)`,
		nullI64(userID), action, target, detailJSON)
	return err
}

// ListAudit 列审计（可按 user_id/action/日期前缀过滤），倒序，限 limit 条。
func (q *Queries) ListAudit(ctx context.Context, userID *int64, action, datePrefix string, limit int) ([]domain.AdminAudit, error) {
	var where []string
	var args []any
	if userID != nil {
		where = append(where, "a.user_id=?")
		args = append(args, *userID)
	}
	if action != "" {
		where = append(where, "a.action=?")
		args = append(args, action)
	}
	if datePrefix != "" {
		where = append(where, "a.created_at LIKE ?")
		args = append(args, datePrefix+"%")
	}
	query := `SELECT a.id, a.user_id, u.username, a.action, a.target, a.detail_json, a.created_at
		FROM admin_audit a LEFT JOIN admin_users u ON u.id = a.user_id`
	if len(where) > 0 {
		query += " WHERE " + strings.Join(where, " AND ")
	}
	query += " ORDER BY a.id DESC LIMIT ?"
	args = append(args, limit)

	rows, err := q.ex.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.AdminAudit
	for rows.Next() {
		var e domain.AdminAudit
		var uid sql.NullInt64
		var uname, target, detail sql.NullString
		if err := rows.Scan(&e.ID, &uid, &uname, &e.Action, &target, &detail, &e.CreatedAt); err != nil {
			return nil, err
		}
		e.UserID = ptrI64(uid)
		e.Username, e.Target, e.DetailJSON = uname.String, target.String, detail.String
		out = append(out, e)
	}
	return out, rows.Err()
}
