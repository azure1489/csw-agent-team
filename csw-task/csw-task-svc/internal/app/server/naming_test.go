package server

import "testing"

func TestDeriveDeliverableNames(t *testing.T) {
	// 普通阶段：责任方=角色名。
	fn, key := deriveDeliverableNames("资讯日更", "2026-06-04", 17, "03-公众号内容", "文案", 1, ".zip")
	if fn != "资讯日更_文案_20260604_r17_v1.zip" {
		t.Fatalf("filename: %s", fn)
	}
	if key != "资讯日更/2026-06-04/r17/03-公众号内容/资讯日更_文案_20260604_r17_v1.zip" {
		t.Fatalf("objectKey: %s", key)
	}

	// 合流阶段：责任方=产出类型（成品）；ext 缺省补 .zip。
	fn, key = deriveDeliverableNames("资讯日更", "2026-06-04", 17, "05-公众号成品", "成品", 2, "")
	if fn != "资讯日更_成品_20260604_r17_v2.zip" {
		t.Fatalf("merge filename: %s", fn)
	}
	if key != "资讯日更/2026-06-04/r17/05-公众号成品/资讯日更_成品_20260604_r17_v2.zip" {
		t.Fatalf("merge objectKey: %s", key)
	}

	// 路径分隔符注入被清理；subject 压缩去除分隔类字符。
	fn, key = deriveDeliverableNames("a/b", "08:30 班次", 3, "01-采..//集", "x\\y", 1, ".zip")
	if fn != "a-b_x-y_0830班次_r3_v1.zip" {
		t.Fatalf("sanitized filename: %s", fn)
	}
	if key != "a-b/08:30 班次/r3/01-采..--集/a-b_x-y_0830班次_r3_v1.zip" {
		t.Fatalf("sanitized objectKey: %s", key)
	}
}
