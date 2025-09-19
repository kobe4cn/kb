use async_trait::async_trait;
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use kb_error::{KbError, Result};
use crate::models::AuthContext;
use crate::permissions::SystemRole;

/// 权限检查特质
#[async_trait]
pub trait PermissionCheck: Send + Sync {
    async fn has_permission(&self, user_id: Uuid, permission: &str, tenant_id: Option<Uuid>) -> Result<bool>;
    async fn has_any_permission(&self, user_id: Uuid, permissions: &[&str], tenant_id: Option<Uuid>) -> Result<bool>;
    async fn has_all_permissions(&self, user_id: Uuid, permissions: &[&str], tenant_id: Option<Uuid>) -> Result<bool>;
    async fn has_role(&self, user_id: Uuid, role: &str, tenant_id: Option<Uuid>) -> Result<bool>;
    async fn get_user_permissions(&self, user_id: Uuid, tenant_id: Option<Uuid>) -> Result<HashSet<String>>;
    async fn get_user_roles(&self, user_id: Uuid, tenant_id: Option<Uuid>) -> Result<Vec<String>>;
}

/// RBAC服务 - 角色基础访问控制
pub struct RbacService {
    db_pool: PgPool,
    permission_cache: tokio::sync::RwLock<HashMap<String, HashSet<String>>>, // user_key -> permissions
    role_cache: tokio::sync::RwLock<HashMap<String, Vec<String>>>, // user_key -> roles
}

impl RbacService {
    pub fn new(db_pool: PgPool) -> Self {
        Self {
            db_pool,
            permission_cache: tokio::sync::RwLock::new(HashMap::new()),
            role_cache: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// 生成缓存键
    fn cache_key(user_id: Uuid, tenant_id: Option<Uuid>) -> String {
        match tenant_id {
            Some(tid) => format!("{}:{}", user_id, tid),
            None => user_id.to_string(),
        }
    }

    /// 清除用户缓存
    pub async fn clear_user_cache(&self, user_id: Uuid, tenant_id: Option<Uuid>) {
        let key = Self::cache_key(user_id, tenant_id);

        let mut permission_cache = self.permission_cache.write().await;
        permission_cache.remove(&key);

        let mut role_cache = self.role_cache.write().await;
        role_cache.remove(&key);
    }

    /// 清除所有缓存
    pub async fn clear_all_cache(&self) {
        let mut permission_cache = self.permission_cache.write().await;
        permission_cache.clear();

        let mut role_cache = self.role_cache.write().await;
        role_cache.clear();
    }

    /// 为用户分配角色
    pub async fn assign_role(
        &self,
        user_id: Uuid,
        role_id: Uuid,
        tenant_id: Option<Uuid>,
        granted_by: Uuid,
    ) -> Result<()> {
        // 简化实现：跳过角色存在性检查，由数据库外键约束保证

        // 插入用户角色关联
        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id, tenant_id, granted_by)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id, role_id, tenant_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(role_id)
        .bind(tenant_id)
        .bind(granted_by)
        .execute(&self.db_pool)
        .await
        .map_err(|e| KbError::Database {
            message: format!("assign_role: {}", e),
            context: None,
        })?;

        // 清除缓存
        self.clear_user_cache(user_id, tenant_id).await;

        Ok(())
    }

    /// 撤销用户角色
    pub async fn revoke_role(
        &self,
        user_id: Uuid,
        role_id: Uuid,
        tenant_id: Option<Uuid>,
    ) -> Result<()> {
        let result = sqlx::query(
            "DELETE FROM user_roles WHERE user_id = $1 AND role_id = $2 AND tenant_id IS NOT DISTINCT FROM $3",
        )
        .bind(user_id)
        .bind(role_id)
        .bind(tenant_id)
        .execute(&self.db_pool)
        .await
        .map_err(|e| KbError::Database {
            message: format!("revoke_role: {}", e),
            context: None,
        })?;

        let deleted = result.rows_affected();

        if deleted == 0 {
            return Err(KbError::NotFound {
                resource: format!("user_role: user:{}, role:{}", user_id, role_id),
            });
        }

        // 清除缓存
        self.clear_user_cache(user_id, tenant_id).await;

        Ok(())
    }

    /// 获取用户的认证上下文
    pub async fn get_auth_context(
        &self,
        user_id: Uuid,
        tenant_id: Option<Uuid>,
        session_id: Option<String>,
        api_key_id: Option<Uuid>,
    ) -> Result<AuthContext> {
        // 简化实现：获取基本的认证上下文
        // 在生产环境中，这里需要查询数据库获取完整的用户信息
        let roles = self.get_user_roles(user_id, tenant_id).await?;
        let permissions = self.get_user_permissions(user_id, tenant_id).await?;

        Ok(AuthContext {
            user_id,
            username: "user".to_string(), // 简化：实际应从数据库获取
            email: "user@example.com".to_string(), // 简化：实际应从数据库获取
            display_name: None,
            status: crate::models::UserStatus::Active,
            roles,
            permissions,
            tenant_id,
            session_id,
            api_key_id,
        })
    }

    /// 检查用户是否为超级管理员
    pub async fn is_super_admin(&self, user_id: Uuid) -> Result<bool> {
        self.has_role(user_id, SystemRole::SUPER_ADMIN, None).await
    }

    /// 检查用户是否为租户管理员
    pub async fn is_tenant_admin(&self, user_id: Uuid, tenant_id: Uuid) -> Result<bool> {
        self.has_role(user_id, SystemRole::TENANT_ADMIN, Some(tenant_id)).await
    }

    /// 创建默认系统角色
    pub async fn create_system_roles(&self) -> Result<()> {
        for role_name in SystemRole::all() {
            let permissions = SystemRole::get_default_permissions(role_name);
            let permissions_json = serde_json::to_value(permissions).map_err(|e| {
                KbError::Internal {
                    message: format!("Failed to serialize permissions: {}", e),
                    details: None,
                }
            })?;

            sqlx::query(
                r#"
                INSERT INTO roles (name, description, is_system, permissions)
                VALUES ($1, $2, true, $3)
                ON CONFLICT (name) DO UPDATE SET
                    permissions = EXCLUDED.permissions,
                    is_system = true
                "#,
            )
            .bind(role_name)
            .bind(format!("系统角色: {}", role_name))
            .bind(permissions_json)
            .execute(&self.db_pool)
            .await
            .map_err(|e| KbError::Database {
                message: format!("create_system_role: {}", e),
                context: None,
            })?;
        }

        Ok(())
    }
}

#[async_trait]
impl PermissionCheck for RbacService {
    async fn has_permission(&self, user_id: Uuid, permission: &str, tenant_id: Option<Uuid>) -> Result<bool> {
        let permissions = self.get_user_permissions(user_id, tenant_id).await?;
        Ok(permissions.contains(permission))
    }

    async fn has_any_permission(&self, user_id: Uuid, permissions: &[&str], tenant_id: Option<Uuid>) -> Result<bool> {
        let user_permissions = self.get_user_permissions(user_id, tenant_id).await?;
        Ok(permissions.iter().any(|p| user_permissions.contains(*p)))
    }

    async fn has_all_permissions(&self, user_id: Uuid, permissions: &[&str], tenant_id: Option<Uuid>) -> Result<bool> {
        let user_permissions = self.get_user_permissions(user_id, tenant_id).await?;
        Ok(permissions.iter().all(|p| user_permissions.contains(*p)))
    }

    async fn has_role(&self, user_id: Uuid, role: &str, tenant_id: Option<Uuid>) -> Result<bool> {
        let roles = self.get_user_roles(user_id, tenant_id).await?;
        Ok(roles.contains(&role.to_string()))
    }

    async fn get_user_permissions(&self, user_id: Uuid, tenant_id: Option<Uuid>) -> Result<HashSet<String>> {
        let cache_key = Self::cache_key(user_id, tenant_id);

        // 尝试从缓存获取
        {
            let cache = self.permission_cache.read().await;
            if let Some(permissions) = cache.get(&cache_key) {
                return Ok(permissions.clone());
            }
        }

        // 从数据库获取
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT r.permissions
            FROM roles r
            JOIN user_roles ur ON r.id = ur.role_id
            WHERE ur.user_id = $1
            AND ur.tenant_id IS NOT DISTINCT FROM $2
            "#,
        )
        .bind(user_id)
        .bind(tenant_id)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| KbError::Database {
            message: format!("get_user_permissions: {}", e),
            context: None,
        })?;

        let mut permissions = HashSet::new();
        for row in rows {
            let permissions_value: serde_json::Value = row.get("permissions");
            if let Ok(perms) = serde_json::from_value::<Vec<String>>(permissions_value) {
                permissions.extend(perms);
            }
        }

        // 更新缓存
        {
            let mut cache = self.permission_cache.write().await;
            cache.insert(cache_key, permissions.clone());
        }

        Ok(permissions)
    }

    async fn get_user_roles(&self, user_id: Uuid, tenant_id: Option<Uuid>) -> Result<Vec<String>> {
        let cache_key = Self::cache_key(user_id, tenant_id);

        // 尝试从缓存获取
        {
            let cache = self.role_cache.read().await;
            if let Some(roles) = cache.get(&cache_key) {
                return Ok(roles.clone());
            }
        }

        // 从数据库获取
        let rows = sqlx::query(
            r#"
            SELECT r.name
            FROM roles r
            JOIN user_roles ur ON r.id = ur.role_id
            WHERE ur.user_id = $1
            AND ur.tenant_id IS NOT DISTINCT FROM $2
            "#,
        )
        .bind(user_id)
        .bind(tenant_id)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| KbError::Database {
            message: format!("get_user_roles: {}", e),
            context: None,
        })?;

        let roles: Vec<String> = rows.into_iter().map(|row| row.get::<String, _>("name")).collect();

        // 更新缓存
        {
            let mut cache = self.role_cache.write().await;
            cache.insert(cache_key, roles.clone());
        }

        Ok(roles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // 注意：实际测试需要数据库连接，这里只是结构示例

    #[test]
    fn test_cache_key_generation() {
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();

        let key_with_tenant = RbacService::cache_key(user_id, Some(tenant_id));
        let key_without_tenant = RbacService::cache_key(user_id, None);

        assert_eq!(key_with_tenant, format!("{}:{}", user_id, tenant_id));
        assert_eq!(key_without_tenant, user_id.to_string());
    }
}