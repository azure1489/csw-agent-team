package server

import (
	"context"
	"encoding/json"
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
)

type reportSweepsReq struct {
	Sweeps []struct {
		SweepKey      string `json:"sweep_key"`
		Platform      string `json:"platform"`
		SourceKey     string `json:"source_key"`
		Tool          string `json:"tool"`
		Query         string `json:"query"`
		StartedAt     string `json:"started_at"`
		EndedAt       string `json:"ended_at"`
		WindowFrom    string `json:"window_from"`
		WindowTo      string `json:"window_to"`
		Found         int    `json:"found"`
		FetchedUnique int    `json:"fetched_unique"`
		Reviewed      int    `json:"reviewed"`
		Unreviewed    int    `json:"unreviewed"`
		Corroborated  int    `json:"corroborated"`
		InWindow      int    `json:"in_window"`
		Registered    int    `json:"registered"`
		Result        string `json:"result"`
		Error         string `json:"error"`
		PagedToEnd    bool   `json:"paged_to_end"`
		TaskID        *int64 `json:"task_id"`
	} `json:"sweeps"`
}

type sweepDTO struct {
	SweepKey      string `json:"sweep_key"`
	Platform      string `json:"platform"`
	SourceKey     string `json:"source_key,omitempty"`
	Tool          string `json:"tool"`
	Query         string `json:"query,omitempty"`
	StartedAt     string `json:"started_at,omitempty"`
	EndedAt       string `json:"ended_at,omitempty"`
	WindowFrom    string `json:"window_from,omitempty"`
	WindowTo      string `json:"window_to,omitempty"`
	Result        string `json:"result"`
	Error         string `json:"error,omitempty"`
	RoleCode      string `json:"role_code,omitempty"`
	Found         int    `json:"found"`
	FetchedUnique int    `json:"fetched_unique"`
	Reviewed      int    `json:"reviewed"`
	Unreviewed    int    `json:"unreviewed"`
	Corroborated  int    `json:"corroborated"`
	InWindow      int    `json:"in_window"`
	Registered    int    `json:"registered"`
	PagedToEnd    bool   `json:"paged_to_end,omitempty"`
}

func toSweepDTOs(sweeps []domain.IntakeSweep) []sweepDTO {
	out := make([]sweepDTO, 0, len(sweeps))
	for _, sw := range sweeps {
		out = append(out, sweepDTO{SweepKey: sw.SweepKey, Platform: sw.Platform, SourceKey: sw.SourceKey, Tool: sw.Tool,
			Query: sw.Query, StartedAt: sw.StartedAt, EndedAt: sw.EndedAt, WindowFrom: sw.WindowFrom, WindowTo: sw.WindowTo,
			Result: sw.Result, Error: sw.Error, RoleCode: sw.RoleCode, Found: sw.Found,
			FetchedUnique: sw.FetchedUnique, Reviewed: sw.Reviewed, Unreviewed: sw.Unreviewed, Corroborated: sw.Corroborated,
			InWindow: sw.InWindow, Registered: sw.Registered, PagedToEnd: sw.PagedToEnd})
	}
	return out
}

type traceDTO struct {
	ItemKey    string `json:"item_key"`
	FromStatus string `json:"from_status,omitempty"`
	ToStatus   string `json:"to_status"`
	ReasonCode string `json:"reason_code,omitempty"`
	Reason     string `json:"reason,omitempty"`
	ActorRole  string `json:"actor_role,omitempty"`
	QuoteRef   string `json:"quote_ref,omitempty"`
	CreatedAt  string `json:"created_at"`
}

func toTraceDTOs(traces []domain.ItemTrace) []traceDTO {
	out := make([]traceDTO, 0, len(traces))
	for _, tr := range traces {
		out = append(out, traceDTO{ItemKey: tr.ItemKey, FromStatus: tr.FromStatus, ToStatus: tr.ToStatus,
			ReasonCode: tr.ReasonCode, Reason: tr.Reason, ActorRole: tr.ActorRole, QuoteRef: tr.QuoteRef,
			CreatedAt: tr.CreatedAt})
	}
	return out
}

// PUT /runs/:id/sweeps —— 批量上报采集轮（参与角色或中枢；按 sweep_key 幂等）
func (s *Server) handleReportSweeps(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req reportSweepsReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	in := make([]engine.SweepInput, 0, len(req.Sweeps))
	for _, sw := range req.Sweeps {
		in = append(in, engine.SweepInput{SweepKey: sw.SweepKey, Platform: sw.Platform, SourceKey: sw.SourceKey,
			Tool: sw.Tool, Query: sw.Query, StartedAt: sw.StartedAt, EndedAt: sw.EndedAt,
			WindowFrom: sw.WindowFrom, WindowTo: sw.WindowTo, Found: sw.Found, InWindow: sw.InWindow,
			Registered: sw.Registered, FetchedUnique: sw.FetchedUnique, Reviewed: sw.Reviewed, Unreviewed: sw.Unreviewed, Corroborated: sw.Corroborated,
			Result: sw.Result, Error: sw.Error, PagedToEnd: sw.PagedToEnd, TaskID: sw.TaskID})
	}
	agent, role := mwAgent(c)
	sweeps, err := s.eng.ReportSweeps(c.Request.Context(), agent, role, id, in)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"sweeps": toSweepDTOs(sweeps)})
}

// GET /runs/:id/intake-trace —— 采集与判断全貌：采集轮、条目溯源、判断轨迹、来源覆盖。
// 02 / 03 据此判断采集是否合理，不必解压交付物；旧 run 没有记录时给空列表而不是报错。
func (s *Server) handleIntakeTrace(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	tr, err := engine.BuildIntakeTrace(c.Request.Context(), s.store.Q(), id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"run_id":   tr.RunID,
		"subject":  tr.Subject,
		"sweeps":   toSweepDTOs(tr.Sweeps),
		"items":    toItemDTOs(tr.Items),
		"traces":   toTraceDTOs(tr.Traces),
		"coverage": tr.Coverage,
	})
}

// intakeSources 启用中的来源台账，随 01 / 02 / 03 的任务详情下发（开工即可对账）。
// 照 recentPosts 的取舍：读不到只记日志，不影响任务详情主响应。
func intakeSources(ctx context.Context, s *Server) []gin.H {
	sources, err := s.store.Q().ListIntakeSources(ctx, true)
	if err != nil {
		s.log.Warn("intake sources", "err", err)
		return nil
	}
	out := make([]gin.H, 0, len(sources))
	for _, src := range sources {
		out = append(out, gin.H{"platform": src.Platform, "source_key": src.SourceKey, "name": src.Name,
			"entry_url": src.EntryURL, "required": src.Required, "note": src.Note})
	}
	return out
}

// ── 逐条判断 ──────────────────────────────────────────────────────────

type reportJudgementsReq struct {
	Judgements []struct {
		CandidateKey   string          `json:"candidate_key"`
		ItemKey        string          `json:"item_key"`
		Platform       string          `json:"platform"`
		PostRef        string          `json:"post_ref"`
		SourceURL      string          `json:"source_url"`
		Tier           string          `json:"tier"`
		Dims           json.RawMessage `json:"dims"`
		ThreeSentences json.RawMessage `json:"three_sentences"`
		Comparison     json.RawMessage `json:"comparison"`
		HeatNote       string          `json:"heat_note"`
		Gaps           json.RawMessage `json:"gaps"`
		Hits           json.RawMessage `json:"hits"`
		Jev            json.RawMessage `json:"jev"`
		RubricVersion  string          `json:"rubric_version"`
		ImageSeen      bool            `json:"image_seen"`
		Carried        bool            `json:"carried"`
	} `json:"judgements"`
}

// judgementDTO 出参。
// 没有分数字段，也不会有——口径是「不打数字分、无权重」。
type judgementDTO struct {
	CandidateKey   string          `json:"candidate_key"`
	ItemKey        string          `json:"item_key,omitempty"`
	Platform       string          `json:"platform"`
	PostRef        string          `json:"post_ref,omitempty"`
	SourceURL      string          `json:"source_url,omitempty"`
	Tier           string          `json:"tier"`
	Dims           json.RawMessage `json:"dims"`
	ThreeSentences json.RawMessage `json:"three_sentences"`
	Comparison     json.RawMessage `json:"comparison"`
	HeatNote       string          `json:"heat_note,omitempty"`
	Gaps           json.RawMessage `json:"gaps"`
	Hits           json.RawMessage `json:"hits"`
	Jev            json.RawMessage `json:"jev,omitempty"`
	RubricVersion  string          `json:"rubric_version,omitempty"`
	RoleCode       string          `json:"role_code,omitempty"`
	CreatedAt      string          `json:"created_at"`
	ImageSeen      bool            `json:"image_seen"`
	Carried        bool            `json:"carried,omitempty"`
}

func toJudgementDTOs(js []domain.IntakeJudgement) []judgementDTO {
	out := make([]judgementDTO, 0, len(js))
	for _, j := range js {
		out = append(out, judgementDTO{
			CandidateKey: j.CandidateKey, ItemKey: j.ItemKey, Platform: j.Platform, PostRef: j.PostRef,
			SourceURL: j.SourceURL, Tier: j.Tier,
			Dims: json.RawMessage(j.DimsJSON), ThreeSentences: json.RawMessage(j.ThreeJSON),
			Comparison: json.RawMessage(j.ComparisonJSON), HeatNote: j.HeatNote,
			Gaps: json.RawMessage(j.GapsJSON), Hits: json.RawMessage(j.HitsJSON),
			Jev: json.RawMessage(j.JevJSON), RubricVersion: j.RubricVersion, RoleCode: j.RoleCode,
			CreatedAt: j.CreatedAt, ImageSeen: j.ImageSeen, Carried: j.Carried,
		})
	}
	return out
}

func rawOrEmpty(r json.RawMessage, fallback string) string {
	if len(r) == 0 {
		return fallback
	}
	return string(r)
}

// PUT /runs/:id/intake-judgements —— 批量上报逐条判断（参与角色或中枢；按 candidate_key 幂等）
func (s *Server) handleReportJudgements(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req reportJudgementsReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	in := make([]engine.JudgementInput, 0, len(req.Judgements))
	for _, j := range req.Judgements {
		in = append(in, engine.JudgementInput{
			CandidateKey: j.CandidateKey, ItemKey: j.ItemKey, Platform: j.Platform, PostRef: j.PostRef,
			SourceURL: j.SourceURL, Tier: j.Tier,
			DimsJSON: rawOrEmpty(j.Dims, ""), ThreeJSON: rawOrEmpty(j.ThreeSentences, "{}"),
			ComparisonJSON: rawOrEmpty(j.Comparison, "{}"), HeatNote: j.HeatNote,
			GapsJSON: rawOrEmpty(j.Gaps, "[]"), HitsJSON: rawOrEmpty(j.Hits, "{}"),
			JevJSON: rawOrEmpty(j.Jev, "{}"), RubricVersion: j.RubricVersion,
			ImageSeen: j.ImageSeen, Carried: j.Carried,
		})
	}
	agent, role := mwAgent(c)
	js, err := s.eng.ReportJudgements(c.Request.Context(), agent, role, id, in)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"judgements": toJudgementDTOs(js)})
}

// GET /runs/:id/intake-judgements —— 判断台账。
// 02 / 03 据此看「每条都判了什么、凭什么」，不必解压交付物；旧 run 没有记录时给空列表。
func (s *Server) handleListJudgements(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	js, err := s.eng.ListJudgements(c.Request.Context(), id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"judgements": toJudgementDTOs(js)})
}
