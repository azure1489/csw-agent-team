package server

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"testing"
)

// TestIntakeEndpoints 采集轮上报与采集全貌的 HTTP 面（daily_news v5）。
func TestIntakeEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	if err := st.Q().ActivateWorkflowVersion(ctx, "daily_news", 5); err != nil {
		t.Fatal(err)
	}
	collectorTok, analystTok := mint("collector"), mint("analyst")
	editor, _ := st.Q().ActiveAgentByRole(ctx, "editor")
	role, _ := st.Q().GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-17", "", "")
	if err != nil {
		t.Fatal(err)
	}
	base := fmt.Sprintf("%s/api/v1/runs/%d", ts.URL, res.Run.ID)

	// 上报两轮采集：一轮成功、一轮失败并写明原因。
	body := `{"sweeps":[
		{"sweep_key":"ig-1","platform":"instagram","source_key":"channel","tool":"csw_mcp","query":"露营椅","found":12,"in_window":5,"registered":2,"result":"ok"},
		{"sweep_key":"xhs-1","platform":"xhs","source_key":"channel","tool":"opencli","result":"failed","error":"登录态失效，已改用网页"}
	]}`
	code, out := httpJSON(t, http.MethodPut, base+"/sweeps", collectorTok, bytes.NewBufferString(body), "application/json")
	if code != 200 || len(out["sweeps"].([]any)) != 2 {
		t.Fatalf("report sweeps: %d %v", code, out)
	}

	// 非本 run 参与角色不能上报。
	if code, out := httpJSON(t, http.MethodPut, base+"/sweeps", analystTok,
		bytes.NewBufferString(`{"sweeps":[{"sweep_key":"x","platform":"web","result":"ok"}]}`), "application/json"); code != 403 {
		t.Fatalf("non-participant sweeps: %d %v", code, out)
	}

	// 失败轮缺 error 被拒。
	if code, out := httpJSON(t, http.MethodPut, base+"/sweeps", collectorTok,
		bytes.NewBufferString(`{"sweeps":[{"sweep_key":"web-1","platform":"web","result":"failed"}]}`), "application/json"); code != 400 ||
		out["code"] != "sweep_error_required" {
		t.Fatalf("failed sweep without error: %d %v", code, out)
	}

	// 登记一条带溯源与一条淘汰。
	items := `{"items":[
		{"item_key":"hxo-3fa91c","title":"HxO 折叠木椅","brand":"HxO","source_url":"https://example.com/hxo",
		 "discovered_via":"ig-1","fetched_at":"2026-09-17T06:10:00Z","evidence_url":"https://example.com/p.jpg","dedup_note":"近 30 天未见同角度"},
		{"item_key":"keen-778899","title":"KEEN 凉鞋","brand":"KEEN","status":"dropped","reason_code":"no_value","reason":"只是配色更新","discovered_via":"ig-1"}
	]}`
	if code, out := httpJSON(t, http.MethodPut, base+"/items", collectorTok, bytes.NewBufferString(items), "application/json"); code != 200 {
		t.Fatalf("upsert items: %d %v", code, out)
	}

	// 采集全貌：采集轮、条目、轨迹、覆盖都在。
	code, out = httpJSON(t, http.MethodGet, base+"/intake-trace", collectorTok, nil, "")
	if code != 200 {
		t.Fatalf("intake-trace: %d %v", code, out)
	}
	if len(out["sweeps"].([]any)) != 2 || len(out["items"].([]any)) != 2 {
		t.Fatalf("trace payload: %v", out)
	}
	if len(out["traces"].([]any)) == 0 {
		t.Fatal("淘汰应留下判断轨迹")
	}
	if len(out["coverage"].([]any)) == 0 {
		t.Fatal("覆盖应给出台账对账结果")
	}
	first := out["items"].([]any)[0].(map[string]any)
	if first["discovered_via"] != "ig-1" || first["dedup_note"] == nil {
		t.Fatalf("条目溯源字段应回显：%v", first)
	}
}

// TestIntakeTraceOnRunWithoutRecords 旧 run 没有采集记录时返回空列表而不是 500。
func TestIntakeTraceOnRunWithoutRecords(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	editor, _ := st.Q().ActiveAgentByRole(ctx, "editor")
	role, _ := st.Q().GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-17", "", "")
	if err != nil {
		t.Fatal(err)
	}
	code, out := httpJSON(t, http.MethodGet,
		fmt.Sprintf("%s/api/v1/runs/%d/intake-trace", ts.URL, res.Run.ID), mint("collector"), nil, "")
	if code != 200 {
		t.Fatalf("legacy run: %d %v", code, out)
	}
	if out["sweeps"] == nil || len(out["sweeps"].([]any)) != 0 {
		t.Fatalf("空数据应是空列表而不是 null：%v", out["sweeps"])
	}
}
