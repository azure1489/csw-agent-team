// Package sqlite 提供 sqlite 连接层、迁移与各表仓储。
package sqlite

import (
	"database/sql"
	"fmt"
	"net/url"
	"os"
	"path/filepath"

	_ "modernc.org/sqlite" // 纯 Go sqlite 驱动，driver 名 "sqlite"
)

// Open 打开（或创建）sqlite 数据库，设好并发/完整性相关 PRAGMA。
//
// 写串行：SetMaxOpenConns(1)——低 QPS 内部服务先求正确（避免 SQLITE_BUSY），
// 后续可拆「读多连接 + 写单连接」双池。WAL/foreign_keys/busy_timeout/synchronous
// 经 DSN `_pragma` 下发，保证每条连接一致。
func Open(dbPath string) (*sql.DB, error) {
	if dir := filepath.Dir(dbPath); dir != "" {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return nil, fmt.Errorf("mkdir db dir: %w", err)
		}
	}

	dsn := "file:" + dbPath + "?" + url.Values{
		"_pragma": {
			"busy_timeout(5000)",
			"journal_mode(WAL)",
			"foreign_keys(1)",
			"synchronous(NORMAL)",
		},
	}.Encode()

	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("open sqlite: %w", err)
	}
	db.SetMaxOpenConns(1)

	if err := db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("ping sqlite: %w", err)
	}
	return db, nil
}
