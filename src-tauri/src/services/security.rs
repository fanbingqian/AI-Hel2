use std::path::Path;

/// URL 白名单验证 — 用户聊天中点击的链接
pub fn is_allowed_external_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    if lower.starts_with("https:") || lower.starts_with("http:") || lower.starts_with("mailto:") {
        return !is_blocked_host(url);
    }
    false
}

/// 应用内部导航 URL 验证
pub fn is_allowed_app_navigation_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    if lower.starts_with("https:") || lower.starts_with("http:") || lower.starts_with("file:") {
        if lower.starts_with("file:") {
            return is_safe_file_path(url.strip_prefix("file://").unwrap_or(url));
        }
        return !is_blocked_host(url);
    }
    false
}

/// WebView 加载 URL 验证 — 仅允许 localhost 特定端口
pub fn is_allowed_webview_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    if !lower.starts_with("http://") {
        // 仅允许 HTTP (localhost 不需要 HTTPS)
        return false;
    }

    let host_port = lower.strip_prefix("http://").unwrap_or("");
    // 仅允许 127.0.0.1 或 localhost
    if !host_port.starts_with("127.0.0.1") && !host_port.starts_with("localhost") {
        return false;
    }

    // 检查端口范围 1024-65535
    if let Some(port_str) = host_port.split(':').nth(1) {
        if let Some(port) = port_str.split('/').next().and_then(|p| p.parse::<u16>().ok()) {
            return port >= 1024;
        }
    }
    // 默认端口 80 不允许 (必须显式指定端口)
    false
}

/// 路径安全 — 日志读取仅允许已知文件名
pub fn is_allowed_log_file(filename: &str) -> bool {
    let allowed = ["agent.log", "errors.log", "gateway.log"];
    allowed.contains(&filename)
}

/// 路径安全 — 文件写入限制在 ~/.ai-hel2/ 目录树内
pub fn is_safe_file_path(path: &str) -> bool {
    let hermes_home = dirs_ai_hel2_home();
    let canonical_base = std::fs::canonicalize(&hermes_home).unwrap_or(hermes_home.clone());

    let resolved = resolve_path(path, &hermes_home);
    match std::fs::canonicalize(&resolved) {
        Ok(canon) => canon.starts_with(&canonical_base),
        // 如果文件不存在，检查父目录
        Err(_) => {
            if let Some(parent) = resolved.parent() {
                std::fs::canonicalize(parent)
                    .map(|p| p.starts_with(&canonical_base))
                    .unwrap_or(false)
            } else {
                false
            }
        }
    }
}

/// 路径遍历防护 — 拒绝包含 ../ 或 ..\ 的路径
pub fn has_path_traversal(path: &str) -> bool {
    path.contains("..")
}

fn is_blocked_host(url: &str) -> bool {
    // 内部网络地址拒绝
    let blocked = [
        "0.0.0.0", "0.0.0.0:",
        "127.0.0.1:", "localhost:",
        "[::1]", "[::]",
    ];
    let lower = url.to_lowercase();
    let stripped = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);

    blocked.iter().any(|b| stripped.starts_with(b))
}

fn resolve_path(path_str: &str, base: &Path) -> std::path::PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn dirs_ai_hel2_home() -> std::path::PathBuf {
    std::env::var("AI_HEL2_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            #[cfg(target_os = "windows")]
            {
                std::env::var("USERPROFILE")
                    .map(|p| std::path::PathBuf::from(p).join(".ai-hel2"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("HOME")
                    .map(|p| std::path::PathBuf::from(p).join(".ai-hel2"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_external_urls() {
        assert!(is_allowed_external_url("https://example.com"));
        assert!(is_allowed_external_url("http://example.com"));
        assert!(is_allowed_external_url("mailto:test@test.com"));
        assert!(!is_allowed_external_url("ftp://example.com"));
        assert!(!is_allowed_external_url("javascript:alert(1)"));
    }

    #[test]
    fn test_blocked_hosts() {
        assert!(!is_allowed_external_url("http://127.0.0.1:8000/path"));
        assert!(!is_allowed_external_url("http://localhost:3000/path"));
        assert!(!is_allowed_external_url("https://0.0.0.0:8080"));
    }

    #[test]
    fn test_webview_url() {
        assert!(is_allowed_webview_url("http://127.0.0.1:8642/"));
        assert!(is_allowed_webview_url("http://localhost:3000/app"));
        assert!(!is_allowed_webview_url("http://127.0.0.1:80/"));
        assert!(!is_allowed_webview_url("http://example.com:8642/"));
        assert!(!is_allowed_webview_url("https://127.0.0.1:8642/"));
    }

    #[test]
    fn test_path_traversal() {
        assert!(has_path_traversal("../etc/passwd"));
        assert!(!has_path_traversal("agent.log"));
    }

    #[test]
    fn test_log_file_names() {
        assert!(is_allowed_log_file("agent.log"));
        assert!(is_allowed_log_file("errors.log"));
        assert!(!is_allowed_log_file("../../../etc/passwd"));
    }
}
