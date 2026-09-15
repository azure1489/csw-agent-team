package wfdef

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

var roles = map[string]domain.Role{
	"editor":    {Code: "editor", IsManagement: true},
	"van":       {Code: "van", IsManagement: true, IsHuman: true},
	"writer":    {Code: "writer"},
	"publisher": {Code: "publisher"},
}

func gates(gs ...Gate) *[]Gate { return &gs }

func sample() *Def {
	long := strings.Repeat("写清楚本阶段要做的事。\n", 20)
	return &Def{
		Schema: Schema,
		Meta:   Meta{Key: "t", Name: "测试", HubRole: "editor", DispatchMode: "auto", TriggerRoles: []string{"editor"}},
		Common: Common{Instructions: Text{Value: "通用约定"}, Acceptance: Text{Value: "通用合规"}},
		Stages: []Stage{
			{Code: "a", Name: "01-写作", Role: "writer", Deps: []string{}, Gates: gates(Gate{Reviewer: "editor", Name: "主编审"}, Gate{Reviewer: "van", Name: "Van审", Relayed: true}),
				DispatchMode: "manual", SLAMinutes: 30, AckMinutes: 5, IdleMinutes: 10, Instructions: Text{Value: long}, SelfCheck: Text{Value: "- 自检"}, Acceptance: Text{Value: "- 验收"}},
			{Code: "b", Name: "02-草稿保存", Role: "publisher", Deps: []string{"a"}, Gates: gates(), ActionClass: "platform_write:wx_draft",
				Instructions: Text{Value: "保存草稿"}, SelfCheck: Text{Value: "- 回读"}, Acceptance: Text{Value: "- 有草稿"}},
		},
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "t_v1.yaml")
	if err := Save(sample(), path, "t_v1"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, "t_v1", "a.instructions.md")); err != nil {
		t.Fatalf("long text should be externalized: %v", err)
	}
	raw, _ := os.ReadFile(path)
	if !strings.Contains(string(raw), "gates: []") || !strings.Contains(string(raw), "file: t_v1/a.instructions.md") || !strings.Contains(string(raw), "instructions: 保存草稿") {
		t.Fatalf("yaml shape:\n%s", raw)
	}
	back, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if cs := Diff(sample(), back); len(cs) != 0 {
		t.Fatalf("round trip changed: %+v", cs)
	}
	// 未知字段被拒（防手误）。
	bad := filepath.Join(dir, "bad.yaml")
	os.WriteFile(bad, []byte("schema: csw-workflow/v1\nmeta: {wf_key: t}\nstagez: []\n"), 0o644)
	if _, err := Load(bad); err == nil {
		t.Fatal("unknown field should fail")
	}
}

func TestToPutAndFromGet(t *testing.T) {
	p := ToPut(sample())
	if p.Stages[0].ActionClass != "read" || p.Stages[1].Gates == nil || len(p.Stages[1].Gates) != 0 || p.TriggerRoles != "editor" {
		t.Fatalf("put body: %+v", p.Stages)
	}
	var g GetBody
	g.Workflow.Key, g.Workflow.Name, g.Workflow.HubRole, g.Workflow.TriggerRoles = "t", "测试", "editor", "editor"
	g.Stages = []APIStage{{Code: "a", Name: "01", RoleCode: "writer", ActionClass: "read"}}
	d := FromGet(g)
	if d.Stages[0].Gates == nil || len(*d.Stages[0].Gates) != 0 || d.Stages[0].ActionClass != "" {
		t.Fatalf("no default gates → explicit 0 gates: %+v", d.Stages[0])
	}
	g.DefaultGates = []APIGate{{ReviewerRole: "editor"}}
	if FromGet(g).Stages[0].Gates != nil {
		t.Fatal("with default gates, a stage without overrides inherits (nil)")
	}
}

func TestDiffKinds(t *testing.T) {
	a, b := sample(), sample()
	b.Stages[0].SLAMinutes = 45
	b.Stages[0].IdleMinutes = 5
	b.Stages[1].Deps = []string{}
	*b.Stages[1].Gates = []Gate{{Reviewer: "editor", Name: "主编核"}}
	b.Stages[1].Acceptance.Value = "- 有草稿\n- 有截图"
	b.Stages = append(b.Stages, Stage{Code: "c", Name: "03-新", Role: "writer", Deps: []string{"b"}, Gates: gates()})
	kinds := map[string]int{}
	for _, c := range Diff(a, b) {
		kinds[c.Kind]++
	}
	if kinds["阶段"] < 2 || kinds["依赖"] != 1 || kinds["闸"] != 1 || kinds["文本"] != 1 {
		t.Fatalf("diff kinds: %v", kinds)
	}
	out := Render(Diff(a, b), b)
	if !strings.Contains(out, "时限：30 → 45") || !strings.Contains(out, "无活动提醒：10 → 5") || !strings.Contains(out, "−a") || !strings.Contains(out, "已在跑的 run") {
		t.Fatalf("render:\n%s", out)
	}
}

func has(fs []Finding, level, rule string) bool {
	for _, f := range fs {
		if f.Level == level && f.Rule == rule {
			return true
		}
	}
	return false
}

func TestLintLocal(t *testing.T) {
	if fs := LintLocal(sample(), roles); Worst(fs) != 0 {
		t.Fatalf("sample should be clean: %+v", fs)
	}
	d := sample()
	d.Stages[1].Gates = nil
	if fs := LintLocal(d, roles); !has(fs, "error", "L1") {
		t.Fatalf("missing gates: key must be L1 error: %+v", fs)
	}
	d = sample()
	d.DefaultGates = []Gate{{Reviewer: "editor"}}
	if fs := LintLocal(d, roles); !has(fs, "error", "L1") {
		t.Fatalf("explicit 0 gates with default gates must be L1 error")
	}
	d = sample()
	d.Stages[1].ActionClass = ""
	if fs := LintLocal(d, roles); !has(fs, "warn", "L2") || Worst(fs) != 3 {
		t.Fatalf("save stage marked read should warn L2: %+v", fs)
	}
	d = sample()
	d.Stages[0].Instructions.Value = "# 整篇方案\n见 §3.2"
	if fs := LintLocal(d, roles); !has(fs, "warn", "L6") {
		t.Fatalf("pasted doc should warn L6")
	}
	d = sample()
	d.Stages[0].Deps = []string{"b"} // a↔b 成环
	d.Stages[1].ActionClass = "platform_write:local_drill"
	fs := LintLocal(d, roles)
	if !has(fs, "error", "V") || Worst(fs) != 1 {
		t.Fatalf("cycle / bad action class must fail server checks: %+v", fs)
	}
}
