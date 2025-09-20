use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tower_http::auth::{AsyncAuthorizeRequest, AsyncRequireAuthorizationLayer};
use uuid::Uuid;

use crate::{jwt::JwtService, models::AuthContext, rbac::RbacService, session::SessionService};
use kb_error::{KbError, Result};

/// 认证中间件
pub struct AuthMiddleware {
    jwt_service: Arc<JwtService>,
    session_service: Arc<SessionService>,
    rbac_service: Arc<RbacService>,
}

impl AuthMiddleware {
    pub fn new(
        jwt_service: Arc<JwtService>,
        session_service: Arc<SessionService>,
        rbac_service: Arc<RbacService>,
    ) -> Self {
        Self {
            jwt_service,
            session_service,
            rbac_service,
        }
    }

    /// 从请求中提取认证信息
    async fn extract_auth_context(&self, headers: &HeaderMap) -> Result<AuthContext> {
        // 首先尝试JWT认证
        if let Some(auth_header) = headers.get("authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Ok(token) = JwtService::extract_token_from_header(auth_str) {
                    if let Ok(claims) = self.jwt_service.verify_access_token(token) {
                        let user_id = claims.user_id()?;
                        let tenant_id = claims.tenant_id()?;

                        // 验证会话（如果有session_id）
                        if let Some(session_id) = &claims.session_id {
                            if !self.session_service.validate_session(session_id).await? {
                                return Err(KbError::Authentication {
                                    message: "Session已过期或无效".to_string(),
                                });
                            }
                            // 更新会话访问时间
                            self.session_service.touch_session(session_id).await?;
                        }

                        // 获取完整的认证上下文
                        return self
                            .rbac_service
                            .get_auth_context(user_id, tenant_id, claims.session_id, None)
                            .await;
                    }
                }
            }
        }

        // 尝试API Key认证
        if let Some(api_key_header) = headers.get("x-api-key") {
            if let Ok(api_key) = api_key_header.to_str() {
                return self.authenticate_api_key(api_key).await;
            }
        }

        // 尝试Cookie认证
        if let Some(cookie_header) = headers.get("cookie") {
            if let Ok(cookie_str) = cookie_header.to_str() {
                if let Some(session_id) = extract_session_from_cookie(cookie_str) {
                    if let Some(session) = self.session_service.get_session(&session_id).await? {
                        if !session.is_expired() {
                            // 更新会话访问时间
                            self.session_service.touch_session(&session_id).await?;

                            return Ok(AuthContext {
                                user_id: session.user_id,
                                username: session.username,
                                email: session.email,
                                display_name: None,
                                status: crate::models::UserStatus::Active,
                                roles: session.roles,
                                permissions: session.permissions,
                                tenant_id: session.tenant_id,
                                session_id: Some(session_id),
                                api_key_id: None,
                            });
                        }
                    }
                }
            }
        }

        Err(KbError::Authentication {
            message: "未提供有效的认证凭据".to_string(),
        })
    }

    /// API Key认证 - 简化版，避免编译时数据库查询
    async fn authenticate_api_key(&self, _api_key: &str) -> Result<AuthContext> {
        // 简化实现，稍后在运行时完善
        Err(KbError::Authentication {
            message: "API Key认证暂未实现".to_string(),
        })
    }
}

impl<B> AsyncAuthorizeRequest<B> for AuthMiddleware
where
    B: Send + 'static,
{
    type RequestBody = B;
    type ResponseBody = axum::body::Body;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<Request<B>, Response<Self::ResponseBody>>,
                > + Send,
        >,
    >;

    fn authorize(&mut self, request: Request<B>) -> Self::Future {
        let jwt_service = self.jwt_service.clone();
        let session_service = self.session_service.clone();
        let rbac_service = self.rbac_service.clone();

        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let headers = &parts.headers;

            let auth_middleware = AuthMiddleware {
                jwt_service,
                session_service,
                rbac_service,
            };

            match auth_middleware.extract_auth_context(headers).await {
                Ok(auth_context) => {
                    let mut request = Request::from_parts(parts, body);
                    request.extensions_mut().insert(auth_context);
                    Ok(request)
                }
                Err(e) => {
                    let error_response = match e {
                        KbError::Authentication { .. } => {
                            (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
                        }
                        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
                            .into_response(),
                    };
                    Err(error_response)
                }
            }
        })
    }
}

/// 权限检查中间件
pub struct RequirePermissionLayer {
    permission: String,
}

impl RequirePermissionLayer {
    pub fn new(permission: String) -> AsyncRequireAuthorizationLayer<Self> {
        AsyncRequireAuthorizationLayer::new(Self { permission })
    }
}

impl<B> AsyncAuthorizeRequest<B> for RequirePermissionLayer
where
    B: Send + 'static,
{
    type RequestBody = B;
    type ResponseBody = axum::body::Body;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<Request<B>, Response<Self::ResponseBody>>,
                > + Send,
        >,
    >;

    fn authorize(&mut self, request: Request<B>) -> Self::Future {
        let required_permission = self.permission.clone();

        Box::pin(async move {
            let auth_context = request.extensions().get::<AuthContext>();

            if let Some(auth) = auth_context {
                if auth.has_permission(&required_permission) || auth.is_super_admin() {
                    Ok(request)
                } else {
                    Err((StatusCode::FORBIDDEN, "权限不足").into_response())
                }
            } else {
                Err((StatusCode::UNAUTHORIZED, "未认证").into_response())
            }
        })
    }
}

/// 多权限检查中间件（任一满足即可）
pub struct RequireAnyPermissionLayer {
    permissions: Vec<String>,
}

impl RequireAnyPermissionLayer {
    pub fn new(permissions: Vec<String>) -> AsyncRequireAuthorizationLayer<Self> {
        AsyncRequireAuthorizationLayer::new(Self { permissions })
    }
}

impl<B> AsyncAuthorizeRequest<B> for RequireAnyPermissionLayer
where
    B: Send + 'static,
{
    type RequestBody = B;
    type ResponseBody = axum::body::Body;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<Request<B>, Response<Self::ResponseBody>>,
                > + Send,
        >,
    >;

    fn authorize(&mut self, request: Request<B>) -> Self::Future {
        let required_permissions = self.permissions.clone();

        Box::pin(async move {
            let auth_context = request.extensions().get::<AuthContext>();

            if let Some(auth) = auth_context {
                let permission_refs: Vec<&str> =
                    required_permissions.iter().map(|s| s.as_str()).collect();
                if auth.has_any_permission(&permission_refs) || auth.is_super_admin() {
                    Ok(request)
                } else {
                    Err((StatusCode::FORBIDDEN, "权限不足").into_response())
                }
            } else {
                Err((StatusCode::UNAUTHORIZED, "未认证").into_response())
            }
        })
    }
}

/// 租户隔离中间件
pub struct RequireTenantAccess;

impl RequireTenantAccess {
    pub fn layer() -> AsyncRequireAuthorizationLayer<Self> {
        AsyncRequireAuthorizationLayer::new(Self)
    }
}

impl<B> AsyncAuthorizeRequest<B> for RequireTenantAccess
where
    B: Send + 'static,
{
    type RequestBody = B;
    type ResponseBody = axum::body::Body;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<Request<B>, Response<Self::ResponseBody>>,
                > + Send,
        >,
    >;

    fn authorize(&mut self, request: Request<B>) -> Self::Future {
        Box::pin(async move {
            let auth_context = request.extensions().get::<AuthContext>();

            if let Some(auth) = auth_context {
                // 超级管理员可以访问所有租户
                if auth.is_super_admin() {
                    return Ok(request);
                }

                // 从路径中提取租户ID并验证
                let path = request.uri().path();
                if let Some(tenant_id_str) = extract_tenant_id_from_path(path) {
                    if let Ok(tenant_id) = Uuid::parse_str(tenant_id_str) {
                        if auth.is_tenant_member(&tenant_id) {
                            Ok(request)
                        } else {
                            Err((StatusCode::FORBIDDEN, "无权访问此租户").into_response())
                        }
                    } else {
                        Err((StatusCode::BAD_REQUEST, "无效的租户ID").into_response())
                    }
                } else {
                    // 如果路径中没有租户ID，检查用户是否有默认租户
                    if auth.tenant_id.is_some() {
                        Ok(request)
                    } else {
                        Err((StatusCode::FORBIDDEN, "需要指定租户").into_response())
                    }
                }
            } else {
                Err((StatusCode::UNAUTHORIZED, "未认证").into_response())
            }
        })
    }
}

// 工具函数
fn extract_session_from_cookie(cookie_str: &str) -> Option<String> {
    cookie_str.split(';').find_map(|part| {
        let trimmed = part.trim();
        if trimmed.starts_with("session_id=") {
            Some(trimmed[11..].to_string())
        } else {
            None
        }
    })
}

fn extract_tenant_id_from_path(path: &str) -> Option<&str> {
    // 假设路径格式为 /api/v1/tenants/{tenant_id}/...
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 5 && parts[3] == "tenants" {
        Some(parts[4])
    } else {
        None
    }
}

/// 审计日志中间件
pub async fn audit_middleware(auth: Option<AuthContext>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start_time = std::time::Instant::now();

    let response = next.run(request).await;

    let duration = start_time.elapsed();
    let status = response.status();

    // 记录审计日志（如果有认证上下文）
    if let Some(auth_context) = auth {
        tracing::info!(
            user_id = %auth_context.user_id,
            username = %auth_context.username,
            method = %method,
            uri = %uri,
            status = %status,
            duration_ms = duration.as_millis(),
            "API访问审计"
        );

        // 这里可以添加写入数据库的审计日志逻辑
        // audit_service.log_api_access(...).await;
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_session_from_cookie() {
        let cookie = "session_id=sess_12345; other=value";
        assert_eq!(
            extract_session_from_cookie(cookie),
            Some("sess_12345".to_string())
        );

        let cookie_no_session = "other=value; another=data";
        assert_eq!(extract_session_from_cookie(cookie_no_session), None);
    }

    #[test]
    fn test_extract_tenant_id_from_path() {
        let path = "/api/v1/tenants/550e8400-e29b-41d4-a716-446655440000/documents";
        assert_eq!(
            extract_tenant_id_from_path(path),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );

        let path_no_tenant = "/api/v1/users";
        assert_eq!(extract_tenant_id_from_path(path_no_tenant), None);
    }
}
