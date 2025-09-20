use chrono::{DateTime, Duration, Utc};
use kb_error::{KbError, Result};
use redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub user_id: Uuid,
    pub username: String,
    pub email: String,
    pub tenant_id: Option<Uuid>,
    pub roles: Vec<String>,
    pub permissions: HashSet<String>,
    pub ip_address: Option<std::net::IpAddr>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_accessed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl SessionInfo {
    pub fn new(
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        roles: Vec<String>,
        permissions: HashSet<String>,
        ip_address: Option<std::net::IpAddr>,
        user_agent: Option<String>,
        ttl_hours: i64,
    ) -> Self {
        let now = Utc::now();
        let session_id = format!("sess_{}", Uuid::new_v4());

        Self {
            session_id,
            user_id,
            username,
            email,
            tenant_id,
            roles,
            permissions,
            ip_address,
            user_agent,
            created_at: now,
            last_accessed_at: now,
            expires_at: now + Duration::hours(ttl_hours),
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn touch(&mut self) {
        self.last_accessed_at = Utc::now();
    }

    pub fn extend_expiry(&mut self, additional_hours: i64) {
        self.expires_at = self.expires_at + Duration::hours(additional_hours);
    }
}

pub struct SessionService {
    redis_client: Option<RedisClient>,
    default_ttl_hours: i64,
}

impl SessionService {
    pub fn new(redis_url: Option<String>, default_ttl_hours: Option<i64>) -> Result<Self> {
        let redis_client = if let Some(url) = redis_url {
            Some(RedisClient::open(url).map_err(|e| KbError::Configuration {
                key: "redis_url".to_string(),
                reason: format!("Failed to connect to Redis: {}", e),
            })?)
        } else {
            None
        };

        Ok(Self {
            redis_client,
            default_ttl_hours: default_ttl_hours.unwrap_or(24),
        })
    }

    /// 创建新会话
    pub async fn create_session(
        &self,
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        roles: Vec<String>,
        permissions: HashSet<String>,
        ip_address: Option<std::net::IpAddr>,
        user_agent: Option<String>,
        remember_me: bool,
    ) -> Result<SessionInfo> {
        let ttl_hours = if remember_me {
            30 * 24 // 30天
        } else {
            self.default_ttl_hours
        };

        let session = SessionInfo::new(
            user_id,
            username,
            email,
            tenant_id,
            roles,
            permissions,
            ip_address,
            user_agent,
            ttl_hours,
        );

        // 保存到Redis（如果可用）
        if let Some(redis_client) = &self.redis_client {
            self.save_session_to_redis(redis_client, &session).await?;
        }

        Ok(session)
    }

    /// 获取会话信息
    pub async fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        if let Some(redis_client) = &self.redis_client {
            self.get_session_from_redis(redis_client, session_id).await
        } else {
            Ok(None)
        }
    }

    /// 更新会话访问时间
    pub async fn touch_session(&self, session_id: &str) -> Result<()> {
        if let Some(redis_client) = &self.redis_client {
            if let Some(mut session) = self
                .get_session_from_redis(redis_client, session_id)
                .await?
            {
                session.touch();
                self.save_session_to_redis(redis_client, &session).await?;
            }
        }
        Ok(())
    }

    /// 删除会话
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        if let Some(redis_client) = &self.redis_client {
            let mut conn =
                redis_client
                    .get_async_connection()
                    .await
                    .map_err(|e| KbError::Network {
                        operation: "redis_connection".to_string(),
                        message: e.to_string(),
                    })?;

            let key = format!("session:{}", session_id);
            conn.del::<_, ()>(&key)
                .await
                .map_err(|e| KbError::Network {
                    operation: "redis_delete".to_string(),
                    message: e.to_string(),
                })?;
        }
        Ok(())
    }

    /// 删除用户的所有会话
    pub async fn delete_user_sessions(&self, user_id: Uuid) -> Result<()> {
        if let Some(_redis_client) = &self.redis_client {
            let sessions = self.get_user_sessions(user_id).await?;
            for session in sessions {
                self.delete_session(&session.session_id).await?;
            }
        }
        Ok(())
    }

    /// 获取用户的所有活跃会话
    pub async fn get_user_sessions(&self, user_id: Uuid) -> Result<Vec<SessionInfo>> {
        if let Some(redis_client) = &self.redis_client {
            let mut conn =
                redis_client
                    .get_async_connection()
                    .await
                    .map_err(|e| KbError::Network {
                        operation: "redis_connection".to_string(),
                        message: e.to_string(),
                    })?;

            // 获取所有会话键
            let pattern = "session:*";
            let keys: Vec<String> = conn.keys(pattern).await.map_err(|e| KbError::Network {
                operation: "redis_keys".to_string(),
                message: e.to_string(),
            })?;

            let mut user_sessions = Vec::new();
            for key in keys {
                if let Ok(session_data) = conn.get::<_, String>(&key).await {
                    if let Ok(session) = serde_json::from_str::<SessionInfo>(&session_data) {
                        if session.user_id == user_id && !session.is_expired() {
                            user_sessions.push(session);
                        }
                    }
                }
            }

            Ok(user_sessions)
        } else {
            Ok(Vec::new())
        }
    }

    /// 清理过期会话
    pub async fn cleanup_expired_sessions(&self) -> Result<usize> {
        if let Some(redis_client) = &self.redis_client {
            let mut conn =
                redis_client
                    .get_async_connection()
                    .await
                    .map_err(|e| KbError::Network {
                        operation: "redis_connection".to_string(),
                        message: e.to_string(),
                    })?;

            let pattern = "session:*";
            let keys: Vec<String> = conn.keys(pattern).await.map_err(|e| KbError::Network {
                operation: "redis_keys".to_string(),
                message: e.to_string(),
            })?;

            let mut cleaned_count = 0;
            for key in keys {
                if let Ok(session_data) = conn.get::<_, String>(&key).await {
                    if let Ok(session) = serde_json::from_str::<SessionInfo>(&session_data) {
                        if session.is_expired() {
                            conn.del::<_, ()>(&key)
                                .await
                                .map_err(|e| KbError::Network {
                                    operation: "redis_delete".to_string(),
                                    message: e.to_string(),
                                })?;
                            cleaned_count += 1;
                        }
                    }
                }
            }

            Ok(cleaned_count)
        } else {
            Ok(0)
        }
    }

    /// 验证会话是否有效
    pub async fn validate_session(&self, session_id: &str) -> Result<bool> {
        if let Some(session) = self.get_session(session_id).await? {
            Ok(!session.is_expired())
        } else {
            Ok(false)
        }
    }

    /// 刷新会话过期时间
    pub async fn refresh_session(&self, session_id: &str, additional_hours: i64) -> Result<()> {
        if let Some(redis_client) = &self.redis_client {
            if let Some(mut session) = self
                .get_session_from_redis(redis_client, session_id)
                .await?
            {
                session.extend_expiry(additional_hours);
                session.touch();
                self.save_session_to_redis(redis_client, &session).await?;
            }
        }
        Ok(())
    }

    /// 保存会话到Redis
    async fn save_session_to_redis(
        &self,
        redis_client: &RedisClient,
        session: &SessionInfo,
    ) -> Result<()> {
        let mut conn = redis_client
            .get_async_connection()
            .await
            .map_err(|e| KbError::Network {
                operation: "redis_connection".to_string(),
                message: e.to_string(),
            })?;

        let key = format!("session:{}", session.session_id);
        let session_data = serde_json::to_string(session).map_err(|e| KbError::Internal {
            message: format!("Failed to serialize session: {}", e),
            details: None,
        })?;

        let ttl_seconds = (session.expires_at - Utc::now()).num_seconds().max(0) as u64;

        conn.set_ex(&key, session_data, ttl_seconds as usize)
            .await
            .map_err(|e| KbError::Network {
                operation: "redis_setex".to_string(),
                message: e.to_string(),
            })?;

        Ok(())
    }

    /// 从Redis获取会话
    async fn get_session_from_redis(
        &self,
        redis_client: &RedisClient,
        session_id: &str,
    ) -> Result<Option<SessionInfo>> {
        let mut conn = redis_client
            .get_async_connection()
            .await
            .map_err(|e| KbError::Network {
                operation: "redis_connection".to_string(),
                message: e.to_string(),
            })?;

        let key = format!("session:{}", session_id);
        let session_data: Option<String> = conn.get(&key).await.map_err(|e| KbError::Network {
            operation: "redis_get".to_string(),
            message: e.to_string(),
        })?;

        if let Some(data) = session_data {
            let session: SessionInfo =
                serde_json::from_str(&data).map_err(|e| KbError::Internal {
                    message: format!("Failed to deserialize session: {}", e),
                    details: None,
                })?;

            if session.is_expired() {
                // 删除过期会话
                let _: () = conn
                    .del::<_, ()>(&key)
                    .await
                    .map_err(|e| KbError::Network {
                        operation: "redis_delete".to_string(),
                        message: e.to_string(),
                    })?;
                Ok(None)
            } else {
                Ok(Some(session))
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[tokio::test]
    async fn test_session_creation() {
        let session_service = SessionService::new(None, Some(24)).unwrap();
        let user_id = Uuid::new_v4();
        let permissions = HashSet::new();

        let session = session_service
            .create_session(
                user_id,
                "testuser".to_string(),
                "test@example.com".to_string(),
                None,
                vec!["user".to_string()],
                permissions,
                None,
                None,
                false,
            )
            .await
            .unwrap();

        assert_eq!(session.user_id, user_id);
        assert!(!session.is_expired());
        assert!(session.session_id.starts_with("sess_"));
    }

    #[test]
    fn test_session_expiry() {
        let now = Utc::now();
        let mut session = SessionInfo {
            session_id: "test_session".to_string(),
            user_id: Uuid::new_v4(),
            username: "test".to_string(),
            email: "test@example.com".to_string(),
            tenant_id: None,
            roles: vec![],
            permissions: HashSet::new(),
            ip_address: None,
            user_agent: None,
            created_at: now,
            last_accessed_at: now,
            expires_at: now - Duration::hours(1), // 已过期
        };

        assert!(session.is_expired());

        // 延长过期时间
        session.extend_expiry(2);
        assert!(!session.is_expired());
    }
}
