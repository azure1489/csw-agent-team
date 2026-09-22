// Package syncer 数据子系统运维 CLI：平台读取探针、发布记录回填与增量同步、按龄期快照、人工导出表格导入、合集人工拆条、编辑反馈导入。
// 与 adminctl 一样直读数据库（须在数据库所在主机运行）；平台读取走 CSW_SYNC_WX_CMD / CSW_SYNC_XHS_CMD 外部命令。
package syncer

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/ledger"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

const usage = `用法：
  syncer probe-wx  [--known <post_id>]            公众号读取可行性探针（退出码 0 可行 / 3 部分 / 1 不可得）
  syncer probe-xhs [--known <post_id>]            小红书读取可行性探针
  syncer backfill --platform wechat|xhs [--since 30d]   回填发布记录（含人工发布；草稿与已发分列）
  syncer sync --platform wechat|xhs [--window 30m]      增量同步（从上次成功覆盖处起，回看 window）
  syncer snapshot --platform wechat|xhs --bucket 24h|72h|7d|30d   按龄期采一次原始指标（缺口分 unavailable / fetch_failed）
  syncer account-snapshot --platform wechat|xhs [--days 30]      采账号区间趋势（原值 + 区间 + 指标定义）
  syncer import-csv --platform wechat|xhs --kind notes|account --file F.csv [--account 名称]
                                                  导入人工导出的明细 / 账号趋势（异常原值原样保留并标记）
  syncer split --platform wechat --post <post_id> --manual items.json [--by 校对人]
                                                  人工校对后的合集拆条（覆盖自动拆条）
  syncer feedback import F.jsonl                  导入历史编辑反馈（每行一条 JSON）
  syncer feedback expire                          把过期的临时反馈置为 expired
  syncer kb-import F.jsonl                        导入知识库范例（每行一篇，写入发布记录并标 is_reference=1）
  syncer memory-import cases F.jsonl              导入选题案例（P5 第二步的 cases.jsonl；原话原样存）
  syncer memory-import rules F.jsonl              导入准则卡草稿（P5 的 rules.jsonl；**不会自动标成她确认过**）
  syncer memory-confirm <rule_key> [--undo]       人工确认一条准则卡（只有确认过的才能当判断依据）`

// Run 执行 syncer，返回退出码。
func Run(args []string) int { return RunWith(args, config.Load(), os.Stdout, nil) }

// RunWith 可注入配置、输出与适配器（测试用）。adapter 为 nil 时按配置建外部命令适配器。
func RunWith(args []string, cfg config.Config, out io.Writer, adapter func(platform string) ledger.Adapter) int {
	if len(args) == 0 {
		fmt.Fprintln(out, usage)
		return 2
	}
	db, err := sqlite.Open(cfg.DBPath)
	if err != nil {
		fmt.Fprintln(out, "打开数据库失败：", err)
		return 1
	}
	defer db.Close()
	if err := sqlite.Migrate(db); err != nil {
		fmt.Fprintln(out, "迁移失败：", err)
		return 1
	}
	st := sqlite.New(db)
	ctx := context.Background()
	flags, pos := parseFlags(args[1:])
	if adapter == nil {
		adapter = func(p string) ledger.Adapter {
			if p == "wechat" {
				// 公众号：显式配置的外部命令优先；否则经 mp-helper 的只读接口读取。
				if cfg.SyncWxCmd == "" && cfg.SyncWxMPHelperURL != "" {
					return &ledger.MPHelperAdapter{BaseURL: cfg.SyncWxMPHelperURL, APIKey: cfg.SyncWxMPHelperKey}
				}
				return &ledger.CommandAdapter{Name: "wechat", Cmd: cfg.SyncWxCmd}
			}
			return &ledger.CommandAdapter{Name: "xhs", Cmd: cfg.SyncXhsCmd}
		}
	}
	account := func(p string) string {
		if v := flags["--account"]; v != "" {
			return v
		}
		if p == "wechat" {
			return cfg.SyncWxAccount
		}
		return cfg.SyncXhsAccount
	}
	platform := flags["--platform"]
	if platform != "" && platform != "wechat" && platform != "xhs" {
		fmt.Fprintln(out, "--platform 须为 wechat 或 xhs")
		return 2
	}
	printJSON := func(v any) {
		b, _ := json.MarshalIndent(v, "", "  ")
		fmt.Fprintln(out, string(b))
	}

	switch args[0] {
	case "probe-wx", "probe-xhs":
		p := "wechat"
		if args[0] == "probe-xhs" {
			p = "xhs"
		}
		rep := ledger.Probe(ctx, adapter(p), flags["--known"])
		id, _ := st.Q().StartSyncRun(ctx, p, "probe", "", "")
		b, _ := json.Marshal(rep.Checks)
		_ = st.Q().FinishSyncRun(ctx, domain.LedgerSyncRun{ID: id, OK: rep.ExitCode() == 0, GapNote: string(b)})
		printJSON(rep)
		return rep.ExitCode()

	case "backfill", "sync":
		if platform == "" {
			fmt.Fprintln(out, "缺 --platform")
			return 2
		}
		since := time.Now().Add(-30 * 24 * time.Hour)
		if args[0] == "backfill" {
			if d, err := parseDur(flags["--since"], 30*24*time.Hour); err == nil {
				since = time.Now().Add(-d)
			} else {
				fmt.Fprintln(out, err)
				return 2
			}
		} else {
			window, err := parseDur(flags["--window"], 30*time.Minute)
			if err != nil {
				fmt.Fprintln(out, err)
				return 2
			}
			if last, _ := st.Q().LastOKSyncWindowTo(ctx, platform); last != "" {
				if t, err := time.Parse(time.RFC3339, last); err == nil {
					since = t.Add(-window)
				}
			}
		}
		res, err := ledger.Sync(ctx, st, adapter(platform), account(platform), args[0], since)
		printJSON(map[string]any{"platform": platform, "since": since.UTC().Format(time.RFC3339), "fetched": res.Fetched,
			"inserted": res.Inserted, "updated": res.Updated, "split": res.Split, "gaps": res.Gaps})
		if err != nil {
			fmt.Fprintln(out, "失败：", err)
			return 1
		}
		return 0

	case "snapshot":
		if platform == "" || flags["--bucket"] == "" {
			fmt.Fprintln(out, "缺 --platform 或 --bucket")
			return 2
		}
		runID, _ := st.Q().StartSyncRun(ctx, platform, "snapshot", "", "")
		n, gaps, err := ledger.Snapshot(ctx, st, adapter(platform), flags["--bucket"])
		kinds := ledger.GapKinds(gaps)
		note, _ := json.Marshal(map[string]any{"bucket": flags["--bucket"], "snapshots": n, "gap_kinds": kinds, "gaps": gaps})
		fin := domain.LedgerSyncRun{ID: runID, OK: err == nil && kinds[ledger.GapFetchFailed] == 0, Inserted: n, GapNote: string(note)}
		if err != nil {
			fin.Error = err.Error()
		}
		_ = st.Q().FinishSyncRun(ctx, fin)
		printJSON(map[string]any{"platform": platform, "bucket": flags["--bucket"], "snapshots": n, "gap_kinds": kinds, "gaps": gaps})
		if err != nil {
			fmt.Fprintln(out, "失败：", err)
			return 1
		}
		return 0

	case "account-snapshot":
		if platform == "" {
			fmt.Fprintln(out, "缺 --platform")
			return 2
		}
		days := 30
		if v := flags["--days"]; v != "" {
			d, err := strconv.Atoi(v)
			if err != nil || d <= 0 {
				fmt.Fprintln(out, "--days 须为正整数")
				return 2
			}
			days = d
		}
		runID, _ := st.Q().StartSyncRun(ctx, platform, "snapshot", "", "")
		s, inserted, err := ledger.SnapshotAccount(ctx, st, adapter(platform), account(platform), days)
		fin := domain.LedgerSyncRun{ID: runID, OK: err == nil, WindowFrom: s.WindowFrom, WindowTo: s.WindowTo, GapNote: s.FlagsJSON}
		if inserted {
			fin.Inserted = 1
		}
		switch {
		case errors.Is(err, ledger.ErrUnavailable):
			fin.GapNote = ledger.GapUnavailable + ":" + err.Error()
		case err != nil:
			fin.GapNote, fin.Error = ledger.GapFetchFailed+":"+err.Error(), err.Error()
		}
		_ = st.Q().FinishSyncRun(ctx, fin)
		if err != nil {
			fmt.Fprintln(out, "账号快照未写入：", err)
			if errors.Is(err, ledger.ErrUnavailable) {
				return 3
			}
			return 1
		}
		fmt.Fprintf(out, "账号快照 %s ~ %s；标记：%s；定义：%s\n", s.WindowFrom, s.WindowTo, or(s.FlagsJSON, "无"), or(s.DefinitionsJSON, "平台未提供"))
		return 0

	case "import-csv":
		return importCSV(ctx, st, out, platform, account(platform), flags)

	case "split":
		return split(ctx, st, out, platform, account(platform), flags)

	case "kb-import":
		if len(pos) == 0 {
			fmt.Fprintln(out, "缺 jsonl 文件")
			return 2
		}
		return importKB(ctx, st, out, pos[0])

	case "memory-import":
		if len(pos) < 2 {
			fmt.Fprintln(out, "用法：syncer memory-import cases|rules F.jsonl")
			return 2
		}
		return importMemory(ctx, st, out, pos[0], pos[1])

	case "memory-confirm":
		if len(pos) == 0 {
			fmt.Fprintln(out, "缺 rule_key")
			return 2
		}
		undo := false
		for _, a := range args {
			if a == "--undo" {
				undo = true
			}
		}
		if err := st.Q().SetRuleConfirmed(ctx, pos[0], !undo); err != nil {
			fmt.Fprintln(out, "失败：", err)
			return 1
		}
		if undo {
			fmt.Fprintf(out, "已撤销确认：%s（这条准则回到草稿，不能再当判断依据）\n", pos[0])
		} else {
			fmt.Fprintf(out, "已确认：%s\n", pos[0])
		}
		return 0

	case "feedback":
		if len(pos) == 0 {
			fmt.Fprintln(out, usage)
			return 2
		}
		switch pos[0] {
		case "import":
			if len(pos) < 2 {
				fmt.Fprintln(out, "缺 jsonl 文件")
				return 2
			}
			return importFeedback(ctx, st, out, pos[1])
		case "expire":
			n, err := st.Q().ExpireFeedback(ctx)
			if err != nil {
				fmt.Fprintln(out, "失败：", err)
				return 1
			}
			fmt.Fprintf(out, "已置过期 %d 条\n", n)
			return 0
		}
	}
	fmt.Fprintln(out, usage)
	return 2
}

func parseFlags(args []string) (map[string]string, []string) {
	f := map[string]string{}
	var pos []string
	for i := 0; i < len(args); i++ {
		if strings.HasPrefix(args[i], "--") && i+1 < len(args) {
			f[args[i]] = args[i+1]
			i++
			continue
		}
		pos = append(pos, args[i])
	}
	return f, pos
}

func parseDur(s string, def time.Duration) (time.Duration, error) {
	if s == "" {
		return def, nil
	}
	if strings.HasSuffix(s, "d") {
		n, err := strconv.Atoi(strings.TrimSuffix(s, "d"))
		if err != nil {
			return 0, fmt.Errorf("时长不合法：%s", s)
		}
		return time.Duration(n) * 24 * time.Hour, nil
	}
	d, err := time.ParseDuration(s)
	if err != nil {
		return 0, fmt.Errorf("时长不合法：%s", s)
	}
	return d, nil
}

// csvDefs 人工导出表格的口径来源：字段名即平台导出列名，定义以导出文件为准。
func csvDefs(file string) string {
	b, _ := json.Marshal(map[string]string{"source": "人工导出 CSV", "file": filepath.Base(file)})
	return string(b)
}

func or(s, def string) string {
	if strings.TrimSpace(s) == "" {
		return def
	}
	return s
}

func importCSV(ctx context.Context, st *sqlite.Store, out io.Writer, platform, account string, flags map[string]string) int {
	if platform == "" || flags["--file"] == "" || flags["--kind"] == "" {
		fmt.Fprintln(out, "缺 --platform / --kind / --file")
		return 2
	}
	f, err := os.Open(flags["--file"])
	if err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	defer f.Close()
	q := st.Q()
	now := time.Now().UTC().Format("2006-01-02T15:04:05Z")
	switch flags["--kind"] {
	case "notes":
		rows, err := ledger.ParseNotes(bufio.NewReader(f))
		if err != nil {
			fmt.Fprintln(out, "解析失败：", err)
			return 1
		}
		runID, _ := q.StartSyncRun(ctx, platform, "import", "", "")
		ins, upd, flagged := 0, 0, 0
		for _, r := range rows {
			raw, _ := json.Marshal(r.Post.Raw)
			id, inserted, err := q.UpsertLedgerPost(ctx, domain.LedgerPost{Platform: platform, Account: account, PostID: r.Post.PostID,
				URL: r.Post.URL, PublishedAt: ledger.NormTime(r.Post.PublishedAt), Title: r.Post.Title, Source: "import", State: "published", RawJSON: string(raw)})
			if err != nil {
				fmt.Fprintln(out, "写入失败：", err)
				return 1
			}
			if inserted {
				ins++
			} else {
				upd++
			}
			mraw, _ := json.Marshal(r.Metrics)
			var fl string
			if len(r.Flags) > 0 {
				b, _ := json.Marshal(r.Flags)
				fl = string(b)
				flagged++
			}
			if _, err := q.InsertMetricSnapshot(ctx, domain.MetricSnapshot{PostRef: id, Platform: platform, AgeBucket: "adhoc",
				CollectedAt: now, RawJSON: string(mraw), FlagsJSON: fl, DefinitionsJSON: csvDefs(flags["--file"])}); err != nil {
				fmt.Fprintln(out, "快照写入失败：", err)
				return 1
			}
		}
		_ = q.FinishSyncRun(ctx, domain.LedgerSyncRun{ID: runID, OK: true, Fetched: len(rows), Inserted: ins, Updated: upd,
			GapNote: fmt.Sprintf("人工导出导入：%s；%d 条带异常标记", flags["--file"], flagged)})
		fmt.Fprintf(out, "导入 %d 条（新增 %d，更新 %d），%d 条带异常标记（refresh_pending / definition_pending）\n", len(rows), ins, upd, flagged)
		return 0
	case "account":
		sum, err := ledger.ParseAccount(bufio.NewReader(f))
		if err != nil {
			fmt.Fprintln(out, "解析失败：", err)
			return 1
		}
		raw, _ := json.Marshal(map[string]any{"totals": sum.Totals, "days": sum.Days})
		var fl string
		if len(sum.Flags) > 0 {
			b, _ := json.Marshal(sum.Flags)
			fl = string(b)
		}
		if _, err := q.InsertAccountSnapshot(ctx, domain.AccountSnapshot{Platform: platform, Account: account, WindowFrom: sum.WindowFrom,
			WindowTo: sum.WindowTo, CollectedAt: now, RawJSON: string(raw), FlagsJSON: fl, DefinitionsJSON: csvDefs(flags["--file"])}); err != nil {
			fmt.Fprintln(out, "写入失败：", err)
			return 1
		}
		fmt.Fprintf(out, "账号快照 %s ~ %s，%d 天；标记：%v\n", sum.WindowFrom, sum.WindowTo, len(sum.Days), sum.Flags)
		return 0
	}
	fmt.Fprintln(out, "--kind 须为 notes 或 account")
	return 2
}

func split(ctx context.Context, st *sqlite.Store, out io.Writer, platform, account string, flags map[string]string) int {
	if platform == "" || flags["--post"] == "" || flags["--manual"] == "" {
		fmt.Fprintln(out, "缺 --platform / --post / --manual")
		return 2
	}
	p, err := st.Q().GetLedgerPost(ctx, platform, account, flags["--post"])
	if err != nil {
		fmt.Fprintln(out, "没有这篇记录：", flags["--post"])
		return 1
	}
	b, err := os.ReadFile(flags["--manual"])
	if err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	var items []domain.LedgerPostItem
	var in []struct {
		ItemKey   string `json:"item_key"`
		Brand     string `json:"brand"`
		Product   string `json:"product"`
		Angle     string `json:"angle"`
		Title     string `json:"title"`
		SourceURL string `json:"source_url"`
	}
	if err := json.Unmarshal(b, &in); err != nil {
		fmt.Fprintln(out, "items.json 须为数组：[{brand,title,product?,angle?,item_key?,source_url?}]")
		return 1
	}
	for _, x := range in {
		items = append(items, domain.LedgerPostItem{ItemKey: x.ItemKey, Brand: x.Brand, Product: x.Product, Angle: x.Angle, Title: x.Title, SourceURL: x.SourceURL})
	}
	by := flags["--by"]
	if by == "" {
		by = "人工"
	}
	if err := st.Q().ReplacePostItems(ctx, p.ID, items, "manual", by); err != nil {
		fmt.Fprintln(out, "写入失败：", err)
		return 1
	}
	fmt.Fprintf(out, "已按人工校对写入 %d 条\n", len(items))
	return 0
}

func importFeedback(ctx context.Context, st *sqlite.Store, out io.Writer, path string) int {
	f, err := os.Open(path)
	if err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 1<<20), 1<<20)
	n, line := 0, 0
	for sc.Scan() {
		line++
		txt := strings.TrimSpace(sc.Text())
		if txt == "" {
			continue
		}
		var r struct {
			Quote     string   `json:"quote"`
			SaidAt    string   `json:"said_at"`
			SourceRef string   `json:"source_ref"`
			ObjectRef string   `json:"object_ref"`
			Kind      string   `json:"kind"`
			Stance    string   `json:"stance"`
			ExpiresAt string   `json:"expires_at"`
			Curated   string   `json:"curated_by"`
			Tags      []string `json:"tags"`
			Images    []string `json:"images"`
		}
		if err := json.Unmarshal([]byte(txt), &r); err != nil || strings.TrimSpace(r.Quote) == "" {
			fmt.Fprintf(out, "第 %d 行无效（须为 JSON 且 quote 非空），已跳过\n", line)
			continue
		}
		imgs := ""
		if len(r.Images) > 0 {
			b, _ := json.Marshal(r.Images)
			imgs = string(b)
		}
		if _, _, err := st.Q().InsertFeedback(ctx, domain.FeedbackRecord{Quote: r.Quote, SaidAt: r.SaidAt, SourceRef: r.SourceRef,
			ObjectRef: r.ObjectRef, ImageRefsJSON: imgs, Kind: r.Kind, Stance: r.Stance, ExpiresAt: ledger.NormTime(r.ExpiresAt),
			CuratedBy: r.Curated, Tags: r.Tags}); err != nil {
			fmt.Fprintf(out, "第 %d 行写入失败：%v\n", line, err)
			return 1
		}
		n++
	}
	fmt.Fprintf(out, "导入编辑反馈 %d 条\n", n)
	return 0
}

// importKB 导入知识库范例（`docs/知识库/范例/kb_import.jsonl`，每行一篇）。
//
// 范例进的是同一张发布记录表，但打 is_reference=1：
// 「正式已发布」与「范例」是判断时的两类对照材料，混成一类就分不清
// 「这事我们发过」和「这种写法我们认可」。
//
// items 一并写进 ledger_post_items（split_by=manual）：范例的价值恰恰在于
// 它把一篇里的几条资讯怎么拆、每条什么角度，都标好了。
func importKB(ctx context.Context, st *sqlite.Store, out io.Writer, path string) int {
	f, err := os.Open(path)
	if err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	// 范例正文是整篇文章，一行能到几十 KB
	sc.Buffer(make([]byte, 1<<20), 8<<20)
	posts, items, line, bad := 0, 0, 0, 0
	for sc.Scan() {
		line++
		txt := strings.TrimSpace(sc.Text())
		if txt == "" {
			continue
		}
		var r struct {
			Platform    string `json:"platform"`
			Account     string `json:"account"`
			PostID      string `json:"post_id"`
			URL         string `json:"url"`
			PublishedAt string `json:"published_at"`
			Title       string `json:"title"`
			BodyText    string `json:"body_text"`
			Source      string `json:"source"`
			State       string `json:"state"`
			Items       []struct {
				Seq     int    `json:"seq"`
				Brand   string `json:"brand"`
				Product string `json:"product"`
				Angle   string `json:"angle"`
				Title   string `json:"title"`
			} `json:"items"`
		}
		if err := json.Unmarshal([]byte(txt), &r); err != nil ||
			strings.TrimSpace(r.PostID) == "" || strings.TrimSpace(r.BodyText) == "" {
			fmt.Fprintf(out, "第 %d 行无效（须为 JSON 且 post_id、body_text 非空），已跳过\n", line)
			bad++
			continue
		}
		if r.Platform == "" {
			r.Platform = "wechat"
		}
		if r.State == "" {
			r.State = "published"
		}
		id, created, err := st.Q().UpsertLedgerPost(ctx, domain.LedgerPost{
			Platform: r.Platform, Account: r.Account, PostID: r.PostID, URL: r.URL,
			PublishedAt: ledger.NormTime(r.PublishedAt), Title: r.Title, BodyText: r.BodyText,
			Source: "import", State: r.State, IsReference: true,
			// 范例的「凭什么说它发布了」就是它自己的公开页地址
			PublishEvidence: r.URL,
		})
		if err != nil {
			fmt.Fprintf(out, "第 %d 行写入失败：%v\n", line, err)
			bad++
			continue
		}
		posts++
		if !created {
			fmt.Fprintf(out, "  %s 已存在，已更新并标为范例\n", r.PostID)
		}
		if len(r.Items) == 0 {
			continue
		}
		list := make([]domain.LedgerPostItem, 0, len(r.Items))
		for i, it := range r.Items {
			seq := it.Seq
			if seq == 0 {
				seq = i + 1
			}
			list = append(list, domain.LedgerPostItem{
				Seq: seq, Brand: it.Brand, Product: it.Product, Angle: it.Angle, Title: it.Title,
			})
		}
		// 范例的拆条是人标的，不是自动拆的
		if err := st.Q().ReplacePostItems(ctx, id, list, "manual", "kb-import"); err != nil {
			fmt.Fprintf(out, "第 %d 行拆条写入失败：%v\n", line, err)
			bad++
			continue
		}
		items += len(list)
	}
	if err := sc.Err(); err != nil {
		fmt.Fprintln(out, "读文件失败：", err)
		return 1
	}
	fmt.Fprintf(out, "范例导入完成：%d 篇、%d 条拆条，跳过 %d 行\n", posts, items, bad)
	if posts == 0 {
		return 1
	}
	return 0
}

// importMemory 导入选题记忆。kind 为 cases 或 rules。
//
// # 为什么两种分开导
//
// 案例存的是**她说过的原话**，准则存的是**我们的归纳**（迁移 0039 的建表注释）。
// 同一个文件里混着导，迟早有人把归纳写进 quote 字段——那条界线一旦破了，
// 案例库就不再是「她说过什么」的可靠记录，而这正是它唯一的价值。
//
// # 导入永远不标「她确认过」
//
// `UpsertSelectionRule` 不碰 `confirmed_by_van`。确认是人的动作，走
// `syncer memory-confirm`。自动置 1 会让未经她确认的归纳直接变成 02 / 03 的
// 判断依据；反过来，重新导入把已确认的冲掉，那条准则会悄悄降级成猜测。
func importMemory(ctx context.Context, st *sqlite.Store, out io.Writer, kind, path string) int {
	if kind != "cases" && kind != "rules" {
		fmt.Fprintln(out, "第一个参数须为 cases 或 rules")
		return 2
	}
	f, err := os.Open(path)
	if err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 1<<20), 8<<20)
	ok, bad, line := 0, 0, 0
	for sc.Scan() {
		line++
		txt := strings.TrimSpace(sc.Text())
		if txt == "" {
			continue
		}
		var err error
		if kind == "cases" {
			err = importOneCase(ctx, st, txt)
		} else {
			err = importOneRule(ctx, st, txt)
		}
		if err != nil {
			// 坏一行不该让整份作废，但要说出是哪一行坏在哪
			fmt.Fprintf(out, "第 %d 行：%v\n", line, err)
			bad++
			continue
		}
		ok++
	}
	if err := sc.Err(); err != nil {
		fmt.Fprintln(out, "读文件：", err)
		return 1
	}
	fmt.Fprintf(out, "导入 %s：成功 %d 条，跳过 %d 条\n", kind, ok, bad)
	if kind == "rules" && ok > 0 {
		fmt.Fprintln(out, "**全部是草稿。** 她逐条确认之前不能当判断依据；"+
			"确认走 syncer memory-confirm <rule_key>。")
	}
	return 0
}

func importOneCase(ctx context.Context, st *sqlite.Store, txt string) error {
	var r struct {
		CaseKey    string `json:"case_key"`
		RunID      *int64 `json:"run_id"`
		ItemKey    string `json:"item_key"`
		Brand      string `json:"brand"`
		Title      string `json:"title"`
		SourceURL  string `json:"source_url"`
		Decision   string `json:"decision"`
		Quote      string `json:"quote"`
		QuoteRef   string `json:"quote_ref"`
		DecidedAt  string `json:"decided_at"`
		JudgedTier string `json:"judged_tier"`
	}
	if err := json.Unmarshal([]byte(txt), &r); err != nil {
		return err
	}
	if strings.TrimSpace(r.CaseKey) == "" {
		return errors.New("缺 case_key")
	}
	// 没有原话的案例进来也没用：案例库的全部价值就是「她说过什么」
	if strings.TrimSpace(r.Quote) == "" {
		return errors.New("缺 quote（原话）——没有原话的不是案例")
	}
	switch r.Decision {
	case "adopted", "rejected", "deferred", "pending_check":
	default:
		return fmt.Errorf("decision 须为 adopted / rejected / deferred / pending_check，收到 %q", r.Decision)
	}
	return st.Q().UpsertSelectionCase(ctx, domain.SelectionCase{
		CaseKey: r.CaseKey, RunID: r.RunID, ItemKey: r.ItemKey, Brand: r.Brand,
		Title: r.Title, SourceURL: r.SourceURL, Decision: r.Decision,
		Quote: r.Quote, QuoteRef: r.QuoteRef, DecidedAt: r.DecidedAt, JudgedTier: r.JudgedTier,
	})
}

func importOneRule(ctx context.Context, st *sqlite.Store, txt string) error {
	var r struct {
		RuleKey     string   `json:"rule_key"`
		Category    string   `json:"category"`
		Text        string   `json:"text"`
		DerivedFrom []string `json:"derived_from"`
		Version     string   `json:"version"`
	}
	if err := json.Unmarshal([]byte(txt), &r); err != nil {
		return err
	}
	if strings.TrimSpace(r.RuleKey) == "" || strings.TrimSpace(r.Text) == "" {
		return errors.New("缺 rule_key 或 text")
	}
	switch r.Category {
	case "":
		r.Category = "frame"
	case "frame", "prefer", "lower", "dedup", "other":
	default:
		return fmt.Errorf("category 须为 frame / prefer / lower / dedup / other，收到 %q", r.Category)
	}
	return st.Q().UpsertSelectionRule(ctx, domain.SelectionRule{
		RuleKey: r.RuleKey, Category: r.Category, Text: r.Text,
		DerivedFrom: strings.Join(r.DerivedFrom, ","), Version: r.Version,
	})
}
