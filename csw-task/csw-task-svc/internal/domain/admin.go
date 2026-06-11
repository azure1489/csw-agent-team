package domain

// 后台用户角色（RBAC，独立于工作流角色）。
const (
	AdminSuperadmin = "superadmin"
	AdminOperator   = "operator"
	AdminViewer     = "viewer"
)

// AdminUser 管理后台人类用户（不含 password_hash，哈希由 store 单独返回）。
type AdminUser struct {
	Username    string
	DisplayName string
	Role        string
	Status      string
	LastLoginAt string
	CreatedAt   string
	ID          int64
}

// AdminAudit 后台操作审计条目。
type AdminAudit struct {
	Action     string
	Target     string
	DetailJSON string
	CreatedAt  string
	Username   string // 关联展示
	UserID     *int64
	ID         int64
}
