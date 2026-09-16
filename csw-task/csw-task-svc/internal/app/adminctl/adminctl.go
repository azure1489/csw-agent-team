// Package adminctl 提供本轮最小运维 CLI（替代第二轮 cmd/adminsrv）：
// migrate / agent / token / workflow 子命令。
package adminctl

import (
	"context"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/workflow"
)

const usage = `用法：
  adminctl migrate up                              应用全部迁移
  adminctl agent list                              列 agent + 有效 token 数
  adminctl token issue <agentID|roleCode> [--label X] [--expires RFC3339]
                                                   签发 token（明文仅打印一次）
  adminctl token revoke <tokenID>                  吊销 token
  adminctl run-report <runID>                      一期用时报告（作业 / 等主编 / 等 Van / 返工轮次）
  adminctl intake-check <runID>                    采集与判断是否合理（覆盖率 / 产出落差 / 窗口 / 淘汰交代 / 来源分布）
  adminctl source list                             来源台账（本期「应该扫哪些」的基准）
  adminctl source add <platform> <key> [--name N] [--entry URL] [--required] [--ref 依据]
                                                   登记一条来源（人工确认才进台账）
  adminctl source disable <platform> <key>         停用一条来源（不再计入覆盖率分母）
  adminctl source backfill <runID> [--apply]       从实际采集轮找出不在台账的来源；--apply 以停用态写入待转正
  adminctl workflow list                           列已激活工作流
  adminctl workflow validate <wf_key>              校验工作流定义（DAG/入口/中枢/闸）
  adminctl user create <username> --role <r> [--name N] [--password P]
                                                   建后台用户（argon2id；缺 --password 则生成并打印）
  adminctl user passwd <username> --password <P>   改后台用户密码
  adminctl user list                               列后台用户`

// Run 执行 adminctl，返回进程退出码。
func Run(args []string) int {
	ctx := context.Background()
	cfg := config.Load()

	db, err := sqlite.Open(cfg.DBPath)
	if err != nil {
		fmt.Println("打开数据库失败：", err)
		return 1
	}
	defer db.Close()
	if err := sqlite.Migrate(db); err != nil {
		fmt.Println("迁移失败：", err)
		return 1
	}
	store := sqlite.New(db)

	if len(args) == 0 {
		fmt.Println(usage)
		return 2
	}

	switch args[0] {
	case "migrate":
		fmt.Println("迁移完成（已 up 到最新）。")
		return 0
	case "agent":
		return agentCmd(ctx, store, args[1:])
	case "token":
		return tokenCmd(ctx, store, args[1:])
	case "run-report":
		return runReport(ctx, store, args[1:])
	case "intake-check":
		return intakeCheck(ctx, store, args[1:])
	case "source":
		return sourceCmd(ctx, store, args[1:])
	case "workflow":
		return workflowCmd(ctx, store, args[1:])
	case "user":
		return userCmd(ctx, store, args[1:])
	default:
		fmt.Println(usage)
		return 2
	}
}

func userCmd(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) == 0 {
		fmt.Println(usage)
		return 2
	}
	switch args[0] {
	case "list":
		users, err := store.Q().ListUsers(ctx)
		if err != nil {
			fmt.Println("列用户失败：", err)
			return 1
		}
		fmt.Printf("%-4s %-14s %-12s %-12s %-8s %s\n", "ID", "用户名", "名称", "角色", "状态", "最近登录")
		for _, u := range users {
			fmt.Printf("%-4d %-14s %-12s %-12s %-8s %s\n", u.ID, u.Username, u.DisplayName, u.Role, u.Status, u.LastLoginAt)
		}
		return 0
	case "create":
		if len(args) < 2 {
			fmt.Println(usage)
			return 2
		}
		username := args[1]
		f := flags(args[2:])
		role := f["role"]
		if role == "" {
			role = "superadmin"
		}
		pw := f["password"]
		generated := false
		if pw == "" {
			t, err := auth.NewToken()
			if err != nil {
				fmt.Println("生成密码失败：", err)
				return 1
			}
			pw, generated = t, true
		}
		hash, err := auth.HashPassword(pw)
		if err != nil {
			fmt.Println("哈希失败：", err)
			return 1
		}
		id, err := store.Q().CreateUser(ctx, username, hash, f["name"], role)
		if err != nil {
			fmt.Println("建用户失败（用户名可能已存在）：", err)
			return 1
		}
		fmt.Printf("已建后台用户 #%d %s（角色 %s）\n", id, username, role)
		if generated {
			fmt.Println("⚠ 自动生成密码（仅此一次显示）：")
			fmt.Println("  " + pw)
		}
		return 0
	case "passwd":
		if len(args) < 2 {
			fmt.Println(usage)
			return 2
		}
		f := flags(args[2:])
		if f["password"] == "" {
			fmt.Println("需 --password")
			return 2
		}
		u, _, err := store.Q().GetUserByUsername(ctx, args[1])
		if err != nil {
			fmt.Println("无此用户：", args[1])
			return 1
		}
		hash, err := auth.HashPassword(f["password"])
		if err != nil {
			fmt.Println("哈希失败：", err)
			return 1
		}
		if err := store.Q().UpdatePassword(ctx, u.ID, hash); err != nil {
			fmt.Println("改密失败：", err)
			return 1
		}
		fmt.Printf("已改 %s 的密码\n", args[1])
		return 0
	default:
		fmt.Println(usage)
		return 2
	}
}

func agentCmd(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) == 0 || args[0] != "list" {
		fmt.Println(usage)
		return 2
	}
	agents, err := store.Q().ListAgents(ctx)
	if err != nil {
		fmt.Println("列 agent 失败：", err)
		return 1
	}
	fmt.Printf("%-4s %-12s %-14s %-6s %s\n", "ID", "角色", "名称", "启用", "有效token")
	for _, a := range agents {
		cnt, _ := store.Q().ActiveTokenCount(ctx, a.ID)
		fmt.Printf("%-4d %-12s %-14s %-6v %d\n", a.ID, a.RoleCode, a.Name, a.Active, cnt)
	}
	return 0
}

func tokenCmd(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) < 2 {
		fmt.Println(usage)
		return 2
	}
	switch args[0] {
	case "issue":
		target := args[1]
		f := flags(args[2:])
		label, expires := f["label"], f["expires"]
		// 解析 agent：数字=id，否则按角色取活跃 agent。
		var agentID int64
		if n, err := strconv.ParseInt(target, 10, 64); err == nil {
			agentID = n
		} else {
			a, err := store.Q().ActiveAgentByRole(ctx, target)
			if err != nil {
				fmt.Printf("找不到角色 %s 的活跃 agent\n", target)
				return 1
			}
			agentID = a.ID
		}
		plain, err := auth.NewToken()
		if err != nil {
			fmt.Println("生成 token 失败：", err)
			return 1
		}
		var expPtr *string
		if expires != "" {
			expPtr = &expires
		}
		tid, err := store.Q().InsertToken(ctx, agentID, auth.HashToken(plain), label, expPtr)
		if err != nil {
			fmt.Println("签发失败：", err)
			return 1
		}
		fmt.Printf("已为 agent #%d 签发 token #%d（label=%q）\n", agentID, tid, label)
		fmt.Println("⚠ 明文仅此一次显示，请妥善保存：")
		fmt.Println("  " + plain)
		return 0
	case "revoke":
		tid, err := strconv.ParseInt(args[1], 10, 64)
		if err != nil {
			fmt.Println("非法 token id")
			return 2
		}
		if err := store.Q().RevokeToken(ctx, tid); err != nil {
			fmt.Println("吊销失败：", err)
			return 1
		}
		fmt.Printf("已吊销 token #%d\n", tid)
		return 0
	default:
		fmt.Println(usage)
		return 2
	}
}

func workflowCmd(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) == 0 {
		fmt.Println(usage)
		return 2
	}
	switch args[0] {
	case "list":
		wfs, err := store.Q().ListActiveWorkflows(ctx)
		if err != nil {
			fmt.Println("列工作流失败：", err)
			return 1
		}
		fmt.Printf("%-14s %-12s %-4s %-8s %s\n", "key", "名称", "版本", "中枢", "派工")
		for _, w := range wfs {
			fmt.Printf("%-14s %-12s v%-3d %-8s %s\n", w.WfKey, w.Name, w.Version, w.HubRoleCode, w.DispatchMode)
		}
		return 0
	case "validate":
		if len(args) < 2 {
			fmt.Println(usage)
			return 2
		}
		wf, err := store.Q().ActiveWorkflowByKey(ctx, args[1])
		if err != nil {
			fmt.Println("无此 active 工作流：", args[1])
			return 1
		}
		rep, err := workflow.Validate(ctx, store.Q(), wf)
		if err != nil {
			fmt.Println("校验失败：", err)
			return 1
		}
		for _, c := range rep.Checks {
			mark := "✅"
			if !c.OK {
				mark = "❌"
			}
			line := fmt.Sprintf("%s %s", mark, c.Name)
			if c.Detail != "" {
				line += "（" + c.Detail + "）"
			}
			fmt.Println(line)
		}
		if rep.AllOK() {
			fmt.Println("全部通过，可激活。")
			return 0
		}
		fmt.Println("存在未通过项。")
		return 1
	default:
		fmt.Println(usage)
		return 2
	}
}

// flags 极简解析 --k v / --k=v 形式的标志为 map。
func flags(args []string) map[string]string {
	m := map[string]string{}
	for i := 0; i < len(args); i++ {
		a := args[i]
		if !strings.HasPrefix(a, "--") {
			continue
		}
		a = strings.TrimPrefix(a, "--")
		if eq := strings.IndexByte(a, '='); eq >= 0 {
			m[a[:eq]] = a[eq+1:]
		} else if i+1 < len(args) && !strings.HasPrefix(args[i+1], "--") {
			m[a] = args[i+1]
			i++
		} else {
			m[a] = ""
		}
	}
	return m
}

// runReport 打印一期的用时拆分：实际作业、等主编、等 Van、返工轮次与失败次数。
func runReport(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) < 1 {
		fmt.Println("用法：adminctl run-report <runID>")
		return 2
	}
	id, err := strconv.ParseInt(args[0], 10, 64)
	if err != nil {
		fmt.Println("run id 须为数字")
		return 2
	}
	rep, err := engine.BuildRunReport(ctx, store.Q(), id)
	if err != nil {
		fmt.Println("取报告失败：", err)
		return 1
	}
	fmt.Printf("r%d（%s，%s）：作业 %d 分钟｜等主编 %d 分钟｜等 Van %d 分钟｜返工 %d 轮｜报告失败 %d 次\n",
		rep.RunID, rep.Subject, rep.Status, rep.WorkMin, rep.WaitHubMin, rep.WaitVanMin, rep.Reworks, rep.Failures)
	fmt.Printf("%-14s %-12s %-10s %6s %8s %10s %8s\n", "阶段", "角色", "状态", "版本", "作业", "等主编", "等Van")
	for _, st := range rep.Stages {
		fmt.Printf("%-14s %-12s %-10s %6d %8d %10d %8d\n", st.Stage, st.Role, st.Status, st.Versions, st.WorkMin, st.WaitHubMin, st.WaitVanMin)
	}
	return 0
}

// intakeCheck 对一期的采集与判断跑八条判据；输出与退出码照 workflow validate。
// 没有上报记录的旧 run 输出「不判定」而不是失败——这一点保证新命令对历史数据安全。
func intakeCheck(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) < 1 {
		fmt.Println("用法：adminctl intake-check <runID>")
		return 2
	}
	id, err := strconv.ParseInt(args[0], 10, 64)
	if err != nil {
		fmt.Println("run id 须为数字")
		return 2
	}
	checks, err := engine.BuildIntakeCheck(ctx, store.Q(), id)
	if err != nil {
		fmt.Println("取结论失败：", err)
		return 1
	}
	bad := 0
	for _, c := range checks {
		mark := "✅"
		switch {
		case c.Skipped:
			mark = "➖"
		case !c.OK:
			mark = "❌"
			bad++
		case c.Warn:
			mark = "⚠️"
		}
		line := fmt.Sprintf("%s %s", mark, c.Name)
		if c.Detail != "" {
			line += "（" + c.Detail + "）"
		}
		fmt.Println(line)
	}
	if bad == 0 {
		fmt.Println("采集过程可核，没有未通过项。")
		return 0
	}
	fmt.Printf("有 %d 项未通过。\n", bad)
	return 1
}

// sourceCmd 维护来源台账。台账只由人工维护：若「扫过的源」自动进表，
// 覆盖率的分母就等于分子，永远 100%，这条判据也就废了。
func sourceCmd(ctx context.Context, store *sqlite.Store, args []string) int {
	if len(args) == 0 {
		fmt.Println(usage)
		return 2
	}
	switch args[0] {
	case "list":
		sources, err := store.Q().ListIntakeSources(ctx, false)
		if err != nil {
			fmt.Println("列来源失败：", err)
			return 1
		}
		fmt.Printf("%-10s %-22s %-28s %-6s %-6s %s\n", "平台", "来源键", "名称", "启用", "必扫", "最近成功")
		for _, s := range sources {
			fmt.Printf("%-10s %-22s %-28s %-6v %-6v %s\n", s.Platform, s.SourceKey, s.Name, s.Enabled, s.Required, s.LastOKAt)
		}
		return 0
	case "add":
		if len(args) < 3 {
			fmt.Println("用法：adminctl source add <platform> <key> [--name N] [--entry URL] [--required] [--ref 依据]")
			return 2
		}
		platform, key := args[1], args[2]
		if !domain.ValidSourcePlatform(platform) {
			fmt.Println("platform 须为 instagram / xhs / web / other")
			return 2
		}
		f := flags(args[3:])
		_, required := f["required"]
		src := domain.IntakeSource{Platform: platform, SourceKey: key, Name: f["name"], EntryURL: f["entry"],
			Enabled: true, Required: required, AddedBy: "adminctl", SourceRef: f["ref"], Note: f["note"]}
		if err := store.Q().UpsertIntakeSource(ctx, src); err != nil {
			fmt.Println("登记失败：", err)
			return 1
		}
		fmt.Printf("已登记来源 %s/%s（必扫 %v）\n", platform, key, required)
		return 0
	case "disable":
		if len(args) < 3 {
			fmt.Println("用法：adminctl source disable <platform> <key>")
			return 2
		}
		n, err := store.Q().SetIntakeSourceEnabled(ctx, args[1], args[2], false)
		if err != nil {
			fmt.Println("停用失败：", err)
			return 1
		}
		if n == 0 {
			fmt.Println("台账里没有这条来源")
			return 1
		}
		fmt.Printf("已停用 %s/%s\n", args[1], args[2])
		return 0
	case "backfill":
		if len(args) < 2 {
			fmt.Println("用法：adminctl source backfill <runID> [--apply]")
			return 2
		}
		id, err := strconv.ParseInt(args[1], 10, 64)
		if err != nil {
			fmt.Println("run id 须为数字")
			return 2
		}
		f := flags(args[2:])
		_, apply := f["apply"]
		sweeps, err := store.Q().ListSweepsByRun(ctx, id)
		if err != nil {
			fmt.Println("读采集轮失败：", err)
			return 1
		}
		known, err := store.Q().ListIntakeSources(ctx, false)
		if err != nil {
			fmt.Println("读台账失败：", err)
			return 1
		}
		have := map[string]bool{}
		for _, s := range known {
			have[s.Platform+"/"+s.SourceKey] = true
		}
		found := map[string]string{}
		for _, sw := range sweeps {
			if sw.SourceKey == "" || sw.SourceKey == "channel" {
				continue
			}
			k := sw.Platform + "/" + sw.SourceKey
			if !have[k] {
				found[k] = sw.Platform
			}
		}
		if len(found) == 0 {
			fmt.Printf("r%d 的采集轮里没有台账之外的来源。\n", id)
			return 0
		}
		keys := make([]string, 0, len(found))
		for k := range found {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			if !apply {
				fmt.Println("待确认：", k)
				continue
			}
			parts := strings.SplitN(k, "/", 2)
			src := domain.IntakeSource{Platform: parts[0], SourceKey: parts[1], Name: parts[1],
				Enabled: false, Required: false, AddedBy: "adminctl backfill",
				SourceRef: fmt.Sprintf("r%d 采集轮", id), Note: "自动回填，待人工确认后启用"}
			if err := store.Q().UpsertIntakeSource(ctx, src); err != nil {
				fmt.Println("写入失败：", k, err)
				return 1
			}
			fmt.Println("已写入（停用态，待转正）：", k)
		}
		if !apply {
			fmt.Printf("共 %d 条；加 --apply 才写入台账（写入后仍是停用态，需人工确认启用）。\n", len(found))
		}
		return 0
	default:
		fmt.Println(usage)
		return 2
	}
}
