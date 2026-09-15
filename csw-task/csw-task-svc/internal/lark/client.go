// Package lark 飞书 Open API 最小客户端：tenant_access_token 缓存 + 群文本消息（仅 net/http）。
package lark

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"
)

// Sender 发群消息的最小接口；notifier 依赖它，测试用 fake 实现。
type Sender interface {
	SendText(ctx context.Context, chatID, text, uuid string) (messageID string, err error)
}

// APIError 飞书业务错误（响应 code != 0）或非预期的 HTTP 响应。
type APIError struct {
	Msg  string
	Code int
	HTTP int
}

func (e *APIError) Error() string {
	return fmt.Sprintf("lark api: http=%d code=%d msg=%s", e.HTTP, e.Code, e.Msg)
}

// tokenInvalid 访问凭证缺失 / 无效 / 过期类错误码：刷新 token 后重试一次。
var tokenInvalid = map[int]bool{99991661: true, 99991663: true, 99991668: true}

// Client 飞书 Open API 客户端。并发安全。
type Client struct {
	now       func() time.Time
	http      *http.Client
	expiry    time.Time
	baseURL   string
	appID     string
	appSecret string
	token     string
	mu        sync.Mutex
}

// New 构造客户端；baseURL 形如 https://open.feishu.cn。
func New(baseURL, appID, appSecret string) *Client {
	return &Client{
		baseURL: strings.TrimRight(baseURL, "/"), appID: appID, appSecret: appSecret,
		http: &http.Client{Timeout: 10 * time.Second}, now: time.Now,
	}
}

type apiResp struct {
	Msg               string          `json:"msg"`
	TenantAccessToken string          `json:"tenant_access_token"`
	Data              json.RawMessage `json:"data"`
	Code              int             `json:"code"`
	Expire            int             `json:"expire"`
}

func (c *Client) post(ctx context.Context, path, token string, body any) (apiResp, error) {
	var out apiResp
	b, err := json.Marshal(body)
	if err != nil {
		return out, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.baseURL+path, bytes.NewReader(b))
	if err != nil {
		return out, err
	}
	req.Header.Set("Content-Type", "application/json; charset=utf-8")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return out, err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err := json.Unmarshal(raw, &out); err != nil {
		return out, &APIError{HTTP: resp.StatusCode, Code: -1, Msg: strings.TrimSpace(string(raw))}
	}
	if out.Code != 0 || resp.StatusCode/100 != 2 {
		return out, &APIError{HTTP: resp.StatusCode, Code: out.Code, Msg: out.Msg}
	}
	return out, nil
}

// tenantToken 取 tenant_access_token；到期前 5 分钟刷新，force 时强制刷新。
func (c *Client) tenantToken(ctx context.Context, force bool) (string, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if !force && c.token != "" && c.now().Before(c.expiry.Add(-5*time.Minute)) {
		return c.token, nil
	}
	r, err := c.post(ctx, "/open-apis/auth/v3/tenant_access_token/internal", "",
		map[string]string{"app_id": c.appID, "app_secret": c.appSecret})
	if err != nil {
		return "", fmt.Errorf("tenant_access_token: %w", err)
	}
	c.token = r.TenantAccessToken
	c.expiry = c.now().Add(time.Duration(r.Expire) * time.Second)
	return c.token, nil
}

// SendText 向群发一条文本消息（可含 <at user_id="ou_xxx">名称</at>）。
// uuid 用于飞书侧去重（同一 uuid 一小时内只发一次），传 csw-outbox-{event_id}。
func (c *Client) SendText(ctx context.Context, chatID, text, uuid string) (string, error) {
	content, err := json.Marshal(map[string]string{"text": text})
	if err != nil {
		return "", err
	}
	body := map[string]string{"receive_id": chatID, "msg_type": "text", "content": string(content), "uuid": uuid}
	for attempt := 0; ; attempt++ {
		tok, err := c.tenantToken(ctx, attempt > 0)
		if err != nil {
			return "", err
		}
		r, err := c.post(ctx, "/open-apis/im/v1/messages?receive_id_type=chat_id", tok, body)
		if err != nil {
			if ae, ok := err.(*APIError); ok && tokenInvalid[ae.Code] && attempt == 0 {
				continue
			}
			return "", err
		}
		var data struct {
			MessageID string `json:"message_id"`
		}
		_ = json.Unmarshal(r.Data, &data)
		return data.MessageID, nil
	}
}
