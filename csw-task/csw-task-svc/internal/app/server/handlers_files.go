package server

import (
	"io"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/files"
)

// POST /files —— 上传 zip（multipart），内容寻址去重，返回下载链接
func (s *Server) handleUpload(c *gin.Context) {
	agent, _ := mwAgent(c)
	ctx := c.Request.Context()

	// 限制请求体大小。
	c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, s.cfg.MaxUploadBytes+(1<<20))
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

	f, err := fh.Open()
	if err != nil {
		s.renderErr(c, err)
		return
	}
	defer f.Close()

	// object_key（可选）：调用方按交付规范拼好对象路径，如
	//   csw/资讯日更/2026-06-10/03-公众号内容/资讯日更_文案_20260610_v1.zip
	// 留空则内容寻址（blobs/<sha>，按 sha 去重）。
	objectKey := c.PostForm("object_key")

	sha, storagePath, size, err := s.blobs.Put(f, objectKey)
	if err != nil {
		s.renderErr(c, err)
		return
	}

	// 仅内容寻址时按 sha 去重；语义 key（规范路径）每个交付物独立留底。
	q := s.store.Q()
	if objectKey == "" {
		if existing, err := q.FileBySHA(ctx, sha); err == nil {
			c.JSON(http.StatusCreated, gin.H{
				"file_id": existing.ID, "download_url": s.downloadURL(existing.StoragePath, existing.ID),
				"sha256": existing.SHA256, "size": existing.ByteSize, "deduped": true,
			})
			return
		}
	}

	uploader := agent.ID
	fileID, err := q.InsertFile(ctx, domain.File{
		SHA256: sha, Filename: fh.Filename, ByteSize: size, ContentType: ct,
		StoragePath: storagePath, UploadedBy: &uploader,
	})
	if err != nil {
		s.renderErr(c, err)
		return
	}
	c.JSON(http.StatusCreated, gin.H{
		"file_id": fileID, "download_url": s.downloadURL(storagePath, fileID), "sha256": sha, "size": size,
	})
}

// GET /files/:id —— 流式下载（支持 Range）
func (s *Server) handleDownload(c *gin.Context) {
	id, err := pathID(c, "id")
	if err != nil {
		s.renderErr(c, err)
		return
	}
	file, err := s.store.Q().GetFile(c.Request.Context(), id)
	if err != nil {
		s.renderErr(c, domain.NotFound("file_not_found", "无此文件"))
		return
	}
	// OSS 等支持公共直链的后端：302 重定向到永久公共 URL（无需鉴权 / 签名）。
	if pu, ok := s.blobs.(files.PublicURLer); ok {
		c.Redirect(http.StatusFound, pu.PublicURL(file.StoragePath))
		return
	}

	// 本地后端：服务端代理流式下载（支持 Range）。
	rd, err := s.blobs.Open(file.StoragePath)
	if err != nil {
		s.renderErr(c, domain.NotFound("blob_missing", "文件内容缺失"))
		return
	}
	defer rd.Close()

	c.Header("Content-Disposition", "attachment; filename=\""+file.Filename+"\"")
	if file.ContentType != "" {
		c.Header("Content-Type", file.ContentType)
	}
	if rs, ok := rd.(io.ReadSeeker); ok {
		http.ServeContent(c.Writer, c.Request, file.Filename, time.Time{}, rs)
	} else {
		_, _ = io.Copy(c.Writer, rd)
	}
}

// downloadURL：OSS 公共后端返回永久公共直链；否则返回经服务端代理的 /files/:id。
func (s *Server) downloadURL(storagePath string, fileID int64) string {
	if pu, ok := s.blobs.(files.PublicURLer); ok {
		return pu.PublicURL(storagePath)
	}
	return s.fileURL(fileID)
}
