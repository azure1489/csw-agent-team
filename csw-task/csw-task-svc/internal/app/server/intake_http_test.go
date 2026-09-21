package server

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"strings"
	"testing"
)

// TestIntakeEndpoints 采集轮上报与采集全貌的 HTTP 面（daily_news v5）。
func TestIntakeEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	if err := st.Q().ActivateWorkflowVersion(ctx, "daily_news", 6); err != nil {
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

// dimsJSON 造一份六维齐全、每维有依据的 dims。
func dimsJSON(verdict string) string {
	parts := make([]string, 0, 6)
	for _, k := range []string{"change", "use", "gain", "compare", "explain", "csw"} {
		parts = append(parts, fmt.Sprintf(`%q:{"verdict":%q,"basis":"正文「改了结构」"}`, k, verdict))
	}
	return "{" + strings.Join(parts, ",") + "}"
}

// TestIntakeJudgementEndpoints 逐条判断的 HTTP 面。
//
// 三条口径硬规则各有一条断言：六维必须齐全且有依据、没读到实图只能落 pending_check、
// 结论只有四档。它们在引擎侧校验（引擎是唯一真相），库里再用 CHECK 兜一道。
func TestIntakeJudgementEndpoints(t *testing.T) {
	ts, st, eng, mint := setupHTTP(t)
	ctx := context.Background()
	collectorTok, analystTok := mint("collector"), mint("analyst")
	editor, _ := st.Q().ActiveAgentByRole(ctx, "editor")
	role, _ := st.Q().GetRole(ctx, "editor")
	res, err := eng.Trigger(ctx, editor, role, "daily_news", "2026-09-22", "", "")
	if err != nil {
		t.Fatal(err)
	}
	base := fmt.Sprintf("%s/api/v1/runs/%d", ts.URL, res.Run.ID)
	put := func(body string) (int, map[string]any) {
		return httpJSON(t, http.MethodPut, base+"/intake-judgements", collectorTok,
			bytes.NewBufferString(body), "application/json")
	}

	// 三条：推荐、不推荐、没读到图的待核。
	body := fmt.Sprintf(`{"judgements":[
		{"candidate_key":"snowpeak-abc123","platform":"instagram","post_ref":"DbKe9gIkoMb","tier":"recommend",
		 "image_seen":true,"dims":%s,"heat_note":"369 赞 · 常态 71 的 5.2×","rubric_version":"v9"},
		{"candidate_key":"keen-778899","platform":"instagram","tier":"not_recommend","image_seen":true,
		 "dims":%s,"gaps":["只有新配色"]},
		{"candidate_key":"noimg-000001","platform":"web","tier":"pending_check","image_seen":false,
		 "dims":%s,"gaps":["未读到实图"]}
	]}`, dimsJSON("yes"), dimsJSON("no"), dimsJSON("unclear"))
	code, out := put(body)
	if code != 200 || len(out["judgements"].([]any)) != 3 {
		t.Fatalf("report judgements: %d %v", code, out)
	}
	// 推荐排在最前，02 直接看台账就够
	if first := out["judgements"].([]any)[0].(map[string]any); first["tier"] != "recommend" {
		t.Fatalf("推荐应排最前，实为 %v", first["tier"])
	}

	// 同键重报即覆盖，不追加
	if code, out := put(fmt.Sprintf(`{"judgements":[{"candidate_key":"keen-778899","tier":"alternate",
		"image_seen":true,"dims":%s}]}`, dimsJSON("unclear"))); code != 200 ||
		len(out["judgements"].([]any)) != 3 {
		t.Fatalf("重报应覆盖而不是追加：%d %v", code, out)
	}

	// 没读到实图却落非待核档 —— 拒
	if code, out := put(fmt.Sprintf(`{"judgements":[{"candidate_key":"x-1","tier":"not_recommend",
		"image_seen":false,"dims":%s}]}`, dimsJSON("no"))); code != 400 ||
		out["code"] != "image_unseen_must_pend" {
		t.Fatalf("没读到实图应只能落待核：%d %v", code, out)
	}

	// 缺一个维度 —— 拒
	if code, out := put(`{"judgements":[{"candidate_key":"x-2","tier":"recommend","image_seen":true,
		"dims":{"change":{"verdict":"yes","basis":"a"}}}]}`); code != 400 || out["code"] != "dim_missing" {
		t.Fatalf("六维不齐应被拒：%d %v", code, out)
	}

	// 维度有结论但没依据 —— 拒（编不出来就判 unclear，不许空着）
	noBasis := `{"change":{"verdict":"yes","basis":""},"use":{"verdict":"yes","basis":"b"},
		"gain":{"verdict":"yes","basis":"b"},"compare":{"verdict":"yes","basis":"b"},
		"explain":{"verdict":"yes","basis":"b"},"csw":{"verdict":"yes","basis":"b"}}`
	if code, out := put(`{"judgements":[{"candidate_key":"x-3","tier":"recommend","image_seen":true,
		"dims":` + noBasis + `}]}`); code != 400 || out["code"] != "dim_basis_required" {
		t.Fatalf("没依据应被拒：%d %v", code, out)
	}

	// 四档之外 —— 拒（没有分数，也没有第五档）
	if code, out := put(fmt.Sprintf(`{"judgements":[{"candidate_key":"x-4","tier":"maybe",
		"image_seen":true,"dims":%s}]}`, dimsJSON("yes"))); code != 400 || out["code"] != "bad_tier" {
		t.Fatalf("非法档应被拒：%d %v", code, out)
	}

	// 非本 run 参与角色不能上报
	if code, out := httpJSON(t, http.MethodPut, base+"/intake-judgements", analystTok,
		bytes.NewBufferString(fmt.Sprintf(`{"judgements":[{"candidate_key":"y-1","tier":"recommend",
			"image_seen":true,"dims":%s}]}`, dimsJSON("yes"))), "application/json"); code != 403 {
		t.Fatalf("非参与角色应被拒：%d %v", code, out)
	}

	// 只读台账
	code, out = httpJSON(t, http.MethodGet, base+"/intake-judgements", analystTok, nil, "")
	if code != 200 || len(out["judgements"].([]any)) != 3 {
		t.Fatalf("list judgements: %d %v", code, out)
	}

	// 采集轮的 tool 可以填 csw_api 了
	if code, out := httpJSON(t, http.MethodPut, base+"/sweeps", collectorTok,
		bytes.NewBufferString(`{"sweeps":[{"sweep_key":"csw-window","platform":"instagram",
			"source_key":"channel","tool":"csw_api","found":438,"in_window":358,"result":"ok"}]}`),
		"application/json"); code != 200 {
		t.Fatalf("csw_api 应在白名单里：%d %v", code, out)
	}
}
