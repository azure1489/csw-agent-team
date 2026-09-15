package ledger

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"
)

// ProbeCheck 探针的一项结论。Status：ok / unavailable / error。
type ProbeCheck struct {
	Name   string `json:"name"`
	Status string `json:"status"`
	Detail string `json:"detail,omitempty"`
	Sample any    `json:"sample,omitempty"`
}

// ProbeReport 平台读取可行性报告（结论写进探针记录与生产约定变更记录）。
type ProbeReport struct {
	Platform string       `json:"platform"`
	RanAt    string       `json:"ran_at"`
	Checks   []ProbeCheck `json:"checks"`
}

// ExitCode 0=全部可行；3=部分可行；1=都不可得。
func (r ProbeReport) ExitCode() int {
	ok := 0
	for _, c := range r.Checks {
		if c.Status == "ok" {
			ok++
		}
	}
	switch {
	case ok == len(r.Checks) && ok > 0:
		return 0
	case ok > 0:
		return 3
	}
	return 1
}

func check(name string, err error, detail string, sample any) ProbeCheck {
	switch {
	case err == nil:
		return ProbeCheck{Name: name, Status: "ok", Detail: detail, Sample: sample}
	case errors.Is(err, ErrUnavailable):
		return ProbeCheck{Name: name, Status: "unavailable", Detail: err.Error()}
	}
	return ProbeCheck{Name: name, Status: "error", Detail: err.Error()}
}

func keys(m map[string]any) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// Probe 逐项验证：列表（近 30 天，是否含已知人工发布 / 草稿与已发能否区分）、单篇正文、单篇指标、账号趋势。
func Probe(ctx context.Context, a Adapter, known string) ProbeReport {
	rep := ProbeReport{Platform: a.Platform(), RanAt: time.Now().UTC().Format(time.RFC3339)}
	posts, err := a.List(ctx, time.Now().Add(-30*24*time.Hour))
	states := map[string]int{}
	found := known == ""
	for _, p := range posts {
		states[p.State]++
		if p.PostID == known {
			found = true
		}
	}
	var sample any
	if len(posts) > 0 {
		sample = map[string]any{"post_id": posts[0].PostID, "title": posts[0].Title, "published_at": posts[0].PublishedAt, "state": posts[0].State}
	}
	rep.Checks = append(rep.Checks, check("列表（近 30 天）", err, fmt.Sprintf("%d 篇；状态分布 %v", len(posts), states), sample))
	if err == nil {
		if known != "" && !found {
			rep.Checks = append(rep.Checks, ProbeCheck{Name: "包含已知人工发布", Status: "error", Detail: "列表里没有 " + known + "：人工发布可能漏同步"})
		} else if known != "" {
			rep.Checks = append(rep.Checks, ProbeCheck{Name: "包含已知人工发布", Status: "ok", Detail: known})
		}
		if len(states) > 1 || states["draft"] > 0 {
			rep.Checks = append(rep.Checks, ProbeCheck{Name: "草稿与已发分列", Status: "ok", Detail: fmt.Sprint(states)})
		} else {
			rep.Checks = append(rep.Checks, ProbeCheck{Name: "草稿与已发分列", Status: "unavailable", Detail: "列表只返回了一种状态，无法确认草稿不会混入已发库"})
		}
	}
	id := known
	if id == "" && len(posts) > 0 {
		id = posts[0].PostID
	}
	if id == "" {
		rep.Checks = append(rep.Checks, ProbeCheck{Name: "单篇明细", Status: "error", Detail: "没有可用的 post_id"})
		return rep
	}
	p, err := a.Get(ctx, id)
	detail := ""
	if err == nil {
		detail = fmt.Sprintf("正文 %d 字", len([]rune(p.Body)))
		if strings.TrimSpace(p.Body) == "" {
			err = fmt.Errorf("%w：正文为空", ErrUnavailable)
		}
	}
	rep.Checks = append(rep.Checks, check("单篇正文", err, detail, nil))
	m, err := a.Metrics(ctx, id)
	rep.Checks = append(rep.Checks, check("单篇指标", err, strings.Join(keys(m.Values), "、"), m.Values))
	acc, err := a.Account(ctx, 30)
	rep.Checks = append(rep.Checks, check("账号 30 天趋势", err, acc.WindowFrom+" ~ "+acc.WindowTo+"；字段 "+strings.Join(keys(acc.Raw), "、"), acc.Raw))
	return rep
}
