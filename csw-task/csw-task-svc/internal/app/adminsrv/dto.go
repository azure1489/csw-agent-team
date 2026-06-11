package adminsrv

import (
	"strconv"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

func itoa(n int64) string { return strconv.FormatInt(n, 10) }

func pathID(c *gin.Context, name string) (int64, error) {
	id, err := strconv.ParseInt(c.Param(name), 10, 64)
	if err != nil {
		return 0, domain.BadRequest("bad_id", "非法 id 参数："+c.Param(name))
	}
	return id, nil
}

func adminUserDTO(u domain.AdminUser) gin.H {
	return gin.H{
		"id": u.ID, "username": u.Username, "display_name": u.DisplayName,
		"role": u.Role, "status": u.Status, "last_login_at": u.LastLoginAt, "created_at": u.CreatedAt,
	}
}

func workflowDTO(w domain.Workflow) gin.H {
	return gin.H{
		"id": w.ID, "wf_key": w.WfKey, "name": w.Name, "version": w.Version,
		"hub_role": w.HubRoleCode, "dispatch_mode": string(w.DispatchMode),
		"trigger_roles": w.TriggerRoles, "status": string(w.Status),
		"common_instructions": w.CommonInstructions, "common_acceptance": w.CommonAcceptance,
	}
}

func stageDTO(st domain.Stage, deps []string, gates []gin.H) gin.H {
	if deps == nil {
		deps = []string{}
	}
	if gates == nil {
		gates = []gin.H{}
	}
	return gin.H{
		"id": st.ID, "seq": st.Seq, "code": st.Code, "name": st.Name,
		"role_code": st.RoleCode, "output_type": st.OutputType, "is_merge": st.IsMerge,
		"instructions": st.Instructions, "self_check_criteria": st.SelfCheckCriteria, "acceptance": st.Acceptance,
		"deps": deps, "gates": gates,
	}
}

func runDTO(r sqlite.RunListItem) gin.H {
	return gin.H{
		"id": r.ID, "workflow_id": r.WorkflowID, "workflow_name": r.WorkflowName, "workflow_key": r.WorkflowKey,
		"subject": r.Subject, "status": r.Status, "current_stage": r.CurrentStage,
		"trigger_name": r.TriggerName, "created_at": r.CreatedAt,
	}
}

func auditDTO(a domain.AdminAudit) gin.H {
	return gin.H{
		"id": a.ID, "user_id": a.UserID, "username": a.Username, "action": a.Action,
		"target": a.Target, "detail": a.DetailJSON, "created_at": a.CreatedAt,
	}
}

func taskBriefDTO(t domain.Task) gin.H {
	return gin.H{
		"id": t.ID, "run_id": t.RunID, "stage_code": t.StageCode, "stage_name": t.StageName,
		"seq": t.Seq, "role_code": t.RoleCode, "is_merge": t.IsMerge,
		"status": string(t.Status), "cur_version": t.CurVersion, "assignee_id": t.AssigneeID,
	}
}

func deliverableDetailDTO(d domain.Deliverable, ups []domain.Upstream, reviews []domain.Review) gin.H {
	us := make([]gin.H, 0, len(ups))
	for _, u := range ups {
		us = append(us, gin.H{"label": u.Label, "url": u.UpstreamURL, "upstream_id": u.UpstreamID})
	}
	rs := make([]gin.H, 0, len(reviews))
	for _, r := range reviews {
		rs = append(rs, gin.H{
			"id": r.ID, "task_gate_id": r.TaskGateID, "reviewer_id": r.ReviewerID, "verdict": string(r.Verdict),
			"comment": r.Comment, "return_direction": r.ReturnDirection, "return_location": r.ReturnLocation,
		})
	}
	return gin.H{
		"id": d.ID, "task_id": d.TaskID, "is_dispatch": d.IsDispatch, "version": d.Version,
		"doc_type": d.DocType, "download_url": d.DownloadURL, "filename": d.Filename, "title": d.Title,
		"summary": d.Summary, "self_check": d.SelfCheck, "editor_note": d.EditorNote,
		"cur_gate": d.CurGate, "returned_at_gate": d.ReturnedAtGate, "status": string(d.Status),
		"upstreams": us, "reviews": rs,
	}
}
