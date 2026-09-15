package wfdef

import (
	"fmt"
	"sort"
	"strings"
)

// Change 一处差异。
type Change struct {
	Kind   string // 基本信息 / 阶段 / 依赖 / 闸 / 文本
	Stage  string // 阶段 code；基本信息为空
	Detail string
}

func gateStr(gs []Gate) string {
	if len(gs) == 0 {
		return "0 闸"
	}
	parts := make([]string, 0, len(gs))
	for _, g := range gs {
		s := g.Reviewer
		if g.Name != "" {
			s = g.Name + "(" + g.Reviewer + ")"
		}
		if g.Relayed {
			s += "·代录"
		}
		parts = append(parts, s)
	}
	return strings.Join(parts, " → ")
}

func effGates(s Stage, def []Gate) []Gate {
	if s.Gates == nil {
		return def
	}
	return *s.Gates
}

func norm(s string) string { return strings.TrimSpace(strings.ReplaceAll(s, "\r\n", "\n")) }

func textChange(label, a, b string) string {
	if norm(a) == norm(b) {
		return ""
	}
	return fmt.Sprintf("%s：%d → %d 字", label, len([]rune(norm(a))), len([]rune(norm(b))))
}

func orRead(s string) string {
	if s == "" || s == "read" {
		return "read"
	}
	return s
}

func orInherit(s string) string {
	if s == "" {
		return "继承"
	}
	return s
}

// Diff 比较 base → next，返回全部差异（空=语义一致）。
func Diff(base, next *Def) []Change {
	var cs []Change
	add := func(kind, stage, detail string) {
		if detail != "" {
			cs = append(cs, Change{Kind: kind, Stage: stage, Detail: detail})
		}
	}
	bm, nm := base.Meta, next.Meta
	if bm.Name != nm.Name {
		add("基本信息", "", fmt.Sprintf("名称：%s → %s", bm.Name, nm.Name))
	}
	if bm.HubRole != nm.HubRole {
		add("基本信息", "", fmt.Sprintf("中枢：%s → %s", bm.HubRole, nm.HubRole))
	}
	if bm.DispatchMode != nm.DispatchMode {
		add("基本信息", "", fmt.Sprintf("派工模式：%s → %s", bm.DispatchMode, nm.DispatchMode))
	}
	if strings.Join(bm.TriggerRoles, ",") != strings.Join(nm.TriggerRoles, ",") {
		add("基本信息", "", fmt.Sprintf("可触发角色：%s → %s", strings.Join(bm.TriggerRoles, ","), strings.Join(nm.TriggerRoles, ",")))
	}
	add("文本", "", textChange("通用作业约定", base.Common.Instructions.Value, next.Common.Instructions.Value))
	add("文本", "", textChange("通用合规项", base.Common.Acceptance.Value, next.Common.Acceptance.Value))
	if gateStr(base.DefaultGates) != gateStr(next.DefaultGates) {
		add("闸", "", fmt.Sprintf("默认闸：%s → %s", gateStr(base.DefaultGates), gateStr(next.DefaultGates)))
	}

	bs, ns := map[string]Stage{}, map[string]Stage{}
	var border, norder []string
	for _, s := range base.Stages {
		bs[s.Code] = s
		border = append(border, s.Code)
	}
	for _, s := range next.Stages {
		ns[s.Code] = s
		norder = append(norder, s.Code)
	}
	for _, c := range border {
		if _, ok := ns[c]; !ok {
			add("阶段", c, "删除阶段 "+bs[c].Name)
		}
	}
	for _, c := range norder {
		n := ns[c]
		b, ok := bs[c]
		if !ok {
			add("阶段", c, fmt.Sprintf("新增阶段 %s（%s，依赖 %s，%s）", n.Name, n.Role, strings.Join(n.Deps, "、"), gateStr(effGates(n, next.DefaultGates))))
			continue
		}
		var fields []string
		cmp := func(label, x, y string) {
			if x != y {
				fields = append(fields, fmt.Sprintf("%s：%s → %s", label, x, y))
			}
		}
		cmp("名称", b.Name, n.Name)
		cmp("角色", b.Role, n.Role)
		cmp("产出类型", b.OutputType, n.OutputType)
		cmp("合流", fmt.Sprint(b.IsMerge), fmt.Sprint(n.IsMerge))
		cmp("派工模式", orInherit(b.DispatchMode), orInherit(n.DispatchMode))
		cmp("动作类别", orRead(b.ActionClass), orRead(n.ActionClass))
		cmp("时限", fmt.Sprint(b.SLAMinutes), fmt.Sprint(n.SLAMinutes))
		cmp("逐条", fmt.Sprint(b.PerItem), fmt.Sprint(n.PerItem))
		for _, f := range fields {
			add("阶段", c, f)
		}
		bd, nd := map[string]bool{}, map[string]bool{}
		for _, d := range b.Deps {
			bd[d] = true
		}
		for _, d := range n.Deps {
			nd[d] = true
		}
		var plus, minus []string
		for d := range nd {
			if !bd[d] {
				plus = append(plus, "+"+d)
			}
		}
		for d := range bd {
			if !nd[d] {
				minus = append(minus, "−"+d)
			}
		}
		sort.Strings(plus)
		sort.Strings(minus)
		add("依赖", c, strings.Join(append(plus, minus...), " "))
		if x, y := gateStr(effGates(b, base.DefaultGates)), gateStr(effGates(n, next.DefaultGates)); x != y {
			add("闸", c, x+" → "+y)
		}
		add("文本", c, textChange("作业手册", b.Instructions.Value, n.Instructions.Value))
		add("文本", c, textChange("自检标准", b.SelfCheck.Value, n.SelfCheck.Value))
		add("文本", c, textChange("验收标准", b.Acceptance.Value, n.Acceptance.Value))
	}
	if len(cs) == 0 && strings.Join(border, ",") != strings.Join(norder, ",") {
		add("阶段", "", "阶段顺序："+strings.Join(border, ",")+" → "+strings.Join(norder, ","))
	} else if strings.Join(border, ",") != strings.Join(norder, ",") {
		add("阶段", "", "阶段顺序有变化")
	}
	return cs
}

// Downstream 从一组阶段出发沿依赖向下能到的全部阶段（影响范围提示用）。
func Downstream(d *Def, from []string) []string {
	dependents := map[string][]string{}
	for _, s := range d.Stages {
		for _, dep := range s.Deps {
			dependents[dep] = append(dependents[dep], s.Code)
		}
	}
	seen := map[string]bool{}
	queue := append([]string{}, from...)
	for len(queue) > 0 {
		c := queue[0]
		queue = queue[1:]
		for _, n := range dependents[c] {
			if !seen[n] {
				seen[n] = true
				queue = append(queue, n)
			}
		}
	}
	var out []string
	for _, s := range d.Stages {
		if seen[s.Code] {
			out = append(out, s.Code)
		}
	}
	return out
}

// Render 差异表（给人和 agent 复述用）。
func Render(cs []Change, next *Def) string {
	if len(cs) == 0 {
		return "无差异。\n"
	}
	var b strings.Builder
	b.WriteString("| 类别 | 阶段 | 变化 |\n|---|---|---|\n")
	touched := map[string]bool{}
	for _, c := range cs {
		st := c.Stage
		if st == "" {
			st = "—"
		} else {
			touched[c.Stage] = true
		}
		fmt.Fprintf(&b, "| %s | %s | %s |\n", c.Kind, st, strings.ReplaceAll(c.Detail, "|", "／"))
	}
	var codes []string
	for c := range touched {
		codes = append(codes, c)
	}
	sort.Strings(codes)
	if len(codes) > 0 {
		b.WriteString("\n影响范围：改动阶段 " + strings.Join(codes, "、"))
		if ds := Downstream(next, codes); len(ds) > 0 {
			b.WriteString("；下游 " + strings.Join(ds, "、"))
		}
		b.WriteString("。\n")
	}
	b.WriteString("已在跑的 run 按触发时的快照继续，不受影响；激活后新触发的 run 才用新定义。\n")
	return b.String()
}
