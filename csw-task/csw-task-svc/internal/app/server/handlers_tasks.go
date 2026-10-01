package server

import (
	"encoding/json"
	"net/http"
	"path/filepath"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/engine"
)

// GET /me/tasks?status=open|all
func (s *Server) handleMyTasks(c *gin.Context) {
	agent, _ := mwAgent(c)
	openOnly := c.DefaultQuery("status", "open") != "all"
	tasks, err := s.store.Q().ListMyTasks(c.Request.Context(), agent.ID, openOnly)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	subjects := map[int64]string{}
	out := make([]gin.H, 0, len(tasks))
	for _, t := range tasks {
		subj, ok := subjects[t.RunID]
		if !ok {
			if r, err := s.store.Q().GetRun(c.Request.Context(), t.RunID); err == nil {
				subj = r.Subject
				subjects[t.RunID] = subj
			}
		}
		out = append(out, gin.H{"task": toTaskDTO(t), "run_subject": subj})
	}
	c.JSON(http.StatusOK, gin.H{"tasks": out, "skill_min_version": s.cfg.SkillMinVersion})
}

// GET /tasks/:id —— 作业手册 + 自检标准 + 验收标准 + 派工单 + 各版本产出 + 闸进度
func (s *Server) handleTaskDetail(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ctx := c.Request.Context()
	q := s.store.Q()
	t, err := q.GetTask(ctx, id)
	if err != nil {
		s.renderErr(c, domain.NotFound("task_not_found", "无此任务"))
		return
	}

	gates, _ := q.ListTaskGates(ctx, id)
	gateDTOs := make([]gateDTO, 0, len(gates))
	for _, g := range gates {
		gateDTOs = append(gateDTOs, gateDTO{GateOrder: g.GateOrder, ReviewerRole: g.ReviewerRole, RelayedByHub: g.RelayedByHub, Name: g.Name})
	}

	delivs, _ := q.ListDeliverablesByTask(ctx, id)
	gateByID := make(map[int64]domain.TaskGate, len(gates))
	for _, g := range gates {
		gateByID[g.ID] = g
	}
	var latestDispatch *deliverableDTO
	produced := make([]deliverableDTO, 0)
	supplements := make([]deliverableDTO, 0)
	for _, d := range delivs {
		ups, _ := q.ListUpstreams(ctx, d.ID)
		dto := toDeliverableDTO(d, ups)
		switch d.Kind {
		case domain.KindDispatch:
			cp := dto
			latestDispatch = &cp // 按 id 递增，最后一条即最新派工单
		case domain.KindSupplement:
			supplements = append(supplements, dto)
		default:
			// 附最新一条审核记录：被退回的版本由此自助读到方向/位置，不依赖群消息原文。
			if rs, err := q.ListReviewsByDeliverable(ctx, d.ID); err == nil && len(rs) > 0 {
				last := rs[len(rs)-1]
				rd := toReviewDTO(last)
				if g, ok := gateByID[last.TaskGateID]; ok {
					rd.GateOrder, rd.GateName = g.GateOrder, g.Name
				}
				dto.LatestReview = &rd
			}
			produced = append(produced, dto)
		}
	}

	var item *itemDTO // 逐条任务附条目登记：标题、品牌、来源、Van 决定原话
	if t.ItemKey != "" {
		if it, err := q.GetItem(ctx, t.RunID, t.ItemKey); err == nil {
			item = &toItemDTOs([]domain.RunItem{it})[0]
		}
	}

	body := gin.H{
		"task":                toTaskDTO(t),
		"item":                item,
		"instructions":        t.Instructions,
		"self_check_criteria": t.SelfCheckCriteria,
		"acceptance":          t.Acceptance,
		"gates":               gateDTOs,
		"dispatch":            latestDispatch,
		"deliverables":        produced,
		"supplements":         supplements,
		"feedback":            feedbackDTOs(taskFeedback(ctx, s, t)),
		"skill_min_version":   s.cfg.SkillMinVersion,
	}
	// 选题前三段随附排重底稿：近 30 天已发（含群发）与最近几期被否决 / 暂缓的条目，开工即可对照，
	// 不用等 Van 或主编逐次补链接。其余阶段不附，避免响应臃肿。
	if t.StageCode == "intake" || t.StageCode == "shortlist" || t.StageCode == "topic" {
		body["recent_posts"] = recentPosts(ctx, s, 30, 40)
		body["recent_rejected"] = recentRejected(ctx, s, 14, 20)
		// 来源台账是「本期应该扫哪些」的基准，开工即可对账；采集轮本身不附，避免响应臃肿。
		body["intake_sources"] = intakeSources(ctx, s)
	}
	c.JSON(http.StatusOK, body)
}

type submitUpstream struct {
	Label      string `json:"label"`
	URL        string `json:"url"`
	UpstreamID *int64 `json:"upstream_id"`
}

type submitReq struct {
	Kind        string           `json:"kind"`                   // 空=output / supplement / edit
	AffectsID   *int64           `json:"affects_deliverable_id"` // supplement 必填
	EditOf      *int             `json:"edit_of"`                // edit 必填
	DiffSummary string           `json:"diff_summary"`           // edit 必填
	DocType     string           `json:"doc_type"`
	DownloadURL string           `json:"download_url"`
	Filename    string           `json:"filename"`
	Title       string           `json:"title"`
	Summary     string           `json:"summary"`
	MetaJSON    string           `json:"meta_json"`
	SelfCheck   string           `json:"self_check"`
	FileID      *int64           `json:"file_id"`
	Upstreams   []submitUpstream `json:"upstreams"`
}

// POST /tasks/:id/deliverables —— agent 提交产出。两种形态：
//   - multipart/form-data（推荐，一步式）：file + doc_type/self_check/title/summary/meta_json/upstreams(JSON)。
//     文件名与存储路径由服务端按 工作流/subject/run/阶段/版本/责任方 派生——agent 不拼路径、不算版本。
//   - application/json（兼容，两步式）：先 POST /files 拿 file_id 再提交。
//
// 两种形态 doc_type 都可省略（缺省取阶段产出类型 output_type）。
func (s *Server) handleSubmit(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	if c.ContentType() == "multipart/form-data" {
		s.handleSubmitMultipart(c, id)
		return
	}

	var req submitReq
	if err := c.ShouldBindJSON(&req); err != nil {
		s.renderErr(c, domain.BadRequest("bad_body", "请求体非法："+err.Error()))
		return
	}
	agent, _ := mwAgent(c)

	// 附了 file 但没给 download_url 时，用本服务下载链接。
	if req.FileID != nil && req.DownloadURL == "" {
		req.DownloadURL = s.fileURL(*req.FileID)
	}

	in := engine.SubmitInput{
		DocType: req.DocType, DownloadURL: req.DownloadURL, Filename: req.Filename,
		Title: req.Title, Summary: req.Summary, MetaJSON: req.MetaJSON, SelfCheck: req.SelfCheck, FileID: req.FileID,
		Kind: domain.DeliverableKind(req.Kind), AffectsID: req.AffectsID, EditOf: req.EditOf, DiffSummary: req.DiffSummary,
	}
	for _, u := range req.Upstreams {
		in.Upstreams = append(in.Upstreams, engine.UpstreamInput{Label: u.Label, URL: u.URL, UpstreamID: u.UpstreamID})
	}

	d, err := s.eng.Submit(c.Request.Context(), agent, id, in)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ups, _ := s.store.Q().ListUpstreams(c.Request.Context(), d.ID)
	c.JSON(http.StatusCreated, gin.H{"deliverable": toDeliverableDTO(d, ups), "task_id": id})
}

// handleSubmitMultipart 一步式提交：收文件 → 服务端派生 filename/object_key → 落 blob → 建交付物。
func (s *Server) handleSubmitMultipart(c *gin.Context, taskID int64) {
	agent, _ := mwAgent(c)
	ctx := c.Request.Context()
	q := s.store.Q()

	c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, s.cfg.MaxUploadBytes+(1<<20))
	kind := domain.DeliverableKind(c.PostForm("kind"))
	switch kind {
	case "":
		kind = domain.KindOutput
	case domain.KindOutput, domain.KindSupplement, domain.KindEdit:
	default:
		s.renderErr(c, domain.BadRequest("bad_kind", "kind 须为 output / supplement / edit"))
		return
	}
	affects, err := optInt64(c.PostForm("affects_deliverable_id"))
	if err != nil {
		s.renderErr(c, domain.BadRequest("bad_affects", "affects_deliverable_id 须为整数"))
		return
	}
	editOf, err := optInt64(c.PostForm("edit_of"))
	if err != nil {
		s.renderErr(c, domain.BadRequest("bad_edit_of", "edit_of 须为整数"))
		return
	}

	task, err := q.GetTask(ctx, taskID)
	if err != nil {
		s.renderErr(c, domain.NotFound("task_not_found", "无此任务"))
		return
	}
	// 前置守卫（引擎事务内还会再守）：避免明知不可提交还白传一份 blob。
	if err := engine.CheckAccept(kind, task.Status); err != nil {
		s.renderErr(c, err)
		return
	}
	if kind == domain.KindOutput {
		if err := engine.RequireAuthorization(ctx, q, task); err != nil {
			s.renderErr(c, err)
			return
		}
	}
	run, err := q.GetRun(ctx, task.RunID)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	wf, err := q.GetWorkflow(ctx, run.WorkflowID)
	if err != nil {
		s.renderErr(c, err)
		return
	}

	// 责任方：普通阶段=角色名（如「文案」）；合流阶段=产出类型（如「成品」，多方合成不署单一角色）。
	owner := task.RoleCode
	if r, err := q.GetRole(ctx, task.RoleCode); err == nil && r.Name != "" {
		owner = r.Name
	}
	if task.IsMerge && task.OutputType != "" {
		owner = task.OutputType
	}
	if kind == domain.KindSupplement {
		owner += "补件" // 补件单独一条版本流，文件名与产出区分
	}

	ver, err := q.MaxDeliverableVersion(ctx, taskID, kind)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ver++

	fh, err := c.FormFile("file")
	if err != nil {
		s.renderErr(c, domain.BadRequest("no_file", "缺少 multipart 字段 file"))
		return
	}
	if fh.Size > s.cfg.MaxUploadBytes {
		s.renderErr(c, domain.BadRequest("too_large", "文件超过大小上限"))
		return
	}
	ct := fh.Header.Get("Content-Type")
	if len(s.cfg.AllowedContentTypes) > 0 && ct != "" && !s.cfg.AllowedContentTypes[ct] {
		s.renderErr(c, domain.BadRequest("bad_content_type", "不支持的 content-type："+ct))
		return
	}

	var upstreams []submitUpstream
	if raw := c.PostForm("upstreams"); raw != "" {
		if err := json.Unmarshal([]byte(raw), &upstreams); err != nil {
			s.renderErr(c, domain.BadRequest("bad_upstreams", "upstreams 须为 JSON 数组：[{label,url,upstream_id?}]"))
			return
		}
	}

	f, err := fh.Open()
	if err != nil {
		s.renderErr(c, err)
		return
	}
	defer f.Close()

	filename, objectKey := deriveDeliverableNames(
		wf.Name, run.Subject, run.ID, task.StageName, task.ItemKey, owner, ver, filepath.Ext(fh.Filename))

	sha, storagePath, size, err := s.blobs.Put(f, objectKey)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	uploader := agent.ID
	fileID, err := q.InsertFile(ctx, domain.File{
		SHA256: sha, Filename: filename, ByteSize: size, ContentType: ct,
		StoragePath: storagePath, UploadedBy: &uploader,
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}

	in := engine.SubmitInput{
		DocType: c.PostForm("doc_type"), DownloadURL: s.downloadURL(storagePath, fileID), Filename: filename,
		Title: c.PostForm("title"), Summary: c.PostForm("summary"),
		MetaJSON: c.PostForm("meta_json"), SelfCheck: c.PostForm("self_check"), FileID: &fileID,
		Kind: kind, AffectsID: affects, DiffSummary: c.PostForm("diff_summary"),
	}
	if editOf != nil {
		v := int(*editOf)
		in.EditOf = &v
	}
	for _, u := range upstreams {
		in.Upstreams = append(in.Upstreams, engine.UpstreamInput{Label: u.Label, URL: u.URL, UpstreamID: u.UpstreamID})
	}

	d, err := s.eng.Submit(ctx, agent, taskID, in)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	ups, _ := q.ListUpstreams(ctx, d.ID)
	c.JSON(http.StatusCreated, gin.H{
		"deliverable": toDeliverableDTO(d, ups), "task_id": taskID,
		"file": gin.H{"file_id": fileID, "sha256": sha, "size": size, "object_key": storagePath},
	})
}

type reasonReq struct {
	Reason string `json:"reason"`
}

// bindReason 读可选 JSON 体里的 reason（空体视为空原因，由引擎判定是否必填）。
func bindReason(c *gin.Context) (string, error) {
	var req reasonReq
	if c.Request.ContentLength == 0 {
		return "", nil
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		return "", domain.BadRequest("bad_body", "请求体非法："+err.Error())
	}
	return req.Reason, nil
}

// POST /tasks/:id/ack —— 执行者接单（dispatched→in_progress）；已接单时再调用只刷新活动时间
func (s *Server) handleAck(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	agent, _ := mwAgent(c)
	t, err := s.eng.Ack(c.Request.Context(), agent, id)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"task": toTaskDTO(t)})
}

// POST /tasks/:id/fail {reason} —— 执行者报告无法完成
func (s *Server) handleFail(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	reason, err := bindReason(c)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	agent, _ := mwAgent(c)
	t, err := s.eng.Fail(c.Request.Context(), agent, id, reason)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"task": toTaskDTO(t)})
}

// POST /tasks/:id/cancel {reason} —— 中枢取消（尚未开始的下游一并取消）
func (s *Server) handleCancel(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	reason, err := bindReason(c)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	agent, role := mwAgent(c)
	res, err := s.eng.Cancel(c.Request.Context(), agent, role, id, reason)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"task": toTaskDTO(res.Task), "cancelled_task_ids": res.Cancelled})
}

// POST /tasks/:id/reopen {reason} —— 中枢重开（失败 / 已取消 / 已通过 → ready 或 blocked，不自动派工）
func (s *Server) handleReopen(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	reason, err := bindReason(c)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	agent, role := mwAgent(c)
	t, err := s.eng.Reopen(c.Request.Context(), agent, role, id, reason)
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{"task": toTaskDTO(t)})
}
