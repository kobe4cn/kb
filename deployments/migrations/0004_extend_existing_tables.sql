-- 扩展现有表以支持权限管理和审计

-- 为现有的 documents 表添加用户关联字段（如果存在）
DO $$
BEGIN
    -- 检查 documents 表是否存在
    IF EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'documents') THEN

        -- 添加创建者字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'documents' AND column_name = 'created_by') THEN
            ALTER TABLE documents ADD COLUMN created_by UUID REFERENCES users(id);
        END IF;

        -- 添加修改者字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'documents' AND column_name = 'updated_by') THEN
            ALTER TABLE documents ADD COLUMN updated_by UUID REFERENCES users(id);
        END IF;

        -- 添加软删除字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'documents' AND column_name = 'deleted_at') THEN
            ALTER TABLE documents ADD COLUMN deleted_at TIMESTAMPTZ;
        END IF;

        -- 添加索引
        CREATE INDEX IF NOT EXISTS idx_documents_created_by ON documents(created_by);
        CREATE INDEX IF NOT EXISTS idx_documents_updated_by ON documents(updated_by);
        CREATE INDEX IF NOT EXISTS idx_documents_deleted_at ON documents(deleted_at);

    END IF;
END
$$;

-- 为现有的 tenants 表添加用户关联字段（如果存在）
DO $$
BEGIN
    -- 检查 tenants 表是否存在
    IF EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'tenants') THEN

        -- 添加创建者字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'created_by') THEN
            ALTER TABLE tenants ADD COLUMN created_by UUID REFERENCES users(id);
        END IF;

        -- 添加软删除字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'deleted_at') THEN
            ALTER TABLE tenants ADD COLUMN deleted_at TIMESTAMPTZ;
        END IF;

        -- 添加状态字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'status') THEN
            ALTER TABLE tenants ADD COLUMN status VARCHAR(20) DEFAULT 'active';
        END IF;

        -- 添加配置字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'config') THEN
            ALTER TABLE tenants ADD COLUMN config JSONB DEFAULT '{}'::jsonb;
        END IF;

        -- 添加索引
        CREATE INDEX IF NOT EXISTS idx_tenants_created_by ON tenants(created_by);
        CREATE INDEX IF NOT EXISTS idx_tenants_deleted_at ON tenants(deleted_at);
        CREATE INDEX IF NOT EXISTS idx_tenants_status ON tenants(status);

    END IF;
END
$$;

-- 为现有的 chunks 表添加用户关联字段（如果存在）
DO $$
BEGIN
    -- 检查 chunks 表是否存在
    IF EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'chunks') THEN

        -- 添加创建者字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'chunks' AND column_name = 'created_by') THEN
            ALTER TABLE chunks ADD COLUMN created_by UUID REFERENCES users(id);
        END IF;

        -- 添加软删除字段
        IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'chunks' AND column_name = 'deleted_at') THEN
            ALTER TABLE chunks ADD COLUMN deleted_at TIMESTAMPTZ;
        END IF;

        -- 添加索引
        CREATE INDEX IF NOT EXISTS idx_chunks_created_by ON chunks(created_by);
        CREATE INDEX IF NOT EXISTS idx_chunks_deleted_at ON chunks(deleted_at);

    END IF;
END
$$;

-- 为 jobs 表添加用户关联字段
DO $$
BEGIN
    -- 添加创建者字段
    IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'jobs' AND column_name = 'created_by') THEN
        ALTER TABLE jobs ADD COLUMN created_by UUID REFERENCES users(id);
    END IF;

    -- 添加租户字段
    IF NOT EXISTS (SELECT FROM information_schema.columns WHERE table_name = 'jobs' AND column_name = 'tenant_id') THEN
        ALTER TABLE jobs ADD COLUMN tenant_id UUID;
    END IF;

    -- 添加索引
    CREATE INDEX IF NOT EXISTS idx_jobs_created_by ON jobs(created_by);
    CREATE INDEX IF NOT EXISTS idx_jobs_tenant_id ON jobs(tenant_id);
END
$$;

-- 创建用户会话表（用于跟踪活跃会话）
CREATE TABLE IF NOT EXISTS user_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id VARCHAR(100) UNIQUE NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMPTZ DEFAULT now(),
    last_accessed_at TIMESTAMPTZ DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    is_active BOOLEAN DEFAULT true
);

-- 创建用户登录历史表
CREATE TABLE IF NOT EXISTS user_login_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ip_address INET,
    user_agent TEXT,
    login_type VARCHAR(20) DEFAULT 'password', -- password, api_key, sso
    success BOOLEAN NOT NULL,
    failure_reason TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

-- 创建权限变更历史表
CREATE TABLE IF NOT EXISTS permission_changes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    target_user_id UUID REFERENCES users(id),
    action VARCHAR(50) NOT NULL, -- assign_role, revoke_role, create_user, etc.
    details JSONB,
    created_at TIMESTAMPTZ DEFAULT now()
);

-- 添加索引
CREATE INDEX IF NOT EXISTS idx_user_sessions_session_id ON user_sessions(session_id);
CREATE INDEX IF NOT EXISTS idx_user_sessions_user_id ON user_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_user_sessions_expires_at ON user_sessions(expires_at);

CREATE INDEX IF NOT EXISTS idx_user_login_history_user_id ON user_login_history(user_id);
CREATE INDEX IF NOT EXISTS idx_user_login_history_created_at ON user_login_history(created_at);
CREATE INDEX IF NOT EXISTS idx_user_login_history_success ON user_login_history(success);

CREATE INDEX IF NOT EXISTS idx_permission_changes_user_id ON permission_changes(user_id);
CREATE INDEX IF NOT EXISTS idx_permission_changes_target_user_id ON permission_changes(target_user_id);
CREATE INDEX IF NOT EXISTS idx_permission_changes_created_at ON permission_changes(created_at);

-- 创建清理过期会话的函数
CREATE OR REPLACE FUNCTION cleanup_expired_sessions()
RETURNS INTEGER AS $$
DECLARE
    deleted_count INTEGER;
BEGIN
    DELETE FROM user_sessions
    WHERE expires_at < now() OR is_active = false;

    GET DIAGNOSTICS deleted_count = ROW_COUNT;
    RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;

-- 创建用户统计视图
CREATE OR REPLACE VIEW user_stats AS
SELECT
    u.id,
    u.username,
    u.email,
    u.status,
    u.created_at,
    u.last_login_at,
    COUNT(DISTINCT ur.role_id) as role_count,
    COUNT(DISTINCT ak.id) as api_key_count,
    COUNT(DISTINCT us.id) as active_session_count,
    MAX(us.last_accessed_at) as last_session_activity
FROM users u
LEFT JOIN user_roles ur ON u.id = ur.user_id
LEFT JOIN api_keys ak ON u.id = ak.user_id AND (ak.expires_at IS NULL OR ak.expires_at > now())
LEFT JOIN user_sessions us ON u.id = us.user_id AND us.is_active = true AND us.expires_at > now()
GROUP BY u.id, u.username, u.email, u.status, u.created_at, u.last_login_at;

-- 创建权限统计视图
CREATE OR REPLACE VIEW permission_stats AS
SELECT
    r.name as role_name,
    COUNT(DISTINCT ur.user_id) as user_count,
    jsonb_array_length(r.permissions) as permission_count,
    r.is_system,
    r.created_at
FROM roles r
LEFT JOIN user_roles ur ON r.id = ur.role_id
GROUP BY r.id, r.name, r.permissions, r.is_system, r.created_at;

-- 创建审计日志清理函数（保留最近6个月的日志）
CREATE OR REPLACE FUNCTION cleanup_old_audit_logs()
RETURNS INTEGER AS $$
DECLARE
    deleted_count INTEGER;
BEGIN
    DELETE FROM audit_logs
    WHERE created_at < now() - INTERVAL '6 months';

    GET DIAGNOSTICS deleted_count = ROW_COUNT;
    RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;

-- 添加一些有用的约束
ALTER TABLE user_sessions ADD CONSTRAINT check_expires_at_future
    CHECK (expires_at > created_at);

ALTER TABLE api_keys ADD CONSTRAINT check_expires_at_future
    CHECK (expires_at IS NULL OR expires_at > created_at);

-- 创建触发器来自动更新 last_accessed_at
CREATE OR REPLACE FUNCTION update_session_access()
RETURNS TRIGGER AS $$
BEGIN
    NEW.last_accessed_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_session_access_trigger
    BEFORE UPDATE ON user_sessions
    FOR EACH ROW
    EXECUTE FUNCTION update_session_access();