package auth

import (
	"fmt"
	"strconv"
	"time"

	"github.com/golang-jwt/jwt/v5"
)

// AccessClaims access JWT 载荷：sub=user_id，role=后台角色。
type AccessClaims struct {
	Role string `json:"role"`
	jwt.RegisteredClaims
}

// UserID 解析 sub 为 int64。
func (c AccessClaims) UserID() (int64, error) {
	return strconv.ParseInt(c.Subject, 10, 64)
}

// SignAccess 签发 access JWT（HS256，短期）。
func SignAccess(secret []byte, userID int64, role string, ttl time.Duration) (string, error) {
	now := time.Now()
	claims := AccessClaims{
		Role: role,
		RegisteredClaims: jwt.RegisteredClaims{
			Subject:   strconv.FormatInt(userID, 10),
			IssuedAt:  jwt.NewNumericDate(now),
			ExpiresAt: jwt.NewNumericDate(now.Add(ttl)),
		},
	}
	return jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString(secret)
}

// ParseAccess 验签并解析 access JWT（强制 HS256，校验过期）。
func ParseAccess(secret []byte, tokenStr string) (*AccessClaims, error) {
	var claims AccessClaims
	_, err := jwt.ParseWithClaims(tokenStr, &claims, func(t *jwt.Token) (any, error) {
		if _, ok := t.Method.(*jwt.SigningMethodHMAC); !ok {
			return nil, fmt.Errorf("unexpected signing method: %v", t.Header["alg"])
		}
		return secret, nil
	})
	if err != nil {
		return nil, err
	}
	return &claims, nil
}

// NewRefresh 生成一枚随机 refresh token（明文经 cookie 下发，库存 sha256）。
func NewRefresh() (string, error) {
	return randomHex(32)
}
