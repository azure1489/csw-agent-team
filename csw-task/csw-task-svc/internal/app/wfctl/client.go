package wfctl

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/cookiejar"
	"strings"
	"time"
)

// Version wfctl 版本（User-Agent 与 --version）。
const Version = "1.0.0"

// apiError 管理后台返回的业务错误。
type apiError struct {
	Code   string
	Msg    string
	Raw    []byte
	Status int
}

func (e *apiError) Error() string {
	if e.Code != "" {
		return fmt.Sprintf("HTTP %d %s：%s", e.Status, e.Code, e.Msg)
	}
	return fmt.Sprintf("HTTP %d：%s", e.Status, strings.TrimSpace(string(e.Raw)))
}

// client 管理后台 API 客户端：access token 在内存，refresh 在 cookie jar；401 先续期、再重新登录各一次。凭证不落盘。
type client struct {
	http   *http.Client
	base   string
	user   string
	pass   string
	access string
}

func newClient(base, user, pass string) (*client, error) {
	jar, err := cookiejar.New(nil)
	if err != nil {
		return nil, err
	}
	return &client{http: &http.Client{Jar: jar, Timeout: 60 * time.Second}, base: strings.TrimRight(base, "/"), user: user, pass: pass}, nil
}

func (c *client) raw(ctx context.Context, method, path string, body any) (int, []byte, error) {
	var rd io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return 0, nil, err
		}
		rd = bytes.NewReader(b)
	}
	req, err := http.NewRequestWithContext(ctx, method, c.base+path, rd)
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("User-Agent", "wfctl/"+Version)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if c.access != "" {
		req.Header.Set("Authorization", "Bearer "+c.access)
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close()
	out, err := io.ReadAll(io.LimitReader(resp.Body, 32<<20))
	return resp.StatusCode, out, err
}

func (c *client) session(ctx context.Context, path string, body any) error {
	status, raw, err := c.raw(ctx, http.MethodPost, path, body)
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return decodeErr(status, raw)
	}
	var r struct {
		AccessToken string `json:"access_token"`
	}
	if err := json.Unmarshal(raw, &r); err != nil || r.AccessToken == "" {
		return fmt.Errorf("登录响应缺 access_token")
	}
	c.access = r.AccessToken
	return nil
}

func (c *client) login(ctx context.Context) error {
	if c.user == "" || c.pass == "" {
		return fmt.Errorf("未配置 CSW_ADMIN_USER / CSW_ADMIN_PASSWORD")
	}
	c.access = ""
	return c.session(ctx, "/admin/login", map[string]string{"username": c.user, "password": c.pass})
}

func decodeErr(status int, raw []byte) error {
	var e struct {
		Code    string `json:"code"`
		Message string `json:"message"`
	}
	_ = json.Unmarshal(raw, &e)
	return &apiError{Status: status, Code: e.Code, Msg: e.Message, Raw: raw}
}

// do 发请求并把 2xx 响应解到 out；401 时先 refresh、再 login 各重试一次。
func (c *client) do(ctx context.Context, method, path string, body, out any) error {
	if c.access == "" {
		if err := c.login(ctx); err != nil {
			return err
		}
	}
	status, raw, err := c.raw(ctx, method, path, body)
	if err != nil {
		return err
	}
	if status == http.StatusUnauthorized {
		c.access = ""
		if err := c.session(ctx, "/admin/refresh", nil); err != nil {
			if err := c.login(ctx); err != nil {
				return err
			}
		}
		if status, raw, err = c.raw(ctx, method, path, body); err != nil {
			return err
		}
	}
	if status/100 != 2 {
		return decodeErr(status, raw)
	}
	if out != nil {
		return json.Unmarshal(raw, out)
	}
	return nil
}
