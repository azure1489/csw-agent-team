package adminsrv

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/auth"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/config"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// setupAdmin 起真实装配的管理后台（迁移 + seed），建一个 operator 并登录，返回 server、store 与 access token。
func setupAdmin(t *testing.T) (*httptest.Server, *sqlite.Store, string) {
	t.Helper()
	db, err := sqlite.Open(filepath.Join(t.TempDir(), "admin.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	if err := sqlite.Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	st := sqlite.New(db)
	hash, err := auth.HashPassword("pw-123456")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := st.Q().CreateUser(context.Background(), "op", hash, "操作员", string(domain.AdminOperator)); err != nil {
		t.Fatalf("create user: %v", err)
	}
	cfg := config.Config{JWTSecret: []byte("test-secret"), AccessTTL: time.Minute, RefreshTTL: time.Hour,
		CORSOrigins: []string{"http://localhost:5173"}}
	ts := httptest.NewServer(New(st, cfg, slog.New(slog.NewTextHandler(io.Discard, nil))).Router())
	t.Cleanup(ts.Close)

	var login struct {
		AccessToken string `json:"access_token"`
	}
	if code := adminDo(t, ts, "", http.MethodPost, "/admin/login", map[string]string{"username": "op", "password": "pw-123456"}, &login); code != 200 {
		t.Fatalf("login: %d", code)
	}
	return ts, st, login.AccessToken
}

func adminDo(t *testing.T, ts *httptest.Server, token, method, path string, body, out any) int {
	t.Helper()
	var rd io.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		rd = bytes.NewReader(b)
	}
	req, _ := http.NewRequest(method, ts.URL+path, rd)
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, path, err)
	}
	defer resp.Body.Close()
	if out != nil {
		_ = json.NewDecoder(resp.Body).Decode(out)
	}
	return resp.StatusCode
}

type rtGate struct {
	ReviewerRole string `json:"reviewer_role"`
	Name         string `json:"name"`
	RelayedByHub bool   `json:"relayed_by_hub"`
}

type rtStage struct {
	Code              string   `json:"code"`
	Name              string   `json:"name"`
	RoleCode          string   `json:"role_code"`
	OutputType        string   `json:"output_type"`
	Instructions      string   `json:"instructions"`
	SelfCheckCriteria string   `json:"self_check_criteria"`
	Acceptance        string   `json:"acceptance"`
	DispatchMode      string   `json:"dispatch_mode"`
	ActionClass       string   `json:"action_class"`
	Deps              []string `json:"deps"`
	Gates             []rtGate `json:"gates"`
	SLAMinutes        int      `json:"sla_minutes"`
	AckMinutes        int      `json:"ack_minutes"`
	IdleMinutes       int      `json:"idle_minutes"`
	IsMerge           bool     `json:"is_merge"`
	PerItem           bool     `json:"per_item"`
}

type rtWorkflow struct {
	Workflow struct {
		Name               string `json:"name"`
		HubRole            string `json:"hub_role"`
		DispatchMode       string `json:"dispatch_mode"`
		TriggerRoles       string `json:"trigger_roles"`
		CommonInstructions string `json:"common_instructions"`
		CommonAcceptance   string `json:"common_acceptance"`
	} `json:"workflow"`
	Stages       []rtStage `json:"stages"`
	DefaultGates []rtGate  `json:"default_gates"`
}

// toPut 按前端 editorToPutBody 的形态把 GET 结果转回 PUT 请求体。
func (w rtWorkflow) toPut() putWorkflowReq {
	req := putWorkflowReq{
		Name: w.Workflow.Name, HubRole: w.Workflow.HubRole, DispatchMode: w.Workflow.DispatchMode,
		TriggerRoles: w.Workflow.TriggerRoles, CommonInstructions: w.Workflow.CommonInstructions,
		CommonAcceptance: w.Workflow.CommonAcceptance,
	}
	for _, s := range w.Stages {
		ps := putStage{
			Code: s.Code, Name: s.Name, RoleCode: s.RoleCode, OutputType: s.OutputType,
			Instructions: s.Instructions, SelfCheckCriteria: s.SelfCheckCriteria, Acceptance: s.Acceptance,
			IsMerge: s.IsMerge, DispatchMode: s.DispatchMode, ActionClass: s.ActionClass,
			SLAMinutes: s.SLAMinutes, PerItem: s.PerItem, Deps: s.Deps,
			AckMinutes: s.AckMinutes, IdleMinutes: s.IdleMinutes,
		}
		for _, g := range s.Gates {
			ps.Gates = append(ps.Gates, putGate(g))
		}
		req.Stages = append(req.Stages, ps)
	}
	for _, g := range w.DefaultGates {
		req.DefaultGates = append(req.DefaultGates, putGate(g))
	}
	return req
}

func stageSigs(w rtWorkflow) []string {
	out := make([]string, 0, len(w.Stages))
	for _, s := range w.Stages {
		deps := append([]string{}, s.Deps...)
		sort.Strings(deps)
		var gs []string
		for _, g := range s.Gates {
			gs = append(gs, fmt.Sprintf("%s/%v/%s", g.ReviewerRole, g.RelayedByHub, g.Name))
		}
		out = append(out, fmt.Sprintf("%s mode=%q action=%s sla=%d ack=%d idle=%d item=%v merge=%v deps=%s gates=%s text=%d/%d/%d",
			s.Code, s.DispatchMode, s.ActionClass, s.SLAMinutes, s.AckMinutes, s.IdleMinutes, s.PerItem, s.IsMerge,
			strings.Join(deps, ","), strings.Join(gs, ";"), len(s.Instructions), len(s.SelfCheckCriteria), len(s.Acceptance)))
	}
	return out
}

func diffSigs(t *testing.T, label string, got, want []string) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("%s: %d stages, want %d", label, len(got), len(want))
	}
	for i := range got {
		if got[i] != want[i] {
			t.Fatalf("%s stage %d:\n got  %s\n want %s", label, i, got[i], want[i])
		}
	}
}

// TestWorkflowRoundTripKeepsStageFields 后台整份保存与复制不丢派工模式、动作类别、时限、条目级与逐阶段闸。
func TestWorkflowRoundTripKeepsStageFields(t *testing.T) {
	ts, st, token := setupAdmin(t)
	wf, err := st.Q().ActiveWorkflowByKey(context.Background(), "daily_news")
	if err != nil {
		t.Fatal(err)
	}
	path := fmt.Sprintf("/admin/workflows/%d", wf.ID)

	var before rtWorkflow
	if code := adminDo(t, ts, token, http.MethodGet, path, nil, &before); code != 200 {
		t.Fatalf("get: %d", code)
	}
	if len(before.Stages) != 13 || len(before.DefaultGates) != 0 {
		t.Fatalf("v2 shape: %d stages, %d default gates", len(before.Stages), len(before.DefaultGates))
	}

	// 改一处（topic 时限 15→20）后整份保存。
	body := before.toPut()
	for i := range body.Stages {
		if body.Stages[i].Code == "topic" {
			body.Stages[i].SLAMinutes = 20
			body.Stages[i].IdleMinutes = 7
		}
	}
	var putResp struct {
		Validation struct {
			AllOK bool `json:"all_ok"`
		} `json:"validation"`
	}
	if code := adminDo(t, ts, token, http.MethodPut, path, body, &putResp); code != 200 || !putResp.Validation.AllOK {
		t.Fatalf("put: %d all_ok=%v", code, putResp.Validation.AllOK)
	}
	var after rtWorkflow
	adminDo(t, ts, token, http.MethodGet, path, nil, &after)
	want := stageSigs(before)
	for i, s := range before.Stages {
		if s.Code == "topic" {
			want[i] = strings.Replace(want[i], "sla=15", "sla=20", 1)
			want[i] = strings.Replace(want[i], "idle=0", "idle=7", 1)
		}
	}
	diffSigs(t, "after put", stageSigs(after), want)

	// 复制为新版本：字段完整带过去。
	var cloned struct {
		ID int64 `json:"id"`
	}
	if code := adminDo(t, ts, token, http.MethodPost, path+"/clone", nil, &cloned); code != 201 {
		t.Fatalf("clone: %d", code)
	}
	var clone rtWorkflow
	adminDo(t, ts, token, http.MethodGet, fmt.Sprintf("/admin/workflows/%d", cloned.ID), nil, &clone)
	diffSigs(t, "clone", stageSigs(clone), want)

	// 非法取值在保存前被拒。
	bad := after.toPut()
	bad.Stages[8].ActionClass = "platform_write:local_drill"
	var errResp struct {
		Code string `json:"code"`
	}
	if code := adminDo(t, ts, token, http.MethodPut, path, bad, &errResp); code != 400 || errResp.Code != "bad_action_class" {
		t.Fatalf("bad action_class: %d %s", code, errResp.Code)
	}
	bad = after.toPut()
	bad.Stages[0].DispatchMode = "sometimes"
	if code := adminDo(t, ts, token, http.MethodPut, path, bad, &errResp); code != 400 || errResp.Code != "bad_dispatch_mode" {
		t.Fatalf("bad dispatch_mode: %d %s", code, errResp.Code)
	}
	bad = after.toPut()
	bad.Stages[0].AckMinutes = -1
	if code := adminDo(t, ts, token, http.MethodPut, path, bad, &errResp); code != 400 || errResp.Code != "bad_alert_minutes" {
		t.Fatalf("bad ack_minutes: %d %s", code, errResp.Code)
	}
}
