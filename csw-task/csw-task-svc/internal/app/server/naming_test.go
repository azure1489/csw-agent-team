package server

import "testing"

func TestDeriveDeliverableNames(t *testing.T) {
	// 普通阶段：责任方=角色名。
	fn, key := deriveDeliverableNames("资讯日更", "2026-06-04", 17, "03-公众号内容", "", "文案", 1, ".zip")
	if fn != "资讯日更_文案_20260604_r17_v1.zip" {
		t.Fatalf("filename: %s", fn)
	}
	if key != "资讯日更/2026-06-04/r17/03-公众号内容/资讯日更_文案_20260604_r17_v1.zip" {
		t.Fatalf("objectKey: %s", key)
	}

	// 合流阶段：责任方=产出类型（成品）；ext 缺省补 .zip。
	fn, key = deriveDeliverableNames("资讯日更", "2026-06-04", 17, "05-公众号成品", "", "成品", 2, "")
	if fn != "资讯日更_成品_20260604_r17_v2.zip" {
		t.Fatalf("merge filename: %s", fn)
	}
	if key != "资讯日更/2026-06-04/r17/05-公众号成品/资讯日更_成品_20260604_r17_v2.zip" {
		t.Fatalf("merge objectKey: %s", key)
	}

	// 路径分隔符注入被清理；subject 压缩去除分隔类字符。
	fn, key = deriveDeliverableNames("a/b", "08:30 班次", 3, "01-采..//集", "", "x\\y", 1, ".zip")
	if fn != "a-b_x-y_0830班次_r3_v1.zip" {
		t.Fatalf("sanitized filename: %s", fn)
	}
	if key != "a-b/08:30 班次/r3/01-采..--集/a-b_x-y_0830班次_r3_v1.zip" {
		t.Fatalf("sanitized objectKey: %s", key)
	}

	// 逐条阶段：同一期同一阶段多个任务，文件名与路径都带条目键，不会互相覆盖（10-01 r60）
	a, ka := deriveDeliverableNames("资讯日更", "2026-10-01", 60, "04-公众号写作", "dodcamp-9a3c68", "文案", 1, ".zip")
	b, kb := deriveDeliverableNames("资讯日更", "2026-10-01", 60, "04-公众号写作", "llamaoffical-21f01b", "文案", 1, ".zip")
	if a != "资讯日更_文案_20261001_r60_dodcamp-9a3c68_v1.zip" {
		t.Fatalf("item filename: %s", a)
	}
	if ka != "资讯日更/2026-10-01/r60/04-公众号写作/dodcamp-9a3c68/资讯日更_文案_20261001_r60_dodcamp-9a3c68_v1.zip" {
		t.Fatalf("item objectKey: %s", ka)
	}
	if a == b || ka == kb {
		t.Fatal("两条的同版本不能落到同一个对象")
	}
	// 条目键里的路径分隔符同样被清理
	_, k := deriveDeliverableNames("w", "s", 1, "st", "../x/y", "o", 1, ".zip")
	if k != "w/s/r1/st/..-x-y/w_o_s_r1_..-x-y_v1.zip" {
		t.Fatalf("sanitized item key: %s", k)
	}
}

