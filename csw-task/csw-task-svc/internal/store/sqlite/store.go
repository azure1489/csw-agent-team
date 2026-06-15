package sqlite

import (
	"context"
	"database/sql"
	"fmt"
)

// execer 抽象 *sql.DB 与 *sql.Tx，供仓储方法在普通连接或事务内复用。
type execer interface {
	ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error)
	QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error)
	QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row
}

// Store 数据访问入口（持有连接池）。
type Store struct {
	db *sql.DB
}

// New 构造 Store。
func New(db *sql.DB) *Store { return &Store{db: db} }

// Queries 承载一个 execer（连接池或事务），所有仓储方法挂在其上。
type Queries struct {
	ex execer
}

// Q 返回基于连接池的查询器（用于只读/单条写）。
func (s *Store) Q() *Queries { return &Queries{ex: s.db} }

// DB 暴露底层 *sql.DB（adminctl/迁移用）。
func (s *Store) DB() *sql.DB { return s.db }

// Tx 在单事务内执行 fn；fn 返回错误则回滚，否则提交。
// 写动作（派工/提交/审核/触发）各放一个 Tx，保证原子。
func (s *Store) Tx(ctx context.Context, fn func(q *Queries) error) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return fmt.Errorf("begin tx: %w", err)
	}
	if err := fn(&Queries{ex: tx}); err != nil {
		_ = tx.Rollback()
		return err
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("commit tx: %w", err)
	}
	return nil
}

// ── 标量/指针辅助 ──

func ptrI64(n sql.NullInt64) *int64 {
	if n.Valid {
		v := n.Int64
		return &v
	}
	return nil
}

func ptrInt(n sql.NullInt64) *int {
	if n.Valid {
		v := int(n.Int64)
		return &v
	}
	return nil
}

func nullI64(p *int64) sql.NullInt64 {
	if p == nil {
		return sql.NullInt64{}
	}
	return sql.NullInt64{Int64: *p, Valid: true}
}

func nullInt(p *int) sql.NullInt64 {
	if p == nil {
		return sql.NullInt64{}
	}
	return sql.NullInt64{Int64: int64(*p), Valid: true}
}

func ptrStr(n sql.NullString) *string {
	if n.Valid {
		v := n.String
		return &v
	}
	return nil
}

func nullStr(p *string) sql.NullString {
	if p == nil || *p == "" {
		return sql.NullString{}
	}
	return sql.NullString{String: *p, Valid: true}
}
