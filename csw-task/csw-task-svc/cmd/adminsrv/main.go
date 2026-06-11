// Command adminsrv 启动管理后台 HTTP 服务（人类操作员 JWT 登录的 /admin/* JSON API）。
package main

import (
	"log/slog"
	"os"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/adminsrv"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func main() {
	log := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	cfg := config.Load()
	if cfg.JWTSecretGenerated {
		log.Warn("CSW_JWT_SECRET 未设置，已生成临时密钥（重启即失效，所有 access/refresh 作废）；生产请显式配置")
	}

	db, err := sqlite.Open(cfg.DBPath)
	if err != nil {
		log.Error("open db", "err", err)
		os.Exit(1)
	}
	defer db.Close()
	if err := sqlite.Migrate(db); err != nil {
		log.Error("migrate", "err", err)
		os.Exit(1)
	}

	store := sqlite.New(db)
	srv := adminsrv.New(store, cfg, log)

	log.Info("adminsrv listening", "addr", cfg.AdminAddr, "db", cfg.DBPath, "cors", cfg.CORSOrigins)
	if err := srv.Router().Run(cfg.AdminAddr); err != nil {
		log.Error("adminsrv exited", "err", err)
		os.Exit(1)
	}
}
