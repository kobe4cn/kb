use bcrypt::{hash, verify, DEFAULT_COST};
use kb_error::{KbError, Result};
use rand::{distributions::Alphanumeric, Rng};

/// 密码服务 - 处理密码哈希、验证和生成
pub struct PasswordService;

impl PasswordService {
    /// 生成密码哈希
    pub fn hash_password(password: &str) -> Result<String> {
        // 验证密码强度
        Self::validate_password_strength(password)?;

        hash(password, DEFAULT_COST).map_err(|e| KbError::Internal {
            message: format!("Failed to hash password: {}", e),
            details: None,
        })
    }

    /// 验证密码
    pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
        verify(password, hash).map_err(|e| KbError::Internal {
            message: format!("Failed to verify password: {}", e),
            details: None,
        })
    }

    /// 生成随机密码
    pub fn generate_password(length: usize) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(length)
            .map(char::from)
            .collect()
    }

    /// 生成API密钥
    pub fn generate_api_key() -> String {
        let prefix = "kb_";
        let key: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        format!("{}{}", prefix, key)
    }

    /// 验证密码强度
    pub fn validate_password_strength(password: &str) -> Result<()> {
        if password.len() < 8 {
            return Err(KbError::Validation {
                message: "密码长度至少8位".to_string(),
            });
        }

        if password.len() > 128 {
            return Err(KbError::Validation {
                message: "密码长度不能超过128位".to_string(),
            });
        }

        let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
        let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = password.chars().any(|c| c.is_ascii_digit());
        let has_special = password
            .chars()
            .any(|c| "!@#$%^&*()_+-=[]{}|;:,.<>?".contains(c));

        let complexity_score = [has_lower, has_upper, has_digit, has_special]
            .iter()
            .map(|&b| if b { 1 } else { 0 })
            .sum::<i32>();

        if complexity_score < 3 {
            return Err(KbError::Validation {
                message: "密码必须包含大写字母、小写字母、数字和特殊字符中的至少3种".to_string(),
            });
        }

        // 检查常见弱密码
        let weak_passwords = [
            "password",
            "123456",
            "12345678",
            "qwerty",
            "abc123",
            "password123",
            "admin",
            "root",
            "user",
            "test",
        ];

        if weak_passwords
            .iter()
            .any(|&weak| password.to_lowercase().contains(weak))
        {
            return Err(KbError::Validation {
                message: "密码不能包含常见的弱密码模式".to_string(),
            });
        }

        Ok(())
    }

    /// 哈希API密钥用于存储
    pub fn hash_api_key(api_key: &str) -> Result<String> {
        hash(api_key, DEFAULT_COST).map_err(|e| KbError::Internal {
            message: format!("Failed to hash API key: {}", e),
            details: None,
        })
    }

    /// 验证API密钥
    pub fn verify_api_key(api_key: &str, hash: &str) -> Result<bool> {
        verify(api_key, hash).map_err(|e| KbError::Internal {
            message: format!("Failed to verify API key: {}", e),
            details: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing() {
        let password = "TestPassword123!";
        let hash = PasswordService::hash_password(password).unwrap();

        assert!(PasswordService::verify_password(password, &hash).unwrap());
        assert!(!PasswordService::verify_password("wrong_password", &hash).unwrap());
    }

    #[test]
    fn test_password_validation() {
        // 有效密码
        assert!(PasswordService::validate_password_strength("TestPass123!").is_ok());

        // 太短
        assert!(PasswordService::validate_password_strength("Test1!").is_err());

        // 缺少复杂度
        assert!(PasswordService::validate_password_strength("testpassword").is_err());

        // 弱密码
        assert!(PasswordService::validate_password_strength("password123").is_err());
    }

    #[test]
    fn test_api_key_generation() {
        let api_key = PasswordService::generate_api_key();
        assert!(api_key.starts_with("kb_"));
        assert_eq!(api_key.len(), 35); // "kb_" + 32 chars
    }
}
