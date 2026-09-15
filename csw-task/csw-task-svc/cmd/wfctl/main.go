// Command wfctl 定义面客户端：导出 / 比对 / 检查 / 草稿写回 / 人工确认激活（走管理后台 API）。
package main

import (
	"os"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/wfctl"
)

func main() {
	os.Exit(wfctl.Run(os.Args[1:]))
}
