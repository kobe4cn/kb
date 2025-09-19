use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Json as ResponseJson},
    routing::{get, post},
    Router,
};
use kb_auth::{
    jwt::JwtService,
    models::{LoginResponse, UserInfo},
    rbac::RbacService,
    session::SessionService,
};
use kb_error::{KbError, Result};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// 应用状态中的认证服务
#[derive(Clone)]
pub struct AuthServices {
    pub jwt_service: Arc<JwtService>,
    pub session_service: Arc<SessionService>,
    pub rbac_service: Arc<RbacService>,
}

impl AuthServices {
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
}

/// 用户登录请求
#[derive(Deserialize)]
pub struct LoginRequestPayload {
    pub username: String,
    pub password: String,
    pub remember_me: Option<bool>,
}

/// 用户注册请求
#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

/// 令牌刷新请求
#[derive(Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

/// 密码修改请求
#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// 创建认证路由
pub fn create_auth_routes() -> Router<AuthServices> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/refresh", post(refresh_token))
        .route("/register", post(register))
        .route("/me", get(get_current_user))
        .route("/change-password", post(change_password))
}

/// 用户登录
async fn login(
    State(auth_services): State<AuthServices>,
    Json(request): Json<LoginRequestPayload>,
) -> Result<impl IntoResponse> {
    // 简化实现：这里应该验证用户凭据
    // 在生产环境中，需要查询数据库验证用户名和密码

    // 模拟用户ID（实际应从数据库获取）
    let user_id = Uuid::new_v4();
    let tenant_id = Some(Uuid::new_v4());

    // 验证密码（简化）
    let password_valid = true; // 实际应调用 PasswordService::verify_password

    if !password_valid {
        return Err(KbError::Authentication {
            message: "用户名或密码错误".to_string(),
        });
    }

    // 创建会话
    let session = auth_services
        .session_service
        .create_session(
            user_id,
            request.username.clone(),
            format!("{}@example.com", request.username), // 简化
            tenant_id,
            vec!["user".to_string()],
            std::collections::HashSet::new(),
            None, // IP地址
            None, // User-Agent
            request.remember_me.unwrap_or(false),
        )
        .await?;

    // 生成JWT令牌
    let access_token = auth_services.jwt_service.generate_access_token(
        user_id,
        request.username.clone(),
        format!("{}@example.com", request.username),
        tenant_id,
        Some(session.session_id.clone()),
    )?;

    let refresh_token = auth_services.jwt_service.generate_refresh_token(
        user_id,
        request.username.clone(),
        format!("{}@example.com", request.username),
        tenant_id,
        Some(session.session_id.clone()),
    )?;

    let response = LoginResponse {
        access_token,
        refresh_token,
        expires_in: 3600, // 1小时
        user: UserInfo {
            id: user_id,
            username: request.username.clone(),
            email: format!("{}@example.com", request.username),
            display_name: None,
            status: kb_auth::models::UserStatus::Active,
        },
    };

    Ok((StatusCode::OK, ResponseJson(response)))
}

/// 用户登出
async fn logout(
    State(_auth_services): State<AuthServices>,
    // TODO: 从请求中提取认证上下文
) -> Result<impl IntoResponse> {
    // 简化实现：在生产环境中需要从中间件获取当前用户的session_id
    // 然后删除对应的会话

    Ok((StatusCode::OK, ResponseJson(serde_json::json!({
        "message": "登出成功"
    }))))
}

/// 刷新访问令牌
async fn refresh_token(
    State(auth_services): State<AuthServices>,
    Json(request): Json<RefreshTokenRequest>,
) -> Result<impl IntoResponse> {
    // 验证刷新令牌
    let claims = auth_services.jwt_service.verify_refresh_token(&request.refresh_token)?;

    // 生成新的访问令牌
    let new_access_token = auth_services.jwt_service.generate_access_token(
        claims.user_id()?,
        claims.username.clone(),
        claims.email.clone(),
        claims.tenant_id()?,
        claims.session_id.clone(),
    )?;

    Ok((StatusCode::OK, ResponseJson(serde_json::json!({
        "access_token": new_access_token,
        "expires_in": 3600
    }))))
}

/// 用户注册
async fn register(
    State(_auth_services): State<AuthServices>,
    Json(_request): Json<RegisterRequest>,
) -> impl IntoResponse {
    // 简化实现：在生产环境中需要完整的用户注册逻辑
    KbError::Internal {
        message: "用户注册功能暂未实现".to_string(),
        details: None,
    }
}

/// 获取当前用户信息
async fn get_current_user(
    State(_auth_services): State<AuthServices>,
    // TODO: 从中间件提取AuthContext
) -> Result<impl IntoResponse> {
    // 简化实现：返回模拟的用户信息
    let user = UserInfo {
        id: Uuid::new_v4(),
        username: "current_user".to_string(),
        email: "user@example.com".to_string(),
        display_name: Some("当前用户".to_string()),
        status: kb_auth::models::UserStatus::Active,
    };

    Ok((StatusCode::OK, ResponseJson(user)))
}

/// 修改密码
async fn change_password(
    State(_auth_services): State<AuthServices>,
    Json(_request): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    // 简化实现：在生产环境中需要完整的密码修改逻辑
    KbError::Internal {
        message: "密码修改功能暂未实现".to_string(),
        details: None,
    }
}

