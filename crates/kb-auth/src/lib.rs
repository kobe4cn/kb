pub mod models;
pub mod jwt;
pub mod middleware;
pub mod rbac;
pub mod session;
pub mod password;
pub mod permissions;

// 重新导出核心类型
pub use models::{User, Role, UserRole, ApiKey, UserStatus, AuthContext};
pub use jwt::{JwtService, Claims};
pub use middleware::{AuthMiddleware, RequirePermissionLayer, RequireAnyPermissionLayer};
pub use rbac::{RbacService, PermissionCheck};
pub use session::{SessionService, SessionInfo};
pub use password::PasswordService;
pub use permissions::{Permission, SystemRole};

// 错误类型
pub use kb_error::{KbError, Result};