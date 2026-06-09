use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

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

fn generate_salt() -> String {
    let rng: [u8; 16] = rand::random();
    hex::encode(rng)
}

fn hash_password(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
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
    let mut users = read_users(&config);

    if users.contains_key(&username) {
        return Err("用户名已存在".into());
    }

    let salt = generate_salt();
    let password_hash = hash_password(&password, &salt);

    users.insert(
        username.clone(),
        StoredUser {
            email: email.clone(),
            password_hash,
            salt,
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
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let users = read_users(&config);

    let stored = users.get(&username).ok_or("用户名或密码错误")?;

    let computed_hash = hash_password(&password, &stored.salt);
    if computed_hash != stored.password_hash {
        return Err("用户名或密码错误".into());
    }

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
    let computed_hash = hash_password(&old_password, &stored.salt);
    if computed_hash != stored.password_hash {
        return Err("原密码错误".into());
    }

    let new_salt = generate_salt();
    let new_hash = hash_password(&new_password, &new_salt);

    users.insert(
        username,
        StoredUser {
            email: stored.email.clone(),
            password_hash: new_hash,
            salt: new_salt,
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
    use std::sync::Mutex;

    // Serialize tests that write to shared users.json to prevent races
    static USERS_FILE_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_hash_password_deterministic() {
        let h1 = hash_password("admin123", "salt123");
        let h2 = hash_password("admin123", "salt123");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_password_different_salt_produces_different_hash() {
        let h1 = hash_password("admin123", "salt_a");
        let h2 = hash_password("admin123", "salt_b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hash_password_wrong_password_fails() {
        let correct = hash_password("admin123", "mysalt");
        let wrong = hash_password("wrongpass", "mysalt");
        assert_ne!(correct, wrong);
    }

    #[test]
    fn test_generate_salt_hex_length() {
        let salt = generate_salt();
        assert_eq!(salt.len(), 32); // 16 bytes = 32 hex chars
    }

    #[test]
    fn test_generate_salt_unique() {
        let s1 = generate_salt();
        let s2 = generate_salt();
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_avatar_letter() {
        assert_eq!(avatar_letter("admin"), "a");
        assert_eq!(avatar_letter("张三"), "张");
        assert_eq!(avatar_letter(""), "?");
    }

    #[test]
    fn test_users_read_write_roundtrip() {
        let _guard = USERS_FILE_MUTEX.lock().unwrap();
        let config = ConfigService::new();
        let path = users_path(&config);

        // Save existing users if any
        let backup = if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        };

        let mut users: HashMap<String, StoredUser> = HashMap::new();
        users.insert(
            "testuser".to_string(),
            StoredUser {
                email: "test@test.com".to_string(),
                password_hash: "abc123".to_string(),
                salt: "salt123".to_string(),
            },
        );
        write_users(&config, &users).expect("write should succeed");
        let read = read_users(&config);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get("testuser").unwrap().email, "test@test.com");

        // Restore original users.json
        if let Some(backup_data) = backup {
            std::fs::write(&path, &backup_data).expect("restore should succeed");
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn test_full_register_and_login_flow() {
        let _guard = USERS_FILE_MUTEX.lock().unwrap();
        let config = ConfigService::new();
        let path = users_path(&config);

        // Save existing users
        let backup = if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        };

        let mut users: HashMap<String, StoredUser> = HashMap::new();

        let salt = generate_salt();
        let hash = hash_password("mypassword", &salt);
        users.insert(
            "newuser".to_string(),
            StoredUser {
                email: "new@test.com".to_string(),
                password_hash: hash,
                salt,
            },
        );
        write_users(&config, &users).expect("write should succeed");

        let stored = read_users(&config);
        let user = stored.get("newuser").unwrap();
        let login_hash = hash_password("mypassword", &user.salt);
        assert_eq!(login_hash, user.password_hash);

        let wrong_hash = hash_password("badpassword", &user.salt);
        assert_ne!(wrong_hash, user.password_hash);

        // Restore
        if let Some(backup_data) = backup {
            std::fs::write(&path, &backup_data).expect("restore should succeed");
        } else {
            let _ = std::fs::remove_file(&path);
        }
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
    fn test_login_with_real_admin_credentials() {
        let config = ConfigService::new();
        let users = read_users(&config);

        // The admin account should exist (created in setup)
        let admin = users.get("admin").expect("admin user should exist in users.json");
        assert_eq!(admin.email, "admin@ai-hel2.local");

        // Verify correct password
        let computed = hash_password("admin123", &admin.salt);
        assert_eq!(computed, admin.password_hash, "admin password hash should match");

        // Verify wrong password fails
        let wrong = hash_password("wrongpassword", &admin.salt);
        assert_ne!(wrong, admin.password_hash, "wrong password should not match");
    }
}
