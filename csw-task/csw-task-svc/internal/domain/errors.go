package domain

import "fmt"

// Error 业务错误，携带 HTTP 状态码 + 机器可读 code（供 handler 统一翻译为 {code,message}）。
type Error struct {
	Code    string
	Message string
	HTTP    int
}

func (e *Error) Error() string { return fmt.Sprintf("%s: %s", e.Code, e.Message) }

// NewError 构造业务错误。
func NewError(http int, code, message string) *Error {
	return &Error{HTTP: http, Code: code, Message: message}
}

// 常用构造器。
func BadRequest(code, msg string) *Error { return NewError(400, code, msg) }
func Forbidden(code, msg string) *Error  { return NewError(403, code, msg) }
func NotFound(code, msg string) *Error   { return NewError(404, code, msg) }
func Conflict(code, msg string) *Error   { return NewError(409, code, msg) }
