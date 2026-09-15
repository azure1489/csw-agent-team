// Command syncer 数据子系统运维 CLI（探针 / 回填 / 同步 / 快照 / 表格导入 / 拆条 / 反馈导入）。
package main

import (
	"os"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/syncer"
)

func main() {
	os.Exit(syncer.Run(os.Args[1:]))
}
