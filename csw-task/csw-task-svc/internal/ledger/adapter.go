// Package ledger 数据子系统：平台读取适配器、探针、同步与快照、合集拆条、人工导出表格导入。
// 平台读取走外部命令（包 mp-helper / opencli 的登录态），本包不假设任何接口一定可用：读不到的字段标「不可得」。
package ledger

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// ErrUnavailable 平台读不到该项（权限、接口或登录态所限），与「值为 0」严格区分。
var ErrUnavailable = errors.New("不可得")

// RemotePost 平台上的一篇内容。State：published / draft。
type RemotePost struct {
	PostID      string         `json:"post_id"`
	URL         string         `json:"url"`
	PublishedAt string         `json:"published_at"`
	Title       string         `json:"title"`
	Body        string         `json:"body"`
	State       string         `json:"state"`
	Raw         map[string]any `json:"raw"`
}

// AccountTrend 账号区间趋势（原值）。
type AccountTrend struct {
	WindowFrom string         `json:"window_from"`
	WindowTo   string         `json:"window_to"`
	Raw        map[string]any `json:"raw"`
}

// Adapter 平台读取能力。
type Adapter interface {
	Platform() string
	List(ctx context.Context, since time.Time) ([]RemotePost, error)
	Get(ctx context.Context, postID string) (RemotePost, error)
	Metrics(ctx context.Context, postID string) (map[string]any, error)
	Account(ctx context.Context, days int) (AccountTrend, error)
}

// CommandAdapter 外部命令适配器。命令约定（stdout 输出 JSON；读不到时退出码 3 并输出 {"unavailable":"原因"}）：
//
//	<cmd> list --since <RFC3339>   → {"posts":[{post_id,url,published_at,title,state,body?,raw?}]}
//	<cmd> get <post_id>            → {"post":{…}}
//	<cmd> metrics <post_id>        → {"metrics":{平台原始字段…}}
//	<cmd> account --days <n>       → {"window_from","window_to","raw":{…}}
//
// cmd 可以是 `ssh agent-host /opt/csw-sync/wx.sh` 这类在持有登录态的主机上执行的命令。
type CommandAdapter struct {
	Name    string // wechat | xhs
	Cmd     string
	Timeout time.Duration
}

// Platform 平台名。
func (a *CommandAdapter) Platform() string { return a.Name }

func (a *CommandAdapter) run(ctx context.Context, out any, args ...string) error {
	if strings.TrimSpace(a.Cmd) == "" {
		return fmt.Errorf("%s 未配置适配命令（CSW_SYNC_WX_CMD / CSW_SYNC_XHS_CMD）", a.Name)
	}
	to := a.Timeout
	if to <= 0 {
		to = 2 * time.Minute
	}
	ctx, cancel := context.WithTimeout(ctx, to)
	defer cancel()
	quoted := make([]string, 0, len(args))
	for _, s := range args {
		quoted = append(quoted, "'"+strings.ReplaceAll(s, "'", `'\''`)+"'")
	}
	cmd := exec.CommandContext(ctx, "sh", "-c", a.Cmd+" "+strings.Join(quoted, " "))
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()
	var ee *exec.ExitError
	if errors.As(err, &ee) && ee.ExitCode() == 3 {
		var u struct {
			Unavailable string `json:"unavailable"`
		}
		_ = json.Unmarshal(stdout.Bytes(), &u)
		return fmt.Errorf("%w：%s", ErrUnavailable, strings.TrimSpace(u.Unavailable))
	}
	if err != nil {
		return fmt.Errorf("%s %s 失败：%v %s", a.Name, args[0], err, strings.TrimSpace(stderr.String()))
	}
	if err := json.Unmarshal(stdout.Bytes(), out); err != nil {
		return fmt.Errorf("%s %s 输出不是约定的 JSON：%w", a.Name, args[0], err)
	}
	return nil
}

// List 列出 since 之后的内容（含草稿）。
func (a *CommandAdapter) List(ctx context.Context, since time.Time) ([]RemotePost, error) {
	var r struct {
		Posts []RemotePost `json:"posts"`
	}
	err := a.run(ctx, &r, "list", "--since", since.UTC().Format(time.RFC3339))
	return r.Posts, err
}

// Get 单篇明细（含正文）。
func (a *CommandAdapter) Get(ctx context.Context, postID string) (RemotePost, error) {
	var r struct {
		Post RemotePost `json:"post"`
	}
	err := a.run(ctx, &r, "get", postID)
	return r.Post, err
}

// Metrics 单篇原始指标。
func (a *CommandAdapter) Metrics(ctx context.Context, postID string) (map[string]any, error) {
	var r struct {
		Metrics map[string]any `json:"metrics"`
	}
	err := a.run(ctx, &r, "metrics", postID)
	return r.Metrics, err
}

// Account 账号区间趋势。
func (a *CommandAdapter) Account(ctx context.Context, days int) (AccountTrend, error) {
	var r AccountTrend
	err := a.run(ctx, &r, "account", "--days", fmt.Sprint(days))
	return r, err
}
