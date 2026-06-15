package server

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// GET /api/v1/roster?chat_id= —— 花名册（与 skill/roster.json 结构等价，供 agent 群播报查 open_id）。
// 缺省取第一个群。返回 { chat_id, roster{role_code:{open_id,name,bot[,user_id][,human]}}, reserved_bots{bot_name:open_id} }。
func (s *Server) handleRoster(c *gin.Context) {
	ctx := c.Request.Context()
	q := s.store.Q()

	var chat domain.Chat
	var err error
	if key := c.Query("chat_id"); key != "" {
		chat, err = q.GetChatByKey(ctx, key)
	} else {
		chat, err = q.FirstChat(ctx)
	}
	if err != nil {
		s.renderErr(c, domain.NotFound("chat_not_found", "无此群（花名册未配置）"))
		return
	}

	members, err := q.ListChatMembers(ctx, chat.ID)
	if err != nil {
		s.renderErr(c, err)
		return
	}

	roster := gin.H{}
	reserved := gin.H{}
	for _, m := range members {
		if m.Kind == "reserved" || m.RoleCode == nil {
			reserved[m.BotName] = m.OpenID
			continue
		}
		entry := gin.H{"open_id": m.OpenID, "name": m.DisplayName, "bot": m.BotName}
		if m.UserID != nil && *m.UserID != "" {
			entry["user_id"] = *m.UserID
		}
		if m.IsHuman {
			entry["human"] = true
		}
		roster[*m.RoleCode] = entry
	}

	c.JSON(http.StatusOK, gin.H{
		"chat_id":       chat.ChatKey,
		"roster":        roster,
		"reserved_bots": reserved,
	})
}
