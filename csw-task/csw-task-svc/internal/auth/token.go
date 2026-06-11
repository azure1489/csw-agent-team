// Package auth 提供运行面 bearer token 的生成与哈希（admin JWT/argon2 属第二轮）。
package auth

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
)

// TokenPrefix 明文 token 前缀（便于识别）。
const TokenPrefix = "csw_live_"

// NewToken 生成一枚随机明文 token（仅签发时返回一次）。
func NewToken() (string, error) {
	s, err := randomHex(32)
	if err != nil {
		return "", err
	}
	return TokenPrefix + s, nil
}

// randomHex 返回 n 字节随机数据的十六进制串。
func randomHex(n int) (string, error) {
	buf := make([]byte, n)
	if _, err := rand.Read(buf); err != nil {
		return "", fmt.Errorf("rand: %w", err)
	}
	return hex.EncodeToString(buf), nil
}

// HashToken 算 sha256(明文)，库里只存哈希。
func HashToken(plain string) string {
	sum := sha256.Sum256([]byte(plain))
	return hex.EncodeToString(sum[:])
}
