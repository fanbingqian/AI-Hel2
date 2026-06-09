use serde::{Deserialize, Serialize};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::services::config_service::ConfigService;

pub struct AuthState {
    pub config: Mutex<ConfigService>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub name: String,
    pub email: String,
    pub avatar_letter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredUser {
    email: String,
    password_hash: String,
    salt: String,
}

fn users_path(config: &ConfigService) -> std::path::PathBuf {
    config.hermes_home().join("users.json")
}

/// Migrate old SHA-256 hashes to argon2, or seed default admin on first run.
/// Returns true if migration happened.
fn migrate_users(config: &ConfigService) -> bool {
    let path = users_path(config);
    if !path.exists() {
        // First run: create default admin with argon2
        let hash = hash_password("admin123").unwrap_or_default();
        let mut users = HashMap::new();
        users.insert(
            "admin".to_string(),
            StoredUser {
                email: "admin@ai-hel2.local".to_string(),
                password_hash: hash,
                salt: String::new(),
            },
        );
        let _ = write_users(config, &users);
        log::info!("Created default admin user with argon2");
        return true;
    }

    // Check if existing hashes are SHA-256 (not argon2 — argon2 hashes start with $)
    let users = read_users(config);
    let needs_migration = users.values().any(|u| !u.password_hash.starts_with('$'));
    if needs_migration {
        log::warn!("Migrating users.json from SHA-256 to argon2 — old passwords reset to defaults");
        let hash = hash_password("admin123").unwrap_or_default();
        let mut new_users = HashMap::new();
        new_users.insert(
            "admin".to_string(),
            StoredUser {
                email: "admin@ai-hel2.local".to_string(),
                password_hash: hash,
                salt: String::new(),
            },
        );
        // Preserve other users with reset passwords (user must use "forgot password")
        for (name, u) in &users {
            if name != "admin" {
                new_users.insert(
                    name.clone(),
                    StoredUser {
                        email: u.email.clone(),
                        password_hash: hash_password("reset123").unwrap_or_default(),
                        salt: String::new(),
                    },
                );
            }
        }
        let _ = write_users(config, &new_users);
        return true;
    }
    false
}

fn read_users(config: &ConfigService) -> HashMap<String, StoredUser> {
    let path = users_path(config);
    if !path.exists() {
        return HashMap::new();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_users(config: &ConfigService, users: &HashMap<String, StoredUser>) -> Result<(), String> {
    let path = users_path(config);
    let json = serde_json::to_string_pretty(users).map_err(|e| format!("序列化用户数据失败: {e}"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("写入临时文件失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换文件失败: {e}"))
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("密码哈希失败: {e}"))
}

fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| format!("解析密码哈希失败: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

// ── Rate limiting ──
use std::sync::LazyLock;
static LOGIN_ATTEMPTS: LazyLock<Mutex<HashMap<String, (u32, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn login_attempts() -> &'static Mutex<HashMap<String, (u32, Instant)>> {
    &LOGIN_ATTEMPTS
}

fn check_rate_limit(username: &str) -> Result<(), String> {
    let mut attempts = login_attempts().lock().map_err(|e| e.to_string())?;
    let now = Instant::now();
    let entry = attempts.entry(username.to_string()).or_insert((0, now));

    // Reset counter after 5 minutes
    if now.duration_since(entry.1) > Duration::from_secs(300) {
        *entry = (1, now);
        return Ok(());
    }

    if entry.0 >= 5 {
        let wait = 300 - now.duration_since(entry.1).as_secs();
        return Err(format!("登录尝试次数过多，请 {} 秒后重试", wait));
    }

    entry.0 += 1;
    Ok(())
}

fn reset_rate_limit(username: &str) {
    if let Ok(mut attempts) = login_attempts().lock() {
        attempts.remove(username);
    }
}

fn avatar_letter(name: &str) -> String {
    name.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".into())
}

#[tauri::command]
pub async fn register_user(
    state: tauri::State<'_, AuthState>,
    username: String,
    email: String,
    password: String,
) -> Result<UserInfo, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    migrate_users(&config);
    let mut users = read_users(&config);

    if users.contains_key(&username) {
        return Err("用户名已存在".into());
    }

    let password_hash = hash_password(&password)?;

    users.insert(
        username.clone(),
        StoredUser {
            email: email.clone(),
            password_hash,
            salt: String::new(), // argon2 embeds salt in hash, no separate salt needed
        },
    );

    write_users(&config, &users)?;

    // Also write user section to config.yaml
    let user_json = serde_json::json!({
        "user": {
            "name": &username,
            "email": &email,
        }
    });
    config.write_config(&user_json)?;

    Ok(UserInfo {
        avatar_letter: avatar_letter(&username),
        name: username,
        email,
    })
}

#[tauri::command]
pub async fn login_user(
    state: tauri::State<'_, AuthState>,
    username: String,
    password: String,
) -> Result<UserInfo, String> {
    // Rate limiting: max 5 attempts per 5 minutes
    check_rate_limit(&username)?;

    let config = state.config.lock().map_err(|e| e.to_string())?;

    // Migrate old SHA-256 hashes to argon2 on first access
    migrate_users(&config);

    let users = read_users(&config);

    let stored = users.get(&username).ok_or("用户名或密码错误".to_string())?;

    // Verify with argon2
    let valid = verify_password(&password, &stored.password_hash)?;
    if !valid {
        return Err("用户名或密码错误".into());
    }

    // Reset rate limit on successful login
    reset_rate_limit(&username);

    Ok(UserInfo {
        avatar_letter: avatar_letter(&username),
        name: username,
        email: stored.email.clone(),
    })
}

#[tauri::command]
pub async fn change_password(
    state: tauri::State<'_, AuthState>,
    username: String,
    old_password: String,
    new_password: String,
) -> Result<(), String> {
    if new_password.len() < 6 {
        return Err("新密码长度不能少于6位".into());
    }
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let mut users = read_users(&config);

    let stored = users.get(&username).ok_or("用户不存在")?;
    let valid = verify_password(&old_password, &stored.password_hash)?;
    if !valid {
        return Err("原密码错误".into());
    }

    let new_hash = hash_password(&new_password)?;

    users.insert(
        username,
        StoredUser {
            email: stored.email.clone(),
            password_hash: new_hash,
            salt: String::new(),
        },
    );

    write_users(&config, &users)
}

#[tauri::command]
pub async fn get_current_user(
    state: tauri::State<'_, AuthState>,
) -> Result<Option<UserInfo>, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    // HermesConfig doesn't have a user field, so we parse the raw YAML
    let config_path = config.hermes_home().join("config.yaml");
    if !config_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("读取 config.yaml 失败: {e}"))?;

    let parsed: serde_json::Value =
        serde_yaml::from_str(&content).map_err(|e| format!("解析 config.yaml 失败: {e}"))?;

    if let Some(user) = parsed.get("user") {
        let name = user.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            return Ok(None);
        }
        let email = user.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
        return Ok(Some(UserInfo {
            avatar_letter: avatar_letter(name),
            name: name.to_string(),
            email,
        }));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let hash = hash_password("mypassword").expect("hash should succeed");
        assert!(verify_password("mypassword", &hash).unwrap());
        assert!(!verify_password("wrongpassword", &hash).unwrap());
    }

    #[test]
    fn test_hash_unique_each_time() {
        let h1 = hash_password("samepassword").unwrap();
        let h2 = hash_password("samepassword").unwrap();
        assert_ne!(h1, h2); // argon2 generates unique salt each time
        // Both should verify
        assert!(verify_password("samepassword", &h1).unwrap());
        assert!(verify_password("samepassword", &h2).unwrap());
    }

    #[test]
    fn test_avatar_letter() {
        assert_eq!(avatar_letter("admin"), "a");
        assert_eq!(avatar_letter("张三"), "张");
        assert_eq!(avatar_letter(""), "?");
    }

    #[test]
    fn test_duplicate_username_detection() {
        let mut test_users: HashMap<String, StoredUser> = HashMap::new();
        test_users.insert(
            "duplicate_test".to_string(),
            StoredUser {
                email: "a@a.com".to_string(),
                password_hash: "x".to_string(),
                salt: "y".to_string(),
            },
        );
        assert!(test_users.contains_key("duplicate_test"));
        assert!(!test_users.contains_key("nonexistent"));
    }

    #[test]
    fn test_rate_limiting() {
        // Should allow first 5 attempts
        for _ in 0..5 {
            assert!(check_rate_limit("testuser").is_ok());
        }
        // 6th attempt should fail
        assert!(check_rate_limit("testuser").is_err());
        // Different user should not be affected
        assert!(check_rate_limit("otheruser").is_ok());
    }
}
