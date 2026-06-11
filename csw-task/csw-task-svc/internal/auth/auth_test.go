package auth

import (
	"testing"
	"time"
)

func TestPassword(t *testing.T) {
	phc, err := HashPassword("s3cret-pw")
	if err != nil {
		t.Fatalf("hash: %v", err)
	}
	ok, err := VerifyPassword(phc, "s3cret-pw")
	if err != nil || !ok {
		t.Fatalf("verify correct: ok=%v err=%v", ok, err)
	}
	ok, _ = VerifyPassword(phc, "wrong")
	if ok {
		t.Fatalf("verify wrong should be false")
	}
	if _, err := VerifyPassword("not-a-phc", "x"); err == nil {
		t.Fatalf("bad phc should error")
	}
}

func TestJWT(t *testing.T) {
	secret := []byte("test-secret")
	tok, err := SignAccess(secret, 42, "superadmin", time.Hour)
	if err != nil {
		t.Fatalf("sign: %v", err)
	}
	claims, err := ParseAccess(secret, tok)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if claims.Role != "superadmin" {
		t.Fatalf("role=%s", claims.Role)
	}
	if uid, _ := claims.UserID(); uid != 42 {
		t.Fatalf("uid=%d", uid)
	}

	// 过期 token 应被拒。
	expired, _ := SignAccess(secret, 1, "viewer", -time.Minute)
	if _, err := ParseAccess(secret, expired); err == nil {
		t.Fatalf("expired token should fail")
	}
	// 错误密钥应被拒。
	if _, err := ParseAccess([]byte("other"), tok); err == nil {
		t.Fatalf("wrong secret should fail")
	}
}
