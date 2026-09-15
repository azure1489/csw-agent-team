package ledger

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

var cst = time.FixedZone("CST", 8*3600)

// NormTime 统一为 UTC「2006-01-02T15:04:05Z」；无时区的按北京时间解释；解析不了原样返回。
func NormTime(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return ""
	}
	if t, err := time.Parse(time.RFC3339, s); err == nil {
		return t.UTC().Format("2006-01-02T15:04:05Z")
	}
	for _, layout := range []string{"2006-01-02 15:04:05", "2006-01-02 15:04", "2006/01/02 15:04:05", "2006/01/02 15:04", "2006-01-02", "2006/01/02", "2006年01月02日 15:04", "2006年01月02日"} {
		if t, err := time.ParseInLocation(layout, s, cst); err == nil {
			return t.UTC().Format("2006-01-02T15:04:05Z")
		}
	}
	return s
}

// Buckets 快照龄期。
var Buckets = map[string]time.Duration{"2h": 2 * time.Hour, "24h": 24 * time.Hour, "72h": 72 * time.Hour, "7d": 7 * 24 * time.Hour, "30d": 30 * 24 * time.Hour}

// SyncResult 一次同步的计数。
type SyncResult struct {
	Fetched, Inserted, Updated, Split int
	Gaps                              []string
}

func rawJSON(m map[string]any) string {
	if len(m) == 0 {
		return ""
	}
	b, _ := json.Marshal(m)
	return string(b)
}

// Sync 拉取 since 之后的内容写入发布记录（草稿与已发分开记状态），缺正文的补拉明细；
// 公众号合集自动拆条（已有拆条的不覆盖，人工校对过的保持）。结果写一条 sync_runs（覆盖范围与缺口）。
func Sync(ctx context.Context, st *sqlite.Store, a Adapter, account, kind string, since time.Time) (SyncResult, error) {
	var res SyncResult
	q := st.Q()
	now := time.Now().UTC()
	runID, err := q.StartSyncRun(ctx, a.Platform(), kind, since.UTC().Format("2006-01-02T15:04:05Z"), now.Format("2006-01-02T15:04:05Z"))
	if err != nil {
		return res, err
	}
	finish := func(ok bool, errText string) {
		_ = q.FinishSyncRun(ctx, domain.LedgerSyncRun{ID: runID, OK: ok, Fetched: res.Fetched, Inserted: res.Inserted,
			Updated: res.Updated, GapNote: strings.Join(res.Gaps, "；"), Error: errText})
	}
	posts, err := a.List(ctx, since)
	if err != nil {
		finish(false, err.Error())
		return res, err
	}
	res.Fetched = len(posts)
	for _, p := range posts {
		if p.Body == "" {
			full, err := a.Get(ctx, p.PostID)
			switch {
			case err == nil:
				p.Body = full.Body
				if p.Title == "" {
					p.Title = full.Title
				}
			case errors.Is(err, ErrUnavailable):
				res.Gaps = append(res.Gaps, p.PostID+" 正文不可得")
			default:
				res.Gaps = append(res.Gaps, p.PostID+" 正文读取失败："+err.Error())
			}
		}
		state := p.State
		if state != "draft" {
			state = "published"
		}
		id, inserted, err := q.UpsertLedgerPost(ctx, domain.LedgerPost{
			Platform: a.Platform(), Account: account, PostID: p.PostID, URL: p.URL, PublishedAt: NormTime(p.PublishedAt),
			Title: p.Title, BodyText: p.Body, Source: "sync", State: state, RawJSON: rawJSON(p.Raw),
		})
		if err != nil {
			finish(false, err.Error())
			return res, err
		}
		if inserted {
			res.Inserted++
		} else {
			res.Updated++
		}
		if a.Platform() == "wechat" && p.Body != "" {
			items, err := q.ListPostItems(ctx, id)
			if err != nil {
				finish(false, err.Error())
				return res, err
			}
			if len(items) == 0 {
				if split := SplitWechat(p.Body); len(split) > 0 {
					if err := q.ReplacePostItems(ctx, id, split, "auto", ""); err != nil {
						finish(false, err.Error())
						return res, err
					}
					res.Split++
				}
			}
		}
	}
	finish(true, "")
	return res, nil
}

// Snapshot 为发布满 bucket 且该龄期尚无快照的内容采一次原始指标。读不到的记为缺口，不写 0。
func Snapshot(ctx context.Context, st *sqlite.Store, a Adapter, bucket string) (int, []string, error) {
	d, ok := Buckets[bucket]
	if !ok {
		return 0, nil, fmt.Errorf("bucket 须为 2h / 24h / 72h / 7d / 30d")
	}
	q := st.Q()
	now := time.Now().UTC()
	due, err := q.PostsDueForSnapshot(ctx, a.Platform(), bucket, now.Add(-d).Format("2006-01-02T15:04:05Z"))
	if err != nil {
		return 0, nil, err
	}
	var gaps []string
	n := 0
	for _, p := range due {
		m, err := a.Metrics(ctx, p.PostID)
		if err != nil {
			gaps = append(gaps, p.PostID+"："+err.Error())
			continue
		}
		c := map[string]string{}
		for k, v := range m {
			c[k] = fmt.Sprint(v)
		}
		var flags string
		if f := NoteFlags(c); len(f) > 0 {
			b, _ := json.Marshal(f)
			flags = string(b)
		}
		ok, err := q.InsertMetricSnapshot(ctx, domain.MetricSnapshot{PostRef: p.ID, Platform: a.Platform(), AgeBucket: bucket,
			CollectedAt: now.Format("2006-01-02T15:04:05Z"), RawJSON: rawJSON(m), FlagsJSON: flags})
		if err != nil {
			return n, gaps, err
		}
		if ok {
			n++
		}
	}
	return n, gaps, nil
}

// TimeZero 很早的时间点（回填全部）。
func TimeZero() time.Time { return time.Date(2000, 1, 1, 0, 0, 0, 0, time.UTC) }
