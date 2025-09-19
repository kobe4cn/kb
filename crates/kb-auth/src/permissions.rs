/// 权限定义模块
/// 定义系统中所有可用的权限和角色

/// 权限常量定义
pub struct Permission;

impl Permission {
    // 用户管理权限
    pub const USERS_READ: &'static str = "users:read";
    pub const USERS_WRITE: &'static str = "users:write";
    pub const USERS_DELETE: &'static str = "users:delete";
    pub const USERS_ADMIN: &'static str = "users:admin";

    // 角色管理权限
    pub const ROLES_READ: &'static str = "roles:read";
    pub const ROLES_WRITE: &'static str = "roles:write";
    pub const ROLES_DELETE: &'static str = "roles:delete";

    // 文档管理权限
    pub const DOCUMENTS_READ: &'static str = "documents:read";
    pub const DOCUMENTS_WRITE: &'static str = "documents:write";
    pub const DOCUMENTS_DELETE: &'static str = "documents:delete";
    pub const DOCUMENTS_ADMIN: &'static str = "documents:admin";

    // 租户管理权限
    pub const TENANTS_READ: &'static str = "tenants:read";
    pub const TENANTS_WRITE: &'static str = "tenants:write";
    pub const TENANTS_DELETE: &'static str = "tenants:delete";
    pub const TENANTS_ADMIN: &'static str = "tenants:admin";

    // 系统管理权限
    pub const SYSTEM_CONFIG: &'static str = "system:config";
    pub const SYSTEM_MONITOR: &'static str = "system:monitor";
    pub const SYSTEM_LOGS: &'static str = "system:logs";
    pub const SYSTEM_ADMIN: &'static str = "system:admin";

    // API密钥管理权限
    pub const API_KEYS_READ: &'static str = "api_keys:read";
    pub const API_KEYS_WRITE: &'static str = "api_keys:write";
    pub const API_KEYS_DELETE: &'static str = "api_keys:delete";

    // 任务管理权限
    pub const JOBS_READ: &'static str = "jobs:read";
    pub const JOBS_WRITE: &'static str = "jobs:write";
    pub const JOBS_DELETE: &'static str = "jobs:delete";
    pub const JOBS_ADMIN: &'static str = "jobs:admin";

    // 查询权限
    pub const QUERY_EXECUTE: &'static str = "query:execute";
    pub const QUERY_ADMIN: &'static str = "query:admin";

    // 返回所有权限列表
    pub fn all() -> Vec<&'static str> {
        vec![
            Self::USERS_READ,
            Self::USERS_WRITE,
            Self::USERS_DELETE,
            Self::USERS_ADMIN,
            Self::ROLES_READ,
            Self::ROLES_WRITE,
            Self::ROLES_DELETE,
            Self::DOCUMENTS_READ,
            Self::DOCUMENTS_WRITE,
            Self::DOCUMENTS_DELETE,
            Self::DOCUMENTS_ADMIN,
            Self::TENANTS_READ,
            Self::TENANTS_WRITE,
            Self::TENANTS_DELETE,
            Self::TENANTS_ADMIN,
            Self::SYSTEM_CONFIG,
            Self::SYSTEM_MONITOR,
            Self::SYSTEM_LOGS,
            Self::SYSTEM_ADMIN,
            Self::API_KEYS_READ,
            Self::API_KEYS_WRITE,
            Self::API_KEYS_DELETE,
            Self::JOBS_READ,
            Self::JOBS_WRITE,
            Self::JOBS_DELETE,
            Self::JOBS_ADMIN,
            Self::QUERY_EXECUTE,
            Self::QUERY_ADMIN,
        ]
    }
}

/// 系统角色定义
pub struct SystemRole;

impl SystemRole {
    pub const SUPER_ADMIN: &'static str = "super_admin";
    pub const TENANT_ADMIN: &'static str = "tenant_admin";
    pub const EDITOR: &'static str = "editor";
    pub const VIEWER: &'static str = "viewer";
    pub const API_USER: &'static str = "api_user";

    /// 获取角色的默认权限
    pub fn get_default_permissions(role: &str) -> Vec<&'static str> {
        match role {
            Self::SUPER_ADMIN => Permission::all(),
            Self::TENANT_ADMIN => vec![
                Permission::USERS_READ,
                Permission::USERS_WRITE,
                Permission::USERS_DELETE,
                Permission::ROLES_READ,
                Permission::DOCUMENTS_READ,
                Permission::DOCUMENTS_WRITE,
                Permission::DOCUMENTS_DELETE,
                Permission::DOCUMENTS_ADMIN,
                Permission::TENANTS_READ,
                Permission::API_KEYS_READ,
                Permission::API_KEYS_WRITE,
                Permission::API_KEYS_DELETE,
                Permission::JOBS_READ,
                Permission::JOBS_WRITE,
                Permission::JOBS_ADMIN,
                Permission::QUERY_EXECUTE,
                Permission::QUERY_ADMIN,
            ],
            Self::EDITOR => vec![
                Permission::DOCUMENTS_READ,
                Permission::DOCUMENTS_WRITE,
                Permission::JOBS_READ,
                Permission::JOBS_WRITE,
                Permission::QUERY_EXECUTE,
            ],
            Self::VIEWER => vec![
                Permission::DOCUMENTS_READ,
                Permission::JOBS_READ,
                Permission::QUERY_EXECUTE,
            ],
            Self::API_USER => vec![
                Permission::DOCUMENTS_READ,
                Permission::DOCUMENTS_WRITE,
                Permission::QUERY_EXECUTE,
            ],
            _ => vec![],
        }
    }

    /// 获取所有系统角色
    pub fn all() -> Vec<&'static str> {
        vec![
            Self::SUPER_ADMIN,
            Self::TENANT_ADMIN,
            Self::EDITOR,
            Self::VIEWER,
            Self::API_USER,
        ]
    }

    /// 检查是否为系统角色
    pub fn is_system_role(role: &str) -> bool {
        Self::all().contains(&role)
    }
}

/// 权限分组 - 用于UI展示
#[derive(Debug, Clone)]
pub struct PermissionGroup {
    pub name: &'static str,
    pub description: &'static str,
    pub permissions: Vec<&'static str>,
}

impl PermissionGroup {
    pub fn all_groups() -> Vec<PermissionGroup> {
        vec![
            PermissionGroup {
                name: "用户管理",
                description: "用户账户的创建、查看、编辑和删除",
                permissions: vec![
                    Permission::USERS_READ,
                    Permission::USERS_WRITE,
                    Permission::USERS_DELETE,
                    Permission::USERS_ADMIN,
                ],
            },
            PermissionGroup {
                name: "角色管理",
                description: "角色和权限的管理",
                permissions: vec![
                    Permission::ROLES_READ,
                    Permission::ROLES_WRITE,
                    Permission::ROLES_DELETE,
                ],
            },
            PermissionGroup {
                name: "文档管理",
                description: "文档的上传、索引、查看和管理",
                permissions: vec![
                    Permission::DOCUMENTS_READ,
                    Permission::DOCUMENTS_WRITE,
                    Permission::DOCUMENTS_DELETE,
                    Permission::DOCUMENTS_ADMIN,
                ],
            },
            PermissionGroup {
                name: "租户管理",
                description: "多租户环境的管理",
                permissions: vec![
                    Permission::TENANTS_READ,
                    Permission::TENANTS_WRITE,
                    Permission::TENANTS_DELETE,
                    Permission::TENANTS_ADMIN,
                ],
            },
            PermissionGroup {
                name: "系统管理",
                description: "系统配置、监控和日志管理",
                permissions: vec![
                    Permission::SYSTEM_CONFIG,
                    Permission::SYSTEM_MONITOR,
                    Permission::SYSTEM_LOGS,
                    Permission::SYSTEM_ADMIN,
                ],
            },
            PermissionGroup {
                name: "API管理",
                description: "API密钥的创建和管理",
                permissions: vec![
                    Permission::API_KEYS_READ,
                    Permission::API_KEYS_WRITE,
                    Permission::API_KEYS_DELETE,
                ],
            },
            PermissionGroup {
                name: "任务管理",
                description: "后台任务的监控和管理",
                permissions: vec![
                    Permission::JOBS_READ,
                    Permission::JOBS_WRITE,
                    Permission::JOBS_DELETE,
                    Permission::JOBS_ADMIN,
                ],
            },
            PermissionGroup {
                name: "查询服务",
                description: "知识库查询和相关功能",
                permissions: vec![
                    Permission::QUERY_EXECUTE,
                    Permission::QUERY_ADMIN,
                ],
            },
        ]
    }
}