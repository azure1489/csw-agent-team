package domain

import (
	"testing"
	"time"
)

func TestReviewRemindDue(t *testing.T) {
	since := time.Date(2026, 9, 28, 9, 6, 0, 0, time.UTC)
	at := func(m int) time.Time { return since.Add(time.Duration(m) * time.Minute) }
	cases := []struct {
		name     string
		now      time.Time
		last     time.Time
		reminded int
		want     bool
	}{
		{"刚交上去不提醒", at(10), time.Time{}, 0, false},
		{"等满 15 分钟第一次提醒", at(15), time.Time{}, 0, true},
		{"第二次在 30 分钟", at(30), at(15), 1, true},
		{"第二次不到点", at(29), at(15), 1, false},
		{"第三次在 60 分钟", at(60), at(30), 2, true},
		{"之后每小时", at(119), at(60), 3, false},
		{"之后每小时到点", at(120), at(60), 3, true},
		{"提醒满上限就停", at(5000), at(4000), ReviewRemindMax, false},
	}
	for _, c := range cases {
		if got := ReviewRemindDue(c.now, since, c.last, c.reminded); got != c.want {
			t.Errorf("%s：got %v want %v", c.name, got, c.want)
		}
	}
}

func TestReviewEscalateDue(t *testing.T) {
	bj := func(h int) time.Time { return time.Date(2026, 9, 29, h, 0, 0, 0, Beijing) }
	if ReviewEscalateDue(bj(10), ReviewEscalateAfter-1, false) {
		t.Error("提醒不够次数不该升级")
	}
	if !ReviewEscalateDue(bj(10), ReviewEscalateAfter, false) {
		t.Error("白天提醒够次数该升级")
	}
	if ReviewEscalateDue(bj(10), ReviewEscalateAfter, true) {
		t.Error("升级过不再升级")
	}
	for _, h := range []int{23, 2, 7} {
		if ReviewEscalateDue(bj(h), 5, false) {
			t.Errorf("北京时间 %d 点不该把 Van 叫起来", h)
		}
	}
	if !ReviewEscalateDue(bj(8), 5, false) {
		t.Error("早上 8 点补升级")
	}
}
