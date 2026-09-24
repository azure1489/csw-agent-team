package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"sort"
	"strings"
	"time"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// maxSweepsPerReport 单次上报的采集轮上限：写连接只有一个，长事务会挤住 notifier 与定时触发。
const maxSweepsPerReport = 200

// SweepInput 一轮采集的上报入参。
type SweepInput struct {
	SweepKey      string
	Platform      string
	SourceKey     string
	Tool          string
	Query         string
	StartedAt     string
	EndedAt       string
	WindowFrom    string
	WindowTo      string
	Result        string
	Error         string
	TaskID        *int64
	Found         int
	FetchedUnique int
	Reviewed      int
	Unreviewed    int
	Corroborated  int
	InWindow      int
	Registered    int
	PagedToEnd    bool
}

// ReportSweeps 批量上报采集轮（本 run 的参与角色或中枢）：(run_id, sweep_key) 幂等，同键重报即更新。
// 台账不在这里回填——若「扫过的源」自动成为基准，覆盖率就会自我实现，永远 100%。
func (e *Engine) ReportSweeps(ctx context.Context, actor domain.Agent, role domain.Role, runID int64, sweeps []SweepInput) ([]domain.IntakeSweep, error) {
	if len(sweeps) == 0 {
		return nil, domain.BadRequest("sweeps_required", "sweeps 不能为空")
	}
	if len(sweeps) > maxSweepsPerReport {
		return nil, domain.BadRequest("too_many_sweeps", fmt.Sprintf("单次最多上报 %d 轮采集，收到 %d", maxSweepsPerReport, len(sweeps)))
	}
	for _, sw := range sweeps {
		if strings.TrimSpace(sw.SweepKey) == "" {
			return nil, domain.BadRequest("bad_sweep_key", "sweep_key 不能为空：同键重报即更新，用它标识一轮采集")
		}
		if !domain.ValidSourcePlatform(sw.Platform) {
			return nil, domain.BadRequest("bad_sweep_platform", "platform 须为 instagram / xhs / web / other，收到："+sw.Platform)
		}
		if sw.Tool != "" && !domain.ValidSweepTool(sw.Tool) {
			return nil, domain.BadRequest("bad_sweep_tool", "tool 须为 csw_mcp / csw_api / opencli / webfetch / other，收到："+sw.Tool)
		}
		if sw.Result != "" && !domain.ValidSweepResult(sw.Result) {
			return nil, domain.BadRequest("bad_sweep_result", "result 须为 ok / failed / partial，收到："+sw.Result)
		}
		if sw.Result == "failed" && strings.TrimSpace(sw.Error) == "" {
			return nil, domain.BadRequest("sweep_error_required", "失败的采集轮须写 error，否则无法判断是没找到还是没去找："+sw.SweepKey)
		}
	}
	var out []domain.IntakeSweep
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, err := q.GetRun(ctx, runID)
		if isNoRows(err) {
			return domain.NotFound("run_not_found", "无此实例")
		}
		if err != nil {
			return err
		}
		if run.Status != domain.RunActive {
			return domain.Conflict("run_not_active", "仅进行中的 run 可上报采集轮")
		}
		wf, err := q.GetWorkflow(ctx, run.WorkflowID)
		if err != nil {
			return err
		}
		if err := requireRunParticipant(ctx, q, wf, role, runID, "上报采集轮"); err != nil {
			return err
		}
		for _, sw := range sweeps {
			tool, result := sw.Tool, sw.Result
			if tool == "" {
				tool = "other"
			}
			if result == "" {
				result = "ok"
			}
			if err := q.UpsertSweep(ctx, domain.IntakeSweep{
				RunID: runID, TaskID: sw.TaskID, SweepKey: sw.SweepKey, Platform: sw.Platform, SourceKey: sw.SourceKey,
				Tool: tool, Query: sw.Query, StartedAt: sw.StartedAt, EndedAt: sw.EndedAt,
				WindowFrom: sw.WindowFrom, WindowTo: sw.WindowTo, Found: sw.Found, InWindow: sw.InWindow,
				Registered: sw.Registered, FetchedUnique: sw.FetchedUnique, Reviewed: sw.Reviewed, Unreviewed: sw.Unreviewed, Corroborated: sw.Corroborated,
				Result: result, Error: sw.Error, PagedToEnd: sw.PagedToEnd,
				ActorID: &actor.ID, RoleCode: role.Code,
			}); err != nil {
				return err
			}
			if result == "ok" && sw.SourceKey != "" {
				if err := q.TouchIntakeSourceOK(ctx, sw.Platform, sw.SourceKey); err != nil {
					return err
				}
			}
		}
		out, err = q.ListSweepsByRun(ctx, runID)
		return err
	})
	return out, err
}

// IntakeTrace 一期的采集与判断全貌：供 02 / 03 与人直接读，不必解压交付物。
type IntakeTrace struct {
	RunID    int64                 `json:"run_id"`
	Subject  string                `json:"subject"`
	Sweeps   []domain.IntakeSweep  `json:"sweeps"`
	Items    []domain.RunItem      `json:"items"`
	Traces   []domain.ItemTrace    `json:"traces"`
	Sources  []domain.IntakeSource `json:"sources"`
	Coverage []CoverageRow         `json:"coverage"`
	// 判断台账。工作台接管 01 之后才有；旧 run 一律为空，判据据此走旧逻辑。
	Judgements []domain.IntakeJudgement `json:"judgements"`
}

// CoverageRow 一个来源本期被扫的情况。
type CoverageRow struct {
	Platform  string `json:"platform"`
	SourceKey string `json:"source_key"`
	Name      string `json:"name"`
	Required  bool   `json:"required"`
	Sweeps    int    `json:"sweeps"`
	OK        int    `json:"ok"`
	Failed    int    `json:"failed"`
	Found     int    `json:"found"`
	InWindow  int    `json:"in_window"`
}

// BuildIntakeTrace 汇总某 run 的采集轮、条目、判断轨迹与来源覆盖。纯只读，不开事务。
// 旧 run 没有这些记录时一律返回空切片而不是 nil，也不报错。
func BuildIntakeTrace(ctx context.Context, q *sqlite.Queries, runID int64) (IntakeTrace, error) {
	out := IntakeTrace{RunID: runID,
		Sweeps: make([]domain.IntakeSweep, 0), Items: make([]domain.RunItem, 0),
		Traces: make([]domain.ItemTrace, 0), Sources: make([]domain.IntakeSource, 0), Coverage: make([]CoverageRow, 0),
		Judgements: make([]domain.IntakeJudgement, 0)}
	run, err := q.GetRun(ctx, runID)
	if isNoRows(err) {
		return out, domain.NotFound("run_not_found", "无此实例")
	}
	if err != nil {
		return out, err
	}
	out.Subject = run.Subject
	if out.Sweeps, err = q.ListSweepsByRun(ctx, runID); err != nil {
		return out, err
	}
	items, err := q.ListItems(ctx, runID)
	if err != nil {
		return out, err
	}
	if items != nil {
		out.Items = items
	}
	if out.Traces, err = q.ListTracesByRun(ctx, runID); err != nil {
		return out, err
	}
	if out.Sources, err = q.ListIntakeSources(ctx, true); err != nil {
		return out, err
	}
	js, err := q.ListJudgementsByRun(ctx, runID)
	if err != nil {
		return out, err
	}
	if js != nil {
		out.Judgements = js
	}
	out.Coverage = coverage(out.Sources, out.Sweeps)
	return out, nil
}

// coverage 按 (platform, source_key) 对账：台账给分母，采集轮给分子。
// 台账里没有、但实际扫过的来源也列出来（Required=false），便于回填时人工确认。
func coverage(sources []domain.IntakeSource, sweeps []domain.IntakeSweep) []CoverageRow {
	idx := map[string]*CoverageRow{}
	key := func(p, k string) string { return p + "\x00" + k }
	rows := make([]*CoverageRow, 0, len(sources)+len(sweeps))
	for _, s := range sources {
		r := &CoverageRow{Platform: s.Platform, SourceKey: s.SourceKey, Name: s.Name, Required: s.Required}
		idx[key(s.Platform, s.SourceKey)] = r
		rows = append(rows, r)
	}
	for _, sw := range sweeps {
		// 通道级台账（source_key=channel）兜住该平台下所有未单独登记的来源。
		r, ok := idx[key(sw.Platform, sw.SourceKey)]
		if !ok {
			r, ok = idx[key(sw.Platform, "channel")]
		}
		if !ok {
			r = &CoverageRow{Platform: sw.Platform, SourceKey: sw.SourceKey, Name: "（不在台账）"}
			idx[key(sw.Platform, sw.SourceKey)] = r
			rows = append(rows, r)
		}
		r.Sweeps++
		r.Found += sw.Found
		r.InWindow += sw.InWindow
		switch sw.Result {
		case "ok":
			r.OK++
		case "failed":
			r.Failed++
		}
	}
	out := make([]CoverageRow, 0, len(rows))
	for _, r := range rows {
		out = append(out, *r)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Platform != out[j].Platform {
			return out[i].Platform < out[j].Platform
		}
		return out[i].SourceKey < out[j].SourceKey
	})
	return out
}

// IntakeCheck 一条判据的结论。Skipped 表示这一期没有相应记录（旧 run），不判定也不算失败。
type IntakeCheck struct {
	Name    string `json:"name"`
	Detail  string `json:"detail"`
	OK      bool   `json:"ok"`
	Warn    bool   `json:"warn"`
	Skipped bool   `json:"skipped"`
}

// BuildIntakeCheck 对一期的采集与判断跑八条判据，回答「这次采集是否合理」。
// 对没有上报记录的旧 run 一律 Skipped，绝不报错——明早那一期之前的 run 都属于这种。
func BuildIntakeCheck(ctx context.Context, q *sqlite.Queries, runID int64) ([]IntakeCheck, error) {
	tr, err := BuildIntakeTrace(ctx, q, runID)
	if err != nil {
		return nil, err
	}
	winFrom, winTo := runWindow(ctx, q, runID, tr.Sweeps)
	hasSweeps, hasTraces := len(tr.Sweeps) > 0, len(tr.Traces) > 0
	out := make([]IntakeCheck, 0, 8)

	out = append(out, checkCoverage(tr, hasSweeps))
	// 有判断台账就换一套对账口径。
	//
	// 为什么必须分支：旧的 checkYield 拿「已审 − 登记」对账，假设的是「审过的就该登记」。
	// 工作台是**每条都判**——一期审 358 条、只登记二十来条，剩下的都判了「不推荐」并留在台账里。
	// 拿旧判据去套，这个落差会永远是三百多，每期必判红。
	// 新口径对的是「判过的条数 = 窗口内候选数」：一条都不许漏判。
	if len(tr.Judgements) > 0 {
		out = append(out, checkJudgedAll(tr))
		out = append(out, checkPendingCheckConsistency(tr))
	} else {
		out = append(out, checkYield(tr, hasSweeps, hasTraces))
	}
	out = append(out, checkWindow(tr, winFrom, winTo))
	out = append(out, checkDropExplained(tr, hasTraces))
	out = append(out, checkDedup(tr))
	out = append(out, checkEvidence(tr))
	out = append(out, checkFallback(tr, hasSweeps))
	out = append(out, checkSourceSpread(tr))
	return out, nil
}

// checkJudgedAll 每条都判：台账行数要盖住窗口内的候选数。
//
// 只在有判断台账时才跑。分母取 csw_api 采集轮的**窗口内**条数（in_window）。
// 不能取去重获取数（fetched_unique）：工作台按发布时间取一个更宽的窗口、再在本地按首次入库时间收口，
// fetched_unique 是收口之前的数——r53 取到 1274、窗口内 99、判了 99，拿 1274 去比就误报「漏判 1175」。
// 没有 csw_api 采集轮就不判定：别的采集方式没有这个计数，硬套会得出假结论。
func checkJudgedAll(tr IntakeTrace) IntakeCheck {
	c := IntakeCheck{Name: "每条都判：窗口内的候选一条都没漏"}
	unique, unreviewed, hasAPI := 0, 0, false
	for _, sw := range tr.Sweeps {
		if sw.Tool != "csw_api" {
			continue
		}
		hasAPI = true
		unique += sw.InWindow
		unreviewed += sw.Unreviewed
	}
	judged, carried, pending := 0, 0, 0
	for _, j := range tr.Judgements {
		judged++
		if j.Carried {
			carried++
		}
		if j.Tier == "pending_check" {
			pending++
		}
	}
	c.Detail = fmt.Sprintf("台账 %d 条（其中已判结转 %d、待核 %d）", judged, carried, pending)
	if !hasAPI {
		c.Skipped = true
		c.Detail += "；本期没有 csw_api 采集轮，没有去重候选数可比，不判定"
		return c
	}
	c.Detail += fmt.Sprintf("；csw_api 窗口内候选 %d、未审 %d", unique, unreviewed)
	if unreviewed > 0 {
		c.Detail += "——工作台每条都判，未审应当为 0；有未审说明这一步没跑完"
		return c
	}
	if unique > 0 && judged < unique {
		c.Detail += fmt.Sprintf("；漏判 %d 条", unique-judged)
		return c
	}
	c.OK = true
	return c
}

// checkPendingCheckConsistency 没读到实图的必须落待核。
//
// 这是口径里的硬规则：待核不是淘汰，补齐图之后要能重评。
// 引擎在上报时已经拒了违规的，库里也有 CHECK；这一条是给人看的第三道——
// 它同时报出「有多少条因为没图而悬着」，那是明天要补的活。
func checkPendingCheckConsistency(tr IntakeTrace) IntakeCheck {
	c := IntakeCheck{Name: "待核一致：没读到实图的都落了待核"}
	unseen, bad := 0, []string{}
	for _, j := range tr.Judgements {
		if j.ImageSeen {
			continue
		}
		unseen++
		if j.Tier != "pending_check" {
			bad = append(bad, j.CandidateKey)
		}
	}
	if unseen == 0 {
		c.OK, c.Detail = true, "每条都读到了实图"
		return c
	}
	if len(bad) > 0 {
		c.Detail = fmt.Sprintf("%d 条没读到实图，其中 %d 条却不是待核：%s",
			unseen, len(bad), strings.Join(bad, "、"))
		return c
	}
	c.OK, c.Warn = true, true
	c.Detail = fmt.Sprintf("%d 条没读到实图，都已落待核——补齐图后要重评，不得就此淘汰", unseen)
	return c
}

func checkCoverage(tr IntakeTrace, hasSweeps bool) IntakeCheck {
	c := IntakeCheck{Name: "来源覆盖：必扫的源是否都扫过"}
	if !hasSweeps {
		c.Skipped, c.Detail = true, "本期没有上报采集轮，不判定"
		return c
	}
	if len(tr.Sources) == 0 {
		c.Skipped, c.Detail = true, "来源台账为空，没有基准可比"
		return c
	}
	var missed []string
	req, opt, optCovered := 0, 0, 0
	for _, r := range tr.Coverage {
		if r.Required {
			req++
			if r.Sweeps == 0 {
				missed = append(missed, r.Platform+"/"+r.SourceKey)
			}
			continue
		}
		if r.Name != "（不在台账）" {
			opt++
			if r.Sweeps > 0 {
				optCovered++
			}
		}
	}
	if len(missed) > 0 {
		c.Detail = fmt.Sprintf("必扫 %d 个来源，有 %d 个本期一轮都没有：%s", req, len(missed), strings.Join(missed, "、"))
		return c
	}
	c.OK = true
	c.Detail = fmt.Sprintf("必扫 %d 个来源全部有采集轮", req)
	if opt > 0 {
		pct := optCovered * 100 / opt
		c.Detail += fmt.Sprintf("；其余 %d 个覆盖 %d%%", opt, pct)
		if pct < 60 {
			c.Warn = true
		}
	}
	return c
}

func checkYield(tr IntakeTrace, hasSweeps, hasTraces bool) IntakeCheck {
	c := IntakeCheck{Name: "产出落差：已审的都落了状态"}
	if !hasSweeps {
		c.Skipped, c.Detail = true, "本期没有上报采集轮，不判定"
		return c
	}
	found, unique, reviewed, unreviewed, corroborated, inWindow := 0, 0, 0, 0, 0, 0
	for _, sw := range tr.Sweeps {
		found += sw.Found
		unique += sw.FetchedUnique
		reviewed += sw.Reviewed
		unreviewed += sw.Unreviewed
		corroborated += sw.Corroborated
		inWindow += sw.InWindow
	}
	// 佐证页（为核实线索去读的官方页、原始出处）算已审但不该成为候选，从对账基数里扣掉。
	candidates := reviewed - corroborated
	if candidates < 0 {
		candidates = 0
	}
	dropped := 0
	for _, it := range tr.Items {
		if it.Status == domain.ItemDropped {
			dropped++
		}
	}
	c.Detail = fmt.Sprintf("接口返回 %d、去重 %d、已审 %d（其中佐证 %d）、未审 %d、窗口内 %d、登记 %d（其中淘汰 %d）",
		found, unique, reviewed, corroborated, unreviewed, inWindow, len(tr.Items), dropped)
	// 拿「已审」对账，不拿「接口返回」对账：获取多不等于漏登记，审过却没落状态才是漏。
	if reviewed == 0 && unreviewed == 0 {
		c.Skipped = true
		c.Detail += "；本期没有分开上报已审与未审，不判定"
		return c
	}
	if gap := candidates - len(tr.Items); gap > 0 {
		if !hasTraces {
			c.Skipped = true
			c.Detail += fmt.Sprintf("；已审（扣除佐证）比登记多 %d 条，本期没有判断轨迹，不判定", gap)
			return c
		}
		c.Detail += fmt.Sprintf("；已审（扣除佐证）比登记多 %d 条——审过的候选就该落状态，不能只留在本地", gap)
		return c
	}
	if unreviewed > 0 {
		c.OK, c.Warn = true, true
		c.Detail += fmt.Sprintf("；另有 %d 条只加载未展开，已如实记为未审（不得事后补成淘汰）", unreviewed)
		return c
	}
	c.OK = true
	return c
}

func checkWindow(tr IntakeTrace, from, to time.Time) IntakeCheck {
	c := IntakeCheck{Name: "窗口合规：披露时间在窗口内、抓取不早于披露"}
	if len(tr.Items) == 0 {
		c.Skipped, c.Detail = true, "本期没有条目，不判定"
		return c
	}
	var bad, unparsed, missing []string
	for _, it := range tr.Items {
		if strings.TrimSpace(it.PublishedAt) == "" {
			missing = append(missing, it.ItemKey)
			continue
		}
		pub, ok := parseLoose(it.PublishedAt)
		if !ok {
			unparsed = append(unparsed, it.ItemKey)
			continue
		}
		if !from.IsZero() && pub.Before(from) {
			bad = append(bad, it.ItemKey+"（早于窗口）")
			continue
		}
		if !to.IsZero() && pub.After(to) {
			bad = append(bad, it.ItemKey+"（晚于窗口）")
			continue
		}
		if f, ok := parseLoose(it.FetchedAt); ok && f.Before(pub) {
			bad = append(bad, it.ItemKey+"（抓取早于披露）")
		}
	}
	if len(bad) > 0 {
		c.Detail = fmt.Sprintf("%d 条越界：%s", len(bad), strings.Join(bad, "、"))
		if len(missing) > 0 {
			c.Detail += fmt.Sprintf("；另有 %d 条没写原始披露时间：%s", len(missing), strings.Join(missing, "、"))
		}
		return c
	}
	judged := len(tr.Items) - len(unparsed) - len(missing)
	c.OK = true
	c.Detail = fmt.Sprintf("%d 条条目的时间都在窗口内", judged)
	if len(missing) > 0 {
		c.Warn = true
		c.Detail += fmt.Sprintf("；%d 条没写原始披露时间，无法判断是否属于当期：%s", len(missing), strings.Join(missing, "、"))
	}
	if len(unparsed) > 0 {
		c.Warn = true
		c.Detail += fmt.Sprintf("；%d 条时间格式无法解析，未判定：%s", len(unparsed), strings.Join(unparsed, "、"))
	}
	return c
}

func checkDropExplained(tr IntakeTrace, hasTraces bool) IntakeCheck {
	c := IntakeCheck{Name: "淘汰交代：没入选的条目都有理由"}
	if !hasTraces {
		c.Skipped, c.Detail = true, "本期没有判断轨迹，不判定"
		return c
	}
	explained := map[string]bool{}
	for _, t := range tr.Traces {
		if t.ReasonCode != "" {
			explained[t.ItemKey] = true
		}
	}
	var naked []string
	for _, it := range tr.Items {
		if it.Status != domain.ItemDropped {
			continue
		}
		if !explained[it.ItemKey] {
			naked = append(naked, it.ItemKey)
		}
	}
	if len(naked) > 0 {
		c.Detail = fmt.Sprintf("%d 条淘汰没有理由码：%s", len(naked), strings.Join(naked, "、"))
		return c
	}
	c.OK = true
	c.Detail = fmt.Sprintf("轨迹 %d 条，淘汰条目都写了理由码", len(tr.Traces))
	return c
}

func checkDedup(tr IntakeTrace) IntakeCheck {
	c := IntakeCheck{Name: "查重对照：每条都写了对照结论"}
	if len(tr.Items) == 0 {
		c.Skipped, c.Detail = true, "本期没有条目，不判定"
		return c
	}
	var missing []string
	for _, it := range tr.Items {
		if it.Status == domain.ItemDropped {
			continue
		}
		if strings.TrimSpace(it.DedupNote) == "" {
			missing = append(missing, it.ItemKey)
		}
	}
	if len(missing) > 0 {
		c.OK, c.Warn = true, true
		c.Detail = fmt.Sprintf("%d 条没写查重结论：%s（提醒，不判失败）", len(missing), strings.Join(missing, "、"))
		return c
	}
	c.OK, c.Detail = true, "在选条目都写了查重结论"
	return c
}

func checkEvidence(tr IntakeTrace) IntakeCheck {
	c := IntakeCheck{Name: "证据可核：预览或截图地址可解析"}
	if len(tr.Items) == 0 {
		c.Skipped, c.Detail = true, "本期没有条目，不判定"
		return c
	}
	var bad []string
	for _, it := range tr.Items {
		if it.Status == domain.ItemDropped {
			continue
		}
		u := strings.TrimSpace(it.EvidenceURL)
		if u == "" {
			bad = append(bad, it.ItemKey+"（空）")
			continue
		}
		p, err := url.Parse(u)
		if err != nil || p.Scheme != "https" || p.Host == "" {
			bad = append(bad, it.ItemKey+"（非 https 或无法解析）")
		}
	}
	if len(bad) > 0 {
		c.OK, c.Warn = true, true
		c.Detail = fmt.Sprintf("%d 条证据地址有问题：%s（提醒，不判失败）", len(bad), strings.Join(bad, "、"))
		return c
	}
	c.OK, c.Detail = true, "在选条目都有可解析的证据地址"
	return c
}

func checkFallback(tr IntakeTrace, hasSweeps bool) IntakeCheck {
	c := IntakeCheck{Name: "失败换源：某个平台不是整轮全败"}
	if !hasSweeps {
		c.Skipped, c.Detail = true, "本期没有上报采集轮，不判定"
		return c
	}
	type stat struct{ ok, failed int }
	byPlatform := map[string]*stat{}
	for _, sw := range tr.Sweeps {
		s := byPlatform[sw.Platform]
		if s == nil {
			s = &stat{}
			byPlatform[sw.Platform] = s
		}
		if sw.Result == "failed" {
			s.failed++
		} else {
			s.ok++
		}
	}
	var dead []string
	for p, s := range byPlatform {
		if s.ok == 0 && s.failed > 0 {
			dead = append(dead, fmt.Sprintf("%s（%d 轮全败）", p, s.failed))
		}
	}
	sort.Strings(dead)
	if len(dead) > 0 {
		c.Detail = "有平台没有取得任何成功采集：" + strings.Join(dead, "、")
		return c
	}
	c.OK, c.Detail = true, "每个动过的平台都至少有一轮成功"
	return c
}

func checkSourceSpread(tr IntakeTrace) IntakeCheck {
	c := IntakeCheck{Name: "来源分布：条目是否集中在单一平台"}
	if len(tr.Items) == 0 {
		c.Skipped, c.Detail = true, "本期没有条目，不判定"
		return c
	}
	platformOf := map[string]string{}
	for _, sw := range tr.Sweeps {
		platformOf[sw.SweepKey] = sw.Platform
	}
	count := map[string]int{}
	unknown := 0
	for _, it := range tr.Items {
		p := platformOf[it.DiscoveredVia]
		if p == "" {
			unknown++
			continue
		}
		count[p]++
	}
	if len(count) == 0 {
		c.Skipped, c.Detail = true, fmt.Sprintf("%d 条条目都没写来自哪一轮采集，无法统计分布", unknown)
		return c
	}
	keys := make([]string, 0, len(count))
	for p := range count {
		keys = append(keys, p)
	}
	sort.Strings(keys)
	parts := make([]string, 0, len(keys))
	top := 0
	for _, p := range keys {
		parts = append(parts, fmt.Sprintf("%s %d 条", p, count[p]))
		if count[p] > top {
			top = count[p]
		}
	}
	known := len(tr.Items) - unknown
	c.OK = true
	c.Detail = strings.Join(parts, "、")
	if unknown > 0 {
		c.Detail += fmt.Sprintf("；%d 条未注明来源轮次", unknown)
	}
	if known > 0 && top*100/known >= 90 && len(tr.Sources) > 1 {
		c.Warn = true
		c.Detail += "；高度集中在单一平台，其余通道等于没产出"
	}
	return c
}

// runWindow 取当期窗口：优先 run_created 事件里 inputs 记的窗口，其次采集轮自报的窗口。
// 两者都没有就返回零值，窗口判据只做「抓取不早于披露」这一半。
func runWindow(ctx context.Context, q *sqlite.Queries, runID int64, sweeps []domain.IntakeSweep) (time.Time, time.Time) {
	var from, to time.Time
	if evs, err := q.ListEventsByRun(ctx, runID); err == nil {
		for _, ev := range evs {
			if ev.Type != domain.EvtRunCreated || ev.DetailJSON == "" {
				continue
			}
			f, t := windowFromInputs(ev.DetailJSON)
			if !f.IsZero() {
				from = f
			}
			if !t.IsZero() {
				to = t
			}
		}
	}
	if from.IsZero() || to.IsZero() {
		for _, sw := range sweeps {
			if from.IsZero() {
				if f, ok := parseLoose(sw.WindowFrom); ok {
					from = f
				}
			}
			if to.IsZero() {
				if t, ok := parseLoose(sw.WindowTo); ok {
					to = t
				}
			}
		}
	}
	return from, to
}

// windowFromInputs 从 run inputs 的 JSON 里捞窗口起止；字段名不固定，宽松匹配，取不到就算了。
func windowFromInputs(js string) (time.Time, time.Time) {
	var from, to time.Time
	for _, kv := range []struct {
		keys []string
		dst  *time.Time
	}{
		{[]string{`"起"`, `"from"`, `"window_from"`, `"起始"`}, &from},
		{[]string{`"止"`, `"to"`, `"window_to"`, `"截止"`}, &to},
	} {
		for _, k := range kv.keys {
			i := strings.Index(js, k)
			if i < 0 {
				continue
			}
			rest := js[i+len(k):]
			j := strings.Index(rest, `"`)
			if j < 0 {
				continue
			}
			rest = rest[j+1:]
			e := strings.Index(rest, `"`)
			if e <= 0 {
				continue
			}
			if t, ok := parseLoose(rest[:e]); ok {
				*kv.dst = t
				break
			}
		}
	}
	return from, to
}

// parseLoose 宽松解析时间：agent 写的格式不统一，解析不了的当「未判定」，不当违规。
func parseLoose(s string) (time.Time, bool) {
	s = strings.TrimSpace(s)
	if s == "" {
		return time.Time{}, false
	}
	for _, layout := range []string{
		time.RFC3339, "2006-01-02T15:04:05", "2006-01-02 15:04:05", "2006-01-02 15:04",
		"2006-01-02T15:04", "2006-01-02", "2006/01/02 15:04", "2006/01/02",
	} {
		if t, err := time.Parse(layout, s); err == nil {
			return t.UTC(), true
		}
	}
	return time.Time{}, false
}

// maxJudgementsPerReport 单次上报的判断上限。
// 一轮约 360 条候选，分两三批发完；写连接只有一个，长事务会挤住 notifier 与定时触发。
const maxJudgementsPerReport = 200

// JudgementInput 一条判断的上报入参。
type JudgementInput struct {
	CandidateKey   string
	ItemKey        string
	Platform       string
	PostRef        string
	SourceURL      string
	Tier           string
	DimsJSON       string
	ThreeJSON      string
	ComparisonJSON string
	HeatNote       string
	GapsJSON       string
	HitsJSON       string
	JevJSON        string
	RubricVersion  string
	ImageSeen      bool
	Carried        bool
}

// ReportJudgements 批量上报逐条判断（本 run 的参与角色或中枢）：(run_id, candidate_key) 幂等。
//
// 这里校验的三条都是口径里的硬规则，不是防手滑：
//   - 六个维度必须齐全，且每维都要有依据。编不出依据就判 unclear，不许空着。
//   - 没读到实图就必须落 pending_check：待核不是淘汰，补齐图后要能重评。
//   - 结论只有四档，没有分数。
func (e *Engine) ReportJudgements(ctx context.Context, actor domain.Agent, role domain.Role, runID int64, in []JudgementInput) ([]domain.IntakeJudgement, error) {
	if len(in) == 0 {
		return nil, domain.BadRequest("judgements_required", "judgements 不能为空")
	}
	if len(in) > maxJudgementsPerReport {
		return nil, domain.BadRequest("too_many_judgements",
			fmt.Sprintf("单次最多上报 %d 条判断，收到 %d", maxJudgementsPerReport, len(in)))
	}
	for _, j := range in {
		if strings.TrimSpace(j.CandidateKey) == "" {
			return nil, domain.BadRequest("bad_candidate_key", "candidate_key 不能为空：同键重报即覆盖，用它标识一条候选")
		}
		if !domain.ValidJudgementTier(j.Tier) {
			return nil, domain.BadRequest("bad_tier",
				"tier 须为 recommend / alternate / not_recommend / pending_check，收到："+j.Tier)
		}
		if j.Platform != "" && !domain.ValidSourcePlatform(j.Platform) {
			return nil, domain.BadRequest("bad_platform", "platform 须为 instagram / xhs / web / other，收到："+j.Platform)
		}
		if !j.ImageSeen && j.Tier != "pending_check" {
			return nil, domain.BadRequest("image_unseen_must_pend",
				"没读到实图的候选只能落 pending_check（待核不是淘汰，补齐后要重评）："+j.CandidateKey)
		}
		if err := validateDims(j.DimsJSON, j.CandidateKey); err != nil {
			return nil, err
		}
	}
	var out []domain.IntakeJudgement
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, err := q.GetRun(ctx, runID)
		if isNoRows(err) {
			return domain.NotFound("run_not_found", "无此实例")
		}
		if err != nil {
			return err
		}
		if run.Status != domain.RunActive {
			return domain.Conflict("run_not_active", "仅进行中的 run 可上报判断")
		}
		wf, err := q.GetWorkflow(ctx, run.WorkflowID)
		if err != nil {
			return err
		}
		if err := requireRunParticipant(ctx, q, wf, role, runID, "上报逐条判断"); err != nil {
			return err
		}
		for _, j := range in {
			platform := j.Platform
			if platform == "" {
				platform = "instagram"
			}
			if err := q.UpsertJudgement(ctx, domain.IntakeJudgement{
				RunID: runID, CandidateKey: j.CandidateKey, ItemKey: j.ItemKey, Platform: platform,
				PostRef: j.PostRef, SourceURL: j.SourceURL, Tier: j.Tier,
				DimsJSON: defaultJSON(j.DimsJSON, "{}"), ThreeJSON: defaultJSON(j.ThreeJSON, "{}"),
				ComparisonJSON: defaultJSON(j.ComparisonJSON, "{}"), HeatNote: j.HeatNote,
				GapsJSON: defaultJSON(j.GapsJSON, "[]"), HitsJSON: defaultJSON(j.HitsJSON, "{}"),
				JevJSON: defaultJSON(j.JevJSON, "{}"), RubricVersion: j.RubricVersion,
				ImageSeen: j.ImageSeen, Carried: j.Carried,
				ActorID: &actor.ID, RoleCode: role.Code,
			}); err != nil {
				return err
			}
		}
		out, err = q.ListJudgementsByRun(ctx, runID)
		return err
	})
	return out, err
}

// ListJudgements 只读：某 run 的全部判断。02 / 03 据此看台账，不必解压交付物。
func (e *Engine) ListJudgements(ctx context.Context, runID int64) ([]domain.IntakeJudgement, error) {
	var out []domain.IntakeJudgement
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		if _, err := q.GetRun(ctx, runID); isNoRows(err) {
			return domain.NotFound("run_not_found", "无此实例")
		} else if err != nil {
			return err
		}
		var err error
		out, err = q.ListJudgementsByRun(ctx, runID)
		return err
	})
	return out, err
}

func defaultJSON(s, fallback string) string {
	if strings.TrimSpace(s) == "" {
		return fallback
	}
	return s
}

// validateDims 六维齐全、每维有依据。
// 这条校验放在引擎而不是只放在采集服务里：引擎是唯一真相，
// 别的 agent 以后也可能上报判断，规则得长在这一侧。
func validateDims(dimsJSON, candidateKey string) error {
	if strings.TrimSpace(dimsJSON) == "" {
		return domain.BadRequest("dims_required", "dims_json 不能为空：六维与依据是判断的主体："+candidateKey)
	}
	var dims map[string]struct {
		Verdict string `json:"verdict"`
		Basis   string `json:"basis"`
	}
	if err := json.Unmarshal([]byte(dimsJSON), &dims); err != nil {
		return domain.BadRequest("bad_dims_json", "dims_json 不是合法 JSON："+candidateKey)
	}
	for _, key := range domain.JudgementDims {
		d, ok := dims[key]
		if !ok {
			return domain.BadRequest("dim_missing",
				fmt.Sprintf("缺维度 %s（六维不增不减）：%s", key, candidateKey))
		}
		if !domain.ValidDimVerdict(d.Verdict) {
			return domain.BadRequest("bad_dim_verdict",
				fmt.Sprintf("维度 %s 的 verdict 须为 yes / no / unclear，收到 %q：%s", key, d.Verdict, candidateKey))
		}
		if strings.TrimSpace(d.Basis) == "" {
			return domain.BadRequest("dim_basis_required",
				fmt.Sprintf("维度 %s 没给依据（编不出来就判 unclear，不许空着）：%s", key, candidateKey))
		}
	}
	return nil
}
