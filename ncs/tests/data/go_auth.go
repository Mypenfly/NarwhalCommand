// go_auth.go — Go 语言认证服务
//
// 真实工程风格的 Go REST API 代码片段，
// 包含 struct 定义、方法、接口实现，
// 用于验证 NCS 对 Go 语法的编辑能力。

package auth

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"time"
)

// Credentials 认证凭据
type Credentials struct {
	Username string `json:"username"`
	Password string `json:"password"`
	Token    string `json:"token,omitempty"`
}

// TokenClaims JWT-like 声明
type TokenClaims struct {
	UserID    string   `json:"user_id"`
	Roles     []string `json:"roles"`
	ExpiresAt int64    `json:"expires_at"`
}

// AuthService 认证服务
type AuthService struct {
	tokens   map[string]TokenClaims
	saltSize int
}

// NewAuthService 创建认证服务实例
func NewAuthService() *AuthService {
	return &AuthService{
		tokens:   make(map[string]TokenClaims),
		saltSize: 16,
	}
}

// GenerateSalt 生成随机盐值
func (s *AuthService) GenerateSalt() (string, error) {
	bytes := make([]byte, s.saltSize)
	_, err := rand.Read(bytes)
	if err != nil {
		return "", errors.New("failed to generate salt")
	}
	return hex.EncodeToString(bytes), nil
}

// ValidateToken 验证令牌有效性
func (s *AuthService) ValidateToken(token string) (*TokenClaims, error) {
	claims, exists := s.tokens[token]
	if !exists {
		return nil, errors.New("token not found")
	}

	if time.Now().Unix() > claims.ExpiresAt {
		delete(s.tokens, token)
		return nil, errors.New("token expired")
	}

	return &claims, nil
}

// StoreToken 存储令牌
func (s *AuthService) StoreToken(userID string, roles []string, ttl time.Duration) (string, error) {
	tokenBytes := make([]byte, 32)
	_, err := rand.Read(tokenBytes)
	if err != nil {
		return "", err
	}

	token := hex.EncodeToString(tokenBytes)
	s.tokens[token] = TokenClaims{
		UserID:    userID,
		Roles:     roles,
		ExpiresAt: time.Now().Add(ttl).Unix(),
	}

	return token, nil
}
