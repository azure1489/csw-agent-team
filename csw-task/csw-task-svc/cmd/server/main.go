// Command server 启动运行面 HTTP 服务（agent/skill 调用的 /api/v1），可选在进程内运行 notifier。
package main

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/server"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/files"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/lark"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/notifier"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func main() {
	log := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	cfg := config.Load()
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

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

	// notifier：引擎单写群播报。凭证缺失且非演练时不启动（服务照常），healthz 显示原因。
	var ntf *notifier.Notifier
	notifierState := "disabled"
	switch {
	case !cfg.NotifierEnabled:
	case !cfg.NotifierDryRun && (cfg.LarkAppID == "" || cfg.LarkAppSecret == ""):
		notifierState = "misconfigured: CSW_LARK_APP_ID / CSW_LARK_APP_SECRET 未设置"
		log.Error("notifier not started", "reason", notifierState)
	default:
		ntf = notifier.New(store, eng, lark.New(cfg.LarkBaseURL, cfg.LarkAppID, cfg.LarkAppSecret), notifier.Options{
			ChatKey: cfg.NotifierChatKey, Poll: cfg.NotifierPoll, OverdueScan: cfg.OverdueScan, DryRun: cfg.NotifierDryRun,
		}, log)
		go ntf.Run(ctx)
	}
	srv.SetHealth(func() map[string]any {
		h := map[string]any{"notifier": notifierState}
		if ntf != nil {
			h["notifier"] = ntf.Status()
		}
		if n, err := store.Q().CountPendingOutbox(context.Background()); err == nil {
			h["outbox_pending"] = n
		}
		return h
	})

	httpSrv := &http.Server{Addr: cfg.Addr, Handler: srv.Router(), ReadHeaderTimeout: 10 * time.Second}
	go func() {
		log.Info("server listening", "addr", cfg.Addr, "db", cfg.DBPath, "blob_backend", cfg.BlobBackend, "notifier", notifierState)
		if err := httpSrv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Error("server exited", "err", err)
			stop()
		}
	}()

	<-ctx.Done()
	log.Info("shutting down")
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := httpSrv.Shutdown(shutdownCtx); err != nil {
		log.Error("shutdown", "err", err)
	}
	if ntf != nil {
		ntf.Wait()
	}
}
