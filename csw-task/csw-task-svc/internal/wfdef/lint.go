package wfdef

import (
	"fmt"
	"regexp"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/workflow"
)

// Finding 一条检查结果。Level 为 error（阻断 push）或 warn（提示）。
type Finding struct {
	Level string
	Rule  string
	Stage string
	Msg   string
}

var h1 = regexp.MustCompile(`(?m)^# `)

// PureInput 定义文件 → 校验器输入（阶段 ID 用序号）。
func (d *Def) PureInput(roles map[string]domain.Role) workflow.PureInput {
	in := workflow.PureInput{HubRole: d.Meta.HubRole, RoleLookup: func(code string) (domain.Role, bool) {
		r, ok := roles[code]
		return r, ok
	}}
	id := map[string]int64{}
	for i, s := range d.Stages {
		id[s.Code] = int64(i + 1)
	}
	for i, s := range d.Stages {
		in.Stages = append(in.Stages, domain.Stage{
			ID: int64(i + 1), Seq: i + 1, Code: s.Code, Name: s.Name, RoleCode: s.Role, OutputType: s.OutputType,
			Instructions: s.Instructions.Value, SelfCheckCriteria: s.SelfCheck.Value, Acceptance: s.Acceptance.Value,
			IsMerge: s.IsMerge, DispatchMode: s.DispatchMode, ActionClass: s.ActionClass, SLAMinutes: s.SLAMinutes, PerItem: s.PerItem,
			AckMinutes: s.AckMinutes, IdleMinutes: s.IdleMinutes,
		})
		for _, dep := range s.Deps {
			if did, ok := id[dep]; ok {
				in.Deps = append(in.Deps, domain.StageDep{StageID: int64(i + 1), DependsOnID: did})
			}
		}
		for gi, g := range effGates(s, d.DefaultGates) {
			sid := int64(i + 1)
			in.Gates = append(in.Gates, domain.Gate{StageID: &sid, GateOrder: gi + 1, ReviewerRole: g.Reviewer, RelayedByHub: g.Relayed, Name: g.Name})
		}
	}
	return in
}

// LintLocal 不依赖服务端的检查：服务端十项校验 + 文件层规则 L1 / L2 / L6 / L7。
func LintLocal(d *Def, roles map[string]domain.Role) []Finding {
	var fs []Finding
	errf := func(rule, stage, format string, a ...any) {
		fs = append(fs, Finding{Level: "error", Rule: rule, Stage: stage, Msg: fmt.Sprintf(format, a...)})
	}
	warnf := func(rule, stage, format string, a ...any) {
		fs = append(fs, Finding{Level: "warn", Rule: rule, Stage: stage, Msg: fmt.Sprintf(format, a...)})
	}
	if d.Meta.Key == "" || d.Meta.Name == "" || d.Meta.HubRole == "" {
		errf("meta", "", "meta.wf_key / name / hub_role 必填")
	}
	codes := map[string]bool{}
	for _, s := range d.Stages {
		if codes[s.Code] {
			errf("L7", s.Code, "阶段 code 重复")
		}
		codes[s.Code] = true
	}
	for _, s := range d.Stages {
		// L1：无默认闸时每个阶段必须显式写 gates:（0 闸写 gates: []），避免漏写被当成 0 闸。
		if len(d.DefaultGates) == 0 && s.Gates == nil {
			errf("L1", s.Code, "未写 gates:（没有默认闸，0 闸请显式写 gates: []）")
		}
		// L1b：有默认闸时引擎表达不了「覆盖为空」。
		if len(d.DefaultGates) > 0 && s.Gates != nil && len(*s.Gates) == 0 {
			errf("L1", s.Code, "有默认闸时无法把单个阶段设为 0 闸（引擎会回退到默认闸）；请去掉默认闸、逐阶段写闸")
		}
		if _, ok := roles[s.Role]; !ok {
			errf("L7", s.Code, "责任角色 %q 不存在", s.Role)
		}
		for _, dep := range s.Deps {
			if !codes[dep] {
				errf("L7", s.Code, "依赖的阶段 %q 不存在", dep)
			}
			if dep == s.Code {
				errf("L7", s.Code, "不能依赖自己")
			}
		}
		// L2：名字像平台写操作却标成 read。
		if orRead(s.ActionClass) == "read" && (strings.Contains(s.Name, "保存") || strings.Contains(s.Name, "发布") ||
			strings.Contains(s.Code, "save") || strings.Contains(s.Code, "publish")) {
			warnf("L2", s.Code, "名称像平台写操作（保存 / 发布），但动作类别是 read；确认不需要授权护栏")
		}
		// L6：疑似整篇粘贴。
		for label, t := range map[string]string{"作业手册": s.Instructions.Value, "自检标准": s.SelfCheck.Value, "验收标准": s.Acceptance.Value} {
			if n := strings.Count(t, "\n") + 1; n > 200 {
				warnf("L6", s.Code, "%s 有 %d 行，疑似整篇粘贴文档；阶段文本只写本阶段要做的事", label, n)
			}
			if h1.MatchString(t) {
				warnf("L6", s.Code, "%s 含一级标题（# ），疑似整篇粘贴文档", label)
			}
			if strings.Contains(t, "§") {
				warnf("L6", s.Code, "%s 引用了文档章节号（§），执行者读不到那份文档", label)
			}
		}
	}
	for _, c := range workflow.ValidatePure(d.PureInput(roles)).Checks {
		if !c.OK {
			errf("V", "", "服务端校验「%s」不通过 %s", c.Name, c.Detail)
		}
	}
	return fs
}

// Worst 汇总退出码：有 error → 1；只有 warn → 3；干净 → 0。
func Worst(fs []Finding) int {
	code := 0
	for _, f := range fs {
		if f.Level == "error" {
			return 1
		}
		code = 3
	}
	return code
}
