// Package workflow 提供工作流定义的校验（激活前必过）。
package workflow

import (
	"context"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// Check 一项校验结果。
type Check struct {
	Name   string
	Detail string
	OK     bool
}

// Report 校验报告。
type Report struct {
	Checks []Check
}

// AllOK 是否全部通过。
func (r Report) AllOK() bool {
	for _, c := range r.Checks {
		if !c.OK {
			return false
		}
	}
	return true
}

// Validate 校验某工作流定义（按 workflow 入参，draft/active 皆可）：
// DAG 无环、入口可达所有、终点可达、中枢为管理类、闸角色存在、每阶段三段非空、至少一阶段。
func Validate(ctx context.Context, q *sqlite.Queries, wf domain.Workflow) (Report, error) {
	var rep Report
	stages, err := q.ListStages(ctx, wf.ID)
	if err != nil {
		return rep, err
	}
	deps, err := q.ListStageDeps(ctx, wf.ID)
	if err != nil {
		return rep, err
	}
	gates, err := q.ListGates(ctx, wf.ID)
	if err != nil {
		return rep, err
	}

	// 邻接：dependsOn → 其下游（dependents）；以及 stage → 其依赖。
	dependents := map[int64][]int64{}
	dependsOn := map[int64][]int64{}
	for _, d := range deps {
		dependents[d.DependsOnID] = append(dependents[d.DependsOnID], d.StageID)
		dependsOn[d.StageID] = append(dependsOn[d.StageID], d.DependsOnID)
	}

	// 1. DAG 无环（对 dependents 图做 DFS）。
	acyclic := isAcyclic(stages, dependents)
	rep.Checks = append(rep.Checks, Check{Name: "DAG 无环", OK: acyclic})

	// 2. 入口（无依赖）可达所有阶段。
	var entries []int64
	for _, st := range stages {
		if len(dependsOn[st.ID]) == 0 {
			entries = append(entries, st.ID)
		}
	}
	reached := bfs(entries, dependents)
	reachAll := len(reached) == len(stages)
	rep.Checks = append(rep.Checks, Check{Name: "入口可达所有阶段", OK: len(entries) > 0 && reachAll,
		Detail: detailEntries(entries, stages)})

	// 3. 终点（无下游）可达。
	terminalOK := true
	for _, st := range stages {
		if len(dependents[st.ID]) == 0 && !reached[st.ID] {
			terminalOK = false
		}
	}
	rep.Checks = append(rep.Checks, Check{Name: "终点可达", OK: terminalOK})

	// 4. 中枢角色为管理类。
	hubOK := false
	if r, err := q.GetRole(ctx, wf.HubRoleCode); err == nil && r.IsManagement {
		hubOK = true
	}
	rep.Checks = append(rep.Checks, Check{Name: "中枢角色为管理类", OK: hubOK, Detail: wf.HubRoleCode})

	// 5. 各闸 reviewer_role 存在。
	gatesOK := true
	missing := ""
	for _, g := range gates {
		if _, err := q.GetRole(ctx, g.ReviewerRole); err != nil {
			gatesOK = false
			missing = g.ReviewerRole
		}
	}
	rep.Checks = append(rep.Checks, Check{Name: "各闸审核角色存在", OK: gatesOK, Detail: missing})

	// 6. 每阶段 instructions/self_check_criteria/acceptance 非空（§6）。
	contentOK := true
	var emptyStages []string
	for _, st := range stages {
		if strings.TrimSpace(st.Instructions) == "" ||
			strings.TrimSpace(st.SelfCheckCriteria) == "" ||
			strings.TrimSpace(st.Acceptance) == "" {
			contentOK = false
			emptyStages = append(emptyStages, st.Name)
		}
	}
	rep.Checks = append(rep.Checks, Check{
		Name:   "每阶段作业手册/自检/验收非空",
		OK:     contentOK,
		Detail: strings.Join(emptyStages, ", "),
	})

	// 至少一个阶段。
	rep.Checks = append(rep.Checks, Check{Name: "至少一个阶段", OK: len(stages) > 0})

	return rep, nil
}

func isAcyclic(stages []domain.Stage, adj map[int64][]int64) bool {
	const (
		white = 0
		gray  = 1
		black = 2
	)
	color := map[int64]int{}
	var dfs func(n int64) bool
	dfs = func(n int64) bool {
		color[n] = gray
		for _, m := range adj[n] {
			switch color[m] {
			case gray:
				return false // 回边 → 有环
			case white:
				if !dfs(m) {
					return false
				}
			}
		}
		color[n] = black
		return true
	}
	for _, st := range stages {
		if color[st.ID] == white {
			if !dfs(st.ID) {
				return false
			}
		}
	}
	return true
}

func bfs(starts []int64, adj map[int64][]int64) map[int64]bool {
	seen := map[int64]bool{}
	queue := append([]int64{}, starts...)
	for _, s := range starts {
		seen[s] = true
	}
	for len(queue) > 0 {
		n := queue[0]
		queue = queue[1:]
		for _, m := range adj[n] {
			if !seen[m] {
				seen[m] = true
				queue = append(queue, m)
			}
		}
	}
	return seen
}

func detailEntries(entries []int64, stages []domain.Stage) string {
	name := map[int64]string{}
	for _, st := range stages {
		name[st.ID] = st.Name
	}
	out := ""
	for i, e := range entries {
		if i > 0 {
			out += ", "
		}
		out += name[e]
	}
	return out
}
