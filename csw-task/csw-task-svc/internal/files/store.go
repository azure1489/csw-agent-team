// Package files 提供托管文件的内容寻址存储（BlobStore 接口 + 本地 / 阿里云 OSS 实现）。
package files

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

// BlobStore 文件托管抽象（本地内容寻址 / OSS）。
type BlobStore interface {
	// Put 读完整个流算 sha256；objectKey 非空则按该 key 存（交付规范语义路径），
	// 否则内容寻址 <sha 分片>（按 sha 去重）。返回 sha、存储路径(key)、字节数。
	Put(r io.Reader, objectKey string) (sha string, storagePath string, size int64, err error)
	// Open 据存储路径打开内容流（本地返回 *os.File，OSS 返回对象流）。
	Open(storagePath string) (io.ReadCloser, error)
}

// PublicURLer 可选接口：支持永久公共读直链的后端（OSS public-read bucket）。
// handleDownload 检测到该接口时 302 重定向到公共 URL（永久、无需鉴权 / 签名）。
type PublicURLer interface {
	PublicURL(storagePath string) string
}

// LocalStore 本地内容寻址存储：root/<sha[0:2]>/<sha[2:4]>/<sha>。
type LocalStore struct {
	root string
}

// NewLocalStore 构造本地存储（自动建根目录）。
func NewLocalStore(root string) (*LocalStore, error) {
	if err := os.MkdirAll(root, 0o755); err != nil {
		return nil, fmt.Errorf("mkdir blob root: %w", err)
	}
	return &LocalStore{root: root}, nil
}

// relPath 据 sha 算分片相对路径。
func relPath(sha string) string {
	return filepath.Join(sha[0:2], sha[2:4], sha)
}

// Put 先写临时文件并同步算 sha；objectKey 非空则按它落盘，否则内容寻址分片路径（去重）。
func (s *LocalStore) Put(r io.Reader, objectKey string) (string, string, int64, error) {
	tmp, err := os.CreateTemp(s.root, ".upload-*")
	if err != nil {
		return "", "", 0, fmt.Errorf("temp file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // 成功改名后 remove 无操作

	h := sha256.New()
	size, err := io.Copy(io.MultiWriter(tmp, h), r)
	if err != nil {
		tmp.Close()
		return "", "", 0, fmt.Errorf("write blob: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return "", "", 0, fmt.Errorf("close temp: %w", err)
	}

	sha := hex.EncodeToString(h.Sum(nil))
	rel := objectKey
	if rel == "" {
		rel = relPath(sha)
	}
	final := filepath.Join(s.root, filepath.FromSlash(rel))

	if objectKey == "" {
		if _, err := os.Stat(final); err == nil {
			return sha, rel, size, nil // 内容寻址去重
		}
	}
	if err := os.MkdirAll(filepath.Dir(final), 0o755); err != nil {
		return "", "", 0, fmt.Errorf("mkdir blob dir: %w", err)
	}
	if err := os.Rename(tmpName, final); err != nil {
		return "", "", 0, fmt.Errorf("rename blob: %w", err)
	}
	return sha, rel, size, nil
}

// Open 打开存储文件（*os.File 满足 io.ReadCloser，且额外是 io.ReadSeeker 供 Range）。
func (s *LocalStore) Open(storagePath string) (io.ReadCloser, error) {
	return os.Open(filepath.Join(s.root, storagePath))
}
