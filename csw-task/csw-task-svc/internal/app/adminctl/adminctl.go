// Package adminctl 提供本轮最小运维 CLI（替代第二轮 cmd/adminsrv）：
// migrate / agent / token / workflow 子命令。
package adminctl

import (
	"context"
	"fmt"
	"strconv"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/workflow"
)

const usage = `用法：
  adminctl migrate up                              应用全部迁移
  adminctl agent list                              列 agent + 有效 token 数
  adminctl token issue <agentID|roleCode> [--label X] [--expires RFC3339]
                                                   签发 token（明文仅打印一次）
  adminctl token revoke <tokenID>                  吊销 token
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
