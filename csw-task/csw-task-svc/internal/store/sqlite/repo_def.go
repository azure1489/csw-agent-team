package sqlite

import (
	"context"
	"database/sql"
	"errors"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── 角色 ──

// GetRole 取角色。
func (q *Queries) GetRole(ctx context.Context, code string) (domain.Role, error) {
	var r domain.Role
	var mgmt, human int
	err := q.ex.QueryRowContext(ctx,
		`SELECT code, name, is_management, is_human FROM roles WHERE code=?`, code).
		Scan(&r.Code, &r.Name, &mgmt, &human)
	r.IsManagement, r.IsHuman = mgmt == 1, human == 1
	return r, err
}

// ── agent + token（鉴权）──

// AuthByToken 据 token_hash 解析有效（未吊销、未过期）的 agent + 角色 + token id。
func (q *Queries) AuthByToken(ctx context.Context, tokenHash string) (domain.Agent, domain.Role, int64, error) {
	var a domain.Agent
	var r domain.Role
	var tokenID int64
	var active, mgmt, human int
	err := q.ex.QueryRowContext(ctx, `
		SELECT t.id, a.id, a.role_code, a.name, a.active,
		       r.code, r.name, r.is_management, r.is_human
		FROM agent_tokens t
		JOIN agents a ON a.id = t.agent_id
		JOIN roles  r ON r.code = a.role_code
		WHERE t.token_hash = ?
		  AND t.revoked = 0
		  AND (t.expires_at IS NULL OR t.expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now'))`,
		tokenHash).
		Scan(&tokenID, &a.ID, &a.RoleCode, &a.Name, &active,
			&r.Code, &r.Name, &mgmt, &human)
	a.Active = active == 1
	r.IsManagement, r.IsHuman = mgmt == 1, human == 1
	return a, r, tokenID, err
}

// TouchToken 更新 token 最近使用时间。
func (q *Queries) TouchToken(ctx context.Context, tokenID int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE agent_tokens SET last_used_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id=?`, tokenID)
	return err
}

// GetAgent 取 agent。
func (q *Queries) GetAgent(ctx context.Context, id int64) (domain.Agent, error) {
	var a domain.Agent
	var active int
	err := q.ex.QueryRowContext(ctx,
		`SELECT id, role_code, name, active FROM agents WHERE id=?`, id).
		Scan(&a.ID, &a.RoleCode, &a.Name, &active)
	a.Active = active == 1
	return a, err
}

// ActiveAgentByRole 取某角色下唯一活跃 agent（assignee 解析；一角色一 agent 假设）。
func (q *Queries) ActiveAgentByRole(ctx context.Context, roleCode string) (domain.Agent, error) {
	var a domain.Agent
	err := q.ex.QueryRowContext(ctx,
		`SELECT id, role_code, name, 1 FROM agents WHERE role_code=? AND active=1 ORDER BY id LIMIT 1`, roleCode).
		Scan(&a.ID, &a.RoleCode, &a.Name, new(int))
	a.Active = true
	return a, err
}

// ListAgents 列全部 agent（adminctl）。
func (q *Queries) ListAgents(ctx context.Context) ([]domain.Agent, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT id, role_code, name, active FROM agents ORDER BY id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Agent
	for rows.Next() {
		var a domain.Agent
		var active int
		if err := rows.Scan(&a.ID, &a.RoleCode, &a.Name, &active); err != nil {
			return nil, err
		}
		a.Active = active == 1
		out = append(out, a)
	}
	return out, rows.Err()
}

// ActiveTokenCount 统计某 agent 当前有效（未吊销、未过期）token 数。
func (q *Queries) ActiveTokenCount(ctx context.Context, agentID int64) (int, error) {
	var n int
	err := q.ex.QueryRowContext(ctx, `
		SELECT count(*) FROM agent_tokens
		WHERE agent_id=? AND revoked=0
		  AND (expires_at IS NULL OR expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now'))`, agentID).Scan(&n)
	return n, err
}

// InsertToken 写入新 token（明文哈希已在调用方算好），返回 token id。
func (q *Queries) InsertToken(ctx context.Context, agentID int64, tokenHash, label string, expiresAt *string) (int64, error) {
	var exp sql.NullString
	if expiresAt != nil {
		exp = sql.NullString{String: *expiresAt, Valid: true}
	}
	res, err := q.ex.ExecContext(ctx,
		`INSERT INTO agent_tokens (agent_id, token_hash, label, expires_at) VALUES (?,?,?,?)`,
		agentID, tokenHash, label, exp)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// RevokeToken 吊销某 token。
func (q *Queries) RevokeToken(ctx context.Context, tokenID int64) error {
	res, err := q.ex.ExecContext(ctx, `UPDATE agent_tokens SET revoked=1 WHERE id=?`, tokenID)
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return errors.New("token not found")
	}
	return nil
}

// ── 工作流定义（只读消费）──

func scanWorkflow(s interface{ Scan(...any) error }) (domain.Workflow, error) {
	var w domain.Workflow
	var mode, status string
	var common1, common2, trig sql.NullString
	err := s.Scan(&w.ID, &w.WfKey, &w.Name, &w.Version, &w.HubRoleCode,
		&mode, &trig, &common1, &common2, &status)
	w.DispatchMode = domain.DispatchMode(mode)
	w.Status = domain.WorkflowStatus(status)
	w.TriggerRoles, w.CommonInstructions, w.CommonAcceptance = trig.String, common1.String, common2.String
	return w, err
}

const wfCols = `id, wf_key, name, version, hub_role_code, dispatch_mode, trigger_roles, common_instructions, common_acceptance, status`

// ActiveWorkflowByKey 取某 key 当前 active 版本。
func (q *Queries) ActiveWorkflowByKey(ctx context.Context, key string) (domain.Workflow, error) {
	return scanWorkflow(q.ex.QueryRowContext(ctx,
		`SELECT `+wfCols+` FROM workflows WHERE wf_key=? AND status='active'`, key))
}

// GetWorkflow 据 id 取工作流。
func (q *Queries) GetWorkflow(ctx context.Context, id int64) (domain.Workflow, error) {
	return scanWorkflow(q.ex.QueryRowContext(ctx, `SELECT `+wfCols+` FROM workflows WHERE id=?`, id))
}

// ListActiveWorkflows 列全部 active 工作流（供选择触发）。
func (q *Queries) ListActiveWorkflows(ctx context.Context) ([]domain.Workflow, error) {
	rows, err := q.ex.QueryContext(ctx, `SELECT `+wfCols+` FROM workflows WHERE status='active' ORDER BY wf_key`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Workflow
	for rows.Next() {
		w, err := scanWorkflow(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, w)
	}
	return out, rows.Err()
}

// ListStages 列某工作流全部阶段（按 seq）。
func (q *Queries) ListStages(ctx context.Context, workflowID int64) ([]domain.Stage, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT id, workflow_id, seq, code, name, role_code, output_type,
		       instructions, self_check_criteria, acceptance, is_merge
		FROM workflow_stages WHERE workflow_id=? ORDER BY seq`, workflowID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Stage
	for rows.Next() {
		var st domain.Stage
		var ot, ins, sc, ac sql.NullString
		var merge int
		if err := rows.Scan(&st.ID, &st.WorkflowID, &st.Seq, &st.Code, &st.Name, &st.RoleCode,
			&ot, &ins, &sc, &ac, &merge); err != nil {
			return nil, err
		}
		st.OutputType, st.Instructions, st.SelfCheckCriteria, st.Acceptance = ot.String, ins.String, sc.String, ac.String
		st.IsMerge = merge == 1
		out = append(out, st)
	}
	return out, rows.Err()
}

// ListStageDeps 列某工作流全部依赖边。
func (q *Queries) ListStageDeps(ctx context.Context, workflowID int64) ([]domain.StageDep, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT stage_id, depends_on_id FROM workflow_stage_deps WHERE workflow_id=?`, workflowID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.StageDep
	for rows.Next() {
		var d domain.StageDep
		if err := rows.Scan(&d.StageID, &d.DependsOnID); err != nil {
			return nil, err
		}
		out = append(out, d)
	}
	return out, rows.Err()
}

// ── 定义写（管理后台用）──

// WorkflowListItem 工作流列表项（含阶段数）。
type WorkflowListItem struct {
	domain.Workflow
	StageCount int
}

// ListWorkflowsAll 列全部工作流（全版本），可按 name/key 搜索、status 过滤；含阶段数。
func (q *Queries) ListWorkflowsAll(ctx context.Context, search, status string) ([]WorkflowListItem, error) {
	query := `SELECT ` + wfCols + `,
		(SELECT count(*) FROM workflow_stages s WHERE s.workflow_id = workflows.id) AS stage_count
		FROM workflows`
	var where []string
	var args []any
	if search != "" {
		where = append(where, "(name LIKE ? OR wf_key LIKE ?)")
		args = append(args, "%"+search+"%", "%"+search+"%")
	}
	if status != "" {
		where = append(where, "status = ?")
		args = append(args, status)
	}
	if len(where) > 0 {
		query += " WHERE " + strings.Join(where, " AND ")
	}
	query += " ORDER BY wf_key, version DESC"

	rows, err := q.ex.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []WorkflowListItem
	for rows.Next() {
		var w domain.Workflow
		var mode, st string
		var c1, c2, trig sql.NullString
		var cnt int
		if err := rows.Scan(&w.ID, &w.WfKey, &w.Name, &w.Version, &w.HubRoleCode,
			&mode, &trig, &c1, &c2, &st, &cnt); err != nil {
			return nil, err
		}
		w.DispatchMode, w.Status = domain.DispatchMode(mode), domain.WorkflowStatus(st)
		w.TriggerRoles, w.CommonInstructions, w.CommonAcceptance = trig.String, c1.String, c2.String
		out = append(out, WorkflowListItem{Workflow: w, StageCount: cnt})
	}
	return out, rows.Err()
}

// nextWorkflowVersion 取某 key 的下一个版本号（新 key → 1）。
func (q *Queries) nextWorkflowVersion(ctx context.Context, wfKey string) (int, error) {
	var v int
	err := q.ex.QueryRowContext(ctx,
		`SELECT COALESCE(MAX(version),0)+1 FROM workflows WHERE wf_key=?`, wfKey).Scan(&v)
	return v, err
}

// CreateWorkflowDraft 新建工作流草稿（version 自增），返回 id。
func (q *Queries) CreateWorkflowDraft(ctx context.Context, wfKey, name, hubRole, dispatchMode, triggerRoles string) (int64, error) {
	ver, err := q.nextWorkflowVersion(ctx, wfKey)
	if err != nil {
		return 0, err
	}
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO workflows (wf_key, name, version, hub_role_code, dispatch_mode, trigger_roles, status)
		VALUES (?,?,?,?,?,?,'draft')`,
		wfKey, name, ver, hubRole, dispatchMode, triggerRoles)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// UpdateWorkflowBasics 改草稿基本信息。
func (q *Queries) UpdateWorkflowBasics(ctx context.Context, id int64, name, hubRole, dispatchMode, triggerRoles, commonInstructions, commonAcceptance string) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE workflows SET name=?, hub_role_code=?, dispatch_mode=?, trigger_roles=?,
			common_instructions=?, common_acceptance=?
		WHERE id=?`,
		name, hubRole, dispatchMode, triggerRoles, commonInstructions, commonAcceptance, id)
	return err
}

// SetWorkflowStatus 改工作流状态。
func (q *Queries) SetWorkflowStatus(ctx context.Context, id int64, status domain.WorkflowStatus) error {
	_, err := q.ex.ExecContext(ctx, `UPDATE workflows SET status=? WHERE id=?`, string(status), id)
	return err
}

// ArchiveOtherActive 归档同 key 下除 exceptID 外的所有 active 版本（激活前调，避免 uq_wf_active 冲突）。
func (q *Queries) ArchiveOtherActive(ctx context.Context, wfKey string, exceptID int64) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE workflows SET status='archived' WHERE wf_key=? AND status='active' AND id<>?`, wfKey, exceptID)
	return err
}

// InsertStage 插入阶段定义，返回 id。
func (q *Queries) InsertStage(ctx context.Context, s domain.Stage) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `
		INSERT INTO workflow_stages
		  (workflow_id, seq, code, name, role_code, output_type, instructions, self_check_criteria, acceptance, is_merge)
		VALUES (?,?,?,?,?,?,?,?,?,?)`,
		s.WorkflowID, s.Seq, s.Code, s.Name, s.RoleCode, s.OutputType,
		s.Instructions, s.SelfCheckCriteria, s.Acceptance, b2i(s.IsMerge))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// InsertStageDep 插入依赖边。
func (q *Queries) InsertStageDep(ctx context.Context, workflowID, stageID, dependsOnID int64) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO workflow_stage_deps (workflow_id, stage_id, depends_on_id) VALUES (?,?,?)`,
		workflowID, stageID, dependsOnID)
	return err
}

// InsertGate 插入审核闸（stage_id 为 nil 即默认闸）。
func (q *Queries) InsertGate(ctx context.Context, g domain.Gate) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO workflow_gates (workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name) VALUES (?,?,?,?,?,?)`,
		g.WorkflowID, nullI64(g.StageID), g.GateOrder, g.ReviewerRole, b2i(g.RelayedByHub), g.Name)
	return err
}

// DeleteWorkflowChildren 清空某工作流的 stages/deps/gates（整份重提前）。
func (q *Queries) DeleteWorkflowChildren(ctx context.Context, workflowID int64) error {
	for _, t := range []string{"workflow_stage_deps", "workflow_gates", "workflow_stages"} {
		if _, err := q.ex.ExecContext(ctx, `DELETE FROM `+t+` WHERE workflow_id=?`, workflowID); err != nil {
			return err
		}
	}
	return nil
}

// ListGates 列某工作流全部闸（默认 stage_id NULL + 覆盖）。
func (q *Queries) ListGates(ctx context.Context, workflowID int64) ([]domain.Gate, error) {
	rows, err := q.ex.QueryContext(ctx,
		`SELECT id, workflow_id, stage_id, gate_order, reviewer_role, relayed_by_hub, name
		 FROM workflow_gates WHERE workflow_id=? ORDER BY gate_order`, workflowID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.Gate
	for rows.Next() {
		var g domain.Gate
		var stageID sql.NullInt64
		var name sql.NullString
		var relayed int
		if err := rows.Scan(&g.ID, &g.WorkflowID, &stageID, &g.GateOrder, &g.ReviewerRole, &relayed, &name); err != nil {
			return nil, err
		}
		g.StageID = ptrI64(stageID)
		g.RelayedByHub = relayed == 1
		g.Name = name.String
		out = append(out, g)
	}
	return out, rows.Err()
}

// ── 角色（管理后台写）──

// RoleWithCount 角色 + 成员数。
type RoleWithCount struct {
	domain.Role
	MemberCount int
}

// ListRolesWithCounts 列全部角色 + 各自成员数。
func (q *Queries) ListRolesWithCounts(ctx context.Context) ([]RoleWithCount, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT r.code, r.name, r.is_management, r.is_human,
		       (SELECT count(*) FROM agents a WHERE a.role_code = r.code) AS member_count
		FROM roles r ORDER BY r.code`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []RoleWithCount
	for rows.Next() {
		var rc RoleWithCount
		var mgmt, human int
		if err := rows.Scan(&rc.Code, &rc.Name, &mgmt, &human, &rc.MemberCount); err != nil {
			return nil, err
		}
		rc.IsManagement, rc.IsHuman = mgmt == 1, human == 1
		out = append(out, rc)
	}
	return out, rows.Err()
}

// CreateRole 新建角色。
func (q *Queries) CreateRole(ctx context.Context, code, name string, isManagement, isHuman bool) error {
	_, err := q.ex.ExecContext(ctx,
		`INSERT INTO roles (code, name, is_management, is_human) VALUES (?,?,?,?)`,
		code, name, b2i(isManagement), b2i(isHuman))
	return err
}

// UpdateRole 改角色 name/is_management/is_human。
func (q *Queries) UpdateRole(ctx context.Context, code, name string, isManagement, isHuman bool) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE roles SET name=?, is_management=?, is_human=? WHERE code=?`,
		name, b2i(isManagement), b2i(isHuman), code)
	return err
}

// ── 成员（管理后台写）──

// CreateAgent 新建成员，返回 id。
func (q *Queries) CreateAgent(ctx context.Context, roleCode, name string) (int64, error) {
	res, err := q.ex.ExecContext(ctx, `INSERT INTO agents (role_code, name) VALUES (?,?)`, roleCode, name)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// UpdateAgent 改成员 name/role/active。
func (q *Queries) UpdateAgent(ctx context.Context, id int64, name, roleCode string, active bool) error {
	_, err := q.ex.ExecContext(ctx,
		`UPDATE agents SET name=?, role_code=?, active=? WHERE id=?`, name, roleCode, b2i(active), id)
	return err
}

// TokenInfo token 列表项（含派生状态）。
type TokenInfo struct {
	Label      string
	Status     string // active/revoked/expired
	LastUsedAt string
	ExpiresAt  string
	CreatedAt  string
	ID         int64
}

// ListTokensByAgent 列某成员的 token（含派生状态）。
func (q *Queries) ListTokensByAgent(ctx context.Context, agentID int64) ([]TokenInfo, error) {
	rows, err := q.ex.QueryContext(ctx, `
		SELECT id, label, last_used_at, expires_at, revoked, created_at,
		       (expires_at IS NOT NULL AND expires_at <= strftime('%Y-%m-%dT%H:%M:%SZ','now')) AS expired
		FROM agent_tokens WHERE agent_id=? ORDER BY id DESC`, agentID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []TokenInfo
	for rows.Next() {
		var ti TokenInfo
		var label, last, exp sql.NullString
		var revoked, expired int
		if err := rows.Scan(&ti.ID, &label, &last, &exp, &revoked, &ti.CreatedAt, &expired); err != nil {
			return nil, err
		}
		ti.Label, ti.LastUsedAt, ti.ExpiresAt = label.String, last.String, exp.String
		switch {
		case revoked == 1:
			ti.Status = "revoked"
		case expired == 1:
			ti.Status = "expired"
		default:
			ti.Status = "active"
		}
		out = append(out, ti)
	}
	return out, rows.Err()
}
