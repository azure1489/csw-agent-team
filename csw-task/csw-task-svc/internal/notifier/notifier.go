// Package notifier 事件接续：轮询 outbox → 渲染 → 发编辑部群；定时扫逾期；阶段通过后按需发进度卡。
// 群播报由引擎单写（agent 不再自己发群消息），同一事件只发一次。
package notifier

import (
	"context"
	"fmt"
	"log/slog"
	"sync"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/lark"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// Options 运行参数（零值取默认）。
type Options struct {
	Now         func() time.Time
	ChatKey     string        // 目标群 chat_id；空=花名册第一个群
	Poll        time.Duration // 默认 2s
	OverdueScan time.Duration // 默认 60s
	StaleAfter  time.Duration // 早于此的积压通知不再补发，默认 30m
	Lease       time.Duration // 领取租约，默认 2m
	Batch       int           // 每轮最多条数，默认 20
	MaxAttempts int           // 失败达此次数转 dead，默认 12
	DryRun      bool          // 只渲染并写日志、不真正发送
}

// progressTriggers 触发进度卡的事件（其余事件本身的播报已足够，不再追加进度卡）。
var progressTriggers = map[string]bool{
	domain.EvtStagePassed: true, domain.EvtTaskFailed: true, domain.EvtAuthorizationRequired: true,
}

// Notifier 通知发送器。
type Notifier struct {
	lastSent time.Time
	store    *sqlite.Store
	eng      *engine.Engine
	sender   lark.Sender
	log      *slog.Logger
	done     chan struct{}
	lastErr  string
	opt      Options
	mu       sync.Mutex
	running  bool
}

// New 构造 Notifier。
func New(store *sqlite.Store, eng *engine.Engine, sender lark.Sender, opt Options, log *slog.Logger) *Notifier {
	if opt.Now == nil {
		opt.Now = time.Now
	}
	if opt.Poll <= 0 {
		opt.Poll = 2 * time.Second
	}
	if opt.OverdueScan <= 0 {
		opt.OverdueScan = time.Minute
	}
	if opt.StaleAfter <= 0 {
		opt.StaleAfter = 30 * time.Minute
	}
	if opt.Lease <= 0 {
		opt.Lease = 2 * time.Minute
	}
	if opt.Batch <= 0 {
		opt.Batch = 20
	}
	if opt.MaxAttempts <= 0 {
		opt.MaxAttempts = 12
	}
	return &Notifier{store: store, eng: eng, sender: sender, opt: opt, log: log, done: make(chan struct{})}
}

// Run 主循环，随 ctx 退出。
func (n *Notifier) Run(ctx context.Context) {
	defer close(n.done)
	n.setRunning(true)
	defer n.setRunning(false)
	poll := time.NewTicker(n.opt.Poll)
	defer poll.Stop()
	scan := time.NewTicker(n.opt.OverdueScan)
	defer scan.Stop()
	n.log.Info("notifier started", "dry_run", n.opt.DryRun, "poll", n.opt.Poll.String())
	for {
		select {
		case <-ctx.Done():
			n.log.Info("notifier stopped")
			return
		case <-scan.C:
			if _, err := n.ScanOverdue(ctx); err != nil {
				n.fail(err)
			}
			if _, err := n.ScanStalls(ctx); err != nil {
				n.fail(err)
			}
			if _, err := n.ScanStalledRuns(ctx); err != nil {
				n.fail(err)
			}
			if _, err := n.ScanReviewWaits(ctx); err != nil {
				n.fail(err)
			}
		case <-poll.C:
			if _, err := n.Flush(ctx); err != nil {
				n.fail(err)
			}
		}
	}
}

// Wait 等 Run 退出。
func (n *Notifier) Wait() { <-n.done }

// Status /healthz 用。
func (n *Notifier) Status() map[string]any {
	n.mu.Lock()
	defer n.mu.Unlock()
	st := "stopped"
	if n.running {
		st = "running"
	}
	out := map[string]any{"state": st, "dry_run": n.opt.DryRun}
	if n.lastErr != "" {
		out["last_error"] = n.lastErr
	}
	if !n.lastSent.IsZero() {
		out["last_sent_at"] = n.lastSent.UTC().Format(time.RFC3339)
	}
	return out
}

func (n *Notifier) setRunning(v bool) {
	n.mu.Lock()
	n.running = v
	n.mu.Unlock()
}

func (n *Notifier) fail(err error) {
	n.log.Warn("notifier", "err", err)
	n.mu.Lock()
	n.lastErr = err.Error()
	n.mu.Unlock()
}

// backoff 5s·2^(n-1)，上限 30 分钟。
func backoff(attempts int) time.Duration {
	d := 5 * time.Second
	for i := 1; i < attempts && d < 30*time.Minute; i++ {
		d *= 2
	}
	if d > 30*time.Minute {
		d = 30 * time.Minute
	}
	return d
}

func (n *Notifier) loadRoster(ctx context.Context) (domain.Chat, roster, error) {
	q := n.store.Q()
	var chat domain.Chat
	var err error
	if n.opt.ChatKey != "" {
		chat, err = q.GetChatByKey(ctx, n.opt.ChatKey)
	} else {
		chat, err = q.FirstChat(ctx)
	}
	if err != nil {
		return chat, nil, fmt.Errorf("花名册未配置目标群：%w", err)
	}
	members, err := q.ListChatMembers(ctx, chat.ID)
	if err != nil {
		return chat, nil, err
	}
	r := roster{}
	for _, m := range members {
		if m.Kind == "mapped" && m.RoleCode != nil {
			r[*m.RoleCode] = member{OpenID: m.OpenID, Name: m.DisplayName}
		}
	}
	return chat, r, nil
}

// Flush 发一轮到期通知，返回成功条数。
func (n *Notifier) Flush(ctx context.Context) (int, error) {
	now := n.opt.Now()
	var rows []domain.Outbox
	if err := n.store.Tx(ctx, func(q *sqlite.Queries) error {
		var err error
		rows, err = q.ClaimDueOutbox(ctx, now, n.opt.Batch, n.opt.Lease)
		return err
	}); err != nil || len(rows) == 0 {
		return 0, err
	}
	q := n.store.Q()
	chat, r, rerr := n.loadRoster(ctx)
	sent := 0
	cards := map[int64]bool{}
	for _, o := range rows {
		if created, err := time.Parse(time.RFC3339, o.CreatedAt); err == nil && now.Sub(created) > n.opt.StaleAfter {
			if err := q.MarkOutboxFailed(ctx, o.ID, o.Attempts, now, "stale：积压超过 "+n.opt.StaleAfter.String()+"，不再补发", true); err != nil {
				return sent, err
			}
			continue
		}
		if rerr != nil {
			if err := n.markFailed(ctx, o, now, rerr); err != nil {
				return sent, err
			}
			continue
		}
		text, downgraded := render(o, r)
		if downgraded {
			n.log.Warn("notifier: van target not allowed for event, downgraded to hub", "outbox_id", o.ID, "event", o.EventType)
		}
		msgID := "dry-run"
		if n.opt.DryRun {
			n.log.Info("notifier dry-run", "outbox_id", o.ID, "event", o.EventType, "chat", chat.ChatKey, "text", text)
		} else {
			id, err := n.sender.SendText(ctx, chat.ChatKey, text, fmt.Sprintf("csw-outbox-%d", o.EventID))
			if err != nil {
				n.fail(err)
				if err := n.markFailed(ctx, o, now, err); err != nil {
					return sent, err
				}
				continue
			}
			msgID = id
		}
		if err := q.MarkOutboxSent(ctx, o.ID, msgID, now); err != nil {
			return sent, err
		}
		sent++
		n.mu.Lock()
		n.lastSent = now
		n.mu.Unlock()
		if o.RunID != nil && progressTriggers[o.EventType] {
			cards[*o.RunID] = true
		}
	}
	for runID := range cards {
		n.progressCard(ctx, chat, runID)
	}
	return sent, nil
}

func (n *Notifier) markFailed(ctx context.Context, o domain.Outbox, now time.Time, cause error) error {
	attempts := o.Attempts + 1
	return n.store.Q().MarkOutboxFailed(ctx, o.ID, attempts, now.Add(backoff(attempts)), cause.Error(), attempts >= n.opt.MaxAttempts)
}

// progressCard 进度摘要有变化才发；发送失败不更新摘要（下次再试）。
func (n *Notifier) progressCard(ctx context.Context, chat domain.Chat, runID int64) {
	q := n.store.Q()
	p, err := engine.BuildProgress(ctx, q, runID, n.opt.Now())
	if err != nil {
		n.fail(err)
		return
	}
	old, err := q.GetRunProgressDigest(ctx, runID)
	if err != nil || old == p.Digest {
		return
	}
	text := renderProgress(p)
	if n.opt.DryRun {
		n.log.Info("notifier dry-run progress", "run_id", runID, "text", text)
	} else if _, err := n.sender.SendText(ctx, chat.ChatKey, text, fmt.Sprintf("csw-progress-%d-%s", runID, p.Digest)); err != nil {
		n.fail(err)
		return
	}
	if err := q.SetRunProgressDigest(ctx, runID, p.Digest); err != nil {
		n.fail(err)
	}
}

// ScanOverdue 扫逾期任务并各提醒一次（通知进 outbox，下一轮 Flush 发出），返回本次提醒条数。
func (n *Notifier) ScanOverdue(ctx context.Context) (int, error) {
	tasks, err := n.store.Q().ListOverdueTasks(ctx)
	if err != nil {
		return 0, err
	}
	count := 0
	for _, t := range tasks {
		ok, err := n.eng.NotifyOverdue(ctx, t.ID)
		if err != nil {
			return count, err
		}
		if ok {
			count++
		}
	}
	return count, nil
}

// ScanStalledRuns 扫整期停滞的 run 并各提醒中枢一次（同一次停滞只提醒一次），返回本次提醒条数。
func (n *Notifier) ScanStalledRuns(ctx context.Context) (int, error) {
	runs, err := n.store.Q().ListStalledRuns(ctx)
	if err != nil {
		return 0, err
	}
	count := 0
	for _, r := range runs {
		ok, err := n.eng.NotifyRunStalled(ctx, r.ID)
		if err != nil {
			return count, err
		}
		if ok {
			count++
		}
	}
	return count, nil
}

// ScanStalls 扫接续告警（未接单 / 未接单升级 / 接单后无活动），每类每次停滞只提醒一次，返回本次提醒条数。
// 升级只发生在已提醒过执行者之后，所以同一轮不会同时发出提醒与升级。
func (n *Notifier) ScanStalls(ctx context.Context) (int, error) {
	count := 0
	for _, k := range []domain.StallKind{domain.StallEscalate, domain.StallAck, domain.StallIdle} {
		tasks, err := n.store.Q().ListStalledTasks(ctx, k)
		if err != nil {
			return count, err
		}
		for _, t := range tasks {
			ok, err := n.eng.NotifyStall(ctx, t.ID, k)
			if err != nil {
				return count, err
			}
			if ok {
				count++
			}
		}
	}
	return count, nil
}

// ScanReviewWaits 扫待审：到档就再提醒审核方一次，提醒够次数仍无动作就升级 Van（夜间不升级）。
// 返回本次发出的提醒与升级条数。
func (n *Notifier) ScanReviewWaits(ctx context.Context) (int, error) {
	waits, err := n.store.Q().ListReviewWaits(ctx)
	if err != nil {
		return 0, err
	}
	now := n.opt.Now()
	count := 0
	for _, w := range waits {
		since, err := time.Parse(time.RFC3339, w.Since)
		if err != nil {
			continue
		}
		var last time.Time
		if w.LastRemindedAt != "" {
			last, _ = time.Parse(time.RFC3339, w.LastRemindedAt)
		}
		waited := int(now.Sub(since).Minutes())
		if domain.ReviewEscalateDue(now, w.Reminded, w.EscalatedAt != "") {
			ok, err := n.eng.NotifyReviewEscalated(ctx, w, waited)
			if err != nil {
				return count, err
			}
			if ok {
				count++
			}
		}
		if domain.ReviewRemindDue(now, since, last, w.Reminded) {
			ok, err := n.eng.NotifyReviewWaiting(ctx, w, waited)
			if err != nil {
				return count, err
			}
			if ok {
				count++
			}
		}
	}
	return count, nil
}
