package server

import (
	"fmt"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func (s *Server) fileURL(id int64) string {
	return fmt.Sprintf("%s/api/v1/files/%d", s.cfg.BaseURL, id)
}

type taskDTO struct {
	StageCode  string `json:"stage_code"`
	StageName  string `json:"stage_name"`
	RoleCode   string `json:"role_code"`
	Status     string `json:"status"`
	AssigneeID *int64 `json:"assignee_id"`
	ID         int64  `json:"id"`
	RunID      int64  `json:"run_id"`
	Seq        int    `json:"seq"`
	CurVersion int    `json:"cur_version"`
	IsMerge    bool   `json:"is_merge"`
}

func toTaskDTO(t domain.Task) taskDTO {
	return taskDTO{
		ID: t.ID, RunID: t.RunID, StageCode: t.StageCode, StageName: t.StageName,
		Seq: t.Seq, RoleCode: t.RoleCode, IsMerge: t.IsMerge, Status: string(t.Status),
		CurVersion: t.CurVersion, AssigneeID: t.AssigneeID,
	}
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
