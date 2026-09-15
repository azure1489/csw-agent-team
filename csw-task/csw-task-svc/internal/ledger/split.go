package ledger

import (
	"regexp"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// heading 合集里的条目标题：「品牌｜一句话标题」（可带 # 或 ** 包裹）；导读行「01｜品牌｜一句话」不算。
var (
	heading = regexp.MustCompile(`^(?:#{1,4}\s*)?(?:\*\*)?\s*([^｜|\n#*]{1,40}?)\s*[｜|]\s*([^\n]+?)\s*(?:\*\*)?\s*$`)
	tocLine = regexp.MustCompile(`^\s*\d{1,2}\s*[｜|]`)
)

// SplitWechat 按正文小节把公众号合集切成条目（自动拆分，需人工校对一次）。
func SplitWechat(body string) []domain.LedgerPostItem {
	var out []domain.LedgerPostItem
	seen := map[string]bool{}
	for _, line := range strings.Split(body, "\n") {
		l := strings.TrimSpace(line)
		if l == "" || tocLine.MatchString(l) || strings.HasPrefix(l, "![") || strings.HasPrefix(l, "http") {
			continue
		}
		m := heading.FindStringSubmatch(l)
		if m == nil {
			continue
		}
		brand, title := strings.TrimSpace(m[1]), strings.TrimSpace(m[2])
		if brand == "" || title == "" || len([]rune(brand)) > 30 || seen[brand+"｜"+title] {
			continue
		}
		seen[brand+"｜"+title] = true
		out = append(out, domain.LedgerPostItem{Brand: brand, Title: title, SplitBy: "auto"})
	}
	return out
}
