package ledger

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"sort"
	"strings"
	"testing"
	"time"
)

// fakeMPHelper 模拟 mp-helper 只读接口：已发布 2 条（其一为两篇的多图文、发文早于窗口）、草稿 1 条、
// 2026-09-10 的统计含 1 篇已发布与 1 篇群发；getbizsummary 未授权。
func fakeMPHelper(t *testing.T, calls *[]string) *httptest.Server {
	t.Helper()
	cst := time.FixedZone("CST", 8*3600)
	ts := func(s string) int64 {
		v, _ := time.ParseInLocation("2006-01-02 15:04", s, cst)
		return v.Unix()
	}
	write := func(w http.ResponseWriter, v any) { _ = json.NewEncoder(w).Encode(v) }
	mux := http.NewServeMux()
	mux.HandleFunc("/api/v1/published", func(w http.ResponseWriter, r *http.Request) {
		write(w, map[string]any{"total_count": 2, "item_count": 2, "item": []any{
			map[string]any{"article_id": "art1", "update_time": ts("2026-09-10 10:00"), "content": map[string]any{"news_item": []any{
				map[string]any{"title": "发布一", "url": "http://mp.weixin.qq.com/s?__biz=B&mid=100&idx=1&sn=x&chksm=aaa#rd"}}}},
			map[string]any{"article_id": "art2", "update_time": ts("2026-09-05 09:00"), "content": map[string]any{"news_item": []any{
				map[string]any{"title": "旧一", "url": "http://mp.weixin.qq.com/s?__biz=B&mid=200&idx=1&sn=y"},
				map[string]any{"title": "旧二", "url": "http://mp.weixin.qq.com/s?__biz=B&mid=200&idx=2&sn=z"}}}},
		}})
	})
	mux.HandleFunc("/api/v1/published/", func(w http.ResponseWriter, r *http.Request) {
		switch strings.TrimPrefix(r.URL.Path, "/api/v1/published/") {
		case "art1":
			write(w, map[string]any{"news_item": []any{map[string]any{"title": "发布一", "content": "<p>正文一</p>", "url": "u1"}}})
		case "art2":
			write(w, map[string]any{"news_item": []any{map[string]any{"title": "旧一", "content": "<p>a</p>"}, map[string]any{"title": "旧二", "content": "<p>b</p>"}}})
		default:
			w.WriteHeader(502)
			write(w, map[string]any{"error": map[string]any{"code": "wechat_error", "message": "GetPublished Error , errcode=53600", "wechat_errcode": 53600}})
		}
	})
	mux.HandleFunc("/api/v1/drafts", func(w http.ResponseWriter, r *http.Request) {
		write(w, map[string]any{"total_count": 1, "item": []any{map[string]any{"media_id": "m1", "update_time": ts("2026-09-11 08:00"),
			"content": map[string]any{"news_item": []any{map[string]any{"title": "草稿一"}}}}}})
	})
	mux.HandleFunc("/api/v1/drafts/m1", func(w http.ResponseWriter, r *http.Request) {
		write(w, map[string]any{"news_item": []any{map[string]any{"title": "草稿一", "content": "<p>草稿正文</p>"}}})
	})
	mux.HandleFunc("/api/v1/datacube/", func(w http.ResponseWriter, r *http.Request) {
		api := strings.TrimPrefix(r.URL.Path, "/api/v1/datacube/")
		*calls = append(*calls, api+" "+r.URL.Query().Get("begin")+"~"+r.URL.Query().Get("end"))
		switch api {
		case "getarticletotaldetail":
			if r.URL.Query().Get("begin") != "2026-09-10" {
				write(w, map[string]any{"list": []any{}})
				return
			}
			write(w, map[string]any{"list": []any{
				map[string]any{"ref_date": "2026-09-10", "msgid": "100_1", "publish_type": 1, "title": "发布一",
					"content_url": "https://mp.weixin.qq.com/s?__biz=B&mid=100&idx=1&sn=x&chksm=bbb", "detail_list": []any{map[string]any{"stat_date": "2026-09-10", "read_user": 120}}},
				map[string]any{"ref_date": "2026-09-10", "msgid": "300_1", "publish_type": 0, "title": "群发一",
					"content_url": "https://mp.weixin.qq.com/s?__biz=B&mid=300&idx=1&sn=q", "detail_list": []any{map[string]any{"stat_date": "2026-09-10", "read_user": 50}}},
			}})
		case "getusersummary", "getusercumulate":
			write(w, map[string]any{"list": []any{map[string]any{"ref_date": r.URL.Query().Get("begin")}}})
		default:
			w.WriteHeader(502)
			write(w, map[string]any{"error": map[string]any{"code": "wechat_error", "message": "errcode=48001 api unauthorized", "wechat_errcode": 48001}})
		}
	})
	return httptest.NewServer(mux)
}

func newTestMPHelper(t *testing.T) (*MPHelperAdapter, *[]string) {
	calls := &[]string{}
	srv := fakeMPHelper(t, calls)
	t.Cleanup(srv.Close)
	now := time.Date(2026, 9, 15, 12, 0, 0, 0, time.FixedZone("CST", 8*3600))
	return &MPHelperAdapter{BaseURL: srv.URL, APIKey: "k", Now: func() time.Time { return now }}, calls
}

func TestMPHelperListMergesPublishedMassAndDrafts(t *testing.T) {
	a, _ := newTestMPHelper(t)
	since := time.Date(2026, 9, 8, 0, 0, 0, 0, time.FixedZone("CST", 8*3600))
	posts, err := a.List(context.Background(), since)
	if err != nil {
		t.Fatal(err)
	}
	var got []string
	for _, p := range posts {
		got = append(got, p.State+" "+p.PostID+" "+p.Title)
	}
	sort.Strings(got)
	want := []string{"draft draft:m1 草稿一", "published art1 发布一", "published msg:2026-09-10:300_1 群发一"}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Fatalf("posts = %v\nwant %v", got, want)
	}
}

func TestMPHelperGet(t *testing.T) {
	a, _ := newTestMPHelper(t)
	ctx := context.Background()
	p, err := a.Get(ctx, "art1")
	if err != nil || p.Body != "<p>正文一</p>" || p.State != "published" {
		t.Fatalf("art1 = %+v err=%v", p, err)
	}
	if p, err := a.Get(ctx, "art2#2"); err != nil || p.Title != "旧二" {
		t.Fatalf("art2#2 = %+v err=%v", p, err)
	}
	if p, err := a.Get(ctx, "draft:m1"); err != nil || p.State != "draft" || p.Body != "<p>草稿正文</p>" {
		t.Fatalf("draft = %+v err=%v", p, err)
	}
	if _, err := a.Get(ctx, "msg:2026-09-10:300_1"); !errors.Is(err, ErrUnavailable) {
		t.Fatalf("mass get err = %v, want ErrUnavailable", err)
	}
	if _, err := a.Get(ctx, "nope"); err == nil || errors.Is(err, ErrUnavailable) {
		t.Fatalf("unknown article err = %v, want plain error", err)
	}
}

func TestMPHelperMetrics(t *testing.T) {
	a, _ := newTestMPHelper(t)
	ctx := context.Background()
	m, err := a.Metrics(ctx, "art1")
	if err != nil || mpStr(m.Values, "msgid") != "100_1" || m.Definitions["detail_list.read_user"] == nil {
		t.Fatalf("art1 metrics = %+v err=%v", m, err)
	}
	m, err = a.Metrics(ctx, "msg:2026-09-10:300_1")
	if err != nil || mpStr(m.Values, "publish_type") != "0" {
		t.Fatalf("mass metrics = %+v err=%v", m, err)
	}
	if _, err := a.Metrics(ctx, "art2"); !errors.Is(err, ErrUnavailable) {
		t.Fatalf("art2 metrics err = %v, want ErrUnavailable（统计里没有）", err)
	}
	if _, err := a.Metrics(ctx, "draft:m1"); !errors.Is(err, ErrUnavailable) {
		t.Fatalf("draft metrics err = %v", err)
	}
}

func TestMPHelperAccountChunksAndUnavailable(t *testing.T) {
	a, calls := newTestMPHelper(t)
	acc, err := a.Account(context.Background(), 14)
	if err != nil {
		t.Fatal(err)
	}
	if acc.WindowFrom != "2026-09-01" || acc.WindowTo != "2026-09-14" {
		t.Fatalf("window = %s ~ %s", acc.WindowFrom, acc.WindowTo)
	}
	if l, _ := acc.Raw["getusersummary"].([]any); len(l) != 2 {
		t.Fatalf("getusersummary = %v（应分两段各一条）", acc.Raw["getusersummary"])
	}
	if miss, _ := acc.Raw["_unavailable"].(map[string]string); miss["getbizsummary"] == "" {
		t.Fatalf("_unavailable = %v", acc.Raw["_unavailable"])
	}
	joined := strings.Join(*calls, "|")
	if !strings.Contains(joined, "getusersummary 2026-09-01~2026-09-07") || !strings.Contains(joined, "getusersummary 2026-09-08~2026-09-14") {
		t.Fatalf("calls = %v", *calls)
	}
}

func TestMPHelperProbeEndToEnd(t *testing.T) {
	a, _ := newTestMPHelper(t)
	rep := Probe(context.Background(), a, "art1")
	st := map[string]string{}
	for _, c := range rep.Checks {
		st[c.Name] = c.Status
	}
	for _, name := range []string{"列表（近 30 天）", "包含已知人工发布", "草稿与已发分列", "单篇正文", "单篇指标", "账号 30 天趋势"} {
		if st[name] != "ok" {
			t.Fatalf("probe %s = %q（全部：%v）", name, st[name], st)
		}
	}
}
