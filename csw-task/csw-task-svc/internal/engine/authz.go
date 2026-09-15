package engine

import (
	"context"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// authorized 判断任务的动作类别在所属 run 是否已获授权；read 类恒为 true。
// 返回所需 scope 供提示。动作类别非法按未授权处理（fail-closed）。
func authorized(ctx context.Context, q *sqlite.Queries, task domain.Task) (bool, domain.AuthScope, error) {
	ac := task.ActionClass
	if ac == "" {
		ac = domain.ActionRead
	}
	isWrite, scope, ok := domain.ParseActionClass(ac)
	if !ok {
		return false, domain.AuthScope(ac), nil
	}
	if !isWrite {
		return true, "", nil
	}
	has, err := q.HasActiveAuthorization(ctx, task.RunID, domain.SatisfyingScopes(scope))
	return has, scope, err
}

// RequireAuthorization 平台写操作守卫：动作类别为 platform_write:<scope> 的任务，要求所属 run 有有效授权。
// 手动派工、提交在事务内调用；运行面 handler 在事务外预检时也调用（避免白传一份文件）。
func RequireAuthorization(ctx context.Context, q *sqlite.Queries, task domain.Task) error {
	ok, scope, err := authorized(ctx, q, task)
	if err != nil {
		return err
	}
	if !ok {
		return domain.Conflict("authorization_required", "该任务是平台写操作，本期 run 尚无有效授权："+string(scope))
	}
	return nil
}

// GrantResult 授权录入结果。
type GrantResult struct {
	Authorization domain.RunAuthorization
	Dispatched    []int64 // 因本次授权续派的任务
	Existing      bool    // 已有同范围有效授权，未重复写入
}

// Grant 中枢录入 run 授权（须带 Van 原话），随后续派本 run 中因缺授权停在 ready 的自动派工任务。
// expiresAt 为 UTC「2006-01-02T15:04:05Z」格式或 nil（不过期）。
func (e *Engine) Grant(ctx context.Context, hub domain.Agent, role domain.Role, runID int64, scope domain.AuthScope, sourceQuote string, expiresAt *string) (GrantResult, error) {
	var res GrantResult
	if !domain.ValidAuthScope(string(scope)) {
		return res, domain.BadRequest("bad_scope", "scope 须为 local_drill / wx_draft / wx_publish / xhs_draft / xhs_publish")
	}
	sourceQuote = strings.TrimSpace(sourceQuote)
	if sourceQuote == "" {
		return res, domain.BadRequest("source_quote_required", "授权须附 Van 原话（source_quote）")
	}
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, wf, err := hubRun(ctx, q, role, runID)
		if err != nil {
			return err
		}
		if run.Status != domain.RunActive {
			return domain.Conflict("run_not_active", "仅进行中的 run 可录入授权，当前："+string(run.Status))
		}

		existing, err := q.ActiveAuthorizationByScope(ctx, runID, scope)
		switch {
		case err == nil:
			res.Authorization, res.Existing = existing, true
		case isNoRows(err):
			a := domain.RunAuthorization{RunID: runID, Scope: scope, GrantedBy: &hub.ID, SourceQuote: sourceQuote}
			if expiresAt != nil {
				a.ExpiresAt = *expiresAt
			}
			id, err := q.InsertAuthorization(ctx, a)
			if err != nil {
				return err
			}
			if res.Authorization, err = q.GetAuthorization(ctx, id); err != nil {
				return err
			}
			if err := q.InsertEvent(ctx, domain.Event{RunID: &runID, ActorID: &hub.ID, Type: domain.EvtAuthorizationGranted,
				DetailJSON: evtDetail(map[string]any{"scope": string(scope), "source_quote": sourceQuote, "expires_at": a.ExpiresAt})}); err != nil {
				return err
			}
		default:
			return err
		}

		// 续派：ready 的平台写任务，现已授权，且有效派工模式为自动（或合流）。
		tasks, err := q.ListTasksByRun(ctx, runID)
		if err != nil {
			return err
		}
		for _, t := range tasks {
			if t.Status != domain.TaskReady {
				continue
			}
			if isWrite, _, _ := domain.ParseActionClass(t.ActionClass); !isWrite {
				continue
			}
			if effectiveDispatchMode(t, wf) != domain.DispatchAuto && !t.IsMerge {
				continue
			}
			ok, _, err := authorized(ctx, q, t)
			if err != nil {
				return err
			}
			if !ok {
				continue
			}
			if _, err := e.dispatchInternal(ctx, q, t, wf, "", nil, resolveAssignee(ctx, q, wf.HubRoleCode)); err != nil {
				return err
			}
			res.Dispatched = append(res.Dispatched, t.ID)
		}
		return nil
	})
	return res, err
}

// Revoke 中枢撤销 run 某范围的授权。已派出的任务不收回，但此后提交会被授权守卫拒绝。
func (e *Engine) Revoke(ctx context.Context, hub domain.Agent, role domain.Role, runID int64, scope domain.AuthScope) (int64, error) {
	if !domain.ValidAuthScope(string(scope)) {
		return 0, domain.BadRequest("bad_scope", "scope 须为 local_drill / wx_draft / wx_publish / xhs_draft / xhs_publish")
	}
	var n int64
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		if _, _, err := hubRun(ctx, q, role, runID); err != nil {
			return err
		}
		var err error
		if n, err = q.RevokeAuthorizations(ctx, runID, scope, &hub.ID); err != nil {
			return err
		}
		if n == 0 {
			return domain.NotFound("authorization_not_found", "该 run 没有此范围的未撤销授权："+string(scope))
		}
		return q.InsertEvent(ctx, domain.Event{RunID: &runID, ActorID: &hub.ID, Type: domain.EvtAuthorizationRevoked,
			DetailJSON: evtDetail(map[string]any{"scope": string(scope)})})
	})
	return n, err
}

// hubRun 取 run 与其工作流，并校验调用者为该工作流中枢。
func hubRun(ctx context.Context, q *sqlite.Queries, role domain.Role, runID int64) (domain.Run, domain.Workflow, error) {
	run, err := q.GetRun(ctx, runID)
	if isNoRows(err) {
		return run, domain.Workflow{}, domain.NotFound("run_not_found", "无此实例")
	}
	if err != nil {
		return run, domain.Workflow{}, err
	}
	wf, err := q.GetWorkflow(ctx, run.WorkflowID)
	if err != nil {
		return run, wf, err
	}
	if role.Code != wf.HubRoleCode {
		return run, wf, domain.Forbidden("not_hub", "仅中枢角色可操作授权")
	}
	return run, wf, nil
}
