package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// evtDetail 序列化事件 detail；失败返回空串（事件仍写入）。
func evtDetail(m map[string]any) string {
	b, err := json.Marshal(m)
	if err != nil {
		return ""
	}
	return string(b)
}

// ReviewInput 审核入参。人工闸（Van）须附原话：SourceQuote，或 Comment 以「Van：」开头（兼容现行代录写法）。
type ReviewInput struct {
	Verdict         domain.Verdict
	Comment         string
	ReturnDirection string
	ReturnLocation  string
	DecisionType    string
	SourceQuote     string
	ItemsJSON       string // 条目范围：JSON 字符串数组
	ExpectedVersion *int   // 审核人以为的当前版本；与任务当前版本不符则 409
}

// vanQuote 现行代录写法 comment「Van：…」视为已附原话。
func vanQuote(comment string) string {
	c := strings.TrimSpace(comment)
	for _, p := range []string{"Van：", "Van:", "van：", "van:"} {
		if strings.HasPrefix(c, p) && strings.TrimSpace(strings.TrimPrefix(c, p)) != "" {
			return c
		}
	}
	return ""
}

// decorateDecision 把业务决定写进事件 detail（有值才写）。
func decorateDecision(m map[string]any, in ReviewInput, quote string) map[string]any {
	if in.DecisionType != "" {
		m["decision_type"] = in.DecisionType
	}
	if quote != "" {
		m["source_quote"] = quote
	}
	if in.ItemsJSON != "" {
		var items []string
		_ = json.Unmarshal([]byte(in.ItemsJSON), &items)
		m["items"] = items
	}
	return m
}

// ReviewResult 审核结果。
type ReviewResult struct {
	Deliverable domain.Deliverable
	TaskStatus  domain.TaskStatus
	GatePassed  bool // 本道闸是否通过
	Final       bool // 是否末闸通过（task passed）
}

// Review 按闸推进审核某交付物的当前版本：作用于 cur_gate+1 这道闸。
// 普通闸校验调用者角色=reviewer_role；人工闸（relayed_by_hub）由中枢代录。
// pass → cur_gate+1，末闸则 task passed 并重算下游；reject → returned 并记 returned_at_gate（必填方向+位置）。
func (e *Engine) Review(ctx context.Context, actor domain.Agent, role domain.Role, deliverableID int64, in ReviewInput) (ReviewResult, error) {
	if in.Verdict != domain.VerdictPass && in.Verdict != domain.VerdictReject {
		return ReviewResult{}, domain.BadRequest("bad_verdict", "verdict 须为 pass 或 reject")
	}
	if in.Verdict == domain.VerdictReject && (in.ReturnDirection == "" || in.ReturnLocation == "") {
		return ReviewResult{}, domain.BadRequest("return_required", "退回必填 return_direction 和 return_location")
	}
	if in.DecisionType != "" && !domain.ValidDecisionType(in.DecisionType) {
		return ReviewResult{}, domain.BadRequest("bad_decision_type", "decision_type 取值不合法："+in.DecisionType)
	}
	if in.ItemsJSON != "" {
		var items []string
		if err := json.Unmarshal([]byte(in.ItemsJSON), &items); err != nil {
			return ReviewResult{}, domain.BadRequest("bad_items_json", "items_json 须为条目键的 JSON 字符串数组")
		}
	}

	var res ReviewResult
	err := e.store.Tx(ctx, func(q *sqlite.Queries) error {
		d, err := q.GetDeliverable(ctx, deliverableID)
		if isNoRows(err) {
			return domain.NotFound("deliverable_not_found", "无此交付物")
		}
		if err != nil {
			return err
		}
		if d.Kind == domain.KindDispatch || d.Kind == domain.KindSupplement {
			return domain.BadRequest("not_reviewable", "派工单与补件不参与审核")
		}
		if d.Status != domain.DelSubmitted && d.Status != domain.DelInReview {
			return domain.Conflict("not_in_review", "交付物不在待审状态："+string(d.Status))
		}

		task, err := q.GetTask(ctx, d.TaskID)
		if err != nil {
			return err
		}
		if d.Version != task.CurVersion {
			return domain.Conflict("stale_version", "该交付物非任务当前版本")
		}
		if task.Status != domain.TaskReview {
			return domain.Conflict("not_in_review", "任务不在审核中："+string(task.Status))
		}
		if in.ExpectedVersion != nil && *in.ExpectedVersion != task.CurVersion {
			return domain.Conflict("expected_version_mismatch",
				fmt.Sprintf("审核针对 v%d，任务当前是 v%d，请先读最新版本", *in.ExpectedVersion, task.CurVersion))
		}
		run, err := q.GetRun(ctx, task.RunID)
		if err != nil {
			return err
		}
		wf, err := q.GetWorkflow(ctx, run.WorkflowID)
		if err != nil {
			return err
		}

		nextGate := d.CurGate + 1
		tg, err := q.TaskGateByOrder(ctx, task.ID, nextGate)
		if isNoRows(err) {
			return domain.Conflict("no_gate", "无对应审核闸（越级或已到末闸）")
		}
		if err != nil {
			return err
		}

		// 权限：人工闸中枢代录；普通闸须 reviewer_role 本人。
		if tg.RelayedByHub {
			if role.Code != wf.HubRoleCode {
				return domain.Forbidden("relay_requires_hub", "人工闸须由中枢代录")
			}
		} else if role.Code != tg.ReviewerRole {
			return domain.Forbidden("not_reviewer", "当前角色非该闸审核角色")
		}
		quote := strings.TrimSpace(in.SourceQuote)
		if tg.RelayedByHub && quote == "" {
			if quote = vanQuote(in.Comment); quote == "" {
				return domain.BadRequest("source_quote_required", "人工闸结论须附 Van 原话：source_quote，或 comment 以「Van：」开头")
			}
		}

		reviewerID := actor.ID
		if _, err := q.InsertReview(ctx, domain.Review{
			DeliverableID:   deliverableID,
			TaskGateID:      tg.ID,
			ReviewerID:      &reviewerID,
			Verdict:         in.Verdict,
			Comment:         in.Comment,
			ReturnDirection: in.ReturnDirection,
			ReturnLocation:  in.ReturnLocation,
			DecisionType:    in.DecisionType,
			SourceQuote:     quote,
			ItemsJSON:       in.ItemsJSON,
			ExpectedVersion: in.ExpectedVersion,
		}); err != nil {
			return err
		}

		if in.Verdict == domain.VerdictPass {
			gates, err := q.ListTaskGates(ctx, task.ID)
			if err != nil {
				return err
			}
			passDetail := evtDetail(decorateDecision(map[string]any{
				"gate": nextGate, "gate_name": tg.Name, "comment": in.Comment, "final": nextGate >= len(gates),
			}, in, quote))
			if nextGate >= len(gates) {
				// 末闸通过：task/deliverable passed，重算下游。
				if err := q.SetDeliverableGate(ctx, deliverableID, nextGate, domain.DelPassed, nil); err != nil {
					return err
				}
				if err := q.SetTaskPassed(ctx, task.ID, d.Version); err != nil {
					return err
				}
				if err := q.InsertEvent(ctx, domain.Event{RunID: &run.ID, TaskID: &task.ID, DeliverableID: &deliverableID, ActorID: &reviewerID, Type: domain.EvtGatePassed, DetailJSON: passDetail}); err != nil {
					return err
				}
				if err := e.stagePassed(ctx, q, task, wf, d.Version); err != nil {
					return err
				}
				res.Final = true
			} else {
				if err := q.SetDeliverableGate(ctx, deliverableID, nextGate, domain.DelInReview, nil); err != nil {
					return err
				}
				ng, err := q.TaskGateByOrder(ctx, task.ID, nextGate+1)
				if err != nil {
					return err
				}
				if err := emit(ctx, q, domain.Event{RunID: &run.ID, TaskID: &task.ID, DeliverableID: &deliverableID, ActorID: &reviewerID, Type: domain.EvtGatePassed, DetailJSON: passDetail},
					reviewNotice(task, wf.HubRoleCode, ng, d.Version, deliverableID, d.DownloadURL, map[string]any{"passed_gate_name": tg.Name})); err != nil {
					return err
				}
			}
			res.GatePassed = true
		} else {
			// reject：退回，记退回闸位；不推进下游。退回原因写进事件 detail（timeline 可读）。
			nx := nextGate
			if err := q.SetDeliverableGate(ctx, deliverableID, d.CurGate, domain.DelReturned, &nx); err != nil {
				return err
			}
			if err := q.SetTaskReturned(ctx, task.ID); err != nil {
				return err
			}
			// 退回等同重新交办：按时限重新计时。
			if task.SLAMinutes > 0 {
				if err := q.SetTaskDue(ctx, task.ID, task.SLAMinutes); err != nil {
					return err
				}
			}
			rejDetail := evtDetail(decorateDecision(map[string]any{
				"gate": nextGate, "gate_name": tg.Name,
				"return_direction": in.ReturnDirection, "return_location": in.ReturnLocation, "comment": in.Comment,
			}, in, quote))
			if err := emit(ctx, q, domain.Event{RunID: &run.ID, TaskID: &task.ID, DeliverableID: &deliverableID, ActorID: &reviewerID, Type: domain.EvtGateReturned, DetailJSON: rejDetail},
				&notice{target: task.RoleCode, hub: wf.HubRoleCode, cc: []string{wf.HubRoleCode}, payload: taskPayload(task, map[string]any{
					"version": d.Version, "gate_name": tg.Name, "return_direction": in.ReturnDirection,
					"return_location": in.ReturnLocation, "comment": in.Comment,
				})}); err != nil {
				return err
			}
		}

		if res.Deliverable, err = q.GetDeliverable(ctx, deliverableID); err != nil {
			return err
		}
		t2, err := q.GetTask(ctx, task.ID)
		if err != nil {
			return err
		}
		res.TaskStatus = t2.Status
		return nil
	})
	return res, err
}
