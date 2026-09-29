package notifier

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
)

// member 花名册里一个角色的 @ 信息。
type member struct {
	OpenID string
	Name   string
}

// roster role_code → 群成员。
type roster map[string]member

var scopeText = map[string]string{
	"local_drill": "本地演练", "wx_draft": "公众号草稿", "wx_publish": "公众号发布",
	"xhs_draft": "小红书草稿", "xhs_publish": "小红书发布",
}

// at 渲染真 at-tag；花名册查不到 open_id 时回退纯文本（调用方负责抄送中枢）。
func (r roster) at(role string) (string, bool) {
	if m, ok := r[role]; ok && m.OpenID != "" {
		name := m.Name
		if name == "" {
			name = role
		}
		return fmt.Sprintf(`<at user_id="%s">%s</at>`, m.OpenID, name), true
	}
	return "@" + role + "（花名册无此角色）", false
}

type payload map[string]any

func (p payload) s(k string) string {
	switch v := p[k].(type) {
	case string:
		return v
	case float64:
		return fmt.Sprintf("%d", int64(v))
	case nil:
		return ""
	default:
		return fmt.Sprint(v)
	}
}

func (p payload) list(k string) string {
	arr, _ := p[k].([]any)
	out := make([]string, 0, len(arr))
	for _, v := range arr {
		out = append(out, fmt.Sprint(v))
	}
	return strings.Join(out, "、")
}

func or(s, def string) string {
	if strings.TrimSpace(s) == "" {
		return def
	}
	return s
}

// vanAllowed 白名单：只有「待 Van 闸审核」与「待审升级」的通知才能主送 Van；其余一律降级给中枢。
func vanAllowed(eventType string, p payload) bool {
	if eventType == domain.EvtReviewEscalated {
		return true
	}
	return (eventType == domain.EvtSubmitted || eventType == domain.EvtGatePassed) && p.s("next_reviewer") == "van"
}

// render 把一条 outbox 渲染成群消息文本（≤3 行、必带 ids）。返回文本与是否发生了 Van 降级。
func render(o domain.Outbox, r roster) (string, bool) {
	var p payload
	_ = json.Unmarshal([]byte(o.PayloadJSON), &p)
	if p == nil {
		p = payload{}
	}
	hub := p.s("hub_role")
	target := o.TargetRole
	downgraded := false
	if target == "van" && !vanAllowed(o.EventType, p) {
		target, downgraded = hub, true
	}
	cc := []string{}
	for _, c := range strings.Split(o.CCRoles, ",") {
		if c = strings.TrimSpace(c); c != "" && c != target && (c != "van" || vanAllowed(o.EventType, p)) {
			cc = append(cc, c)
		}
	}
	to, ok := r.at(target)
	if !ok && target != hub && hub != "" {
		cc = append(cc, hub) // @ 不到人：抄送中枢兜底
	}

	run, stage, task, ver := p.s("run_id"), p.s("stage_name"), p.s("task_id"), p.s("version")
	var line string
	switch o.EventType {
	case domain.EvtDispatched:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 新派工：%s。详情与上游见引擎任务 #%s", run, stage, task, to, or(p.s("note"), "按任务说明执行"), task)
	case domain.EvtSubmitted, domain.EvtGatePassed:
		head := fmt.Sprintf("【r%s·%s·v%s】", run, stage, ver)
		switch {
		case p.s("next_reviewer") == "van" && target == "van":
			line = fmt.Sprintf("%s%s 请审（%s）：%s", head, to, p.s("next_gate_name"), p.s("download_url"))
			if pg := p.s("passed_gate_name"); pg != "" {
				line = fmt.Sprintf("%s%s %s已过，请审（%s）：%s", head, to, pg, p.s("next_gate_name"), p.s("download_url"))
			}
		case o.EventType == domain.EvtSubmitted:
			line = fmt.Sprintf("%s%s 待审（%s）交付物#%s：%s %s", head, to, p.s("next_gate_name"), p.s("deliverable_id"), or(p.s("self_check"), "已自检"), p.s("download_url"))
		default:
			line = fmt.Sprintf("%s%s %s已过，待%s：%s", head, to, p.s("passed_gate_name"), p.s("next_gate_name"), p.s("download_url"))
		}
		if p.s("edit") == "true" {
			line += "（主编定点编辑：" + p.s("diff_summary") + "）"
		}
	case domain.EvtGateReturned:
		line = fmt.Sprintf("【r%s·%s·v%s】%s 退回（%s）：方向=%s；位置=%s", run, stage, ver, to, p.s("gate_name"), p.s("return_direction"), p.s("return_location"))
	case domain.EvtStagePassed:
		line = fmt.Sprintf("【r%s·%s】%s 已通过 v%s。下一棒：%s", run, stage, to, ver, or(p.list("next_stages"), "无（本支线结束）"))
	case domain.EvtTaskReady:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 已就绪，待派工（本阶段手动派工）", run, stage, task, to)
	case domain.EvtAuthorizationRequired:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 平台写操作待授权：%s。录入 Van 原话后引擎自动派工", run, stage, task, to, or(scopeText[p.s("scope")], p.s("scope")))
	case domain.EvtSupplementArrived:
		line = fmt.Sprintf("【r%s·%s·补件#%s】%s 补件已到（影响交付物#%s）%s。待返工：%s", run, stage, ver, to, p.s("affects_deliverable_id"), or(p.s("summary"), ""), or(p.list("rework_stages"), "无"))
	case domain.EvtTaskFailed:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 执行方报告无法完成：%s", run, stage, task, to, p.s("reason"))
	case domain.EvtOverdue:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 已逾期（时限 %s 分钟，截止 %s）", run, stage, task, to, p.s("sla_minutes"), p.s("due_at"))
	case domain.EvtAckOverdue:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 已派工 %s 分钟仍未接单：请先接单再开工；做不了请报告失败", run, stage, task, to, p.s("minutes"))
	case domain.EvtAckEscalated:
		line = fmt.Sprintf("【r%s·%s·任务#%s】%s 执行方（%s）派工 %s 分钟仍未接单，请改派、重开或取消", run, stage, task, to, p.s("assignee_role"), p.s("minutes"))
	case domain.EvtTaskIdle:
		if p.s("role_code") != "" && p.s("role_code") == hub {
			// 中枢自己产出的（03 选题方案 / 07 整合稿）：心跳不算进展。给了「心跳」这个出口，
			// 主编每次被叫醒就补一次心跳、看一张图就收尾，03 一步一步挪了一个下午（09-29 r56 #594）
			line = fmt.Sprintf("【r%s·%s·任务#%s】%s 接单后 %s 分钟没有产物：请在这一轮里连续做到提交，只回心跳或进度不算进展；做不了就报告失败并写明原因", run, stage, task, to, p.s("minutes"))
		} else {
			line = fmt.Sprintf("【r%s·%s·任务#%s】%s 接单后 %s 分钟没有心跳或产物：请提交、心跳或报告失败", run, stage, task, to, p.s("minutes"))
		}
	case domain.EvtReviewWaiting:
		head := fmt.Sprintf("【r%s·%s·v%s】", run, stage, ver)
		if p.s("relayed") == "true" {
			line = fmt.Sprintf("%s%s %s 已等 %s 分钟（第 %s 次提醒）：请跟进 Van 的决定并代录；交付物#%s", head, to, or(p.s("gate_name"), "Van 闸"), p.s("minutes"), p.s("reminded"), p.s("deliverable_id"))
		} else {
			line = fmt.Sprintf("%s%s 待审已 %s 分钟（%s，第 %s 次提醒）：请在这一轮里给出结论——通过或退回交付物#%s；只回进度不算处理，审不了请写明原因", head, to, p.s("minutes"), or(p.s("gate_name"), "当前闸"), p.s("reminded"), p.s("deliverable_id"))
		}
	case domain.EvtReviewEscalated:
		line = fmt.Sprintf("【r%s·%s·v%s】%s 待审已 %s 分钟（%s），已提醒审核方 %s 次仍无动作，请过问；交付物#%s", run, stage, ver, to, p.s("minutes"), or(p.s("gate_name"), "当前闸"), p.s("reminded"), p.s("deliverable_id"))
	case domain.EvtRunStalled:
		// run 级事件没有 task / stage，单独成句：整期还活着却没人在动，要中枢接出下一步。
		line = fmt.Sprintf("【r%s·整期停滞】%s 本期已 %s 分钟没有任何人在动，还没走完：%s。\n报失败或收到退回后要在同一轮接出下一步——重开/重派具体任务，或明确宣布本期停止；只发状态播报不算处置",
			run, to, p.s("minutes"), or(p.s("pending"), "（无待办任务，可考虑 close 或 abort 收尾）"))
	default:
		line = fmt.Sprintf("【r%s·%s】%s %s", run, stage, to, o.EventType)
	}
	if len(cc) > 0 {
		ats := make([]string, 0, len(cc))
		for _, c := range cc {
			a, _ := r.at(c)
			ats = append(ats, a)
		}
		line += "\n抄送 " + strings.Join(ats, " ")
	}
	return line, downgraded
}

// renderProgress 进度卡（不 @ 人，只作群内仪表盘）。
func renderProgress(p engine.Progress) string {
	var b strings.Builder
	fmt.Fprintf(&b, "【r%d 进度】已交付 %d/%d｜当前：%s", p.RunID, p.Done, p.Total, or(strings.Join(p.CurrentNodes, "、"), "无"))
	var blockers []string
	for _, t := range p.Tasks {
		if t.Blocker != nil && t.Blocker.Kind != "upstream" {
			blockers = append(blockers, t.StageName+"："+t.Blocker.Text)
		}
		if len(blockers) == 3 {
			break
		}
	}
	if len(blockers) > 0 {
		b.WriteString("\n卡点：" + strings.Join(blockers, "；"))
	}
	if len(p.Authorizations) > 0 {
		labels := make([]string, 0, len(p.Authorizations))
		for _, a := range p.Authorizations {
			labels = append(labels, or(scopeText[a], a))
		}
		b.WriteString("\n授权：" + strings.Join(labels, "、"))
	}
	return b.String()
}
