package sqlite

import (
	"context"
	"database/sql"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── chats ──

const chatCols = `id, chat_key, name, note, created_at, updated_at`

func scanChat(s interface{ Scan(...any) error }) (domain.Chat, error) {
	var c domain.Chat
	var note sql.NullString
	err := s.Scan(&c.ID, &c.ChatKey, &c.Name, &note, &c.CreatedAt, &c.UpdatedAt)
	c.Note = note.String
	return c, err
}

// ListChats 列全部群（按 id）。
func (q *Queries) ListChats(ctx context.Context) ([]domain.Chat, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+chatCols+` FROM chats ORDER BY id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Chat
	for rows.Next() {
		c, err := scanChat(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

// GetChat 据 id 取群。
func (q *Queries) GetChat(ctx context.Context, id int64) (domain.Chat, error) {
	return scanChat(q.ex.QueryRowContext(ctx, `SELECT `+chatCols+` FROM chats WHERE id=?`, id))
}

// GetChatByKey 据 chat_key（lark chat_id）取群。
func (q *Queries) GetChatByKey(ctx context.Context, key string) (domain.Chat, error) {
	return scanChat(q.ex.QueryRowContext(ctx, `SELECT `+chatCols+` FROM chats WHERE chat_key=?`, key))
}

// FirstChat 取首个群（运行面 /roster 缺省）。
func (q *Queries) FirstChat(ctx context.Context) (domain.Chat, error) {
	return scanChat(q.ex.QueryRowContext(ctx, `SELECT `+chatCols+` FROM chats ORDER BY id LIMIT 1`))
}

// CreateChat 新建群，返回 id。
func (q *Queries) CreateChat(ctx context.Context, chatKey, name, note string) (int64, error) {
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO chats (chat_key, name, note) VALUES (?,?,?)`, chatKey, name, note)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// UpdateChat 改群名/备注（chat_key 不可改）。
func (q *Queries) UpdateChat(ctx context.Context, id int64, name, note string) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE chats SET name=?, note=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`,
		name, note, id)
	return err
}

// DeleteChat 删群（成员经 ON DELETE CASCADE 一并删）。
func (q *Queries) DeleteChat(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `DELETE FROM chats WHERE id=?`, id)
	return err
}

// ── chat_members ──

const memberCols = `id, chat_id, kind, role_code, open_id, user_id, display_name, bot_name, is_human, sort, created_at, updated_at`

func scanMember(s interface{ Scan(...any) error }) (domain.ChatMember, error) {
	var m domain.ChatMember
	var roleCode, userID sql.NullString
	var isHuman int
	err := s.Scan(&m.ID, &m.ChatID, &m.Kind, &roleCode, &m.OpenID, &userID,
		&m.DisplayName, &m.BotName, &isHuman, &m.Sort, &m.CreatedAt, &m.UpdatedAt)
	m.RoleCode = ptrStr(roleCode)
	m.UserID = ptrStr(userID)
	m.IsHuman = isHuman == 1
	return m, err
}

// ListChatMembers 列某群全部成员（按 sort, id）。
func (q *Queries) ListChatMembers(ctx context.Context, chatID int64) ([]domain.ChatMember, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT `+memberCols+` FROM chat_members WHERE chat_id=? ORDER BY sort, id`, chatID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.ChatMember
	for rows.Next() {
		m, err := scanMember(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, m)
	}
	return out, rows.Err()
}

// GetChatMember 取单个成员。
func (q *Queries) GetChatMember(ctx context.Context, id int64) (domain.ChatMember, error) {
	return scanMember(q.ex.QueryRowContext(ctx, `SELECT `+memberCols+` FROM chat_members WHERE id=?`, id))
}

// CreateChatMember 新建成员，返回 id。
func (q *Queries) CreateChatMember(ctx context.Context, m domain.ChatMember) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO chat_members (chat_id, kind, role_code, open_id, user_id, display_name, bot_name, is_human, sort)
		VALUES (?,?,?,?,?,?,?,?,?)`,
		m.ChatID, m.Kind, nullStr(m.RoleCode), m.OpenID, nullStr(m.UserID),
		m.DisplayName, m.BotName, b2i(m.IsHuman), m.Sort)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// UpdateChatMember 改成员（chat_id 不变）。
func (q *Queries) UpdateChatMember(ctx context.Context, m domain.ChatMember) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE chat_members SET kind=?, role_code=?, open_id=?, user_id=?, display_name=?, bot_name=?, is_human=?, sort=?,
			updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE id=?`,
		m.Kind, nullStr(m.RoleCode), m.OpenID, nullStr(m.UserID),
		m.DisplayName, m.BotName, b2i(m.IsHuman), m.Sort, m.ID)
	return err
}

// DeleteChatMember 删成员。
func (q *Queries) DeleteChatMember(ctx context.Context, id int64) error {
	_, err := q.ex.ExecContext(ctx, `DELETE FROM chat_members WHERE id=?`, id)
	return err
}
