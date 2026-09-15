// Package wfdef 工作流定义的文件形态（YAML，schema: csw-workflow/v1）与管理后台 API 形态之间的转换、差异与本地检查。
package wfdef

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

// Schema 当前文件格式标识。
const Schema = "csw-workflow/v1"

// Text 长文本：可内联字符串，也可写成 {file: 相对路径} 引用 YAML 所在目录下的 md 文件。
type Text struct {
	Value string
	File  string // 非空表示来自外部文件
}

// UnmarshalYAML 接受字符串或 {file: 路径}。
func (t *Text) UnmarshalYAML(n *yaml.Node) error {
	switch n.Kind {
	case yaml.ScalarNode:
		t.Value = n.Value
		return nil
	case yaml.MappingNode:
		var m struct {
			File string `yaml:"file"`
		}
		if err := n.Decode(&m); err != nil {
			return err
		}
		if strings.TrimSpace(m.File) == "" {
			return fmt.Errorf("第 %d 行：文本引用须写 file: 路径", n.Line)
		}
		t.File = m.File
		return nil
	}
	return fmt.Errorf("第 %d 行：文本须为字符串或 {file: 路径}", n.Line)
}

// MarshalYAML 有 File 时写引用，否则写内联字符串。
func (t Text) MarshalYAML() (any, error) {
	if t.File != "" {
		return map[string]string{"file": t.File}, nil
	}
	return t.Value, nil
}

// Gate 审核闸。relayed=true 表示人工闸（如 Van）由中枢代录。
type Gate struct {
	Reviewer string `yaml:"reviewer"`
	Name     string `yaml:"name,omitempty"`
	Relayed  bool   `yaml:"relayed,omitempty"`
}

// Stage 阶段。Gates 为 nil 表示未写（沿用默认闸）；指向空切片表示显式 0 闸。
type Stage struct {
	Code         string   `yaml:"code"`
	Name         string   `yaml:"name"`
	Role         string   `yaml:"role"`
	OutputType   string   `yaml:"output_type,omitempty"`
	IsMerge      bool     `yaml:"is_merge,omitempty"`
	Deps         []string `yaml:"deps,flow"`
	Gates        *[]Gate  `yaml:"gates,omitempty"`
	DispatchMode string   `yaml:"dispatch_mode,omitempty"` // 空=继承工作流 / manual / auto
	ActionClass  string   `yaml:"action_class,omitempty"`  // 空=read / platform_write:<scope>
	SLAMinutes   int      `yaml:"sla_minutes,omitempty"`
	AckMinutes   int      `yaml:"ack_minutes,omitempty"`  // 派工后多久未接单提醒；0=默认
	IdleMinutes  int      `yaml:"idle_minutes,omitempty"` // 接单后多久无活动提醒；0=默认
	PerItem      bool     `yaml:"per_item,omitempty"`
	Instructions Text     `yaml:"instructions"`
	SelfCheck    Text     `yaml:"self_check"`
	Acceptance   Text     `yaml:"acceptance"`
}

// Meta 基本信息。Version 为导出来源版本，push 时忽略（服务端自动建新版本）。
type Meta struct {
	Key          string   `yaml:"wf_key"`
	Name         string   `yaml:"name"`
	Version      int      `yaml:"version,omitempty"`
	HubRole      string   `yaml:"hub_role"`
	DispatchMode string   `yaml:"dispatch_mode"`
	TriggerRoles []string `yaml:"trigger_roles,flow"`
}

// Common 通用段：拼在每个阶段的作业手册 / 验收标准之前。
type Common struct {
	Instructions Text `yaml:"instructions"`
	Acceptance   Text `yaml:"acceptance"`
}

// Def 一份工作流定义文件。
type Def struct {
	Schema       string  `yaml:"schema"`
	Meta         Meta    `yaml:"meta"`
	Common       Common  `yaml:"common"`
	DefaultGates []Gate  `yaml:"default_gates"`
	Stages       []Stage `yaml:"stages"`
}

// texts 遍历全部长文本字段（含位置标签），供解析 file: 与外置。
func (d *Def) texts() []struct {
	label string
	t     *Text
} {
	out := []struct {
		label string
		t     *Text
	}{{"common.instructions", &d.Common.Instructions}, {"common.acceptance", &d.Common.Acceptance}}
	for i := range d.Stages {
		s := &d.Stages[i]
		out = append(out,
			struct {
				label string
				t     *Text
			}{s.Code + ".instructions", &s.Instructions},
			struct {
				label string
				t     *Text
			}{s.Code + ".self_check", &s.SelfCheck},
			struct {
				label string
				t     *Text
			}{s.Code + ".acceptance", &s.Acceptance})
	}
	return out
}

// Load 读 YAML（未知字段报错），解析 file: 引用。
func Load(path string) (*Def, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	dec := yaml.NewDecoder(bytes.NewReader(raw))
	dec.KnownFields(true)
	var d Def
	if err := dec.Decode(&d); err != nil {
		return nil, fmt.Errorf("%s：%w", path, err)
	}
	if d.Schema != Schema {
		return nil, fmt.Errorf("%s：schema 须为 %s，收到 %q", path, Schema, d.Schema)
	}
	dir := filepath.Dir(path)
	for _, x := range d.texts() {
		if x.t.File == "" {
			continue
		}
		p := filepath.Join(dir, x.t.File)
		b, err := os.ReadFile(p)
		if err != nil {
			return nil, fmt.Errorf("%s 引用的文件读不到（%s）：%w", x.label, x.t.File, err)
		}
		x.t.Value = strings.TrimRight(string(b), "\n")
	}
	return &d, nil
}

// Save 写 YAML。externalDir 非空时把多行或较长文本外置为 externalDir 下的 md（路径相对 YAML 目录）。
func Save(d *Def, path, externalDir string) error {
	dir := filepath.Dir(path)
	if externalDir != "" {
		if err := os.MkdirAll(filepath.Join(dir, externalDir), 0o755); err != nil {
			return err
		}
		for _, x := range d.texts() {
			if !strings.Contains(x.t.Value, "\n") && len([]rune(x.t.Value)) <= 120 {
				x.t.File = ""
				continue
			}
			rel := filepath.ToSlash(filepath.Join(externalDir, x.label+".md"))
			if err := os.WriteFile(filepath.Join(dir, rel), []byte(x.t.Value+"\n"), 0o644); err != nil {
				return err
			}
			x.t.File = rel
		}
	}
	var buf bytes.Buffer
	buf.WriteString("# 工作流定义文件（" + Schema + "）。由 wfctl export 生成；改完用 wfctl lint / diff 检查，再 wfctl push --draft。\n")
	enc := yaml.NewEncoder(&buf)
	enc.SetIndent(2)
	if err := enc.Encode(d); err != nil {
		return err
	}
	return os.WriteFile(path, buf.Bytes(), 0o644)
}

// ── 管理后台 API 形态 ──

// APIGate 管理后台的闸形态。
type APIGate struct {
	ReviewerRole string `json:"reviewer_role"`
	Name         string `json:"name"`
	GateOrder    int    `json:"gate_order,omitempty"`
	RelayedByHub bool   `json:"relayed_by_hub"`
}

// APIStage GET /admin/workflows/:id 的阶段形态，也是 PUT 的阶段形态。
type APIStage struct {
	Code              string    `json:"code"`
	Name              string    `json:"name"`
	RoleCode          string    `json:"role_code"`
	OutputType        string    `json:"output_type"`
	Instructions      string    `json:"instructions"`
	SelfCheckCriteria string    `json:"self_check_criteria"`
	Acceptance        string    `json:"acceptance"`
	DispatchMode      string    `json:"dispatch_mode"`
	ActionClass       string    `json:"action_class"`
	Deps              []string  `json:"deps"`
	Gates             []APIGate `json:"gates"`
	SLAMinutes        int       `json:"sla_minutes"`
	AckMinutes        int       `json:"ack_minutes"`
	IdleMinutes       int       `json:"idle_minutes"`
	IsMerge           bool      `json:"is_merge"`
	PerItem           bool      `json:"per_item"`
}

// GetBody GET /admin/workflows/:id 响应。
type GetBody struct {
	Workflow struct {
		Key                string `json:"wf_key"`
		Name               string `json:"name"`
		HubRole            string `json:"hub_role"`
		DispatchMode       string `json:"dispatch_mode"`
		TriggerRoles       string `json:"trigger_roles"`
		Status             string `json:"status"`
		CommonInstructions string `json:"common_instructions"`
		CommonAcceptance   string `json:"common_acceptance"`
		ID                 int64  `json:"id"`
		Version            int    `json:"version"`
	} `json:"workflow"`
	Stages       []APIStage `json:"stages"`
	DefaultGates []APIGate  `json:"default_gates"`
}

// PutBody PUT /admin/workflows/:id 请求体。
type PutBody struct {
	Reason             string     `json:"reason,omitempty"`
	ChangeSummary      string     `json:"change_summary,omitempty"`
	Name               string     `json:"name"`
	HubRole            string     `json:"hub_role"`
	DispatchMode       string     `json:"dispatch_mode"`
	TriggerRoles       string     `json:"trigger_roles"`
	CommonInstructions string     `json:"common_instructions"`
	CommonAcceptance   string     `json:"common_acceptance"`
	Stages             []APIStage `json:"stages"`
	DefaultGates       []APIGate  `json:"default_gates"`
}

func gatesFromAPI(gs []APIGate) []Gate {
	out := make([]Gate, 0, len(gs))
	for _, g := range gs {
		out = append(out, Gate{Reviewer: g.ReviewerRole, Name: g.Name, Relayed: g.RelayedByHub})
	}
	return out
}

func gatesToAPI(gs []Gate) []APIGate {
	out := make([]APIGate, 0, len(gs))
	for i, g := range gs {
		out = append(out, APIGate{ReviewerRole: g.Reviewer, Name: g.Name, RelayedByHub: g.Relayed, GateOrder: i + 1})
	}
	return out
}

func splitCSV(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// FromGet 管理后台 GET 响应 → 定义文件。无默认闸时，没有覆盖闸的阶段写成显式 0 闸。
func FromGet(g GetBody) *Def {
	d := &Def{Schema: Schema, DefaultGates: gatesFromAPI(g.DefaultGates)}
	w := g.Workflow
	d.Meta = Meta{Key: w.Key, Name: w.Name, Version: w.Version, HubRole: w.HubRole, DispatchMode: w.DispatchMode, TriggerRoles: splitCSV(w.TriggerRoles)}
	d.Common = Common{Instructions: Text{Value: w.CommonInstructions}, Acceptance: Text{Value: w.CommonAcceptance}}
	for _, s := range g.Stages {
		st := Stage{
			Code: s.Code, Name: s.Name, Role: s.RoleCode, OutputType: s.OutputType, IsMerge: s.IsMerge,
			Deps: append([]string{}, s.Deps...), DispatchMode: s.DispatchMode, ActionClass: s.ActionClass,
			SLAMinutes: s.SLAMinutes, PerItem: s.PerItem, AckMinutes: s.AckMinutes, IdleMinutes: s.IdleMinutes,
			Instructions: Text{Value: s.Instructions}, SelfCheck: Text{Value: s.SelfCheckCriteria}, Acceptance: Text{Value: s.Acceptance},
		}
		if st.ActionClass == "read" {
			st.ActionClass = ""
		}
		if len(s.Gates) > 0 || len(g.DefaultGates) == 0 {
			gs := gatesFromAPI(s.Gates)
			st.Gates = &gs
		}
		d.Stages = append(d.Stages, st)
	}
	return d
}

// ToPut 定义文件 → PUT 请求体（阶段顺序即 seq）。
func ToPut(d *Def) PutBody {
	p := PutBody{
		Name: d.Meta.Name, HubRole: d.Meta.HubRole, DispatchMode: d.Meta.DispatchMode,
		TriggerRoles: strings.Join(d.Meta.TriggerRoles, ","), CommonInstructions: d.Common.Instructions.Value,
		CommonAcceptance: d.Common.Acceptance.Value, DefaultGates: gatesToAPI(d.DefaultGates),
	}
	for _, s := range d.Stages {
		as := APIStage{
			Code: s.Code, Name: s.Name, RoleCode: s.Role, OutputType: s.OutputType, IsMerge: s.IsMerge,
			Deps: append([]string{}, s.Deps...), DispatchMode: s.DispatchMode, ActionClass: s.ActionClass,
			SLAMinutes: s.SLAMinutes, PerItem: s.PerItem, AckMinutes: s.AckMinutes, IdleMinutes: s.IdleMinutes,
			Instructions:      s.Instructions.Value,
			SelfCheckCriteria: s.SelfCheck.Value, Acceptance: s.Acceptance.Value, Gates: []APIGate{},
		}
		if as.ActionClass == "" {
			as.ActionClass = "read"
		}
		if s.Gates != nil {
			as.Gates = gatesToAPI(*s.Gates)
		}
		p.Stages = append(p.Stages, as)
	}
	return p
}
