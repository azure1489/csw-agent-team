// Package wfctl 定义面客户端：把工作流定义导出为 YAML、本地改、检查、比对，再以草稿写回管理后台；
// 激活必须由人在终端里确认。只走管理后台已有 API（JWT 登录），不直连数据库。
package wfctl

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/wfdef"
)

const usage = `wfctl ` + Version + ` · 工作流定义面客户端（走管理后台 API）

用法：
  wfctl export <key>[@ver] [-o 目录]          导出为 YAML（长文本外置为同名目录下的 md）；缺省导出当前 active
  wfctl diff <file> [--against <key>@<ver>]   与服务端版本比对（缺省比 active）
  wfctl lint <file>                           本地 + 服务端规则检查
  wfctl push <file> --draft --reason "…"      新建草稿版本并整份写入（不激活）
  wfctl validate <key>@<ver>                  服务端十项校验
  wfctl activate <key>@<ver> --reason "…"     打印差异 → 在终端手输 <key>@<ver> 确认 → 激活
  wfctl history <key>                         版本与变更记录（含修改原因）
  wfctl rollback <key>@<ver> --reason "…"     把旧版本复制为新草稿（前滚；激活另行确认）
  wfctl roles | agents | roster               只读：角色 / 成员 / 花名册

环境变量：CSW_ADMIN_URL（如 https://tasks.aworld.ltd）· CSW_ADMIN_USER · CSW_ADMIN_PASSWORD
退出码：0 成功 · 1 失败 · 2 用法错误或需要人确认 · 3 lint 只有警告`

// Env 运行环境（测试可注入）。
type Env struct {
	In     io.Reader
	Out    io.Writer
	URL    string
	User   string
	Pass   string
	IsTTY  bool
	AllowY bool // 是否接受 --yes（仅人工在终端外使用；skill 禁用）
}

// Run 从进程环境运行，返回退出码。
func Run(args []string) int {
	fi, _ := os.Stdin.Stat()
	tty := fi != nil && fi.Mode()&os.ModeCharDevice != 0
	return RunWith(args, Env{
		In: os.Stdin, Out: os.Stdout, URL: os.Getenv("CSW_ADMIN_URL"),
		User: os.Getenv("CSW_ADMIN_USER"), Pass: os.Getenv("CSW_ADMIN_PASSWORD"), IsTTY: tty, AllowY: true,
	})
}

type opts struct {
	flags map[string]string
	pos   []string
}

func parse(args []string) opts {
	o := opts{flags: map[string]string{}}
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch a {
		case "--draft", "--yes":
			o.flags[a] = "true"
		case "--reason", "-o", "--against":
			if i+1 < len(args) {
				o.flags[a] = args[i+1]
				i++
			} else {
				o.flags[a] = ""
			}
		default:
			o.pos = append(o.pos, a)
		}
	}
	return o
}

type ctl struct {
	env Env
	c   *client
	ctx context.Context
}

func (x *ctl) printf(format string, a ...any) { fmt.Fprintf(x.env.Out, format, a...) }

// RunWith 以给定环境运行。
func RunWith(args []string, env Env) int {
	if len(args) == 0 || args[0] == "-h" || args[0] == "--help" || args[0] == "help" {
		fmt.Fprintln(env.Out, usage)
		return 2
	}
	if args[0] == "--version" {
		fmt.Fprintln(env.Out, "wfctl", Version)
		return 0
	}
	if env.URL == "" {
		fmt.Fprintln(env.Out, "未配置 CSW_ADMIN_URL")
		return 2
	}
	c, err := newClient(env.URL, env.User, env.Pass)
	if err != nil {
		fmt.Fprintln(env.Out, err)
		return 1
	}
	x := &ctl{env: env, c: c, ctx: context.Background()}
	o := parse(args[1:])
	var code int
	switch args[0] {
	case "export":
		code, err = x.export(o)
	case "diff":
		code, err = x.diff(o)
	case "lint":
		code, err = x.lint(o)
	case "push":
		code, err = x.push(o)
	case "validate":
		code, err = x.validate(o)
	case "activate":
		code, err = x.activate(o)
	case "history":
		code, err = x.history(o)
	case "rollback":
		code, err = x.rollback(o)
	case "roles":
		code, err = x.roles()
	case "agents":
		code, err = x.agents()
	case "roster":
		code, err = x.roster()
	default:
		fmt.Fprintln(env.Out, usage)
		return 2
	}
	if err != nil {
		x.printf("错误：%v\n", err)
		if code == 0 {
			code = 1
		}
	}
	return code
}

// ── 工作流定位 ──

type wfRow struct {
	Key     string `json:"wf_key"`
	Name    string `json:"name"`
	Status  string `json:"status"`
	ID      int64  `json:"id"`
	Version int    `json:"version"`
}

func (x *ctl) versions(key string) ([]wfRow, error) {
	var r struct {
		Workflows []wfRow `json:"workflows"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/workflows?search="+url.QueryEscape(key), nil, &r); err != nil {
		return nil, err
	}
	var out []wfRow
	for _, w := range r.Workflows {
		if w.Key == key {
			out = append(out, w)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Version > out[j].Version })
	return out, nil
}

// resolve 解析 <key>[@ver]；不带版本时取 active。
func (x *ctl) resolve(ref string) (wfRow, error) {
	key, verS, hasVer := strings.Cut(ref, "@")
	vs, err := x.versions(key)
	if err != nil {
		return wfRow{}, err
	}
	if len(vs) == 0 {
		return wfRow{}, fmt.Errorf("没有工作流 %s", key)
	}
	if !hasVer {
		for _, v := range vs {
			if v.Status == "active" {
				return v, nil
			}
		}
		return wfRow{}, fmt.Errorf("%s 没有 active 版本，请写明 %s@<版本>", key, key)
	}
	ver, err := strconv.Atoi(strings.TrimPrefix(strings.ToLower(verS), "v"))
	if err != nil {
		return wfRow{}, fmt.Errorf("版本号不合法：%s", verS)
	}
	for _, v := range vs {
		if v.Version == ver {
			return v, nil
		}
	}
	return wfRow{}, fmt.Errorf("%s 没有版本 v%d", key, ver)
}

func (x *ctl) fetch(id int64) (*wfdef.Def, wfdef.GetBody, error) {
	var g wfdef.GetBody
	if err := x.c.do(x.ctx, http.MethodGet, fmt.Sprintf("/admin/workflows/%d", id), nil, &g); err != nil {
		return nil, g, err
	}
	return wfdef.FromGet(g), g, nil
}

func (x *ctl) roleMap() (map[string]domain.Role, error) {
	var r struct {
		Roles []struct {
			Code         string `json:"code"`
			Name         string `json:"name"`
			IsManagement bool   `json:"is_management"`
			IsHuman      bool   `json:"is_human"`
			MemberCount  int    `json:"member_count"`
		} `json:"roles"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/roles", nil, &r); err != nil {
		return nil, err
	}
	m := map[string]domain.Role{}
	for _, ro := range r.Roles {
		m[ro.Code] = domain.Role{Code: ro.Code, Name: ro.Name, IsManagement: ro.IsManagement, IsHuman: ro.IsHuman}
	}
	return m, nil
}

// ── 命令 ──

func (x *ctl) export(o opts) (int, error) {
	if len(o.pos) != 1 {
		return 2, errors.New("用法：wfctl export <key>[@ver] [-o 目录]")
	}
	row, err := x.resolve(o.pos[0])
	if err != nil {
		return 1, err
	}
	def, _, err := x.fetch(row.ID)
	if err != nil {
		return 1, err
	}
	dir := o.flags["-o"]
	if dir == "" {
		dir = "."
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return 1, err
	}
	base := fmt.Sprintf("%s_v%d", row.Key, row.Version)
	path := filepath.Join(dir, base+".yaml")
	if err := wfdef.Save(def, path, base); err != nil {
		return 1, err
	}
	x.printf("已导出 %s@v%d（%s）→ %s\n", row.Key, row.Version, row.Status, path)
	return 0, nil
}

func (x *ctl) baseFor(o opts, key string) (wfRow, *wfdef.Def, error) {
	ref := key
	if a := o.flags["--against"]; a != "" {
		ref = a
	}
	row, err := x.resolve(ref)
	if err != nil {
		return row, nil, err
	}
	def, _, err := x.fetch(row.ID)
	return row, def, err
}

func (x *ctl) diff(o opts) (int, error) {
	if len(o.pos) != 1 {
		return 2, errors.New("用法：wfctl diff <file> [--against <key>@<ver>]")
	}
	local, err := wfdef.Load(o.pos[0])
	if err != nil {
		return 1, err
	}
	row, base, err := x.baseFor(o, local.Meta.Key)
	if err != nil {
		return 1, err
	}
	x.printf("对比基线：%s@v%d（%s）\n\n%s", row.Key, row.Version, row.Status, wfdef.Render(wfdef.Diff(base, local), local))
	return 0, nil
}

// serverFindings 需要服务端状态的规则：L3 角色没有活跃成员；L4 被删阶段在进行中的 run 里还有未完成任务。
func (x *ctl) serverFindings(local *wfdef.Def) ([]wfdef.Finding, error) {
	var fs []wfdef.Finding
	var ag struct {
		Agents []struct {
			RoleCode string `json:"role_code"`
			Active   bool   `json:"active"`
		} `json:"agents"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/agents", nil, &ag); err != nil {
		return nil, err
	}
	active := map[string]bool{}
	for _, a := range ag.Agents {
		if a.Active {
			active[a.RoleCode] = true
		}
	}
	for _, s := range local.Stages {
		if !active[s.Role] {
			fs = append(fs, wfdef.Finding{Level: "warn", Rule: "L3", Stage: s.Code, Msg: fmt.Sprintf("角色 %s 没有活跃成员，派工会落空", s.Role)})
		}
	}
	row, err := x.resolve(local.Meta.Key)
	if err != nil {
		return fs, nil // 新工作流：没有 active 版本可比
	}
	base, _, err := x.fetch(row.ID)
	if err != nil {
		return nil, err
	}
	keep := map[string]bool{}
	for _, s := range local.Stages {
		keep[s.Code] = true
	}
	var removed []string
	for _, s := range base.Stages {
		if !keep[s.Code] {
			removed = append(removed, s.Code)
		}
	}
	if len(removed) == 0 {
		return fs, nil
	}
	var runs struct {
		Runs []struct {
			ID int64 `json:"id"`
		} `json:"runs"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/runs?status=active&workflow="+url.QueryEscape(local.Meta.Key), nil, &runs); err != nil {
		return nil, err
	}
	for _, r := range runs.Runs {
		var rd struct {
			Tasks []struct {
				StageCode string `json:"stage_code"`
				Status    string `json:"status"`
			} `json:"tasks"`
		}
		if err := x.c.do(x.ctx, http.MethodGet, fmt.Sprintf("/admin/runs/%d", r.ID), nil, &rd); err != nil {
			return nil, err
		}
		for _, t := range rd.Tasks {
			for _, rc := range removed {
				if t.StageCode == rc && t.Status != "passed" && t.Status != "cancelled" {
					fs = append(fs, wfdef.Finding{Level: "warn", Rule: "L4", Stage: rc,
						Msg: fmt.Sprintf("要删除的阶段在进行中的 run #%d 里还有未完成任务（%s）；旧 run 按快照继续，不受新定义影响", r.ID, t.Status)})
				}
			}
		}
	}
	return fs, nil
}

func (x *ctl) runLint(local *wfdef.Def) ([]wfdef.Finding, error) {
	roles, err := x.roleMap()
	if err != nil {
		return nil, err
	}
	fs := wfdef.LintLocal(local, roles)
	sf, err := x.serverFindings(local)
	if err != nil {
		return nil, err
	}
	return append(fs, sf...), nil
}

func (x *ctl) printFindings(fs []wfdef.Finding) {
	if len(fs) == 0 {
		x.printf("检查通过：无错误、无警告。\n")
		return
	}
	for _, f := range fs {
		lv := "警告"
		if f.Level == "error" {
			lv = "错误"
		}
		st := f.Stage
		if st == "" {
			st = "—"
		}
		x.printf("[%s][%s] %s：%s\n", lv, f.Rule, st, f.Msg)
	}
}

func (x *ctl) lint(o opts) (int, error) {
	if len(o.pos) != 1 {
		return 2, errors.New("用法：wfctl lint <file>")
	}
	local, err := wfdef.Load(o.pos[0])
	if err != nil {
		return 1, err
	}
	fs, err := x.runLint(local)
	if err != nil {
		return 1, err
	}
	x.printFindings(fs)
	return wfdef.Worst(fs), nil
}

func (x *ctl) push(o opts) (int, error) {
	if len(o.pos) != 1 || o.flags["--draft"] != "true" {
		return 2, errors.New("用法：wfctl push <file> --draft --reason \"…\"（push 只建草稿，不会激活）")
	}
	reason := strings.TrimSpace(o.flags["--reason"])
	if reason == "" {
		return 2, errors.New("--reason 必填：写明为什么改（用户原话或操作员说明），会进审计")
	}
	local, err := wfdef.Load(o.pos[0])
	if err != nil {
		return 1, err
	}
	fs, err := x.runLint(local)
	if err != nil {
		return 1, err
	}
	if wfdef.Worst(fs) == 1 {
		x.printFindings(fs)
		return 1, errors.New("检查有错误，未写入")
	}
	summary := "新工作流"
	if row, err := x.resolve(local.Meta.Key); err == nil {
		base, _, err := x.fetch(row.ID)
		if err != nil {
			return 1, err
		}
		cs := wfdef.Diff(base, local)
		if len(cs) == 0 {
			x.printf("与 %s@v%d 无差异，未建草稿。\n", row.Key, row.Version)
			return 0, nil
		}
		lines := make([]string, 0, len(cs))
		for _, c := range cs {
			lines = append(lines, strings.TrimSpace(c.Kind+" "+c.Stage+" "+c.Detail))
		}
		summary = fmt.Sprintf("相对 v%d：", row.Version) + strings.Join(lines, "；")
		if r := []rune(summary); len(r) > 1800 {
			summary = string(r[:1800]) + "…"
		}
	}
	var created struct {
		ID      int64 `json:"id"`
		Version int   `json:"version"`
	}
	if err := x.c.do(x.ctx, http.MethodPost, "/admin/workflows", map[string]string{
		"wf_key": local.Meta.Key, "name": local.Meta.Name, "hub_role": local.Meta.HubRole,
		"dispatch_mode": local.Meta.DispatchMode, "trigger_roles": strings.Join(local.Meta.TriggerRoles, ","),
	}, &created); err != nil {
		return 1, err
	}
	body := wfdef.ToPut(local)
	body.Reason, body.ChangeSummary = reason, summary
	if err := x.c.do(x.ctx, http.MethodPut, fmt.Sprintf("/admin/workflows/%d", created.ID), body, nil); err != nil {
		return 1, fmt.Errorf("草稿 v%d 已建但写入失败：%w", created.Version, err)
	}
	back, _, err := x.fetch(created.ID)
	if err != nil {
		return 1, err
	}
	if cs := wfdef.Diff(local, back); len(cs) > 0 {
		x.printf("%s", wfdef.Render(cs, back))
		return 1, fmt.Errorf("草稿 v%d 回读与本地文件不一致，请检查后再推", created.Version)
	}
	base := fmt.Sprintf("%s_v%d", local.Meta.Key, created.Version)
	out := filepath.Join(filepath.Dir(o.pos[0]), base+".yaml")
	if err := wfdef.Save(back, out, base); err != nil {
		return 1, err
	}
	x.printFindings(fs)
	x.printf("已建草稿 %s@v%d（未激活），回读一致，已导出 %s。\n下一步：wfctl validate %s@v%d；由用户确认后 wfctl activate %s@v%d --reason \"…\"。\n",
		local.Meta.Key, created.Version, out, local.Meta.Key, created.Version, local.Meta.Key, created.Version)
	return 0, nil
}

type checks struct {
	AllOK  bool `json:"all_ok"`
	Checks []struct {
		Name   string `json:"name"`
		Detail string `json:"detail"`
		OK     bool   `json:"ok"`
	} `json:"checks"`
}

func (x *ctl) printChecks(c checks) {
	for _, ch := range c.Checks {
		mark := "✅"
		if !ch.OK {
			mark = "❌"
		}
		d := ""
		if ch.Detail != "" {
			d = "（" + ch.Detail + "）"
		}
		x.printf("%s %s%s\n", mark, ch.Name, d)
	}
}

func (x *ctl) validate(o opts) (int, error) {
	if len(o.pos) != 1 || !strings.Contains(o.pos[0], "@") {
		return 2, errors.New("用法：wfctl validate <key>@<ver>")
	}
	row, err := x.resolve(o.pos[0])
	if err != nil {
		return 1, err
	}
	var c checks
	if err := x.c.do(x.ctx, http.MethodPost, fmt.Sprintf("/admin/workflows/%d/validate", row.ID), nil, &c); err != nil {
		return 1, err
	}
	x.printChecks(c)
	if !c.AllOK {
		return 1, nil
	}
	x.printf("%s@v%d 校验全部通过。\n", row.Key, row.Version)
	return 0, nil
}

func (x *ctl) activate(o opts) (int, error) {
	if len(o.pos) != 1 || !strings.Contains(o.pos[0], "@") {
		return 2, errors.New("用法：wfctl activate <key>@<ver> --reason \"…\"")
	}
	reason := strings.TrimSpace(o.flags["--reason"])
	if reason == "" {
		return 2, errors.New("--reason 必填：写明激活依据（用户当轮原话）")
	}
	row, err := x.resolve(o.pos[0])
	if err != nil {
		return 1, err
	}
	if row.Status != "draft" {
		return 1, fmt.Errorf("%s@v%d 是 %s，只有草稿可以激活", row.Key, row.Version, row.Status)
	}
	next, _, err := x.fetch(row.ID)
	if err != nil {
		return 1, err
	}
	if cur, err := x.resolve(row.Key); err == nil {
		base, _, err := x.fetch(cur.ID)
		if err != nil {
			return 1, err
		}
		x.printf("将用 v%d 替换当前 active v%d：\n\n%s\n", row.Version, cur.Version, wfdef.Render(wfdef.Diff(base, next), next))
	}
	want := fmt.Sprintf("%s@v%d", row.Key, row.Version)
	switch {
	case o.flags["--yes"] == "true" && x.env.AllowY:
	case !x.env.IsTTY:
		return 2, errors.New("激活需要人在终端里确认：请由用户本人执行 wfctl activate，并按提示手输版本号")
	default:
		x.printf("确认激活请手输 %s：", want)
		line, _ := bufio.NewReader(x.env.In).ReadString('\n')
		if strings.TrimSpace(line) != want && strings.TrimSpace(line) != strings.Replace(want, "@v", "@", 1) {
			return 2, errors.New("输入不一致，未激活")
		}
	}
	var c checks
	err = x.c.do(x.ctx, http.MethodPost, fmt.Sprintf("/admin/workflows/%d/activate", row.ID), map[string]string{"reason": reason}, &c)
	var ae *apiError
	if errors.As(err, &ae) && ae.Code == "validation_failed" {
		_ = json.Unmarshal(ae.Raw, &c)
		x.printChecks(c)
		return 1, errors.New("校验未通过，未激活")
	}
	if err != nil {
		return 1, err
	}
	x.printf("已激活 %s（旧版自动归档；已在跑的 run 不受影响）。\n", want)
	return 0, nil
}

func (x *ctl) history(o opts) (int, error) {
	if len(o.pos) != 1 {
		return 2, errors.New("用法：wfctl history <key>")
	}
	key := strings.Split(o.pos[0], "@")[0]
	vs, err := x.versions(key)
	if err != nil {
		return 1, err
	}
	x.printf("版本：\n")
	for _, v := range vs {
		x.printf("  v%d  %s  %s\n", v.Version, v.Status, v.Name)
	}
	var au struct {
		Audit []struct {
			Username  string `json:"username"`
			Action    string `json:"action"`
			Target    string `json:"target"`
			Detail    string `json:"detail"`
			CreatedAt string `json:"created_at"`
		} `json:"audit"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/audit?target_prefix="+url.QueryEscape(key+"@"), nil, &au); err != nil {
		return 1, err
	}
	x.printf("\n变更记录（新 → 旧）：\n")
	for _, a := range au.Audit {
		var d struct {
			Reason        string `json:"reason"`
			ChangeSummary string `json:"change_summary"`
			FromVersion   int    `json:"from_version"`
		}
		_ = json.Unmarshal([]byte(a.Detail), &d)
		line := fmt.Sprintf("  %s  %-18s %-16s %s", a.CreatedAt, a.Action, a.Target, a.Username)
		if d.FromVersion > 0 {
			line += fmt.Sprintf("  自 v%d", d.FromVersion)
		}
		if d.Reason != "" {
			line += "  原因：" + d.Reason
		}
		x.printf("%s\n", line)
		if d.ChangeSummary != "" {
			x.printf("      摘要：%s\n", d.ChangeSummary)
		}
	}
	return 0, nil
}

func (x *ctl) rollback(o opts) (int, error) {
	if len(o.pos) != 1 || !strings.Contains(o.pos[0], "@") {
		return 2, errors.New("用法：wfctl rollback <key>@<ver> --reason \"…\"")
	}
	reason := strings.TrimSpace(o.flags["--reason"])
	if reason == "" {
		return 2, errors.New("--reason 必填")
	}
	row, err := x.resolve(o.pos[0])
	if err != nil {
		return 1, err
	}
	var r struct {
		ID      int64 `json:"id"`
		Version int   `json:"version"`
	}
	if err := x.c.do(x.ctx, http.MethodPost, fmt.Sprintf("/admin/workflows/%d/clone", row.ID), map[string]string{"reason": "回滚到 v" + strconv.Itoa(row.Version) + "：" + reason}, &r); err != nil {
		return 1, err
	}
	x.printf("已把 %s@v%d 复制为草稿 v%d（前滚回退，未激活）。\n确认后执行：wfctl activate %s@v%d --reason \"…\"\n", row.Key, row.Version, r.Version, row.Key, r.Version)
	return 0, nil
}

func (x *ctl) roles() (int, error) {
	m, err := x.roleMap()
	if err != nil {
		return 1, err
	}
	var codes []string
	for c := range m {
		codes = append(codes, c)
	}
	sort.Strings(codes)
	for _, c := range codes {
		r := m[c]
		tags := []string{}
		if r.IsManagement {
			tags = append(tags, "管理类")
		}
		if r.IsHuman {
			tags = append(tags, "人类")
		}
		x.printf("%-12s %s %s\n", c, r.Name, strings.Join(tags, "·"))
	}
	return 0, nil
}

func (x *ctl) agents() (int, error) {
	var r struct {
		Agents []struct {
			Name     string `json:"name"`
			RoleCode string `json:"role_code"`
			Tokens   []struct {
				Status string `json:"status"`
			} `json:"tokens"`
			ID     int64 `json:"id"`
			Active bool  `json:"active"`
		} `json:"agents"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/agents", nil, &r); err != nil {
		return 1, err
	}
	for _, a := range r.Agents {
		n := 0
		for _, t := range a.Tokens {
			if t.Status == "active" {
				n++
			}
		}
		st := "启用"
		if !a.Active {
			st = "停用"
		}
		x.printf("#%-3d %-12s %s  %s  有效 token %d 个\n", a.ID, a.RoleCode, a.Name, st, n)
	}
	return 0, nil
}

func (x *ctl) roster() (int, error) {
	var chats struct {
		Chats []struct {
			ChatKey string `json:"chat_key"`
			Name    string `json:"name"`
			ID      int64  `json:"id"`
		} `json:"chats"`
	}
	if err := x.c.do(x.ctx, http.MethodGet, "/admin/chats", nil, &chats); err != nil {
		return 1, err
	}
	for _, ch := range chats.Chats {
		x.printf("群：%s（%s）\n", ch.Name, ch.ChatKey)
		var ms struct {
			Members []map[string]any `json:"members"`
		}
		if err := x.c.do(x.ctx, http.MethodGet, fmt.Sprintf("/admin/chats/%d/members", ch.ID), nil, &ms); err != nil {
			return 1, err
		}
		for _, m := range ms.Members {
			x.printf("  %-8v %-12v %v  %v\n", m["kind"], m["role_code"], m["display_name"], m["open_id"])
		}
	}
	return 0, nil
}

var _ = time.Second
