package files

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net/url"
	"os"
	"strings"

	"github.com/aliyun/aliyun-oss-go-sdk/oss"
)

// OSSStore 阿里云 OSS 存储。对象以 public-read 上传，下载用永久公共直链（实现 BlobStore + PublicURLer）。
// 对象 key 一律落在 prefix 下（prefix = 本服务在 bucket 内的根目录，如 csw/）：
// 语义路径 <prefix>/<objectKey>（交付规范路径），内容寻址 <prefix>/<sha 分片>。
type OSSStore struct {
	bucket     *oss.Bucket
	prefix     string // 归一化为 "" 或 "xxx/"
	publicBase string // https://<bucket>.<host>
}

// NewOSSStore 构造 OSS 存储。
func NewOSSStore(endpoint, bucketName, keyID, keySecret, prefix string) (*OSSStore, error) {
	if endpoint == "" || bucketName == "" || keyID == "" || keySecret == "" {
		return nil, fmt.Errorf("oss 配置不完整：endpoint/bucket/access_key_id/access_key_secret 均必填")
	}
	client, err := oss.New(endpoint, keyID, keySecret)
	if err != nil {
		return nil, fmt.Errorf("oss client: %w", err)
	}
	bkt, err := client.Bucket(bucketName)
	if err != nil {
		return nil, fmt.Errorf("oss bucket: %w", err)
	}
	prefix = strings.Trim(prefix, "/")
	if prefix != "" {
		prefix += "/"
	}
	scheme, host := "https", endpoint
	if strings.HasPrefix(host, "http://") {
		scheme, host = "http", host[len("http://"):]
	} else {
		host = strings.TrimPrefix(host, "https://")
	}
	host = strings.TrimRight(host, "/")
	return &OSSStore{bucket: bkt, prefix: prefix, publicBase: scheme + "://" + bucketName + "." + host}, nil
}

// shaKey 内容寻址 key：<prefix>/<sha[0:2]>/<sha[2:4]>/<sha>。
func (s *OSSStore) shaKey(sha string) string {
	return s.prefix + sha[0:2] + "/" + sha[2:4] + "/" + sha
}

// Put 落临时文件算 sha；objectKey 非空则按它存（语义路径，每个交付物独立留底、直接覆盖上传），
// 否则内容寻址并按 sha 去重。
func (s *OSSStore) Put(r io.Reader, objectKey string) (string, string, int64, error) {
	tmp, err := os.CreateTemp("", "csw-oss-*")
	if err != nil {
		return "", "", 0, fmt.Errorf("temp file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)

	h := sha256.New()
	size, err := io.Copy(io.MultiWriter(tmp, h), r)
	if err != nil {
		tmp.Close()
		return "", "", 0, fmt.Errorf("buffer blob: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return "", "", 0, fmt.Errorf("close temp: %w", err)
	}

	sha := hex.EncodeToString(h.Sum(nil))
	key := s.prefix + strings.TrimLeft(objectKey, "/")
	if objectKey == "" {
		key = s.shaKey(sha)
		if exist, err := s.bucket.IsObjectExist(key); err != nil {
			return "", "", 0, fmt.Errorf("oss head: %w", err)
		} else if exist {
			return sha, key, size, nil // 内容寻址去重
		}
	}

	f, err := os.Open(tmpName)
	if err != nil {
		return "", "", 0, fmt.Errorf("reopen temp: %w", err)
	}
	defer f.Close()
	if err := s.bucket.PutObject(key, f, oss.ObjectACL(oss.ACLPublicRead)); err != nil {
		return "", "", 0, fmt.Errorf("oss put: %w", err)
	}
	return sha, key, size, nil
}

// Open 拉取对象内容流（fallback；公共后端主路径走 PublicURL 302）。
func (s *OSSStore) Open(storagePath string) (io.ReadCloser, error) {
	return s.bucket.GetObject(storagePath)
}

// PublicURL 永久公共读直链；对路径各段做 URL 编码（key 可含中文，如 csw/资讯日更/…），保留 "/"。
func (s *OSSStore) PublicURL(storagePath string) string {
	parts := strings.Split(storagePath, "/")
	for i, p := range parts {
		parts[i] = url.PathEscape(p)
	}
	return s.publicBase + "/" + strings.Join(parts, "/")
}
