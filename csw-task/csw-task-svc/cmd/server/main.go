// Command server 启动运行面 HTTP 服务（agent/skill 调用的 /api/v1）。
package main

import (
	"log/slog"
	"os"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/server"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/files"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func main() {
	log := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	cfg := config.Load()

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

	var blobs files.BlobStore
	if cfg.BlobBackend == "oss" {
		blobs, err = files.NewOSSStore(cfg.OSSEndpoint, cfg.OSSBucket, cfg.OSSAccessKeyID, cfg.OSSAccessKeySecret, cfg.OSSPrefix)
	} else {
		blobs, err = files.NewLocalStore(cfg.BlobDir)
	}
	if err != nil {
		log.Error("blob store", "err", err)
		os.Exit(1)
	}

	store := sqlite.New(db)
	eng := engine.New(store)
	srv := server.New(store, eng, blobs, cfg, log)

	log.Info("server listening", "addr", cfg.Addr, "db", cfg.DBPath, "blob_backend", cfg.BlobBackend)
	if err := srv.Router().Run(cfg.Addr); err != nil {
		log.Error("server exited", "err", err)
		os.Exit(1)
	}
}
