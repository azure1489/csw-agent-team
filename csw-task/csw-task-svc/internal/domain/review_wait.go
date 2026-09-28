package domain

import "time"

// 待审提醒：交付物停在某道闸上没人审。
//
// 09-28 r56 #592：v5 交上去时主编正在退回 v4，这条 @ 并进了那一轮被漏掉，之后八小时没人动。
// 逾期、未接单、无活动三类告警都只看执行方，「待审」一直是盲区。
const (
	EvtReviewWaiting   = "review_waiting"   // 待审超过阈值：提醒审核方（Van 闸提醒中枢去跟 Van）
	EvtReviewEscalated = "review_escalated" // 提醒 ReviewEscalateAfter 次仍无动作：升级 Van，抄送中枢

	// ReviewEscalateAfter 同一次等待提醒满这么多次还没人审，就升级到 Van。
	ReviewEscalateAfter = 3
	// ReviewRemindMax 同一次等待最多提醒这么多次（约一天），之后只在进度卡上挂着。
	ReviewRemindMax = 24
)

// reviewGaps 第 n 次提醒距上一次（第 1 次距开始等待）至少隔多久：15、15、30 分钟，之后每小时。
// 即开始等待后 15、30、60 分钟各提醒一次，再往后每小时一次。
var reviewGaps = []time.Duration{15 * time.Minute, 15 * time.Minute, 30 * time.Minute}

func reviewGap(reminded int) time.Duration {
	if reminded < len(reviewGaps) {
		return reviewGaps[reminded]
	}
	return time.Hour
}

// ReviewRemindDue 该不该再提醒一次。since = 开始等这道闸的时刻；last = 上次提醒时刻（零值 = 没提醒过）。
func ReviewRemindDue(now, since, last time.Time, reminded int) bool {
	if reminded >= ReviewRemindMax {
		return false
	}
	from := since
	if reminded > 0 && !last.IsZero() {
		from = last
	}
	return now.Sub(from) >= reviewGap(reminded)
}

// Beijing 群里的人按北京时间作息；夜间不把人叫起来。固定时区，不依赖运行环境的 tzdata。
var Beijing = time.FixedZone("UTC+8", 8*3600)

// ReviewEscalateDue 该不该升级到 Van：提醒够次数、还没升级过、且在北京时间 08:00–23:00。
// 夜里到点的等到早上 8 点再升级。
func ReviewEscalateDue(now time.Time, reminded int, escalated bool) bool {
	if escalated || reminded < ReviewEscalateAfter {
		return false
	}
	h := now.In(Beijing).Hour()
	return h >= 8 && h < 23
}
