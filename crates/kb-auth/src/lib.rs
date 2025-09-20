pub mod jwt;
pub mod middleware;
pub mod models;
pub mod password;
pub mod permissions;
pub mod rbac;
pub mod session;

// 重新导出核心类型
pub use jwt::{Claims, JwtService};
pub use middleware::{AuthMiddleware, RequireAnyPermissionLayer, RequirePermissionLayer};
pub use models::{ApiKey, AuthContext, Role, User, UserRole, UserStatus};
pub use password::PasswordService;
pub use permissions::{Permission, SystemRole};
pub use rbac::{PermissionCheck, RbacService};
pub use session::{SessionInfo, SessionService};

// 错误类型
pub use kb_error::{KbError, Result};
