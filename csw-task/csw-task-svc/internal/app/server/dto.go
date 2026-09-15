package server

import (
	"fmt"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func (s *Server) fileURL(id int64) string {
	return fmt.Sprintf("%s/api/v1/files/%d", s.cfg.BaseURL, id)
}

type taskDTO struct {
	StageCode      string `json:"stage_code"`
	StageName      string `json:"stage_name"`
	RoleCode       string `json:"role_code"`
	OutputType     string `json:"output_type"`
	Status         string `json:"status"`
	ActionClass    string `json:"action_class"`
	DispatchMode   string `json:"dispatch_mode,omitempty"`
	ItemKey        string `json:"item_key"`
	FailReason     string `json:"fail_reason,omitempty"`
	DueAt          string `json:"due_at,omitempty"`
	DispatchedAt   string `json:"dispatched_at,omitempty"`
	StartedAt      string `json:"started_at,omitempty"`
	CompletedAt    string `json:"completed_at,omitempty"`
	LastActivityAt string `json:"last_activity_at,omitempty"`
	AssigneeID     *int64 `json:"assignee_id"`
	ID             int64  `json:"id"`
	RunID          int64  `json:"run_id"`
	Seq            int    `json:"seq"`
	CurVersion     int    `json:"cur_version"`
	SLAMinutes     int    `json:"sla_minutes,omitempty"`
	IsMerge        bool   `json:"is_merge"`
	ReworkPending  bool   `json:"rework_pending"`
}

func toTaskDTO(t domain.Task) taskDTO {
	ac := t.ActionClass
	if ac == "" {
		ac = domain.ActionRead
	}
	return taskDTO{
		ID: t.ID, RunID: t.RunID, StageCode: t.StageCode, StageName: t.StageName,
		Seq: t.Seq, RoleCode: t.RoleCode, OutputType: t.OutputType, IsMerge: t.IsMerge, Status: string(t.Status),
		CurVersion: t.CurVersion, AssigneeID: t.AssigneeID,
		ActionClass: ac, DispatchMode: string(t.DispatchMode), SLAMinutes: t.SLAMinutes,
		ItemKey: t.ItemKey, FailReason: t.FailReason, DueAt: t.DueAt, DispatchedAt: t.DispatchedAt,
		StartedAt: t.StartedAt, CompletedAt: t.CompletedAt, LastActivityAt: t.LastActivityAt, ReworkPending: t.ReworkPending,
	}
}

// authorizationDTO run 授权记录；status 为 active / revoked / expired。
type authorizationDTO struct {
	Scope       string `json:"scope"`
	SourceQuote string `json:"source_quote"`
	GrantedAt   string `json:"granted_at"`
	ExpiresAt   string `json:"expires_at,omitempty"`
	RevokedAt   string `json:"revoked_at,omitempty"`
	Status      string `json:"status"`
	GrantedBy   *int64 `json:"granted_by"`
	ID          int64  `json:"id"`
}

func toAuthorizationDTO(a domain.RunAuthorization, now string) authorizationDTO {
	st := "active"
	switch {
	case a.RevokedAt != "":
		st = "revoked"
	case a.ExpiresAt != "" && a.ExpiresAt <= now:
		st = "expired"
	}
	return authorizationDTO{
		ID: a.ID, Scope: string(a.Scope), SourceQuote: a.SourceQuote, GrantedAt: a.GrantedAt,
		ExpiresAt: a.ExpiresAt, RevokedAt: a.RevokedAt, Status: st, GrantedBy: a.GrantedBy,
	}
}

// reviewDTO 一条审核记录（task 详情随交付物带出最新一条，返工不再依赖群消息原文）。
type reviewDTO struct {
	Verdict         string `json:"verdict"`
	Comment         string `json:"comment,omitempty"`
	ReturnDirection string `json:"return_direction,omitempty"`
	ReturnLocation  string `json:"return_location,omitempty"`
	GateName        string `json:"gate_name,omitempty"`
	GateOrder       int    `json:"gate_order"`
}

type upstreamDTO struct {
	Label      string `json:"label"`
	URL        string `json:"url"`
	UpstreamID *int64 `json:"upstream_id"`
}

type deliverableDTO struct {
	DocType        string        `json:"doc_type"`
	DownloadURL    string        `json:"download_url"`
	Filename       string        `json:"filename"`
	Title          string        `json:"title"`
	Summary        string        `json:"summary"`
	SelfCheck      string        `json:"self_check"`
	EditorNote     string        `json:"editor_note"`
	Status         string        `json:"status"`
	Upstreams      []upstreamDTO `json:"upstreams,omitempty"`
	LatestReview   *reviewDTO    `json:"latest_review,omitempty"`
	ReturnedAtGate *int          `json:"returned_at_gate"`
	FileID         *int64        `json:"file_id"`
	ID             int64         `json:"id"`
	Version        int           `json:"version"`
	CurGate        int           `json:"cur_gate"`
	IsDispatch     bool          `json:"is_dispatch"`
}

func toDeliverableDTO(d domain.Deliverable, ups []domain.Upstream) deliverableDTO {
	dto := deliverableDTO{
		ID: d.ID, Version: d.Version, IsDispatch: d.IsDispatch, DocType: d.DocType,
		FileID: d.FileID, DownloadURL: d.DownloadURL, Filename: d.Filename, Title: d.Title,
		Summary: d.Summary, SelfCheck: d.SelfCheck, EditorNote: d.EditorNote,
		CurGate: d.CurGate, ReturnedAtGate: d.ReturnedAtGate, Status: string(d.Status),
	}
	for _, u := range ups {
		dto.Upstreams = append(dto.Upstreams, upstreamDTO{Label: u.Label, URL: u.UpstreamURL, UpstreamID: u.UpstreamID})
	}
	return dto
}

type gateDTO struct {
	ReviewerRole string `json:"reviewer_role"`
	Name         string `json:"name"`
	GateOrder    int    `json:"gate_order"`
	RelayedByHub bool   `json:"relayed_by_hub"`
}
