use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use kb_error::{KbError, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,                // subject (user ID)
    pub username: String,           // username
    pub email: String,              // email
    pub tenant_id: Option<String>,  // tenant ID
    pub session_id: Option<String>, // session ID
    pub exp: i64,                   // expiration timestamp
    pub iat: i64,                   // issued at timestamp
    pub iss: String,                // issuer
    pub aud: String,                // audience
    pub jti: String,                // JWT ID
    pub typ: String,                // token type: "access" or "refresh"
}

impl Claims {
    pub fn new_access_token(
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        session_id: Option<String>,
        expires_in_hours: i64,
    ) -> Self {
        let now = Utc::now();
        let exp = now + Duration::hours(expires_in_hours);

        Self {
            sub: user_id.to_string(),
            username,
            email,
            tenant_id: tenant_id.map(|id| id.to_string()),
            session_id,
            exp: exp.timestamp(),
            iat: now.timestamp(),
            iss: "kb-auth".to_string(),
            aud: "kb-api".to_string(),
            jti: Uuid::new_v4().to_string(),
            typ: "access".to_string(),
        }
    }

    pub fn new_refresh_token(
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        session_id: Option<String>,
        expires_in_days: i64,
    ) -> Self {
        let now = Utc::now();
        let exp = now + Duration::days(expires_in_days);

        Self {
            sub: user_id.to_string(),
            username,
            email,
            tenant_id: tenant_id.map(|id| id.to_string()),
            session_id,
            exp: exp.timestamp(),
            iat: now.timestamp(),
            iss: "kb-auth".to_string(),
            aud: "kb-api".to_string(),
            jti: Uuid::new_v4().to_string(),
            typ: "refresh".to_string(),
        }
    }

    pub fn user_id(&self) -> Result<Uuid> {
        Uuid::parse_str(&self.sub).map_err(|e| KbError::Authentication {
            message: format!("Invalid user ID in token: {}", e),
        })
    }

    pub fn tenant_id(&self) -> Result<Option<Uuid>> {
        match &self.tenant_id {
            Some(id) => Ok(Some(Uuid::parse_str(id).map_err(|e| {
                KbError::Authentication {
                    message: format!("Invalid tenant ID in token: {}", e),
                }
            })?)),
            None => Ok(None),
        }
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.exp, 0).unwrap_or_else(Utc::now)
    }

    pub fn issued_at(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.iat, 0).unwrap_or_else(Utc::now)
    }

    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() >= self.exp
    }

    pub fn is_access_token(&self) -> bool {
        self.typ == "access"
    }

    pub fn is_refresh_token(&self) -> bool {
        self.typ == "refresh"
    }
}

pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    validation: Validation,
}

impl JwtService {
    pub fn new(secret: &str) -> Self {
        let encoding_key = EncodingKey::from_secret(secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());

        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&["kb-auth"]);
        validation.set_audience(&["kb-api"]);

        Self {
            encoding_key,
            decoding_key,
            validation,
        }
    }

    /// 生成访问令牌
    pub fn generate_access_token(
        &self,
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        session_id: Option<String>,
    ) -> Result<String> {
        let claims = Claims::new_access_token(
            user_id, username, email, tenant_id, session_id, 24, // 24小时过期
        );

        encode(&Header::default(), &claims, &self.encoding_key).map_err(|e| KbError::Internal {
            message: format!("Failed to generate access token: {}", e),
            details: None,
        })
    }

    /// 生成刷新令牌
    pub fn generate_refresh_token(
        &self,
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        session_id: Option<String>,
    ) -> Result<String> {
        let claims = Claims::new_refresh_token(
            user_id, username, email, tenant_id, session_id, 30, // 30天过期
        );

        encode(&Header::default(), &claims, &self.encoding_key).map_err(|e| KbError::Internal {
            message: format!("Failed to generate refresh token: {}", e),
            details: None,
        })
    }

    /// 验证并解码JWT令牌
    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        decode::<Claims>(token, &self.decoding_key, &self.validation)
            .map(|data| data.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => KbError::Authentication {
                    message: "Token已过期".to_string(),
                },
                jsonwebtoken::errors::ErrorKind::InvalidToken => KbError::Authentication {
                    message: "无效的Token".to_string(),
                },
                jsonwebtoken::errors::ErrorKind::InvalidSignature => KbError::Authentication {
                    message: "Token签名无效".to_string(),
                },
                _ => KbError::Authentication {
                    message: format!("Token验证失败: {}", e),
                },
            })
    }

    /// 从Authorization header中提取token
    pub fn extract_token_from_header(authorization: &str) -> Result<&str> {
        if let Some(token) = authorization.strip_prefix("Bearer ") {
            Ok(token)
        } else {
            Err(KbError::Authentication {
                message: "Invalid Authorization header format".to_string(),
            })
        }
    }

    /// 验证访问令牌
    pub fn verify_access_token(&self, token: &str) -> Result<Claims> {
        let claims = self.verify_token(token)?;

        if !claims.is_access_token() {
            return Err(KbError::Authentication {
                message: "Not an access token".to_string(),
            });
        }

        Ok(claims)
    }

    /// 验证刷新令牌
    pub fn verify_refresh_token(&self, token: &str) -> Result<Claims> {
        let claims = self.verify_token(token)?;

        if !claims.is_refresh_token() {
            return Err(KbError::Authentication {
                message: "Not a refresh token".to_string(),
            });
        }

        Ok(claims)
    }

    /// 生成Token对
    pub fn generate_token_pair(
        &self,
        user_id: Uuid,
        username: String,
        email: String,
        tenant_id: Option<Uuid>,
        session_id: Option<String>,
    ) -> Result<(String, String)> {
        let access_token = self.generate_access_token(
            user_id,
            username.clone(),
            email.clone(),
            tenant_id,
            session_id.clone(),
        )?;

        let refresh_token =
            self.generate_refresh_token(user_id, username, email, tenant_id, session_id)?;

        Ok((access_token, refresh_token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_token_generation_and_verification() {
        let jwt_service = JwtService::new("test_secret_key_123456789");
        let user_id = Uuid::new_v4();
        let username = "testuser".to_string();
        let email = "test@example.com".to_string();

        // 生成访问令牌
        let access_token = jwt_service
            .generate_access_token(user_id, username.clone(), email.clone(), None, None)
            .unwrap();

        // 验证访问令牌
        let claims = jwt_service.verify_access_token(&access_token).unwrap();
        assert_eq!(claims.user_id().unwrap(), user_id);
        assert_eq!(claims.username, username);
        assert_eq!(claims.email, email);
        assert!(claims.is_access_token());
    }

    #[test]
    fn test_jwt_token_pair_generation() {
        let jwt_service = JwtService::new("test_secret_key_123456789");
        let user_id = Uuid::new_v4();
        let username = "testuser".to_string();
        let email = "test@example.com".to_string();

        let (access_token, refresh_token) = jwt_service
            .generate_token_pair(user_id, username, email, None, None)
            .unwrap();

        // 验证访问令牌
        let access_claims = jwt_service.verify_access_token(&access_token).unwrap();
        assert!(access_claims.is_access_token());

        // 验证刷新令牌
        let refresh_claims = jwt_service.verify_refresh_token(&refresh_token).unwrap();
        assert!(refresh_claims.is_refresh_token());
    }

    #[test]
    fn test_extract_token_from_header() {
        let auth_header = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...";
        let token = JwtService::extract_token_from_header(auth_header).unwrap();
        assert_eq!(token, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...");

        // 测试无效格式
        let invalid_header = "Invalid token";
        assert!(JwtService::extract_token_from_header(invalid_header).is_err());
    }
}
