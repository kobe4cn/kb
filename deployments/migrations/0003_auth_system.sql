-- Authentication and Authorization System
-- 创建用户状态枚举类型
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'suspended', 'pending');

-- 用户表
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(50) UNIQUE NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    display_name VARCHAR(100),
    avatar_url TEXT,
    status user_status DEFAULT 'pending',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now(),
    last_login_at TIMESTAMPTZ,

    CONSTRAINT users_username_check CHECK (length(username) >= 3),
    CONSTRAINT users_email_check CHECK (email ~* '^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$')
);

-- 角色表
CREATE TABLE IF NOT EXISTS roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(50) UNIQUE NOT NULL,
    description TEXT,
    is_system BOOLEAN DEFAULT false,
    permissions JSONB DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ DEFAULT now(),

    CONSTRAINT roles_name_check CHECK (length(name) >= 2)
);

-- 用户角色关联表
CREATE TABLE IF NOT EXISTS user_roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    tenant_id UUID, -- 可选，支持多租户
    granted_by UUID REFERENCES users(id),
    granted_at TIMESTAMPTZ DEFAULT now(),

    UNIQUE(user_id, role_id, tenant_id)
);

-- API 密钥表
CREATE TABLE IF NOT EXISTS api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash TEXT NOT NULL,
    name VARCHAR(100) NOT NULL,
    scopes JSONB DEFAULT '[]'::jsonb,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT now(),

    CONSTRAINT api_keys_name_check CHECK (length(name) >= 1)
);

-- 审计日志表
CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id),
    action VARCHAR(100) NOT NULL,
    resource_type VARCHAR(50),
    resource_id UUID,
    details JSONB,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

-- 系统配置表
CREATE TABLE IF NOT EXISTS system_config (
    key VARCHAR(255) PRIMARY KEY,
    value JSONB NOT NULL,
    description TEXT,
    updated_by UUID REFERENCES users(id),
    updated_at TIMESTAMPTZ DEFAULT now(),

    CONSTRAINT system_config_key_check CHECK (length(key) >= 1)
);

-- 创建索引
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_status ON users(status);
CREATE INDEX IF NOT EXISTS idx_users_created_at ON users(created_at);

CREATE INDEX IF NOT EXISTS idx_roles_name ON roles(name);
CREATE INDEX IF NOT EXISTS idx_roles_is_system ON roles(is_system);

CREATE INDEX IF NOT EXISTS idx_user_roles_user_id ON user_roles(user_id);
CREATE INDEX IF NOT EXISTS idx_user_roles_role_id ON user_roles(role_id);
CREATE INDEX IF NOT EXISTS idx_user_roles_tenant_id ON user_roles(tenant_id);

CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX IF NOT EXISTS idx_api_keys_key_hash ON api_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_api_keys_expires_at ON api_keys(expires_at);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user_id ON audit_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource_type ON audit_logs(resource_type);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs(created_at);

-- 创建更新时间触发器函数
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- 为users表添加更新时间触发器
CREATE TRIGGER update_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- 插入默认系统角色
INSERT INTO roles (name, description, is_system, permissions) VALUES
    ('super_admin', '超级管理员，拥有所有权限', true, '["users:read","users:write","users:delete","users:admin","roles:read","roles:write","roles:delete","documents:read","documents:write","documents:delete","documents:admin","tenants:read","tenants:write","tenants:delete","tenants:admin","system:config","system:monitor","system:logs","system:admin","api_keys:read","api_keys:write","api_keys:delete","jobs:read","jobs:write","jobs:delete","jobs:admin","query:execute","query:admin"]'::jsonb),
    ('tenant_admin', '租户管理员，管理租户内的资源', true, '["users:read","users:write","users:delete","roles:read","documents:read","documents:write","documents:delete","documents:admin","tenants:read","api_keys:read","api_keys:write","api_keys:delete","jobs:read","jobs:write","jobs:admin","query:execute","query:admin"]'::jsonb),
    ('editor', '编辑者，可以管理文档', true, '["documents:read","documents:write","jobs:read","jobs:write","query:execute"]'::jsonb),
    ('viewer', '查看者，只能查看和查询', true, '["documents:read","jobs:read","query:execute"]'::jsonb),
    ('api_user', 'API用户，用于程序化访问', true, '["documents:read","documents:write","query:execute"]'::jsonb)
ON CONFLICT (name) DO UPDATE SET
    permissions = EXCLUDED.permissions,
    is_system = true,
    description = EXCLUDED.description;

-- 创建默认管理员用户（密码：Admin123!）
-- 注意：生产环境中应该修改默认密码
INSERT INTO users (username, email, password_hash, display_name, status) VALUES
    ('admin', 'admin@example.com', '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewtsWR4v2DQU7FBe', '系统管理员', 'active')
ON CONFLICT (username) DO NOTHING;

-- 为默认管理员分配超级管理员角色
INSERT INTO user_roles (user_id, role_id, granted_by)
SELECT
    u.id,
    r.id,
    u.id
FROM users u, roles r
WHERE u.username = 'admin' AND r.name = 'super_admin'
ON CONFLICT DO NOTHING;

-- 插入一些基础系统配置
INSERT INTO system_config (key, value, description) VALUES
    ('auth.jwt_expiry_hours', '24', 'JWT访问令牌过期时间（小时）'),
    ('auth.refresh_expiry_days', '30', '刷新令牌过期时间（天）'),
    ('auth.session_timeout_hours', '24', '会话超时时间（小时）'),
    ('auth.max_login_attempts', '5', '最大登录尝试次数'),
    ('auth.lockout_duration_minutes', '30', '账户锁定时长（分钟）'),
    ('system.maintenance_mode', 'false', '系统维护模式'),
    ('system.registration_enabled', 'false', '是否开放用户注册')
ON CONFLICT (key) DO NOTHING;