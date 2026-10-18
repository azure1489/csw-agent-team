package engine

import (
	"context"
	"strings"
	"unicode/utf8"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

// noticeMaxRunes 群消息正文上限：要说清的事写进引擎记录，群里只放要对方回复或动手的那几句。
const noticeMaxRunes = 1500

// HubNotice 中枢按角色发一条群消息，由引擎用花名册真 @ 收件人（10-18 用户定）。
//
// 主编机器人自己在群里发的消息 @ 不到人：飞书 open_id 按应用区分，它拿不到其他 agent 在它应用下的 id，
// 只在文字里写「Van」也收不到通知（10-18 r80：主编请 Van 确认两图例外，Van 没被提醒）。
// 引擎的通知组件维护着全体角色的花名册，这里只记事件、写 outbox，由它代发。
func (e *Engine) HubNotice(ctx context.Context, hub domain.Agent, role domain.Role, runID int64, to []string, text string) error {
	text = strings.TrimSpace(text)
	if text == "" {
		return domain.BadRequest("text_required", "text 不能为空：写清要对方回复或动手的事")
	}
	if utf8.RuneCountInString(text) > noticeMaxRunes {
		return domain.BadRequest("text_too_long", "text 太长：群消息只写要对方回复或动手的事，详情放引擎记录")
	}
	var roles []string
	seen := map[string]bool{}
	for _, r := range to {
		if r = strings.TrimSpace(r); r != "" && !seen[r] {
			seen[r] = true
			roles = append(roles, r)
		}
	}
	if len(roles) == 0 {
		return domain.BadRequest("to_required", "to 不能为空：写收件角色（如 van、collector、writer）")
	}
	return e.store.Tx(ctx, func(q *sqlite.Queries) error {
		run, wf, err := hubRun(ctx, q, role, runID, "发群消息")
		if err != nil {
			return err
		}
		for _, r := range roles {
			if _, err := q.GetRole(ctx, r); err != nil {
				if isNoRows(err) {
					return domain.BadRequest("unknown_role", "无此角色："+r)
				}
				return err
			}
		}
		detail := evtDetail(map[string]any{"to": roles, "text": text})
		return emit(ctx, q, domain.Event{RunID: &run.ID, ActorID: &hub.ID, Type: domain.EvtHubNotice, DetailJSON: detail},
			&notice{target: roles[0], hub: wf.HubRoleCode, cc: roles[1:], payload: map[string]any{
				"run_id": run.ID, "text": text, "to": roles,
			}})
	})
}
