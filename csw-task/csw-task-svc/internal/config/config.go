// Package config 加载服务配置，运行面与管理后台共用。
// 优先级：环境变量 > config.yaml（CSW_CONFIG 指定，默认 ./config.yaml）> 内置默认。
package config

import (
	"crypto/rand"
	"encoding/hex"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

// Version 服务版本（/admin/overview · /admin/settings 展示）。
const Version = "0.2.0"

// Config 服务配置。
type Config struct {
	Addr      string // 运行面 HTTP 监听地址，如 :8080
	AdminAddr string // 管理后台 HTTP 监听地址，如 :8081
	DataDir   string // 运行时数据根目录（sqlite + blobs）
	DBPath    string // sqlite 文件路径
	BlobDir   string // 托管文件根目录
	BaseURL   string // 拼 download_url 用，如 http://localhost:8080

	MaxUploadBytes      int64           // 上传大小上限
	AllowedContentTypes map[string]bool // content-type 白名单（空=不限制）

	// ── 管理后台（JWT + CORS）──
	JWTSecret          []byte        // access JWT HS256 密钥
	JWTSecretGenerated bool          // true=env 未设、启动时随机生成（仅 dev，重启即失效）
	AccessTTL          time.Duration // access JWT 有效期
	RefreshTTL         time.Duration // refresh 有效期
	CORSOrigins        []string      // 允许的前端 origin（AllowCredentials 下不能用 *）
	CookieSecure       bool          // refresh cookie Secure 标志（生产置 true）

	// ── 文件存储后端 ──
	BlobBackend        string // "local"（默认）| "oss"
	OSSEndpoint        string
	OSSBucket          string
	OSSAccessKeyID     string
	OSSAccessKeySecret string
	OSSPrefix          string // 对象 key 前缀，默认 "blobs"
}

// Load 加载配置：先把 config.yaml 的值注入未设置的环境变量，再按 env（含注入值）读取。
func Load() Config {
	applyConfigFile()

	dataDir := env("CSW_DATA_DIR", "./data")
	cfg := Config{
		Addr:           env("CSW_ADDR", ":8080"),
		AdminAddr:      env("CSW_ADMIN_ADDR", ":8081"),
		DataDir:        dataDir,
		DBPath:         env("CSW_DB_PATH", filepath.Join(dataDir, "csw-task.db")),
		BlobDir:        env("CSW_BLOB_DIR", filepath.Join(dataDir, "blobs")),
		BaseURL:        strings.TrimRight(env("CSW_BASE_URL", "http://localhost:8080"), "/"),
		MaxUploadBytes: envInt64("CSW_MAX_UPLOAD_BYTES", 64<<20), // 64 MiB
		AccessTTL:      envDuration("CSW_JWT_ACCESS_TTL", 15*time.Minute),
		RefreshTTL:     envDuration("CSW_JWT_REFRESH_TTL", 7*24*time.Hour),
		CORSOrigins:    parseCSV(env("CSW_ADMIN_CORS_ORIGIN", "http://localhost:5173")),
		CookieSecure:   envBool("CSW_COOKIE_SECURE", false),

		BlobBackend:        strings.ToLower(env("CSW_BLOB_BACKEND", "local")),
		OSSEndpoint:        env("CSW_OSS_ENDPOINT", ""),
		OSSBucket:          env("CSW_OSS_BUCKET", ""),
		OSSAccessKeyID:     env("CSW_OSS_ACCESS_KEY_ID", ""),
		OSSAccessKeySecret: env("CSW_OSS_ACCESS_KEY_SECRET", ""),
		OSSPrefix:          env("CSW_OSS_PREFIX", "blobs"),
	}
	cfg.AllowedContentTypes = parseSet(env("CSW_ALLOWED_CONTENT_TYPES",
		"application/zip,application/x-zip-compressed,application/octet-stream"))

	if s := os.Getenv("CSW_JWT_SECRET"); s != "" {
		cfg.JWTSecret = []byte(s)
	} else {
		buf := make([]byte, 32)
		_, _ = rand.Read(buf)
		cfg.JWTSecret = []byte(hex.EncodeToString(buf))
		cfg.JWTSecretGenerated = true
	}
	return cfg
}

// fileConfig 对应 config.yaml 的结构（见 configs/config.example.yaml）。
type fileConfig struct {
	DataDir string `yaml:"data_dir"`
	DBPath  string `yaml:"db_path"`
	BlobDir string `yaml:"blob_dir"`
	Server  struct {
		Addr                string   `yaml:"addr"`
		BaseURL             string   `yaml:"base_url"`
		MaxUploadBytes      int64    `yaml:"max_upload_bytes"`
		AllowedContentTypes []string `yaml:"allowed_content_types"`
	} `yaml:"server"`
	Admin struct {
		Addr          string   `yaml:"addr"`
		JWTSecret     string   `yaml:"jwt_secret"`
		JWTAccessTTL  string   `yaml:"jwt_access_ttl"`
		JWTRefreshTTL string   `yaml:"jwt_refresh_ttl"`
		CORSOrigins   []string `yaml:"cors_origins"`
		CookieSecure  *bool    `yaml:"cookie_secure"`
	} `yaml:"admin"`
	Storage struct {
		Backend string `yaml:"backend"`
		OSS     struct {
			Endpoint        string `yaml:"endpoint"`
			Bucket          string `yaml:"bucket"`
			AccessKeyID     string `yaml:"access_key_id"`
			AccessKeySecret string `yaml:"access_key_secret"`
			Prefix          string `yaml:"prefix"`
		} `yaml:"oss"`
	} `yaml:"storage"`
}

// applyConfigFile 读 config.yaml，把其中的值注入【未设置】的环境变量（env 优先）。
// 文件不存在 / 解析失败 → 静默跳过（纯 env 加载，完全向后兼容）。
func applyConfigFile() {
	path := os.Getenv("CSW_CONFIG")
	if path == "" {
		path = "config.yaml"
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return
	}
	var fc fileConfig
	if yaml.Unmarshal(data, &fc) != nil {
		return
	}
	setIfUnset("CSW_DATA_DIR", fc.DataDir)
	setIfUnset("CSW_DB_PATH", fc.DBPath)
	setIfUnset("CSW_BLOB_DIR", fc.BlobDir)
	setIfUnset("CSW_ADDR", fc.Server.Addr)
	setIfUnset("CSW_BASE_URL", fc.Server.BaseURL)
	if fc.Server.MaxUploadBytes != 0 {
		setIfUnset("CSW_MAX_UPLOAD_BYTES", strconv.FormatInt(fc.Server.MaxUploadBytes, 10))
	}
	if len(fc.Server.AllowedContentTypes) > 0 {
		setIfUnset("CSW_ALLOWED_CONTENT_TYPES", strings.Join(fc.Server.AllowedContentTypes, ","))
	}
	setIfUnset("CSW_ADMIN_ADDR", fc.Admin.Addr)
	setIfUnset("CSW_JWT_SECRET", fc.Admin.JWTSecret)
	setIfUnset("CSW_JWT_ACCESS_TTL", fc.Admin.JWTAccessTTL)
	setIfUnset("CSW_JWT_REFRESH_TTL", fc.Admin.JWTRefreshTTL)
	if len(fc.Admin.CORSOrigins) > 0 {
		setIfUnset("CSW_ADMIN_CORS_ORIGIN", strings.Join(fc.Admin.CORSOrigins, ","))
	}
	if fc.Admin.CookieSecure != nil {
		setIfUnset("CSW_COOKIE_SECURE", strconv.FormatBool(*fc.Admin.CookieSecure))
	}
	setIfUnset("CSW_BLOB_BACKEND", fc.Storage.Backend)
	setIfUnset("CSW_OSS_ENDPOINT", fc.Storage.OSS.Endpoint)
	setIfUnset("CSW_OSS_BUCKET", fc.Storage.OSS.Bucket)
	setIfUnset("CSW_OSS_ACCESS_KEY_ID", fc.Storage.OSS.AccessKeyID)
	setIfUnset("CSW_OSS_ACCESS_KEY_SECRET", fc.Storage.OSS.AccessKeySecret)
	setIfUnset("CSW_OSS_PREFIX", fc.Storage.OSS.Prefix)
}

func setIfUnset(key, val string) {
	if val != "" && os.Getenv(key) == "" {
		_ = os.Setenv(key, val)
	}
}

func env(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

func envInt64(k string, def int64) int64 {
	if v := os.Getenv(k); v != "" {
		if n, err := strconv.ParseInt(v, 10, 64); err == nil {
			return n
		}
	}
	return def
}

func envBool(k string, def bool) bool {
	if v := os.Getenv(k); v != "" {
		if b, err := strconv.ParseBool(v); err == nil {
			return b
		}
	}
	return def
}

func envDuration(k string, def time.Duration) time.Duration {
	if v := os.Getenv(k); v != "" {
		if d, err := time.ParseDuration(v); err == nil {
			return d
		}
	}
	return def
}

func parseSet(s string) map[string]bool {
	m := map[string]bool{}
	for _, p := range parseCSV(s) {
		m[p] = true
	}
	return m
}

func parseCSV(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}
