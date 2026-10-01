package server

import (
	"fmt"
	"strings"
)

// sanitizeSeg 清理路径/文件名片段：去首尾空白，把路径分隔符替换为 "-"，防止拼出越界路径；
// 纯 "."/".." 段替换为 "_"（本地后端 filepath.Join 下的目录穿越兜底）。
func sanitizeSeg(s string) string {
	s = strings.TrimSpace(s)
	s = strings.ReplaceAll(s, "/", "-")
	s = strings.ReplaceAll(s, "\\", "-")
	if s == "." || s == ".." {
		return "_"
	}
	return s
}

// compactSubject 压缩 subject 作文件名片段（如 2026-06-04 → 20260604）：去掉分隔类字符。
func compactSubject(s string) string {
	return strings.Map(func(r rune) rune {
		switch r {
		case '-', '_', '/', '\\', ':', ' ':
			return -1
		}
		return r
	}, s)
}

// deriveDeliverableNames 服务端派生交付物文件名与语义存储路径（命名/路径逻辑归接口层，agent 不拼路径不算版本）：
//
//	filename  = {工作流名}_{责任方}_{subject 压缩}_r{run}[_{条目}]_v{n}{ext}  如 资讯日更_文案_20260604_r17_v1.zip
//	objectKey = {工作流名}/{subject}/r{run}/{阶段名}[/{条目}]/{filename}       如 资讯日更/2026-06-04/r17/03-公众号内容/…zip
//
// 逐条阶段（写作、素材核等）同一期同一阶段有多个任务，各带条目键：**文件名与路径都要带上条目**，
// 否则几条的同版本会落到同一个对象上、后传的覆盖先传的（10-01 r60：04 写作五条下载到的都是 llama 稿）。
//
// 责任方由调用方算好传入（普通阶段=角色名；合流阶段=产出类型，如「成品」）。
// 阶段名沿用定义快照（形如「03-公众号内容」，自带序号，路径天然按阶段排序）。
// OSS 后端的最终对象 key 还会挂上 CSW_OSS_PREFIX（如 csw/），见 files.OSSStore。
func deriveDeliverableNames(wfName, subject string, runID int64, stageName, itemKey, owner string, version int, ext string) (filename, objectKey string) {
	if ext == "" {
		ext = ".zip"
	}
	wf, st, ow, it := sanitizeSeg(wfName), sanitizeSeg(stageName), sanitizeSeg(owner), sanitizeSeg(itemKey)
	item := ""
	if it != "" {
		item = "_" + it
	}
	filename = fmt.Sprintf("%s_%s_%s_r%d%s_v%d%s", wf, ow, compactSubject(subject), runID, item, version, ext)
	dir := fmt.Sprintf("%s/%s/r%d/%s", wf, sanitizeSeg(subject), runID, st)
	if it != "" {
		dir += "/" + it
	}
	objectKey = dir + "/" + filename
	return filename, objectKey
}
