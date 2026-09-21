package sqlite

import (
	"database/sql"
	"embed"
	"fmt"

	"github.com/pressly/goose/v3"
)

//go:embed migrations/*.sql
var migrationsFS embed.FS

// Migrate 把 embed 的 goose 迁移全部 up 到最新。
// DownTo 回滚到指定版本。只给迁移演练与回滚测试用，不在任何服务路径上调用——
// 生产回滚要人盯着，先备份再手动走。
func DownTo(db *sql.DB, version int64) error {
	goose.SetBaseFS(migrationsFS)
	if err := goose.SetDialect("sqlite3"); err != nil {
		return fmt.Errorf("goose dialect: %w", err)
	}
	if err := goose.DownTo(db, "migrations", version); err != nil {
		return fmt.Errorf("goose down-to %d: %w", version, err)
	}
	return nil
}

func Migrate(db *sql.DB) error {
	goose.SetBaseFS(migrationsFS)
	if err := goose.SetDialect("sqlite3"); err != nil {
		return fmt.Errorf("goose dialect: %w", err)
	}
	if err := goose.Up(db, "migrations"); err != nil {
		return fmt.Errorf("goose up: %w", err)
	}
	return nil
}
