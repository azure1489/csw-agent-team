package adminsrv

import (
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// ── 群（chats）──

func memberDTO(m domain.ChatMember) gin.H {
	role := ""
	if m.RoleCode != nil {
		role = *m.RoleCode
	}
	userID := ""
	if m.UserID != nil {
		userID = *m.UserID
	}
	return gin.H{
		"id": m.ID, "chat_id": m.ChatID, "kind": m.Kind, "role_code": role,
		"open_id": m.OpenID, "user_id": userID, "display_name": m.DisplayName,
		"bot_name": m.BotName, "is_human": m.IsHuman, "sort": m.Sort,
	}
}

// GET /admin/chats —— 列群（含成员数）
func (s *Server) handleListChats(c *gin.Context) {
	ctx := c.Request.Context()
	chats, err := s.store.Q().ListChats(ctx)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(chats))
	for _, ch := range chats {
		members, _ := s.store.Q().ListChatMembers(ctx, ch.ID)
		out = append(out, gin.H{
			"id": ch.ID, "chat_key": ch.ChatKey, "name": ch.Name, "note": ch.Note,
			"member_count": len(members), "created_at": ch.CreatedAt, "updated_at": ch.UpdatedAt,
		})
	}
	c.JSON(http.StatusOK, gin.H{"chats": out})
}

// GET /admin/chats/:id/members —— 列某群成员
func (s *Server) handleListChatMembers(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	members, err := s.store.Q().ListChatMembers(c.Request.Context(), id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(members))
	for _, m := range members {
		out = append(out, memberDTO(m))
	}
	c.JSON(http.StatusOK, gin.H{"members": out})
}

type chatReq struct {
	ChatKey string `json:"chat_key"`
	Name    string `json:"name" binding:"required"`
	Note    string `json:"note"`
}

// POST /admin/chats
func (s *Server) handleCreateChat(c *gin.Context) {
	var req chatReq
	if err := c.ShouldBindJSON(&req); err != nil || req.ChatKey == "" {
		s.renderErr(c, domain.BadRequest("bad_body", "chat_key/name 必填"))
		return
	}
	id, err := s.store.Q().CreateChat(c.Request.Context(), req.ChatKey, req.Name, req.Note)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "chat_create", req.ChatKey, "")
	c.JSON(http.StatusCreated, gin.H{"id": id})
}

// PATCH /admin/chats/:id —— 改群名/备注（chat_key 不可改）
func (s *Server) handleUpdateChat(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req chatReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	if err := s.store.Q().UpdateChat(c.Request.Context(), id, req.Name, req.Note); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "chat_update", c.Param("id"), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// DELETE /admin/chats/:id —— 删群（superadmin；成员级联删）
func (s *Server) handleDeleteChat(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	if err := s.store.Q().DeleteChat(c.Request.Context(), id); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "chat_delete", c.Param("id"), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// ── 成员（chat_members）──

type memberReq struct {
	Kind        string `json:"kind"`      // mapped | reserved
	RoleCode    string `json:"role_code"` // mapped 必填
	OpenID      string `json:"open_id" binding:"required"`
	UserID      string `json:"user_id"`
	DisplayName string `json:"display_name" binding:"required"`
	BotName     string `json:"bot_name" binding:"required"`
	IsHuman     bool   `json:"is_human"`
	Sort        int    `json:"sort"`
}

// toMember 把请求体规整为 domain.ChatMember（校验 + 空串转 nil）。
func (req memberReq) toMember(chatID int64) (domain.ChatMember, error) {
	kind := req.Kind
	if kind == "" {
		kind = "mapped"
	}
	if kind != "mapped" && kind != "reserved" {
		return domain.ChatMember{}, domain.BadRequest("bad_kind", "kind 须为 mapped 或 reserved")
	}
	m := domain.ChatMember{
		ChatID: chatID, Kind: kind, OpenID: req.OpenID,
		DisplayName: req.DisplayName, BotName: req.BotName, IsHuman: req.IsHuman, Sort: req.Sort,
	}
	if kind == "mapped" {
		if req.RoleCode == "" {
			return domain.ChatMember{}, domain.BadRequest("role_required", "mapped 成员必须指定 role_code")
		}
		rc := req.RoleCode
		m.RoleCode = &rc
	}
	if req.UserID != "" {
		uid := req.UserID
		m.UserID = &uid
	}
	return m, nil
}

// POST /admin/chats/:id/members
func (s *Server) handleCreateChatMember(c *gin.Context) {
	chatID, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	var req memberReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	m, err := req.toMember(chatID)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	id, err := s.store.Q().CreateChatMember(c.Request.Context(), m)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "member_create", m.BotName, "")
	c.JSON(http.StatusCreated, gin.H{"id": id})
}

// PATCH /admin/chats/:id/members/:mid
func (s *Server) handleUpdateChatMember(c *gin.Context) {
	chatID, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	mid, err := pathID(c, "mid")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	existing, err := s.store.Q().GetChatMember(c.Request.Context(), mid)
	if err != nil || existing.ChatID != chatID {
		s.renderErr(c, domain.NotFound("member_not_found", "无此成员"))
		return
	}
	var req memberReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	m, err := req.toMember(chatID)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	m.ID = mid
	if err := s.store.Q().UpdateChatMember(c.Request.Context(), m); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "member_update", strconv.FormatInt(mid, 10), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}

// DELETE /admin/chats/:id/members/:mid
func (s *Server) handleDeleteChatMember(c *gin.Context) {
	chatID, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	mid, err := pathID(c, "mid")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	existing, err := s.store.Q().GetChatMember(c.Request.Context(), mid)
	if err != nil || existing.ChatID != chatID {
		s.renderErr(c, domain.NotFound("member_not_found", "无此成员"))
		return
	}
	if err := s.store.Q().DeleteChatMember(c.Request.Context(), mid); err != nil {
		s.renderErr(c, err)
		return
	}
	s.audit(c, "member_delete", strconv.FormatInt(mid, 10), "")
	c.JSON(http.StatusOK, gin.H{"ok": true})
}
