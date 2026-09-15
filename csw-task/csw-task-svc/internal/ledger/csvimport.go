package ledger

import (
	"crypto/sha1"
	"encoding/csv"
	"encoding/hex"
	"fmt"
	"io"
	"strconv"
	"strings"
)

// 人工导出表格的列名别名（按平台导出习惯；未识别的列原样进 raw）。
var aliases = map[string][]string{
	"post_id":      {"笔记ID", "笔记id", "作品ID", "文章ID", "ID", "id"},
	"url":          {"笔记链接", "链接", "文章链接", "作品链接"},
	"title":        {"笔记标题", "标题", "作品标题", "文章标题"},
	"published_at": {"发布时间", "首次发布时间", "发表时间"},
	"impressions":  {"曝光", "曝光量", "曝光数"},
	"views":        {"观看", "观看量", "阅读", "阅读量", "阅读数"},
	"likes":        {"点赞", "点赞数"},
	"comments":     {"评论", "评论数"},
	"saves":        {"收藏", "收藏数"},
	"shares":       {"分享", "分享数"},
	"follows":      {"涨粉", "净涨粉", "关注"},
	"new_follows":  {"新增关注", "新增粉丝", "新增"},
	"unfollows":    {"取消关注", "取消", "掉粉"},
	"avg_watch":    {"平均观看时长", "人均观看时长", "平均阅读时长"},
	"date":         {"日期", "时间"},
}

func canon(header string) string {
	h := strings.TrimSpace(strings.TrimPrefix(header, "\ufeff"))
	for k, as := range aliases {
		for _, a := range as {
			if h == a {
				return k
			}
		}
	}
	return ""
}

// number 解析表格数值：去逗号、百分号与单位；不是数字返回 false（保留原文，不当 0）。
func number(s string) (float64, bool) {
	s = strings.TrimSpace(strings.NewReplacer(",", "", "%", "", "秒", "", "s", "").Replace(s))
	if s == "" || s == "-" || s == "--" {
		return 0, false
	}
	f, err := strconv.ParseFloat(s, 64)
	return f, err == nil
}

// NoteRow 笔记列表的一行：内容信息 + 原始指标。
type NoteRow struct {
	Post    RemotePost
	Metrics map[string]any
	Flags   []string
}

func rowMap(header, rec []string) (map[string]string, map[string]any) {
	canonical := map[string]string{}
	raw := map[string]any{}
	for i, h := range header {
		if i >= len(rec) {
			break
		}
		v := strings.TrimSpace(rec[i])
		name := strings.TrimSpace(strings.TrimPrefix(h, "\ufeff"))
		if f, ok := number(v); ok {
			raw[name] = f
		} else {
			raw[name] = v
		}
		if c := canon(h); c != "" {
			canonical[c] = v
		}
	}
	return canonical, raw
}

// NoteFlags 单篇异常标记：有观看而曝光为 0 → pending_refresh（平台数据可能尚未刷新，不能认定表现差）。
func NoteFlags(c map[string]string) []string {
	var flags []string
	views, vok := number(c["views"])
	imp, iok := number(c["impressions"])
	if vok && iok && views > 0 && imp == 0 {
		flags = append(flags, "pending_refresh:观看>0 而曝光=0")
	}
	if w, ok := number(c["avg_watch"]); ok && vok && views > 0 && w == 0 {
		flags = append(flags, "pending_refresh:观看>0 而平均时长=0")
	}
	return flags
}

// ParseNotes 解析笔记 / 文章列表明细。没有 ID 列时用 链接 或 标题+发布时间 派生稳定 ID。
func ParseNotes(r io.Reader) ([]NoteRow, error) {
	cr := csv.NewReader(r)
	cr.FieldsPerRecord = -1
	header, err := cr.Read()
	if err != nil {
		return nil, fmt.Errorf("读表头：%w", err)
	}
	var out []NoteRow
	for {
		rec, err := cr.Read()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}
		c, raw := rowMap(header, rec)
		if c["title"] == "" && c["post_id"] == "" && c["url"] == "" {
			continue
		}
		id := c["post_id"]
		if id == "" {
			src := c["url"]
			if src == "" {
				src = c["title"] + "|" + c["published_at"]
			}
			sum := sha1.Sum([]byte(src))
			id = "csv-" + hex.EncodeToString(sum[:5])
		}
		metrics := map[string]any{}
		for _, k := range []string{"impressions", "views", "likes", "comments", "saves", "shares", "follows", "avg_watch"} {
			if v, ok := number(c[k]); ok {
				metrics[k] = v
			}
		}
		out = append(out, NoteRow{
			Post:    RemotePost{PostID: id, URL: c["url"], Title: c["title"], PublishedAt: c["published_at"], State: "published", Raw: raw},
			Metrics: metrics, Flags: NoteFlags(c),
		})
	}
	return out, nil
}

// AccountSummary 账号区间汇总：逐列求和的原值 + 逐日明细 + 异常标记。
type AccountSummary struct {
	WindowFrom string
	WindowTo   string
	Totals     map[string]float64
	Days       []map[string]any
	Flags      []string
}

// ParseAccount 解析账号近 N 日趋势表（逐日一行）。净涨粉 ≠ 新增 − 取消 时原样保留并标记差值。
func ParseAccount(r io.Reader) (AccountSummary, error) {
	var s AccountSummary
	cr := csv.NewReader(r)
	cr.FieldsPerRecord = -1
	header, err := cr.Read()
	if err != nil {
		return s, fmt.Errorf("读表头：%w", err)
	}
	s.Totals = map[string]float64{}
	for {
		rec, err := cr.Read()
		if err == io.EOF {
			break
		}
		if err != nil {
			return s, err
		}
		c, raw := rowMap(header, rec)
		if d := c["date"]; d != "" {
			if s.WindowFrom == "" || d < s.WindowFrom {
				s.WindowFrom = d
			}
			if d > s.WindowTo {
				s.WindowTo = d
			}
		}
		for i, h := range header {
			if i >= len(rec) || canon(h) == "date" {
				continue
			}
			if f, ok := number(rec[i]); ok {
				s.Totals[strings.TrimSpace(strings.TrimPrefix(h, "\ufeff"))] += f
			}
		}
		s.Days = append(s.Days, raw)
	}
	net, nok := sumCanon(header, s.Totals, "follows")
	add, aok := sumCanon(header, s.Totals, "new_follows")
	sub, sok := sumCanon(header, s.Totals, "unfollows")
	if nok && aok && sok && add-sub != net {
		s.Flags = append(s.Flags, fmt.Sprintf("net_mismatch:净涨粉 %.0f，新增−取消 %.0f，差 %.0f（原样保留，待平台定义或刷新时间解释）", net, add-sub, net-(add-sub)))
	}
	return s, nil
}

func sumCanon(header []string, totals map[string]float64, key string) (float64, bool) {
	for _, h := range header {
		if canon(h) == key {
			v, ok := totals[strings.TrimSpace(strings.TrimPrefix(h, "\ufeff"))]
			return v, ok
		}
	}
	return 0, false
}
