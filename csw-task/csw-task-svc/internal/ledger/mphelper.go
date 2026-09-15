package ledger

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

// MPHelperAdapter 公众号读取：经 mp-helper 的只读接口（已发布列表与单篇、草稿、数据统计），响应为微信原始返回。
//
// 内容来源两路合并：
//   - 已发布列表（freepublish）只含「发布」的图文，post_id 为 article_id（多图文第 n 篇为 article_id#n）；
//   - 「群发」的图文不在已发布列表里，从单篇统计（getarticletotaldetail，按发文日期返回当日全部图文）补齐，
//     post_id 为 msg:<发文日期>:<msgid>；这类图文没有正文接口，单篇正文标不可得；
//   - 草稿 post_id 为 draft:<media_id>（多图文加 #n），状态 draft。
//
// 统计按北京时间的自然日取，当天的数据次日才有。微信返回未授权（48001）、接口下线（47009）时一律返回 ErrUnavailable。
type MPHelperAdapter struct {
	BaseURL string
	APIKey  string
	HTTP    *http.Client
	Now     func() time.Time
}

var mpCST = time.FixedZone("CST", 8*3600)

const (
	mpDateFmt  = "2006-01-02"
	mpMaxItems = 1000 // 列表分页上限，防止异常返回导致死循环
	mpMaxDays  = 92   // 群发补齐最多回看的天数
)

// mpUnavailableCodes 这些 errcode 表示平台不提供（权限或接口所限），不是采集失败。
var mpUnavailableCodes = map[int64]bool{48001: true, 47009: true}

// Platform 平台名。
func (a *MPHelperAdapter) Platform() string { return "wechat" }

func (a *MPHelperAdapter) now() time.Time {
	if a.Now != nil {
		return a.Now()
	}
	return time.Now()
}

func mpDayOf(t time.Time) time.Time {
	y, m, d := t.In(mpCST).Date()
	return time.Date(y, m, d, 0, 0, 0, 0, mpCST)
}

func (a *MPHelperAdapter) get(ctx context.Context, path string, q url.Values, out any) error {
	if strings.TrimSpace(a.BaseURL) == "" {
		return fmt.Errorf("wechat 未配置 mp-helper 地址（CSW_SYNC_WX_MPHELPER_URL）")
	}
	u := strings.TrimRight(a.BaseURL, "/") + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+a.APIKey)
	hc := a.HTTP
	if hc == nil {
		hc = &http.Client{Timeout: 30 * time.Second}
	}
	resp, err := hc.Do(req)
	if err != nil {
		return fmt.Errorf("mp-helper %s：%w", path, err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 16<<20))
	if err != nil {
		return fmt.Errorf("mp-helper %s：%w", path, err)
	}
	if resp.StatusCode != http.StatusOK {
		var e struct {
			Error struct {
				Code          string `json:"code"`
				Message       string `json:"message"`
				WechatErrcode int64  `json:"wechat_errcode"`
			} `json:"error"`
		}
		_ = json.Unmarshal(body, &e)
		if mpUnavailableCodes[e.Error.WechatErrcode] {
			return fmt.Errorf("%w：微信 errcode=%d %s", ErrUnavailable, e.Error.WechatErrcode, e.Error.Message)
		}
		msg := strings.TrimSpace(e.Error.Message)
		if msg == "" {
			msg = strings.TrimSpace(string(body))
		}
		return fmt.Errorf("mp-helper %s → HTTP %d %s", path, resp.StatusCode, msg)
	}
	if err := json.Unmarshal(body, out); err != nil {
		return fmt.Errorf("mp-helper %s 返回不是 JSON：%w", path, err)
	}
	return nil
}

type mpItem struct {
	ArticleID  string `json:"article_id"`
	MediaID    string `json:"media_id"`
	UpdateTime int64  `json:"update_time"`
	Content    struct {
		NewsItem []map[string]any `json:"news_item"`
	} `json:"content"`
}

// pages 取完一个分页列表（已发布或草稿，不含正文）。
func (a *MPHelperAdapter) pages(ctx context.Context, path string) ([]mpItem, error) {
	var all []mpItem
	for offset := 0; offset < mpMaxItems; {
		var p struct {
			TotalCount int      `json:"total_count"`
			Item       []mpItem `json:"item"`
		}
		q := url.Values{"offset": {strconv.Itoa(offset)}, "count": {"20"}, "no_content": {"1"}}
		if err := a.get(ctx, path, q, &p); err != nil {
			return nil, err
		}
		all = append(all, p.Item...)
		offset += len(p.Item)
		if len(p.Item) == 0 || offset >= p.TotalCount {
			break
		}
	}
	return all, nil
}

// articleDetail 某个发文日期的单篇统计（当日发出的全部图文，含逐日明细）。
func (a *MPHelperAdapter) articleDetail(ctx context.Context, day time.Time) ([]map[string]any, error) {
	d := day.Format(mpDateFmt)
	var r struct {
		List []map[string]any `json:"list"`
	}
	err := a.get(ctx, "/api/v1/datacube/getarticletotaldetail", url.Values{"begin": {d}, "end": {d}}, &r)
	return r.List, err
}

func mpStr(m map[string]any, k string) string {
	if v, ok := m[k]; ok && v != nil {
		return fmt.Sprint(v)
	}
	return ""
}

// mpURLKey 图文链接的稳定键（mid + idx；否则 sn；短链取路径），忽略 chksm 等随访问变化的参数。
func mpURLKey(raw string) string {
	if strings.TrimSpace(raw) == "" {
		return ""
	}
	u, err := url.Parse(strings.ReplaceAll(raw, "&amp;", "&"))
	if err != nil {
		return ""
	}
	q := u.Query()
	if mid := q.Get("mid"); mid != "" {
		idx := q.Get("idx")
		if idx == "" {
			idx = "1"
		}
		return "mid:" + mid + ":" + idx
	}
	if sn := q.Get("sn"); sn != "" {
		return "sn:" + sn
	}
	if strings.HasPrefix(u.Path, "/s/") {
		return "path:" + u.Path
	}
	return ""
}

// mpSubID 多图文第 n 篇（从 1 起）加 #n，单篇保持原 id。
func mpSubID(id string, i, n int) string {
	if n <= 1 || i == 0 {
		return id
	}
	return fmt.Sprintf("%s#%d", id, i+1)
}

// mpSplitSub 拆出 id 与篇序（0 起）。
func mpSplitSub(id string) (string, int) {
	if j := strings.LastIndex(id, "#"); j > 0 {
		if n, err := strconv.Atoi(id[j+1:]); err == nil && n >= 1 {
			return id[:j], n - 1
		}
	}
	return id, 0
}

// mpRaw 平台原始字段（去掉正文，正文单独放 Body），附来源标记。
func mpRaw(n map[string]any, extra map[string]any) map[string]any {
	raw := make(map[string]any, len(n)+len(extra))
	for k, v := range n {
		if k != "content" && k != "detail_list" {
			raw[k] = v
		}
	}
	for k, v := range extra {
		raw[k] = v
	}
	return raw
}

func mpDeleted(n map[string]any) bool {
	b, _ := n["is_deleted"].(bool)
	return b
}

// List 列出 since 之后的内容：已发布（发布）+ 群发（从统计补齐）+ 草稿。
func (a *MPHelperAdapter) List(ctx context.Context, since time.Time) ([]RemotePost, error) {
	var posts []RemotePost
	pub, err := a.pages(ctx, "/api/v1/published")
	if err != nil {
		return nil, err
	}
	seen := map[string]bool{}
	for _, it := range pub {
		t := time.Unix(it.UpdateTime, 0)
		for i, n := range it.Content.NewsItem {
			if k := mpURLKey(mpStr(n, "url")); k != "" {
				seen[k] = true
			}
			if t.Before(since) || mpDeleted(n) {
				continue
			}
			posts = append(posts, RemotePost{
				PostID: mpSubID(it.ArticleID, i, len(it.Content.NewsItem)), URL: mpStr(n, "url"), Title: mpStr(n, "title"),
				PublishedAt: t.UTC().Format(time.RFC3339), State: "published",
				Raw: mpRaw(n, map[string]any{"article_id": it.ArticleID, "update_time": it.UpdateTime, "source": "freepublish"}),
			})
		}
	}
	mass, err := a.massPosts(ctx, since, seen)
	if err != nil && !errors.Is(err, ErrUnavailable) {
		return nil, err
	}
	posts = append(posts, mass...)
	drafts, err := a.pages(ctx, "/api/v1/drafts")
	if err != nil {
		return nil, err
	}
	for _, it := range drafts {
		t := time.Unix(it.UpdateTime, 0)
		if t.Before(since) {
			continue
		}
		for i, n := range it.Content.NewsItem {
			posts = append(posts, RemotePost{
				PostID: "draft:" + mpSubID(it.MediaID, i, len(it.Content.NewsItem)), URL: mpStr(n, "url"), Title: mpStr(n, "title"),
				PublishedAt: t.UTC().Format(time.RFC3339), State: "draft",
				Raw: mpRaw(n, map[string]any{"media_id": it.MediaID, "update_time": it.UpdateTime, "source": "draft"}),
			})
		}
	}
	return posts, nil
}

// massPosts 按发文日期逐日取统计，补齐不在已发布列表里的图文（群发）。当天的统计次日才有，不取。
func (a *MPHelperAdapter) massPosts(ctx context.Context, since time.Time, seen map[string]bool) ([]RemotePost, error) {
	today := mpDayOf(a.now())
	start := mpDayOf(since)
	if earliest := today.AddDate(0, 0, -mpMaxDays); start.Before(earliest) {
		start = earliest
	}
	var out []RemotePost
	for d := start; d.Before(today); d = d.AddDate(0, 0, 1) {
		items, err := a.articleDetail(ctx, d)
		if err != nil {
			return out, err
		}
		for _, it := range items {
			if seen[mpURLKey(mpStr(it, "content_url"))] {
				continue
			}
			ref := mpStr(it, "ref_date")
			at := d
			if t, err := time.ParseInLocation(mpDateFmt, ref, mpCST); err == nil {
				at = t
			} else {
				ref = d.Format(mpDateFmt)
			}
			out = append(out, RemotePost{
				PostID: "msg:" + ref + ":" + mpStr(it, "msgid"), URL: mpStr(it, "content_url"), Title: mpStr(it, "title"),
				PublishedAt: at.UTC().Format(time.RFC3339), State: "published",
				Raw: mpRaw(it, map[string]any{"source": "datacube"}),
			})
		}
	}
	return out, nil
}

func mpPick(items []map[string]any, idx int, postID string) (map[string]any, error) {
	if idx < 0 || idx >= len(items) {
		return nil, fmt.Errorf("%s：没有第 %d 篇", postID, idx+1)
	}
	return items[idx], nil
}

// Get 单篇明细（含正文）。群发图文没有正文接口，返回 ErrUnavailable。
func (a *MPHelperAdapter) Get(ctx context.Context, postID string) (RemotePost, error) {
	if strings.HasPrefix(postID, "msg:") {
		return RemotePost{PostID: postID, State: "published"}, fmt.Errorf("%w：群发图文没有正文接口（只能从统计取标题与链接）", ErrUnavailable)
	}
	state, path, id := "published", "/api/v1/published/", postID
	if strings.HasPrefix(postID, "draft:") {
		state, path, id = "draft", "/api/v1/drafts/", strings.TrimPrefix(postID, "draft:")
	}
	id, idx := mpSplitSub(id)
	var r struct {
		NewsItem []map[string]any `json:"news_item"`
	}
	if err := a.get(ctx, path+url.PathEscape(id), nil, &r); err != nil {
		return RemotePost{}, err
	}
	n, err := mpPick(r.NewsItem, idx, postID)
	if err != nil {
		return RemotePost{}, err
	}
	return RemotePost{
		PostID: postID, URL: mpStr(n, "url"), Title: mpStr(n, "title"), Body: mpStr(n, "content"), State: state,
		Raw: mpRaw(n, map[string]any{"source": strings.TrimSuffix(strings.TrimPrefix(path, "/api/v1/"), "/")}),
	}, nil
}

// mpArticleDefs 单篇统计字段说明（微信原字段名；口径未经官方说明核实的标「口径待核」）。
var mpArticleDefs = map[string]any{
	"ref_date":                        "发文日期（北京时间）",
	"msgid":                           "图文消息 id（消息 id_篇序）",
	"publish_type":                    "发文方式原值：实测 1 为「发布」、0 为「群发」（口径待核）",
	"content_url":                     "图文链接",
	"detail_list":                     "发文后逐日明细，每项 stat_date 为统计日（是否为累计值口径待核）",
	"detail_list.read_user":           "阅读人数",
	"detail_list.read_user_source":    "阅读人数按来源场景拆分（scene_desc 为场景名，含「全部」）",
	"detail_list.share_user":          "分享人数",
	"detail_list.read_subscribe_user": "阅读后关注人数",
	"detail_list.read_delivery_rate":  "送达阅读率",
	"detail_list.read_avg_activetime": "平均阅读时长（单位口径待核）",
	"detail_list.zaikan_user":         "在看人数",
	"detail_list.like_user":           "点赞人数",
	"detail_list.praise_money":        "赞赏金额",
	"detail_list.comment_count":       "留言数",
	"detail_list.collection_user":     "收藏人数",
	"detail_list.read_jump_position":  "阅读跳出位置分布（position 为文章分段，rate 为比例）",
}

// Metrics 单篇原始指标：到发文日期（及此前两日，兼容发布后改稿导致更新时间晚于发文日）的统计里匹配该篇。
func (a *MPHelperAdapter) Metrics(ctx context.Context, postID string) (PostMetrics, error) {
	var day time.Time
	var match func(map[string]any) bool
	switch {
	case strings.HasPrefix(postID, "draft:"):
		return PostMetrics{}, fmt.Errorf("%w：草稿没有数据", ErrUnavailable)
	case strings.HasPrefix(postID, "msg:"):
		parts := strings.SplitN(strings.TrimPrefix(postID, "msg:"), ":", 2)
		d, err := time.ParseInLocation(mpDateFmt, parts[0], mpCST)
		if len(parts) != 2 || err != nil {
			return PostMetrics{}, fmt.Errorf("无法识别的 post_id：%s", postID)
		}
		day, msgid := d, parts[1]
		match = func(it map[string]any) bool { return mpStr(it, "msgid") == msgid }
		return a.findMetrics(ctx, postID, day, match)
	default:
		id, idx := mpSplitSub(postID)
		pub, err := a.pages(ctx, "/api/v1/published")
		if err != nil {
			return PostMetrics{}, err
		}
		for _, it := range pub {
			if it.ArticleID != id {
				continue
			}
			n, err := mpPick(it.Content.NewsItem, idx, postID)
			if err != nil {
				return PostMetrics{}, err
			}
			key, title := mpURLKey(mpStr(n, "url")), mpStr(n, "title")
			day = mpDayOf(time.Unix(it.UpdateTime, 0))
			match = func(m map[string]any) bool {
				if key != "" {
					return mpURLKey(mpStr(m, "content_url")) == key
				}
				return mpStr(m, "title") == title
			}
			return a.findMetrics(ctx, postID, day, match)
		}
		return PostMetrics{}, fmt.Errorf("已发布列表里没有 %s", postID)
	}
}

func (a *MPHelperAdapter) findMetrics(ctx context.Context, postID string, day time.Time, match func(map[string]any) bool) (PostMetrics, error) {
	today := mpDayOf(a.now())
	for back := 0; back < 3; back++ {
		d := day.AddDate(0, 0, -back)
		if !d.Before(today) {
			continue
		}
		items, err := a.articleDetail(ctx, d)
		if err != nil {
			return PostMetrics{}, err
		}
		for _, it := range items {
			if match(it) {
				return PostMetrics{Values: it, Definitions: mpArticleDefs}, nil
			}
		}
	}
	return PostMetrics{}, fmt.Errorf("%w：%s 的统计尚未生成（发文次日可取）或不在统计范围内", ErrUnavailable, postID)
}

// mpAccountDefs 账号级接口说明。
var mpAccountDefs = map[string]any{
	"getusersummary":  "用户增减：ref_date 日期、user_source 关注来源编号、new_user 新增关注、cancel_user 取消关注",
	"getusercumulate": "累计用户：ref_date 日期、cumulate_user 总用户量",
	"getbizsummary":   "账号每日阅读汇总：ref_date 日期、detail 为当日阅读、分享等原值（口径待核）",
	"_unavailable":    "本次读不到的接口与原因（平台权限或接口所限）",
}

// Account 账号区间趋势：最近 days 天（到昨天为止），按 7 天一段取三个账号级接口，原值拼接。
func (a *MPHelperAdapter) Account(ctx context.Context, days int) (AccountTrend, error) {
	if days <= 0 {
		days = 30
	}
	end := mpDayOf(a.now()).AddDate(0, 0, -1)
	begin := end.AddDate(0, 0, -(days - 1))
	raw := map[string]any{}
	missing := map[string]string{}
	for _, api := range []string{"getusersummary", "getusercumulate", "getbizsummary"} {
		var list []any
		var err error
		for b := begin; !b.After(end) && err == nil; b = b.AddDate(0, 0, 7) {
			e := b.AddDate(0, 0, 6)
			if e.After(end) {
				e = end
			}
			var r struct {
				List []any `json:"list"`
			}
			err = a.get(ctx, "/api/v1/datacube/"+api, url.Values{"begin": {b.Format(mpDateFmt)}, "end": {e.Format(mpDateFmt)}}, &r)
			list = append(list, r.List...)
		}
		switch {
		case errors.Is(err, ErrUnavailable):
			missing[api] = err.Error()
		case err != nil:
			return AccountTrend{}, err
		default:
			raw[api] = list
		}
	}
	if len(raw) == 0 {
		return AccountTrend{}, fmt.Errorf("%w：账号级接口都读不到 %v", ErrUnavailable, missing)
	}
	if len(missing) > 0 {
		raw["_unavailable"] = missing
	}
	return AccountTrend{WindowFrom: begin.Format(mpDateFmt), WindowTo: end.Format(mpDateFmt), Raw: raw, Definitions: mpAccountDefs}, nil
}

var _ Adapter = (*MPHelperAdapter)(nil)
