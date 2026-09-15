package server

import (
	"encoding/json"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/store/sqlite"
)

var relSince = regexp.MustCompile(`^(\d+)([hd])$`)

// parseSince 支持 30d / 72h 或 RFC3339 / 2006-01-02；返回 UTC 秒级字符串。
func parseSince(s string) (string, error) {
	if s == "" {
		return "", nil
	}
	if m := relSince.FindStringSubmatch(s); m != nil {
		n, _ := strconv.Atoi(m[1])
		d := time.Duration(n) * time.Hour
		if m[2] == "d" {
			d *= 24
		}
		return time.Now().UTC().Add(-d).Format("2006-01-02T15:04:05Z"), nil
	}
	for _, layout := range []string{time.RFC3339, "2006-01-02"} {
		if t, err := time.Parse(layout, s); err == nil {
			return t.UTC().Format("2006-01-02T15:04:05Z"), nil
		}
	}
	return "", domain.BadRequest("bad_since", "since 须为 30d / 72h 或日期")
}

// GET /ledger/posts?since=30d&platform=&brand=&state=&q= —— 发布记录查重；查不到时说明覆盖范围，不下「从未发布」的结论
func (s *Server) handleLedgerPosts(c *gin.Context) {
	ctx := c.Request.Context()
	since, err := parseSince(c.DefaultQuery("since", "30d"))
	if err != nil {
		s.renderErr(c, err)
		return
	}
	q := s.store.Q()
	posts, err := q.ListLedgerPosts(ctx, sqlite.LedgerFilter{Since: since, Platform: c.Query("platform"), Brand: c.Query("brand"),
		State: c.Query("state"), Q: c.Query("q")})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	out := make([]gin.H, 0, len(posts))
	for _, p := range posts {
		items, _ := q.ListPostItems(ctx, p.ID)
		is := make([]gin.H, 0, len(items))
		for _, it := range items {
			is = append(is, gin.H{"seq": it.Seq, "brand": it.Brand, "product": it.Product, "title": it.Title, "angle": it.Angle,
				"item_key": it.ItemKey, "source_url": it.SourceURL, "split_by": it.SplitBy})
		}
		out = append(out, gin.H{"platform": p.Platform, "account": p.Account, "post_id": p.PostID, "url": p.URL,
			"published_at": p.PublishedAt, "title": p.Title, "state": p.State, "source": p.Source, "items": is})
	}
	cov, err := q.LedgerCoverage(ctx)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	cs := make([]gin.H, 0, len(cov))
	for _, r := range cov {
		cs = append(cs, gin.H{"platform": r.Platform, "kind": r.Kind, "last_ok_at": r.FinishedAt, "window_from": r.WindowFrom,
			"window_to": r.WindowTo, "gap_note": r.GapNote})
	}
	resp := gin.H{"posts": out, "coverage": cs, "since": since, "verdict": "found"}
	if len(out) == 0 {
		resp["verdict"] = "not_found_in_synced_records"
		resp["note"] = "在已同步的记录中未发现；这不代表从未发布，覆盖范围见 coverage"
		if len(cs) == 0 {
			resp["note"] = "还没有任何成功的同步记录，无法判断是否发过"
		}
	}
	c.JSON(http.StatusOK, resp)
}

type feedbackDTO struct {
	Quote     string   `json:"quote"`
	SaidAt    string   `json:"said_at,omitempty"`
	SourceRef string   `json:"source_ref,omitempty"`
	ObjectRef string   `json:"object_ref,omitempty"`
	Kind      string   `json:"kind"`
	Stance    string   `json:"stance"`
	ExpiresAt string   `json:"expires_at,omitempty"`
	Tags      []string `json:"tags"`
	Images    []string `json:"images,omitempty"`
	ID        int64    `json:"id"`
}

func feedbackDTOs(rs []domain.FeedbackRecord) []feedbackDTO {
	out := make([]feedbackDTO, 0, len(rs))
	for _, r := range rs {
		d := feedbackDTO{ID: r.ID, Quote: r.Quote, SaidAt: r.SaidAt, SourceRef: r.SourceRef, ObjectRef: r.ObjectRef,
			Kind: r.Kind, Stance: r.Stance, ExpiresAt: r.ExpiresAt, Tags: r.Tags}
		if r.ImageRefsJSON != "" {
			_ = json.Unmarshal([]byte(r.ImageRefsJSON), &d.Images)
		}
		if d.Tags == nil {
			d.Tags = []string{}
		}
		out = append(out, d)
	}
	return out
}

func taskFeedback(ctx interface {
	Deadline() (time.Time, bool)
	Done() <-chan struct{}
	Err() error
	Value(any) any
}, s *Server, t domain.Task) []domain.FeedbackRecord {
	rs, err := engine.TaskFeedback(ctx, s.store.Q(), t, 5)
	if err != nil {
		s.log.Warn("task feedback", "err", err)
		return nil
	}
	return rs
}

// GET /memory/feedback?tags=brand:nanga,stage:topic&limit=
func (s *Server) handleListFeedback(c *gin.Context) {
	var tags []string
	for _, t := range strings.Split(c.Query("tags"), ",") {
		if t = strings.TrimSpace(t); t != "" {
			tags = append(tags, t)
		}
	}
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	if limit <= 0 {
		limit = 20 // SQLite 的 LIMIT 负数等于不限，不能直接透传
	}
	if limit > 100 {
		limit = 100
	}
	rs, err := s.store.Q().ListActiveFeedback(c.Request.Context(), tags, limit)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"feedback": feedbackDTOs(rs)})
}

type createFeedbackReq struct {
	Quote        string   `json:"quote" binding:"required"`
	SaidAt       string   `json:"said_at"`
	SourceRef    string   `json:"source_ref"`
	ObjectRef    string   `json:"object_ref"`
	Kind         string   `json:"kind"`
	Stance       string   `json:"stance"`
	ExpiresAt    string   `json:"expires_at"`
	Tags         []string `json:"tags"`
	Images       []string `json:"images"`
	SupersedesID *int64   `json:"supersedes_id"`
}

// POST /memory/feedback —— 管理类角色（主编）录入编辑反馈（原话必填；temporary 须带过期时间）
func (s *Server) handleCreateFeedback(c *gin.Context) {
	agent, role := mwAgent(c)
	if !role.IsManagement {
		s.renderErr(c, domain.Forbidden("not_management", "只有管理类角色可录入编辑反馈"))
		return
	}
	var req createFeedbackReq
	if err := c.ShouldBindJSON(&req); err != nil || strings.TrimSpace(req.Quote) == "" {
		s.renderErr(c, domain.BadRequest("bad_body", "quote（原话）必填"))
		return
	}
	switch req.Kind {
	case "":
		req.Kind = "recent"
	case "long_term", "case", "temporary", "recent":
	default:
		s.renderErr(c, domain.BadRequest("bad_kind", "kind 须为 long_term / case / temporary / recent"))
		return
	}
	if req.Stance == "" {
		req.Stance = "explicit"
	}
	if req.Stance != "explicit" && req.Stance != "inferred" {
		s.renderErr(c, domain.BadRequest("bad_stance", "stance 须为 explicit / inferred"))
		return
	}
	if req.Kind == "temporary" && req.ExpiresAt == "" {
		s.renderErr(c, domain.BadRequest("expires_required", "temporary 反馈须带 expires_at"))
		return
	}
	if req.ExpiresAt != "" {
		t, err := time.Parse(time.RFC3339, req.ExpiresAt)
		if err != nil {
			s.renderErr(c, domain.BadRequest("bad_expires_at", "expires_at 须为 RFC3339"))
			return
		}
		req.ExpiresAt = t.UTC().Format("2006-01-02T15:04:05Z")
	}
	imgs := ""
	if len(req.Images) > 0 {
		b, _ := json.Marshal(req.Images)
		imgs = string(b)
	}
	id, _, err := s.store.Q().InsertFeedback(c.Request.Context(), domain.FeedbackRecord{
		Quote: req.Quote, SaidAt: req.SaidAt, SourceRef: req.SourceRef, ObjectRef: req.ObjectRef, ImageRefsJSON: imgs,
		Kind: req.Kind, Stance: req.Stance, ExpiresAt: req.ExpiresAt, CuratedBy: agent.Name, Tags: req.Tags, SupersedesID: req.SupersedesID,
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusCreated, gin.H{"id": id})
}
