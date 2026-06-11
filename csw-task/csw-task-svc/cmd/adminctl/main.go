// Command adminctl 本轮最小运维 CLI：迁移 / agent / token / 工作流校验。
package main

import (
	"os"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/app/adminctl"
)

func main() {
	os.Exit(adminctl.Run(os.Args[1:]))
}
